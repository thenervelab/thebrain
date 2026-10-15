//! Storage migrations for `pallet-compute-scoring`.

/// v0 → v1: operator stake moves from the `cmps/stk` named reserve to the
/// [`HoldReason::ComputeStake`] fungible hold.
pub mod v1 {
    use crate::pallet::{
        Config, HoldReason, Pallet, SlashBeneficiary, StakeUnbonding, StakedAmount,
        STAKE_RESERVE_ID,
    };
    use alloc::collections::BTreeSet;
    use core::marker::PhantomData;
    use frame_support::{
        pallet_prelude::*,
        traits::{fungible::MutateHold, NamedReservableCurrency, OnRuntimeUpgrade},
    };
    use sp_runtime::traits::{Saturating, Zero};

    const LOG_TARGET: &str = "runtime::compute-scoring::migration";

    /// Most ledger accounts one upgrade converts. The migration is
    /// single-block, so its read/write count must stay bounded. On mainnet
    /// the stake ledger is EMPTY at the time of this upgrade (stake was
    /// never enabled), so the bound is a guard, not a budget: if it is ever
    /// hit, the version is NOT bumped and the next upgrade resumes the
    /// remainder (the per-account conversion is idempotent).
    pub const MAX_ACCOUNTS_PER_UPGRADE: u32 = 1024;

    /// Converts every `cmps/stk` named reserve into a
    /// [`HoldReason::ComputeStake`] hold, and clears the deprecated
    /// [`SlashBeneficiary`].
    ///
    /// Idempotent twice over: guarded on the on-chain storage version, and
    /// an account whose stake reserve is already zero is a no-op.
    pub struct MigrateStakeReserveToHold<T>(PhantomData<T>);

    /// Every account the stake ledger knows. `top_up_stake` is the only
    /// writer of the stake reserve and always writes [`StakedAmount`], and
    /// [`StakeUnbonding`] covers an owner whose active stake was fully
    /// unbonded — so no stake reserve exists outside this set.
    fn ledger_owners<T: Config>(limit: usize) -> BTreeSet<T::AccountId> {
        StakedAmount::<T>::iter_keys()
            .chain(StakeUnbonding::<T>::iter_keys())
            .take(limit)
            .collect()
    }

    impl<T: Config> OnRuntimeUpgrade for MigrateStakeReserveToHold<T> {
        fn on_runtime_upgrade() -> Weight {
            if Pallet::<T>::on_chain_storage_version() >= 1 {
                log::info!(target: LOG_TARGET, "stake already on holds (v1); skipping");
                return T::DbWeight::get().reads(1);
            }

            let limit = MAX_ACCOUNTS_PER_UPGRADE as usize;
            // One key over the limit tells "exactly at the limit" apart from
            // "more left".
            let owners = ledger_owners::<T>(limit.saturating_add(1));
            // Version read + one read per key iterated (at most limit + 1).
            let mut reads: u64 = 1u64.saturating_add(owners.len() as u64);
            let mut writes: u64 = 0;

            let truncated = owners.len() > limit;
            let mut converted: u32 = 0;
            let mut total = BalanceOf::<T>::zero();
            let mut failed: u32 = 0;

            for who in owners.iter().take(limit) {
                let reserved = T::DepositCurrency::reserved_balance_named(&STAKE_RESERVE_ID, who);
                reads = reads.saturating_add(1);
                if reserved.is_zero() {
                    continue;
                }
                let not_unreserved =
                    T::DepositCurrency::unreserve_named(&STAKE_RESERVE_ID, who, reserved);
                let moved = reserved.saturating_sub(not_unreserved);
                // unreserve + hold: account + reserves + holds.
                writes = writes.saturating_add(3);
                match T::StakeCurrency::hold(&HoldReason::ComputeStake.into(), who, moved) {
                    Ok(()) => {
                        converted = converted.saturating_add(1);
                        total = total.saturating_add(moved);
                    }
                    Err(e) => {
                        // Never leave stake FREE: put it back on the named
                        // reserve it came from (just released, so this
                        // cannot fail for lack of funds) and say so loudly.
                        let restored =
                            T::DepositCurrency::reserve_named(&STAKE_RESERVE_ID, who, moved);
                        failed = failed.saturating_add(1);
                        log::error!(
                            target: LOG_TARGET,
                            "could not hold {:?} stake for {:?}: {:?}; re-reserved: {:?}",
                            moved,
                            who,
                            e,
                            restored
                        );
                    }
                }
            }

            if SlashBeneficiary::<T>::exists() {
                SlashBeneficiary::<T>::kill();
                writes = writes.saturating_add(1);
                log::info!(target: LOG_TARGET, "cleared deprecated SlashBeneficiary");
            }
            reads = reads.saturating_add(1);

            if owners.is_empty() {
                log::info!(target: LOG_TARGET, "stake ledger empty: nothing to migrate");
            } else {
                log::info!(
                    target: LOG_TARGET,
                    "converted {} stake reserve(s) totalling {:?} to holds ({} failed)",
                    converted,
                    total,
                    failed
                );
            }

            if truncated {
                // Do not stamp v1: the next upgrade resumes the rest.
                log::error!(
                    target: LOG_TARGET,
                    "more than {} stake ledger accounts; version NOT bumped, re-run next upgrade",
                    limit
                );
            } else {
                StorageVersion::new(1).put::<Pallet<T>>();
                writes = writes.saturating_add(1);
            }

            T::DbWeight::get().reads_writes(reads, writes)
        }

        #[cfg(feature = "try-runtime")]
        fn pre_upgrade() -> Result<alloc::vec::Vec<u8>, sp_runtime::TryRuntimeError> {
            use codec::Encode;
            let limit = MAX_ACCOUNTS_PER_UPGRADE as usize;
            let owners = ledger_owners::<T>(limit.saturating_add(1));
            frame_support::ensure!(
                owners.len() <= limit,
                "stake ledger exceeds MAX_ACCOUNTS_PER_UPGRADE"
            );
            Ok((owners.len() as u32).encode())
        }

        #[cfg(feature = "try-runtime")]
        fn post_upgrade(_state: alloc::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
            use frame_support::traits::fungible::InspectHold;
            frame_support::ensure!(
                Pallet::<T>::on_chain_storage_version() == 1,
                "compute-scoring storage version not bumped to 1"
            );
            frame_support::ensure!(
                !SlashBeneficiary::<T>::exists(),
                "deprecated SlashBeneficiary still set"
            );
            for who in ledger_owners::<T>(MAX_ACCOUNTS_PER_UPGRADE as usize) {
                frame_support::ensure!(
                    T::DepositCurrency::reserved_balance_named(&STAKE_RESERVE_ID, &who).is_zero(),
                    "a cmps/stk named reserve survived the migration"
                );
                let logical = StakedAmount::<T>::get(&who).saturating_add(
                    StakeUnbonding::<T>::get(&who)
                        .map(|(a, _)| a)
                        .unwrap_or_else(Zero::zero),
                );
                frame_support::ensure!(
                    T::StakeCurrency::balance_on_hold(&HoldReason::ComputeStake.into(), &who)
                        >= logical,
                    "ComputeStake hold is short of the stake ledger"
                );
            }
            Ok(())
        }
    }

    type BalanceOf<T> = <<T as Config>::DepositCurrency as frame_support::traits::Currency<
        <T as frame_system::Config>::AccountId,
    >>::Balance;
}

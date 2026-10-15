//! Storage migrations for `pallet-compute-scoring`.

/// v0 → v1: operator stake moves from the `cmps/stk` named reserve to the
/// [`HoldReason::ComputeStake`] fungible hold.
///
/// Two halves, so the upgrade block stays bounded whatever the pre-upgrade
/// ledger holds (`top_up_stake` is callable while `StakeEnabled` is off, so
/// the ledger is not provably empty at enactment):
///
/// - [`MigrateStakeReserveToHold`] converts at most
///   [`MAX_ACCOUNTS_PER_UPGRADE`] ledger accounts eagerly and stamps v1.
/// - [`convert_legacy_stake`] is also called lazily by every call that
///   moves stake balance (`top_up_stake`, `claim_unstaked`, `do_slash`), so
///   an account the eager pass did not reach is converted the first time it
///   is touched. Until then its stake stays on the named reserve: locked,
///   never free.
///
/// On mainnet at spec 92018 the ledger is empty, so the eager pass is a
/// version bump.
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

    /// Most ledger accounts the eager pass converts in the upgrade block.
    /// The rest are converted lazily (see the module docs).
    pub const MAX_ACCOUNTS_PER_UPGRADE: u32 = 512;

    /// Storage accesses of one conversion that moves funds: `Reserves`,
    /// `Account` and `Holds`, each read and written (the ledger reads are
    /// charged by the caller).
    pub const CONVERT_READS_WRITES: (u64, u64) = (3, 3);

    type BalanceOf<T> = <<T as Config>::DepositCurrency as frame_support::traits::Currency<
        <T as frame_system::Config>::AccountId,
    >>::Balance;

    /// Outcome of converting one account.
    pub enum Converted<B> {
        /// No `cmps/stk` reserve: nothing to do (already on the hold, or
        /// never staked).
        Nothing,
        /// The whole reserve moved onto the hold.
        Moved(B),
        /// The conversion was rolled back; the stake is still on the reserve.
        Failed,
    }

    /// The stake the ledger says `who` has: active + unbonding.
    fn logical_stake<T: Config>(who: &T::AccountId) -> BalanceOf<T> {
        StakedAmount::<T>::get(who).saturating_add(
            StakeUnbonding::<T>::get(who)
                .map(|(a, _)| a)
                .unwrap_or_else(Zero::zero),
        )
    }

    /// Move `who`'s whole `cmps/stk` reserve onto the
    /// [`HoldReason::ComputeStake`] hold, atomically: on any shortfall or
    /// hold failure the storage layer rolls the account back to its prior
    /// state, so stake is never left free. Idempotent — a zero reserve is
    /// [`Converted::Nothing`].
    pub fn convert_legacy_stake<T: Config>(who: &T::AccountId) -> Converted<BalanceOf<T>> {
        let reserved = T::DepositCurrency::reserved_balance_named(&STAKE_RESERVE_ID, who);
        if reserved.is_zero() {
            return Converted::Nothing;
        }
        let logical = logical_stake::<T>(who);
        if reserved != logical {
            // Not fatal: the reserve is the owner's locked money and moves
            // as-is, exactly as locked as before. Logged for reconciliation.
            log::warn!(
                target: LOG_TARGET,
                "{:?}: stake reserve {:?} != ledger {:?}; moving the reserve as-is",
                who,
                reserved,
                logical
            );
        }
        let result = frame_support::storage::with_storage_layer(|| -> DispatchResult {
            let not_unreserved =
                T::DepositCurrency::unreserve_named(&STAKE_RESERVE_ID, who, reserved);
            ensure!(not_unreserved.is_zero(), DispatchError::Other("partial unreserve"));
            T::StakeCurrency::hold(&HoldReason::ComputeStake.into(), who, reserved)
        });
        match result {
            Ok(()) => Converted::Moved(reserved),
            Err(e) => {
                log::error!(
                    target: LOG_TARGET,
                    "{:?}: could not move {:?} stake to the hold: {:?}; left on the reserve",
                    who,
                    reserved,
                    e
                );
                Converted::Failed
            }
        }
    }

    /// Eager half of v0 → v1: converts up to [`MAX_ACCOUNTS_PER_UPGRADE`]
    /// `cmps/stk` reserves into holds, clears the deprecated
    /// [`SlashBeneficiary`], and stamps v1. Guarded on the storage version,
    /// so it runs once.
    pub struct MigrateStakeReserveToHold<T>(PhantomData<T>);

    impl<T: Config> OnRuntimeUpgrade for MigrateStakeReserveToHold<T> {
        fn on_runtime_upgrade() -> Weight {
            if Pallet::<T>::on_chain_storage_version() >= 1 {
                log::info!(target: LOG_TARGET, "stake already on holds (v1); skipping");
                return T::DbWeight::get().reads(1);
            }

            let limit = MAX_ACCOUNTS_PER_UPGRADE as usize;
            let (convert_reads, convert_writes) = CONVERT_READS_WRITES;
            // Version read.
            let mut reads: u64 = 1;
            let mut writes: u64 = 0;
            let mut seen: BTreeSet<T::AccountId> = BTreeSet::new();
            let mut converted: u32 = 0;
            let mut failed: u32 = 0;
            let mut total = BalanceOf::<T>::zero();
            let mut truncated = false;

            // Lazy iteration: at most `limit` distinct owners are ever
            // touched, plus the one key that proves there are more.
            for who in StakedAmount::<T>::iter_keys().chain(StakeUnbonding::<T>::iter_keys()) {
                reads = reads.saturating_add(1);
                if seen.contains(&who) {
                    continue;
                }
                if seen.len() >= limit {
                    truncated = true;
                    break;
                }
                // `Reserves` + the two ledger maps.
                reads = reads.saturating_add(3);
                match convert_legacy_stake::<T>(&who) {
                    Converted::Nothing => {}
                    Converted::Moved(amount) => {
                        converted = converted.saturating_add(1);
                        total = total.saturating_add(amount);
                    }
                    Converted::Failed => failed = failed.saturating_add(1),
                }
                reads = reads.saturating_add(convert_reads);
                writes = writes.saturating_add(convert_writes);
                seen.insert(who);
            }

            reads = reads.saturating_add(1);
            if SlashBeneficiary::<T>::exists() {
                SlashBeneficiary::<T>::kill();
                writes = writes.saturating_add(1);
                log::info!(target: LOG_TARGET, "cleared deprecated SlashBeneficiary");
            }

            if seen.is_empty() {
                log::info!(target: LOG_TARGET, "stake ledger empty: nothing to migrate");
            } else {
                log::info!(
                    target: LOG_TARGET,
                    "{} ledger account(s) visited: {} stake reserve(s) totalling {:?} moved to holds, {} failed",
                    seen.len(),
                    converted,
                    total,
                    failed
                );
            }
            if truncated || failed > 0 {
                log::warn!(
                    target: LOG_TARGET,
                    "accounts left on the legacy reserve (truncated: {}, failed: {}); they convert on their next stake call or slash",
                    truncated,
                    failed
                );
            }

            StorageVersion::new(1).put::<Pallet<T>>();
            writes = writes.saturating_add(1);

            T::DbWeight::get().reads_writes(reads, writes)
        }

        #[cfg(feature = "try-runtime")]
        fn pre_upgrade() -> Result<alloc::vec::Vec<u8>, sp_runtime::TryRuntimeError> {
            use codec::Encode;
            use frame_support::traits::fungible::InspectHold;
            // Per owner: stake reserve + stake hold, which the migration may
            // only move between, never change.
            let owners: BTreeSet<T::AccountId> = StakedAmount::<T>::iter_keys()
                .chain(StakeUnbonding::<T>::iter_keys())
                .collect();
            let snapshot: alloc::vec::Vec<(T::AccountId, BalanceOf<T>)> = owners
                .into_iter()
                .map(|who| {
                    let held = T::DepositCurrency::reserved_balance_named(&STAKE_RESERVE_ID, &who)
                        .saturating_add(T::StakeCurrency::balance_on_hold(
                            &HoldReason::ComputeStake.into(),
                            &who,
                        ));
                    (who, held)
                })
                .collect();
            Ok(snapshot.encode())
        }

        #[cfg(feature = "try-runtime")]
        fn post_upgrade(state: alloc::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
            use codec::Decode;
            use frame_support::traits::fungible::InspectHold;
            let snapshot: alloc::vec::Vec<(T::AccountId, BalanceOf<T>)> =
                Decode::decode(&mut &state[..]).map_err(|_| "undecodable pre_upgrade state")?;
            frame_support::ensure!(
                Pallet::<T>::on_chain_storage_version() == 1,
                "compute-scoring storage version not bumped to 1"
            );
            frame_support::ensure!(
                !SlashBeneficiary::<T>::exists(),
                "deprecated SlashBeneficiary still set"
            );
            for (who, held_before) in snapshot {
                let held_after = T::DepositCurrency::reserved_balance_named(&STAKE_RESERVE_ID, &who)
                    .saturating_add(T::StakeCurrency::balance_on_hold(
                        &HoldReason::ComputeStake.into(),
                        &who,
                    ));
                frame_support::ensure!(
                    held_after == held_before,
                    "stake reserve + hold changed across the migration"
                );
            }
            Ok(())
        }
    }
}

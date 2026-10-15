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

    /// Converts every `cmps/stk` named reserve into a
    /// [`HoldReason::ComputeStake`] hold, and clears the deprecated
    /// [`SlashBeneficiary`].
    ///
    /// Every ledger account is converted in this one upgrade — there is no
    /// page cap, because a cap could leave stake on a reserve the new code
    /// never reads. The cost is linear in the ledger size and fully charged;
    /// on mainnet the ledger is EMPTY at spec 92018 (stake was never
    /// enabled), which the deploy checklist re-reads before the upgrade.
    ///
    /// The version is stamped only if EVERY account converted. An account
    /// whose hold fails keeps its stake on the named reserve (never free),
    /// is logged, and is retried by the next upgrade — the per-account step
    /// is idempotent (a zero stake reserve is a no-op). Until then that
    /// account's stake is merely unclaimable and unslashable, not lost.
    pub struct MigrateStakeReserveToHold<T>(PhantomData<T>);

    /// Every account the stake ledger knows, and the number of storage keys
    /// read to find them. `top_up_stake` is the only writer of the stake
    /// reserve and always writes [`StakedAmount`], and [`StakeUnbonding`]
    /// covers an owner whose active stake was fully unbonded — so no stake
    /// reserve exists outside this set.
    fn ledger_owners<T: Config>() -> (BTreeSet<T::AccountId>, u64) {
        let mut keys: u64 = 0;
        let owners = StakedAmount::<T>::iter_keys()
            .chain(StakeUnbonding::<T>::iter_keys())
            .inspect(|_| keys = keys.saturating_add(1))
            .collect();
        (owners, keys)
    }

    /// The stake the ledger says `who` has on hold: active + unbonding.
    fn logical_stake<T: Config>(who: &T::AccountId) -> BalanceOf<T> {
        StakedAmount::<T>::get(who).saturating_add(
            StakeUnbonding::<T>::get(who)
                .map(|(a, _)| a)
                .unwrap_or_else(Zero::zero),
        )
    }

    /// Outcome of converting one account.
    enum Converted<B> {
        Nothing,
        Moved(B),
        Failed,
    }

    /// Move `who`'s whole `cmps/stk` reserve onto the hold, atomically: on
    /// any shortfall or hold failure the storage layer rolls the account back
    /// to its pre-migration state, so stake is never left free.
    fn convert<T: Config>(who: &T::AccountId) -> Converted<BalanceOf<T>> {
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

    impl<T: Config> OnRuntimeUpgrade for MigrateStakeReserveToHold<T> {
        fn on_runtime_upgrade() -> Weight {
            if Pallet::<T>::on_chain_storage_version() >= 1 {
                log::info!(target: LOG_TARGET, "stake already on holds (v1); skipping");
                return T::DbWeight::get().reads(1);
            }

            let (owners, keys) = ledger_owners::<T>();
            // Version read + the two key scans.
            let mut reads: u64 = 1u64.saturating_add(keys);
            let mut writes: u64 = 0;
            let mut converted: u32 = 0;
            let mut failed: u32 = 0;
            let mut total = BalanceOf::<T>::zero();

            for who in owners.iter() {
                // Reserves + ledger (2 maps) read; then, on a move, Reserves,
                // Account and Holds read and written.
                reads = reads.saturating_add(3);
                match convert::<T>(who) {
                    Converted::Nothing => {}
                    Converted::Moved(amount) => {
                        reads = reads.saturating_add(3);
                        writes = writes.saturating_add(3);
                        converted = converted.saturating_add(1);
                        total = total.saturating_add(amount);
                    }
                    Converted::Failed => {
                        reads = reads.saturating_add(3);
                        failed = failed.saturating_add(1);
                    }
                }
            }

            reads = reads.saturating_add(1);
            if SlashBeneficiary::<T>::exists() {
                SlashBeneficiary::<T>::kill();
                writes = writes.saturating_add(1);
                log::info!(target: LOG_TARGET, "cleared deprecated SlashBeneficiary");
            }

            if owners.is_empty() {
                log::info!(target: LOG_TARGET, "stake ledger empty: nothing to migrate");
            } else {
                log::info!(
                    target: LOG_TARGET,
                    "{} ledger account(s): {} stake reserve(s) totalling {:?} moved to holds, {} failed",
                    owners.len(),
                    converted,
                    total,
                    failed
                );
            }

            if failed == 0 {
                StorageVersion::new(1).put::<Pallet<T>>();
                writes = writes.saturating_add(1);
            } else {
                log::error!(
                    target: LOG_TARGET,
                    "{} account(s) not converted; version left at 0 so the next upgrade retries",
                    failed
                );
            }

            T::DbWeight::get().reads_writes(reads, writes)
        }

        #[cfg(feature = "try-runtime")]
        fn pre_upgrade() -> Result<alloc::vec::Vec<u8>, sp_runtime::TryRuntimeError> {
            use codec::Encode;
            // Per owner: the stake reserve, which must end up on the hold.
            let snapshot: alloc::vec::Vec<(T::AccountId, BalanceOf<T>)> = ledger_owners::<T>()
                .0
                .into_iter()
                .map(|who| {
                    let r = T::DepositCurrency::reserved_balance_named(&STAKE_RESERVE_ID, &who);
                    (who, r)
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
            for (who, reserved_before) in snapshot {
                frame_support::ensure!(
                    T::DepositCurrency::reserved_balance_named(&STAKE_RESERVE_ID, &who).is_zero(),
                    "a cmps/stk named reserve survived the migration"
                );
                frame_support::ensure!(
                    T::StakeCurrency::balance_on_hold(&HoldReason::ComputeStake.into(), &who)
                        == reserved_before,
                    "ComputeStake hold differs from the pre-upgrade stake reserve"
                );
            }
            Ok(())
        }
    }

    type BalanceOf<T> = <<T as Config>::DepositCurrency as frame_support::traits::Currency<
        <T as frame_system::Config>::AccountId,
    >>::Balance;
}

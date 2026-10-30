//! Benchmarks for `pallet-compute-scoring` (thebrain#49 R25).
//!
//! `weights.rs` shipped as a manual estimation with a header telling
//! operators to "run actual benchmarks in your runtime before
//! mainnet" — and there was nothing to run. This file is the thing to
//! run:
//!
//!     frame-omni-bencher v1 benchmark pallet \
//!         --runtime <wasm> --pallet pallet_compute_scoring --extrinsic '*'
//!
//! Coverage picks the calls whose cost is either USER-REACHABLE
//! (registration, stake, pricing — `Pays::No` makes mispricing free
//! for the caller and expensive for the chain) or PARAMETRIC
//! (`vali_submit_epoch_close` scales with the batch). Simple admin
//! setters share the same one-write shape; one representative
//! (`set_stake_floor`) covers the class.
//!
//! `impl_benchmark_test_suite!` runs every benchmark against the mock
//! as an ordinary `cargo test --features runtime-benchmarks` — so the
//! setups here are EXECUTED in CI, not decorative.

#![cfg(feature = "runtime-benchmarks")]

use super::*;
use crate::pallet::Pallet as ComputeScoring;
use frame_benchmarking::v2::*;
use frame_support::traits::{Currency, EnsureOrigin};
use frame_system::RawOrigin;
use sp_std::vec;
use sp_std::vec::Vec;

const SEED: u32 = 0;

fn funded<T: Config>(index: u32) -> T::AccountId
where
    <T::DepositCurrency as Currency<T::AccountId>>::Balance: From<u128>,
{
    let who: T::AccountId = frame_benchmarking::v2::account("acct", index, SEED);
    let _ = T::DepositCurrency::deposit_creating(&who, (1u128 << 60).into());
    who
}

#[benchmarks(
    where
        <T::DepositCurrency as Currency<T::AccountId>>::Balance: From<u128>,
)]
mod benches {
    use super::*;

    #[benchmark]
    fn set_stake_floor() {
        #[extrinsic_call]
        _(RawOrigin::Root, 1_000u128.into());
    }

    #[benchmark]
    fn top_up_stake() {
        let who = funded::<T>(0);
        #[extrinsic_call]
        _(RawOrigin::Signed(who), 1_000u128.into());
    }

    #[benchmark]
    fn request_unstake() {
        let who = funded::<T>(0);
        ComputeScoring::<T>::top_up_stake(RawOrigin::Signed(who.clone()).into(), 1_000u128.into())
            .unwrap();
        #[extrinsic_call]
        _(RawOrigin::Signed(who), 500u128.into());
    }

    /// The `p` half of the weight — the R27 history sweep — is priced
    /// analytically from `Config::MaxEpochPruneKeysPerCall` rather than
    /// measured here, because `Pallet::sweep_epoch_history` does
    /// nothing but `clear_prefix` / `remove`: it is a pure storage-
    /// deletion count, which is exactly what `DbWeight` prices. What
    /// this benchmark DOES do is close an epoch past the retention
    /// window with real history behind it, so the sweep path executes
    /// (and, via `impl_benchmark_test_suite!`, executes in CI) instead
    /// of being skipped by a close that has nothing to prune.
    #[benchmark]
    fn vali_submit_epoch_close(n: Linear<1, 128>) {
        // Seed prunable history so the sweep is exercised, not skipped.
        let retention = crate::pallet::EpochHistoryRetention::<T>::get();
        for i in 0..32u32 {
            let mut vm = [0u8; 32];
            vm[..4].copy_from_slice(&i.to_le_bytes());
            crate::pallet::LiveAttestationCount::<T>::insert(1u64, vm, 1u32);
        }
        crate::pallet::CurrentEpoch::<T>::put(retention + 1);

        let mut updates: Vec<MinerStatusUpdate> = Vec::new();
        for i in 0..n {
            let mut node_id = [0u8; 32];
            node_id[..4].copy_from_slice(&i.to_le_bytes());
            updates.push(MinerStatusUpdate {
                node_id,
                new_status: MinerStatus::Quarantined,
                weight: 1_000,
            });
        }
        let bounded: BoundedVec<_, T::MaxMinerStatusUpdatesPerCall> = updates.try_into().unwrap();
        #[extrinsic_call]
        _(RawOrigin::Root, retention + 2, bounded);
    }

    // --- compute-miner-pay §3 tier prices ---

    fn node(i: u32) -> [u8; 32] {
        let mut node_id = [0u8; 32];
        node_id[..4].copy_from_slice(&i.to_le_bytes());
        node_id
    }

    fn tier_bounds<T: Config>() {
        for tier in DiskTier::ALL {
            TierPriceBounds::<T>::insert(tier, (1u128, 1_000_000_000u128));
        }
        PriceCeiling::<T>::put(1_000_000_000u128);
    }

    /// Worst case: nvme (dual write) over a re-registered node whose only
    /// anchor is the legacy RU one, so every anchor fallback is read.
    #[benchmark]
    fn announce_tier_price_change() {
        let who = funded::<T>(0);
        let node_id = node(0);
        NodeIdToChild::<T>::insert(node_id, who.clone());
        tier_bounds::<T>();
        TierMigrationCursor::<T>::put(TierMigrationStatus::Done);
        PriceMagnitudeAnchor::<T>::insert(node_id, 1_000u128);
        #[extrinsic_call]
        _(RawOrigin::Signed(who), node_id, DiskTier::Nvme, 1_200u128);
        assert!(PendingTierPriceChange::<T>::contains_key(
            node_id,
            DiskTier::Nvme
        ));
    }

    #[benchmark]
    fn apply_tier_price_change() {
        let who = funded::<T>(0);
        let node_id = node(0);
        NodeIdToChild::<T>::insert(node_id, who.clone());
        tier_bounds::<T>();
        TierMigrationCursor::<T>::put(TierMigrationStatus::Done);
        ComputeScoring::<T>::announce_tier_price_change(
            RawOrigin::Signed(who.clone()).into(),
            node_id,
            DiskTier::Nvme,
            1_200u128,
        )
        .unwrap();
        let due = frame_system::Pallet::<T>::block_number()
            .saturating_add(PriceChangeNoticeBlocks::<T>::get());
        frame_system::Pallet::<T>::set_block_number(due);
        #[extrinsic_call]
        _(RawOrigin::Signed(who), node_id, DiskTier::Nvme);
        assert_eq!(TierPrice::<T>::get(node_id, DiskTier::Nvme), Some(1_200));
    }

    #[benchmark]
    fn set_tier_price_bounds() -> Result<(), BenchmarkError> {
        // The runtime's admin origin is not root (thebrain: a pinned
        // authority), so take whatever origin it accepts.
        let origin = T::ComputeScoringAdminOrigin::try_successful_origin()
            .map_err(|_| BenchmarkError::Weightless)?;
        tier_bounds::<T>();
        TierMigrationCursor::<T>::put(TierMigrationStatus::Done);
        #[extrinsic_call]
        _(
            origin as T::RuntimeOrigin,
            DiskTier::Nvme,
            10u128,
            1_000u128,
        );
        Ok(())
    }

    /// `n` priced nodes with an in-bounds pending decrease: alternately
    /// already due (taken as the price) and not yet due (carried per
    /// tier). The legacy bounds are read for every pending.
    #[benchmark]
    fn migrate_tier_prices(n: Linear<1, 64>) -> Result<(), BenchmarkError> {
        let origin = T::ComputeScoringAdminOrigin::try_successful_origin()
            .map_err(|_| BenchmarkError::Weightless)?;
        tier_bounds::<T>();
        for i in 0..n {
            let node_id = node(i);
            NodeIdToChild::<T>::insert(node_id, funded::<T>(i));
            MinerPrice::<T>::insert(node_id, 10_000u128);
            let effective_block: u32 = if i % 2 == 0 { 0 } else { 1_000_000 };
            PendingPriceChange::<T>::insert(
                node_id,
                PriceChange {
                    new_price: 9_000u128,
                    effective_block: effective_block.into(),
                },
            );
        }
        #[extrinsic_call]
        _(origin as T::RuntimeOrigin, n);
        assert_eq!(TierMigrationCursor::<T>::get(), TierMigrationStatus::Done);
        Ok(())
    }

    // Runs every benchmark above as an ordinary test against the
    // mock — `cargo test --features runtime-benchmarks`. The setups
    // are therefore EXECUTED in CI, not decorative.
    impl_benchmark_test_suite!(
        ComputeScoring,
        crate::mock::new_test_ext(),
        crate::mock::TestRuntime
    );
}

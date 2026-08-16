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
use frame_support::traits::Currency;
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

    #[benchmark]
    fn vali_submit_epoch_close(n: Linear<1, 128>) {
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
        let bounded: BoundedVec<_, T::MaxMinerStatusUpdatesPerCall> =
            updates.try_into().unwrap();
        #[extrinsic_call]
        _(RawOrigin::Root, 1u64, bounded);
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

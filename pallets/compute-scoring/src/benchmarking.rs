//! Benchmarks for `pallet-compute-scoring`.
//!
//! `set_miner_status` is benchmarked here (O(1)). `close_epoch` is charged
//! at its worst case (`n = MaxMinersPerEpochClose`) via the hand-estimated
//! `SubstrateWeight::close_epoch(n)` in `weights.rs` — a real linear bench
//! needs the runtime's Arion-backed `Merit` populated with `n` registered
//! miners, which is a runtime-integration benchmark (a follow-up); the
//! worst-case charge is always safe (never under-weighs).

#![cfg(feature = "runtime-benchmarks")]

use super::*;
use crate::pallet::Pallet;
use frame_benchmarking::v2::*;
use frame_system::RawOrigin;

#[benchmarks]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn set_miner_status() {
		let node: NodeId = [7u8; 32];
		#[extrinsic_call]
		_(RawOrigin::Root, node, MinerStatus::Quarantined);

		assert!(crate::pallet::MinerStatuses::<T>::get(node).is_some());
	}

	impl_benchmark_test_suite!(Pallet, crate::mock::new_test_ext(), crate::mock::Test);
}

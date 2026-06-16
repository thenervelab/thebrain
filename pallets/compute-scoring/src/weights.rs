//! Weights for `pallet-compute-scoring`.
//!
//! `close_epoch` scales with the registered-miner count (one Arion read +
//! two writes per miner); `set_miner_status` is O(1). The `SubstrateWeight`
//! values below are conservative hand estimates pending a
//! `cargo run --features runtime-benchmarks` regeneration via
//! `benchmarking.rs` (the worst case is `n = MaxMinersPerEpochClose`).

#![allow(clippy::unnecessary_cast)]

use core::marker::PhantomData;
use frame_support::weights::{constants::RocksDbWeight, Weight};

/// Weight functions needed for `pallet-compute-scoring`.
pub trait WeightInfo {
	/// `n` = number of registered miners snapshotted this epoch.
	fn close_epoch(n: u32) -> Weight;
	fn set_miner_status() -> Weight;
}

/// Weights derived from the Substrate node + `RocksDbWeight`.
pub struct SubstrateWeight<T>(PhantomData<T>);

impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
	fn close_epoch(n: u32) -> Weight {
		// Base + per-miner (1 Arion read + 2 writes). Reads: NodeIdToChild
		// iter (n) + NodeWeightByChild (n); writes: NodeIdToChild mirror
		// (n) + EpochWeights (n) + CurrentEpoch (1).
		Weight::from_parts(15_000_000, 0)
			.saturating_add(Weight::from_parts(25_000_000, 0).saturating_mul(n as u64))
			.saturating_add(RocksDbWeight::get().reads((2 * n + 1) as u64))
			.saturating_add(RocksDbWeight::get().writes((2 * n + 1) as u64))
	}

	fn set_miner_status() -> Weight {
		Weight::from_parts(12_000_000, 0)
			.saturating_add(RocksDbWeight::get().reads(1))
			.saturating_add(RocksDbWeight::get().writes(1))
	}
}

/// Unit weights for mocks / tests.
impl WeightInfo for () {
	fn close_epoch(_n: u32) -> Weight {
		Weight::from_parts(0, 0)
	}
	fn set_miner_status() -> Weight {
		Weight::from_parts(0, 0)
	}
}

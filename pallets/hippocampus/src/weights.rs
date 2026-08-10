//! Weights for pallet-hippocampus
//!
//! TODO: benchmark — these are hand-written estimates on fund-moving paths.
//! Real `frame-benchmarking` weights are required before serious mainnet
//! traffic (tracked in a follow-up issue; raised in PR #36 review).

#![allow(unused_imports)]

use frame_support::{
	traits::Get,
	weights::{constants::RocksDbWeight, Weight},
};
use sp_std::marker::PhantomData;

pub trait WeightInfo {
	fn deposit() -> Weight;
	fn add_requester() -> Weight;
	fn remove_requester() -> Weight;
	fn pay_storage_miners(n: u32) -> Weight;
}

/// Weights using runtime `DbWeight`.
pub struct SubstrateWeight<T>(PhantomData<T>);
impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
	fn deposit() -> Weight {
		// transfer (2 accounts) + TotalDeposited read/write
		Weight::from_parts(50_000_000, 0)
			.saturating_add(T::DbWeight::get().reads(3_u64))
			.saturating_add(T::DbWeight::get().writes(3_u64))
	}

	fn add_requester() -> Weight {
		Weight::from_parts(15_000_000, 0)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}

	fn remove_requester() -> Weight {
		Weight::from_parts(15_000_000, 0)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}

	fn pay_storage_miners(n: u32) -> Weight {
		// Guards + compartment/ledger bookkeeping, then one transfer
		// (2 account reads/writes) per miner.
		Weight::from_parts(30_000_000, 0)
			.saturating_add(T::DbWeight::get().reads(5_u64))
			.saturating_add(T::DbWeight::get().writes(3_u64))
			.saturating_add(
				Weight::from_parts(50_000_000, 0)
					.saturating_add(T::DbWeight::get().reads_writes(2, 2))
					.saturating_mul(n.into()),
			)
	}
}

impl WeightInfo for () {
	fn deposit() -> Weight {
		Weight::from_parts(50_000_000, 0)
			.saturating_add(RocksDbWeight::get().reads(3_u64))
			.saturating_add(RocksDbWeight::get().writes(3_u64))
	}

	fn add_requester() -> Weight {
		Weight::from_parts(15_000_000, 0)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}

	fn remove_requester() -> Weight {
		Weight::from_parts(15_000_000, 0)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}

	fn pay_storage_miners(n: u32) -> Weight {
		Weight::from_parts(30_000_000, 0)
			.saturating_add(RocksDbWeight::get().reads(5_u64))
			.saturating_add(RocksDbWeight::get().writes(3_u64))
			.saturating_add(
				Weight::from_parts(50_000_000, 0)
					.saturating_add(RocksDbWeight::get().reads_writes(2, 2))
					.saturating_mul(n.into()),
			)
	}
}

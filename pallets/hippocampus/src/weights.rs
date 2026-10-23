//! Weights for pallet-hippocampus
//!
//! `deposit`, `add_requester`, `remove_requester` and `request_payment` are
//! **measured** by `benchmarking.rs` (2026-10-02, steps 50, repeat 20, compiled
//! wasm, `--chain benchmark`, on an i5-1135G7 laptop rather than reference
//! hardware — rerun there before relying on the absolute figures). Storage
//! counts and proof sizes are the benchmark's own.
//!
//! `pay_storage_miners` and `pay_compute_miners` are still hand-written
//! estimates.

#![allow(unused_imports)]

use frame_support::{
	traits::Get,
	weights::{constants::RocksDbWeight, Weight},
};
use sp_std::marker::PhantomData;

/// Reads `pay_storage_miners` spends assembling its payee set from the Arion
/// pallet, before it pays anybody.
///
/// Sized off `MaxChildrenTotal` (1_000): a `FamilyChildren` key exists only
/// while a family has at least one Active child (the key is removed at zero by
/// `cleanup_family_when_no_active_children`), so the number of scanned family
/// keys is bounded by the active-child cap, not by `MaxFamilies` (600) — same
/// reasoning that sizes `MaxMinersPerPayout` in the mainnet runtime. On top of
/// those keys, three reads (`ChildRegistrations`, `NodeWeightLastBucket`,
/// `NodeWeightByChild`) for each of at most `MaxChildrenTotal` children.
const SOURCE_SCAN_READS: u64 = 1_000 + 3 * 1_000;

pub trait WeightInfo {
	fn deposit() -> Weight;
	fn add_requester() -> Weight;
	fn remove_requester() -> Weight;
	/// One `request_payment`, which is not an extrinsic: callers add this to
	/// their own weight for every payment they make through the bank.
	fn request_payment() -> Weight;
	fn pay_storage_miners(n: u32) -> Weight;
	fn pay_compute_miners(n: u32) -> Weight;
}

/// Weights using runtime `DbWeight`.
pub struct SubstrateWeight<T>(PhantomData<T>);
impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
	/// Storage: `System::Account` (r:1 w:1) — the bank's; the caller's is
	/// already paid for by the transaction fee.
	/// Storage: `Hippocampus::TotalDeposited` (r:1 w:1)
	fn deposit() -> Weight {
		Weight::from_parts(88_941_000, 3593)
			.saturating_add(T::DbWeight::get().reads(2_u64))
			.saturating_add(T::DbWeight::get().writes(2_u64))
	}

	/// Storage: `Hippocampus::WhitelistedRequesters` (r:1 w:1)
	fn add_requester() -> Weight {
		Weight::from_parts(39_897_000, 3633)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}

	/// Storage: `Hippocampus::WhitelistedRequesters` (r:1 w:1)
	fn remove_requester() -> Weight {
		Weight::from_parts(27_361_000, 3686)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}

	/// Storage: `Hippocampus::DistributionEnabled`, `WhitelistedRequesters`,
	/// `RequesterWithdrawalCap` (r:1 each), `System::Account` (r:2 w:2),
	/// `Hippocampus::TotalDeposited` (r:2), `EmissionPaidOut`,
	/// `ComputeEmissionPaidOut` (r:1 each), `TotalPaidOut`,
	/// `TotalPaidByRequester` (r:1 w:1 each).
	///
	/// Posed with a cap set and a payee that does not exist yet.
	fn request_payment() -> Weight {
		Weight::from_parts(114_257_000, 6370)
			.saturating_add(T::DbWeight::get().reads(11_u64))
			.saturating_add(T::DbWeight::get().writes(4_u64))
	}

	fn pay_storage_miners(n: u32) -> Weight {
		// Guards + compartment/ledger bookkeeping, then one transfer
		// (2 account reads/writes) per miner.
		//
		// `SOURCE_SCAN_READS` covers building the payee set, which the
		// per-miner term does not: the Arion source walks `FamilyChildren`
		// (<= `MaxFamilies` keys) and reads registration, freshness, and
		// weight for each child (<= `MaxChildrenTotal` x 3 network-wide). It
		// is a flat term because the walk is bounded by those registration
		// caps, not by `n`. Revisit if the runtime raises them.
		Weight::from_parts(30_000_000, 0)
			.saturating_add(T::DbWeight::get().reads(5_u64 + SOURCE_SCAN_READS))
			.saturating_add(T::DbWeight::get().writes(3_u64))
			.saturating_add(
				Weight::from_parts(50_000_000, 0)
					.saturating_add(T::DbWeight::get().reads_writes(2, 2))
					.saturating_mul(n.into()),
			)
	}

	fn pay_compute_miners(n: u32) -> Weight {
		// Same shape as `pay_storage_miners`: guards + compute-compartment
		// bookkeeping, then one transfer (2 account reads/writes) per miner.
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
	/// Storage: `System::Account` (r:1 w:1) — the bank's; the caller's is
	/// already paid for by the transaction fee.
	/// Storage: `Hippocampus::TotalDeposited` (r:1 w:1)
	fn deposit() -> Weight {
		Weight::from_parts(88_941_000, 3593)
			.saturating_add(RocksDbWeight::get().reads(2_u64))
			.saturating_add(RocksDbWeight::get().writes(2_u64))
	}

	/// Storage: `Hippocampus::WhitelistedRequesters` (r:1 w:1)
	fn add_requester() -> Weight {
		Weight::from_parts(39_897_000, 3633)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}

	/// Storage: `Hippocampus::WhitelistedRequesters` (r:1 w:1)
	fn remove_requester() -> Weight {
		Weight::from_parts(27_361_000, 3686)
			.saturating_add(RocksDbWeight::get().reads(1_u64))
			.saturating_add(RocksDbWeight::get().writes(1_u64))
	}

	/// Storage: `Hippocampus::DistributionEnabled`, `WhitelistedRequesters`,
	/// `RequesterWithdrawalCap` (r:1 each), `System::Account` (r:2 w:2),
	/// `Hippocampus::TotalDeposited` (r:2), `EmissionPaidOut`,
	/// `ComputeEmissionPaidOut` (r:1 each), `TotalPaidOut`,
	/// `TotalPaidByRequester` (r:1 w:1 each).
	///
	/// Posed with a cap set and a payee that does not exist yet.
	fn request_payment() -> Weight {
		Weight::from_parts(114_257_000, 6370)
			.saturating_add(RocksDbWeight::get().reads(11_u64))
			.saturating_add(RocksDbWeight::get().writes(4_u64))
	}

	fn pay_storage_miners(n: u32) -> Weight {
		Weight::from_parts(30_000_000, 0)
			.saturating_add(RocksDbWeight::get().reads(5_u64 + SOURCE_SCAN_READS))
			.saturating_add(RocksDbWeight::get().writes(3_u64))
			.saturating_add(
				Weight::from_parts(50_000_000, 0)
					.saturating_add(RocksDbWeight::get().reads_writes(2, 2))
					.saturating_mul(n.into()),
			)
	}

	fn pay_compute_miners(n: u32) -> Weight {
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

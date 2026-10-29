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

pub trait WeightInfo: SettlementWeightInfo {
	fn deposit() -> Weight;
	fn add_requester() -> Weight;
	fn remove_requester() -> Weight;
	fn pay_storage_miners(n: u32) -> Weight;
	fn pay_compute_miners(n: u32) -> Weight;
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

/// Weights of the compute settlement calls (10-18).
///
/// Measured with `benchmarking.rs` (each call at its worst case: four approver
/// keys with the signer last; `pay_compute_part` up to `MaxEntriesPerPart`
/// entries, every transfer failing over to the owed account):
/// `hippius benchmark pallet --chain dev --pallet pallet_hippocampus
/// --extrinsic '*' --steps 20 --repeat 10`, 2026-10-29, on a development VM
/// (Xeon E5-2650 @ 2.00GHz, compiled wasm). Execution time and proof size are
/// the measured ones; reads and writes are the larger of the measured and the
/// counted figures. Re-run on the reference machine before mainnet.
pub trait SettlementWeightInfo {
	fn submit_compute_approval() -> Weight;
	fn pay_compute_part(n: u32) -> Weight;
	fn claim_compute_owed() -> Weight;
	fn fund_compute_settlement() -> Weight;
	fn revoke_compute_approval() -> Weight;
	fn schedule_compute_root_change() -> Weight;
	fn apply_compute_root_change() -> Weight;
	fn cancel_compute_root_change() -> Weight;
	fn disable_compute_settlement() -> Weight;
}

fn settlement_weight<W: Get<frame_support::weights::RuntimeDbWeight>>(
	ps: u64,
	proof: u64,
	reads: u64,
	writes: u64,
) -> Weight {
	Weight::from_parts(ps, proof)
		.saturating_add(W::get().reads(reads))
		.saturating_add(W::get().writes(writes))
}

macro_rules! settlement_weights {
	($db:ty) => {
		fn submit_compute_approval() -> Weight {
			// Measured 768.3 µs, 19 reads, 5 writes; counted up to 22 reads
			// (role lists) and 6 writes (completion of a 0-part approval).
			settlement_weight::<$db>(768_264_000, 4_467, 22, 6)
		}
		fn pay_compute_part(n: u32) -> Weight {
			// Measured 292.7 µs + 150.1 µs per entry; counted per entry a
			// failed transfer, the owed transfer and the owed map and total.
			settlement_weight::<$db>(292_666_247, 6_196, 15, 12).saturating_add(
				settlement_weight::<$db>(150_094_314, 2_603, 6, 4).saturating_mul(n.into()),
			)
		}
		fn claim_compute_owed() -> Weight {
			settlement_weight::<$db>(212_763_000, 6_196, 6, 4)
		}
		fn fund_compute_settlement() -> Weight {
			// Worst case: the owed account seeded first (its balance read, a
			// second transfer), then the settlement transfer.
			settlement_weight::<$db>(298_592_000, 6_196, 5, 4)
		}
		fn revoke_compute_approval() -> Weight {
			settlement_weight::<$db>(674_323_000, 4_979, 7, 5)
		}
		fn schedule_compute_root_change() -> Weight {
			// Key validation reads the three role lists.
			settlement_weight::<$db>(107_761_000, 1_602, 5, 1)
		}
		fn apply_compute_root_change() -> Weight {
			// Re-validation plus the heaviest change (defund: a transfer).
			settlement_weight::<$db>(247_158_000, 6_196, 10, 6)
		}
		fn cancel_compute_root_change() -> Weight {
			settlement_weight::<$db>(53_863_000, 3_617, 1, 1)
		}
		fn disable_compute_settlement() -> Weight {
			settlement_weight::<$db>(35_241_000, 0, 0, 2)
		}
	};
}

impl<T: frame_system::Config> SettlementWeightInfo for SubstrateWeight<T> {
	settlement_weights!(T::DbWeight);
}

impl SettlementWeightInfo for () {
	settlement_weights!(RocksDbWeight);
}

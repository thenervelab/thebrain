//! # pallet-compute-scoring
//!
//! The on-chain signal the Hippius §23 confidential-compute scheduler
//! reads to **score** and **rank** miners for VM placement. The scheduler
//! (off-chain, in `hippius-compute`) shells out to a `read-miner-status`
//! binary that decodes this pallet's storage over JSON-RPC; this pallet
//! exists so that decode succeeds with a real merit signal.
//!
//! ## Why a separate pallet (and why these exact names)
//!
//! `read-miner-status` derives storage keys as
//! `twox_128("ComputeScoring") ++ twox_128(<Item>) ++ …` and decodes
//! fixed SCALE shapes. So this pallet MUST be wired into the runtime under
//! the name **`ComputeScoring`**, and its storage items MUST be named
//! exactly `CurrentEpoch`, `NodeIdToChild`, `MinerStatuses`,
//! `EpochWeights` with the value shapes below. The reader is the frozen
//! contract; this pallet is written to satisfy it (see `tests.rs`, which
//! re-asserts the byte layout the reader's `decode_*` functions expect).
//!
//! ## What it does NOT do
//!
//! It does NOT compute merit. The deployed, validator-authority-gated,
//! anti-cheat node weight already lives in [`pallet_arion`]
//! (`NodeWeightByChild`, a `u16` blend of bandwidth/storage/uptime with
//! strike + integrity penalties). This pallet **buckets** that weight into
//! a per-epoch, node_id-keyed `u128` (`EpochWeights`) on each
//! `close_epoch`, and holds the §13 miner status state machine
//! (`MinerStatuses`) the scheduler gates + drains on. One source of merit
//! truth (Arion); this is the epoch+status projection the scheduler reads.

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;
pub mod weights;
pub use weights::WeightInfo;

use codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;

/// A miner's §13 lifecycle status. SCALE-encodes by variant index, so the
/// discriminants are `Active = 0`, `Quarantined = 1`, `Decommissioned = 2`
/// — the exact bytes `read-miner-status::miner_status_label` maps. The
/// declaration order is therefore load-bearing; do NOT reorder.
#[derive(
	Clone, Copy, PartialEq, Eq, Encode, Decode, RuntimeDebug, TypeInfo, MaxEncodedLen,
)]
pub enum MinerStatus {
	/// Schedulable. The registry default — a registered miner with no
	/// `MinerStatuses` row IS Active (the reader assumes this), so an
	/// Active status is stored as the ABSENCE of a row, never a `0` row.
	Active,
	/// §13 quarantine — suspected ghost load / repeated failures.
	/// Excluded from placement + drained from any bound placement.
	Quarantined,
	/// Permanently retired. Excluded from placement.
	Decommissioned,
}

/// The per-miner status record. **Field order is load-bearing**:
/// `read-miner-status::decode_miner_status_entry` reads `status` from the
/// FIRST byte and `last_transition_epoch` from the TRAILING 8 bytes,
/// agnostic to the `BlockNumber` width in the middle. Keep `status` first
/// and `last_transition_epoch` last.
#[derive(
	Clone, PartialEq, Eq, Encode, Decode, RuntimeDebug, TypeInfo, MaxEncodedLen,
)]
pub struct MinerStatusEntry<BlockNumber> {
	/// The status discriminant (head byte).
	pub status: MinerStatus,
	/// Block the status last changed (audit; the reader skips it).
	pub last_transition_block: BlockNumber,
	/// Epoch the status last changed (trailing u64 — the reader's
	/// `data_epoch` fallback for an unscored miner).
	pub last_transition_epoch: u64,
}

use frame_support::pallet_prelude::RuntimeDebug;

/// A miner's 32-byte compute node id (the on-chain registration key).
pub type NodeId = [u8; 32];

/// The per-miner merit source `close_epoch` snapshots into `EpochWeights`.
///
/// The runtime wires this to `pallet-arion` (read `NodeIdToChild` +
/// `NodeWeightByChild`), keeping this pallet decoupled from Arion's full
/// `Config` (and its registration/balances deps) so the test mock stays
/// light. Yields `(node_id, child_account, weight_u128)` for every
/// registered miner.
pub trait MeritSource<AccountId> {
	fn registered_miners() -> sp_std::vec::Vec<(NodeId, AccountId, u128)>;
}

/// No miners — a valid (empty-fleet) source, and the trivial test default.
impl<AccountId> MeritSource<AccountId> for () {
	fn registered_miners() -> sp_std::vec::Vec<(NodeId, AccountId, u128)> {
		sp_std::vec::Vec::new()
	}
}

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;

	#[pallet::config]
	pub trait Config: frame_system::Config {
		/// The aggregated event type.
		type RuntimeEvent: From<Event<Self>>
			+ IsType<<Self as frame_system::Config>::RuntimeEvent>;

		/// Who may close an epoch (snapshot the merit weights) and set a
		/// miner status — the validator set / a council origin, the same
		/// authority that drives Arion's weights.
		type AuthorityOrigin: EnsureOrigin<Self::RuntimeOrigin>;

		/// The per-miner merit weights to snapshot each epoch (the runtime
		/// wires this to pallet-arion).
		type Merit: MeritSource<Self::AccountId>;

		/// Safety cap on miners processed in one `close_epoch` — bounds
		/// the extrinsic's weight. A fleet larger than this fails closed
		/// (the authority must raise the cap via a runtime upgrade rather
		/// than silently skip miners, which would zero their score).
		#[pallet::constant]
		type MaxMinersPerEpochClose: Get<u32>;

		/// Weights.
		type WeightInfo: WeightInfo;
	}

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	// ── Storage (byte-matched to the `read-miner-status` contract) ──────

	/// The current scoring epoch. Bumped by [`Pallet::close_epoch`].
	/// `ValueQuery` (absent ⇒ 0), matching the reader.
	#[pallet::storage]
	pub type CurrentEpoch<T> = StorageValue<_, u64, ValueQuery>;

	/// Registered-miner registry mirror: `node_id → child account`.
	/// Mirrored from `pallet_arion::NodeIdToChild` on each `close_epoch`.
	/// The reader ENUMERATES the keys (every registered miner) and never
	/// decodes the value — but we store the child so the map is useful +
	/// matches Arion's shape.
	#[pallet::storage]
	pub type NodeIdToChild<T: Config> =
		StorageMap<_, Blake2_128Concat, NodeId, T::AccountId, OptionQuery>;

	/// **Sparse** miner status: a row exists ONLY for a non-Active miner
	/// (Quarantined / Decommissioned). Absence ⇒ Active. The reader relies
	/// on this — it enumerates `NodeIdToChild`, not this map.
	#[pallet::storage]
	pub type MinerStatuses<T: Config> =
		StorageMap<_, Blake2_128Concat, NodeId, MinerStatusEntry<BlockNumberFor<T>>, OptionQuery>;

	/// `EpochWeights[epoch][node_id] → u128` — the per-epoch reward weight
	/// the scheduler ranks by (`quality`). Written by `close_epoch` from
	/// Arion's `NodeWeightByChild`. Absent ⇒ not scored this epoch ⇒ the
	/// reader treats `quality = 0` and the miner stale-gates closed.
	#[pallet::storage]
	pub type EpochWeights<T: Config> = StorageDoubleMap<
		_,
		Blake2_128Concat,
		u64,
		Blake2_128Concat,
		NodeId,
		u128,
		OptionQuery,
	>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// An epoch was closed: `epoch` is the NEW current epoch,
		/// `miners_scored` rows were written into `EpochWeights`.
		EpochClosed { epoch: u64, miners_scored: u32 },
		/// A miner's status changed (`Active` clears the row).
		MinerStatusChanged { node_id: NodeId, status: MinerStatus },
	}

	#[pallet::error]
	pub enum Error<T> {
		/// The registered-miner count exceeds `MaxMinersPerEpochClose`.
		/// Raise the cap (runtime upgrade) rather than silently skip.
		TooManyMiners,
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Close the current epoch: snapshot every registered miner's
		/// Arion weight into `EpochWeights[next_epoch][node_id]`, refresh
		/// the `NodeIdToChild` mirror, and bump `CurrentEpoch`.
		///
		/// Authority-gated (`AuthorityOrigin`) — the same validator/council
		/// authority that updates Arion's weights. Idempotent per epoch in
		/// effect: re-running just overwrites the next epoch's rows.
		///
		/// The reader's `data_epoch` freshness gate keys off whether a
		/// miner has an `EpochWeights[current][node_id]` row, so this MUST
		/// run each scoring epoch for the scheduler to see fresh scores.
		#[pallet::call_index(0)]
		#[pallet::weight(T::WeightInfo::close_epoch(T::MaxMinersPerEpochClose::get()))]
		pub fn close_epoch(origin: OriginFor<T>) -> DispatchResult {
			T::AuthorityOrigin::ensure_origin(origin)?;

			let cap = T::MaxMinersPerEpochClose::get();
			let miners = T::Merit::registered_miners();
			// Fail closed (no partial epoch) if over the cap.
			ensure!(miners.len() as u32 <= cap, Error::<T>::TooManyMiners);

			let next = CurrentEpoch::<T>::get().saturating_add(1);
			let mut scored: u32 = 0;
			for (node_id, child, weight) in miners {
				// Refresh the registry mirror so the reader's enumeration
				// reflects the current registration set.
				NodeIdToChild::<T>::insert(node_id, child);
				EpochWeights::<T>::insert(next, node_id, weight);
				scored = scored.saturating_add(1);
			}
			CurrentEpoch::<T>::put(next);
			Self::deposit_event(Event::EpochClosed { epoch: next, miners_scored: scored });
			Ok(())
		}

		/// Set a miner's §13 status. `Active` REMOVES the row (the sparse
		/// invariant the reader depends on); any non-Active status writes a
		/// `MinerStatusEntry` stamped with the current block + epoch.
		/// Authority-gated.
		#[pallet::call_index(1)]
		#[pallet::weight(T::WeightInfo::set_miner_status())]
		pub fn set_miner_status(
			origin: OriginFor<T>,
			node_id: NodeId,
			status: MinerStatus,
		) -> DispatchResult {
			T::AuthorityOrigin::ensure_origin(origin)?;

			match status {
				MinerStatus::Active => {
					// Sparse: clearing to Active is the absence of a row.
					MinerStatuses::<T>::remove(node_id);
				}
				_ => {
					MinerStatuses::<T>::insert(
						node_id,
						MinerStatusEntry {
							status,
							last_transition_block: frame_system::Pallet::<T>::block_number(),
							last_transition_epoch: CurrentEpoch::<T>::get(),
						},
					);
				}
			}
			Self::deposit_event(Event::MinerStatusChanged { node_id, status });
			Ok(())
		}
	}

	impl<T: Config> Pallet<T> {
		/// Read helper: the effective status of a miner (absent row ⇒
		/// Active). Used by tests + any in-runtime consumer.
		pub fn status_of(node_id: &NodeId) -> MinerStatus {
			MinerStatuses::<T>::get(node_id)
				.map(|e| e.status)
				.unwrap_or(MinerStatus::Active)
		}
	}
}

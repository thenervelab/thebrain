//! Tests for `pallet-compute-scoring`.
//!
//! Two kinds: behaviour (close_epoch / set_miner_status semantics) and the
//! **byte-layout conformance** that the hippius-compute `read-miner-status`
//! reader depends on — those tests re-assert exactly what the reader's
//! `decode_miner_status_entry` / `scale_u128` / `miner_status_label`
//! functions expect, so a drift here breaks CI before it breaks the chain.

use crate::mock::*;
use crate::{
	CurrentEpoch, EpochWeights, MinerStatus, MinerStatusEntry, MinerStatuses,
	NodeIdToChild,
};
use codec::Encode;
use frame_support::{assert_noop, assert_ok};
use sp_runtime::DispatchError;

fn nid(b: u8) -> [u8; 32] {
	[b; 32]
}

// ── byte-layout conformance (the reader's frozen contract) ───────────

#[test]
fn miner_status_discriminants_match_the_reader() {
	// read-miner-status::miner_status_label maps 0/1/2.
	assert_eq!(MinerStatus::Active.encode(), vec![0u8]);
	assert_eq!(MinerStatus::Quarantined.encode(), vec![1u8]);
	assert_eq!(MinerStatus::Decommissioned.encode(), vec![2u8]);
}

#[test]
fn miner_status_entry_scale_layout_matches_decode_miner_status_entry() {
	// The reader reads status from byte 0 and last_transition_epoch from
	// the TRAILING 8 bytes, agnostic to the BlockNumber width between.
	let entry = MinerStatusEntry::<u64> {
		status: MinerStatus::Quarantined,
		last_transition_block: 42,
		last_transition_epoch: 1_234_567,
	};
	let bytes = entry.encode();
	// Head byte = status discriminant.
	assert_eq!(bytes[0], 1u8);
	// Trailing 8 bytes = epoch, little-endian (SCALE u64).
	assert_eq!(&bytes[bytes.len() - 8..], &1_234_567u64.to_le_bytes());
	// Mock block number is u64 here, but the reader tolerates u32 too —
	// the invariant is head=status, tail=epoch, which holds either way.
}

#[test]
fn epoch_weight_value_is_a_plain_u128() {
	// EpochWeights values decode via scale_u128 (16 LE bytes).
	let w: u128 = 9_000_000_000_000_000_000_000; // > u64 to prove u128.
	assert_eq!(w.encode(), w.to_le_bytes().to_vec());
}

// ── close_epoch behaviour ────────────────────────────────────────────

#[test]
fn close_epoch_snapshots_merit_into_epoch_weights_and_bumps() {
	new_test_ext().execute_with(|| {
		set_miners(vec![(nid(1), 100, 7), (nid(2), 200, 0)]);
		assert_eq!(CurrentEpoch::<Test>::get(), 0);

		assert_ok!(ComputeScoring::close_epoch(RuntimeOrigin::root()));

		assert_eq!(CurrentEpoch::<Test>::get(), 1);
		// Weights bucketed under the NEW epoch (1), keyed by node_id.
		assert_eq!(EpochWeights::<Test>::get(1, nid(1)), Some(7u128));
		assert_eq!(EpochWeights::<Test>::get(1, nid(2)), Some(0u128));
		// Registry mirror refreshed (child accounts).
		assert_eq!(NodeIdToChild::<Test>::get(nid(1)), Some(100u64));
		assert_eq!(NodeIdToChild::<Test>::get(nid(2)), Some(200u64));
	});
}

#[test]
fn close_epoch_is_authority_gated() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			ComputeScoring::close_epoch(RuntimeOrigin::signed(1)),
			DispatchError::BadOrigin
		);
		assert_eq!(CurrentEpoch::<Test>::get(), 0);
	});
}

#[test]
fn close_epoch_fails_closed_over_the_cap() {
	new_test_ext().execute_with(|| {
		// MaxMinersPerEpochClose is 1_000 in the mock; exceed it.
		let many: Vec<_> = (0..1_001u32)
			.map(|i| {
				let mut n = [0u8; 32];
				n[..4].copy_from_slice(&i.to_le_bytes());
				(n, i as u64, 1u128)
			})
			.collect();
		set_miners(many);
		assert_noop!(
			ComputeScoring::close_epoch(RuntimeOrigin::root()),
			crate::Error::<Test>::TooManyMiners
		);
		// No partial epoch.
		assert_eq!(CurrentEpoch::<Test>::get(), 0);
	});
}

// ── set_miner_status behaviour ───────────────────────────────────────

#[test]
fn set_miner_status_is_sparse_active_clears_the_row() {
	new_test_ext().execute_with(|| {
		// Quarantine writes a row stamped with the current epoch.
		CurrentEpoch::<Test>::put(5);
		assert_ok!(ComputeScoring::set_miner_status(
			RuntimeOrigin::root(),
			nid(1),
			MinerStatus::Quarantined
		));
		let e = MinerStatuses::<Test>::get(nid(1)).expect("row");
		assert_eq!(e.status, MinerStatus::Quarantined);
		assert_eq!(e.last_transition_epoch, 5);
		assert_eq!(ComputeScoring::status_of(&nid(1)), MinerStatus::Quarantined);

		// Recovering to Active REMOVES the row (the sparse invariant).
		assert_ok!(ComputeScoring::set_miner_status(
			RuntimeOrigin::root(),
			nid(1),
			MinerStatus::Active
		));
		assert!(MinerStatuses::<Test>::get(nid(1)).is_none());
		assert_eq!(ComputeScoring::status_of(&nid(1)), MinerStatus::Active);
	});
}

#[test]
fn set_miner_status_is_authority_gated() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			ComputeScoring::set_miner_status(
				RuntimeOrigin::signed(1),
				nid(1),
				MinerStatus::Decommissioned
			),
			DispatchError::BadOrigin
		);
	});
}

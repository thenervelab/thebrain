//! Tests for the emission-side family payout weights (`active_family_weights`).
//!
//! Payouts read the SAME fragmentation-neutral aggregate as `FamilyWeightRaw`:
//! one log2 over the family's summed bytes/bandwidth, raw (no EMA, no clamp).
//! The previous shape — a plain sum of per-child `NodeWeightByChild` scores —
//! rewarded splitting the same data across more children (log2 is concave), a
//! payout multiplier costing only child deposits. These tests pin the repaired
//! contract from the payout side.

use hippius_mainnet_runtime::{AccountId, Runtime, System};
use pallet_arion::{
	ChildRegistration, ChildRegistrations, ChildStatus, CurrentWeightBucket, FamilyChildren,
	NodeQuality, NodeQualityByChild, NodeWeightByChild, NodeWeightLastBucket, Pallet as Arion,
};
use sp_runtime::{AccountId32, BuildStorage};

const BUCKET: u32 = 100;
const TIB: u128 = 1024 * 1024 * 1024 * 1024;

fn account(seed: u8, tag: u8) -> AccountId {
	let mut raw = [seed; 32];
	raw[0] = tag;
	AccountId32::new(raw)
}

fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| {
		System::set_block_number(1);
		CurrentWeightBucket::<Runtime>::put(BUCKET);
	});
	ext
}

fn quality(bytes: u128, bw: u128, uptime: u16) -> NodeQuality {
	NodeQuality {
		shard_data_bytes: bytes,
		bandwidth_bytes: bw,
		uptime_permille: uptime,
		strikes: 0,
		integrity_fails: 0,
	}
}

/// Register a child under a family, index it, and seed its quality directly.
/// Storage-level seeding (not extrinsics): these tests exercise the read path.
fn add_child_with_quality(family: &AccountId, child: &AccountId, q: NodeQuality) {
	let node_id: [u8; 32] = child.clone().into();
	ChildRegistrations::<Runtime>::insert(
		child,
		ChildRegistration {
			family: family.clone(),
			node_id,
			status: ChildStatus::Active,
			deposit: 0u128,
			unbonding_end: 0u32.into(),
		},
	);
	FamilyChildren::<Runtime>::mutate(family, |v| {
		v.try_push(child.clone()).unwrap();
	});
	NodeQualityByChild::<Runtime>::insert(child, q);
	NodeWeightLastBucket::<Runtime>::insert(child, BUCKET);
	// A stale per-child score from the OLD sum-based shape: must be ignored.
	NodeWeightByChild::<Runtime>::insert(child, 50_000u16);
}

fn payout_weight(family: &AccountId) -> u128 {
	Arion::<Runtime>::active_family_weights()
		.into_iter()
		.find(|(f, _)| f == family)
		.map(|(_, w)| w)
		.unwrap_or(0)
}

/// Splitting the same bytes across more children must not change the payout.
/// This is the test that fails on the previous sum-based shape.
#[test]
fn payout_is_fragmentation_neutral() {
	new_test_ext().execute_with(|| {
		let total = 40 * TIB;
		let bw = 2 * TIB;
		let mut weights = Vec::new();
		for (tag, n) in [(1u8, 1u128), (2, 2), (3, 5), (4, 35)] {
			let family = account(10, tag);
			for i in 0..n {
				let child = account(100 + tag, i as u8);
				add_child_with_quality(&family, &child, quality(total / n, bw / n, 1000));
			}
			weights.push(payout_weight(&family));
		}
		let one = weights[0];
		assert!(one > 0, "a family with real data must be paid");
		for (i, w) in weights.iter().enumerate() {
			// Integer division across n children may lose a few bytes; allow
			// only that rounding, never a fragmentation premium.
			let diff = one.abs_diff(*w);
			assert!(
				diff <= one / 100,
				"fragmentation changed the payout: 1 child = {one}, case {i} = {w}"
			);
		}
	});
}

/// An empty child adds nothing to the family's payout weight.
#[test]
fn empty_children_add_no_payout() {
	new_test_ext().execute_with(|| {
		let family = account(11, 1);
		add_child_with_quality(&family, &account(111, 1), quality(4 * TIB, TIB, 1000));
		let before = payout_weight(&family);

		for i in 2..=10u8 {
			add_child_with_quality(&family, &account(111, i), quality(0, 0, 1000));
		}
		let after = payout_weight(&family);
		assert_eq!(before, after, "piling on empty children must not move the payout");
	});
}

/// The families paid are exactly the families scored: same aggregate, same set.
#[test]
fn payout_set_matches_scoring_set() {
	new_test_ext().execute_with(|| {
		// One real family, one dust family (bytes below the floor -> score 0).
		let real = account(12, 1);
		add_child_with_quality(&real, &account(112, 1), quality(4 * TIB, TIB, 1000));
		let dust = account(12, 2);
		add_child_with_quality(&dust, &account(112, 2), quality(1, 0, 1000));

		let paid: Vec<AccountId> =
			Arion::<Runtime>::active_family_weights().into_iter().map(|(f, _)| f).collect();
		assert!(paid.contains(&real), "scored family must be paid");
		assert!(!paid.contains(&dust), "zero-scored family must not appear at all");
	});
}

/// A child whose own registration names another family is counted for no one.
#[test]
fn foreign_child_is_not_paid() {
	new_test_ext().execute_with(|| {
		let family = account(13, 1);
		add_child_with_quality(&family, &account(113, 1), quality(4 * TIB, TIB, 1000));
		let before = payout_weight(&family);

		// Forge the index inconsistency: child listed under `family` but whose
		// registration names another owner.
		let stranger = account(13, 9);
		let child = account(113, 2);
		add_child_with_quality(&stranger, &child, quality(100 * TIB, 10 * TIB, 1000));
		FamilyChildren::<Runtime>::mutate(&family, |v| {
			v.try_push(child.clone()).unwrap();
		});

		assert_eq!(
			payout_weight(&family),
			before,
			"a family must not be paid for a child it does not own"
		);
	});
}

/// A stale child (no fresh weight report) stops counting toward the payout.
#[test]
fn stale_child_is_excluded_from_payout() {
	new_test_ext().execute_with(|| {
		let family = account(14, 1);
		add_child_with_quality(&family, &account(114, 1), quality(4 * TIB, TIB, 1000));
		let fresh_only = payout_weight(&family);

		let stale_child = account(114, 2);
		add_child_with_quality(&family, &stale_child, quality(100 * TIB, 10 * TIB, 1000));
		// Freshness horizon is StaleChildBuckets = 4.
		NodeWeightLastBucket::<Runtime>::insert(&stale_child, BUCKET - 10);

		assert_eq!(
			payout_weight(&family),
			fresh_only,
			"a stale child must not inflate the payout"
		);
	});
}

/// Penalties are not absorbed by a per-node score floor: a family with real
/// bytes but heavy integrity failures earns nothing.
#[test]
fn penalties_zero_out_bad_families() {
	new_test_ext().execute_with(|| {
		let family = account(15, 1);
		let child = account(115, 1);
		// ~1.6 TiB stored, 92 integrity failures (IntegrityFailPenalty = 100):
		// the live case that kept earning under the sum-based shape.
		let mut q = quality(1667 * TIB / 1024, 0, 1000);
		q.strikes = 2;
		q.integrity_fails = 92;
		add_child_with_quality(&family, &child, q);

		assert_eq!(
			payout_weight(&family),
			0,
			"penalties must be able to zero a family, not vanish into a floor"
		);
	});
}

/// `active_family_weights` never returns zero entries (MaxMinersPerPayout
/// bounds the whole payout call; zeros would waste slots and can fail it).
#[test]
fn no_zero_entries_in_payout_set() {
	new_test_ext().execute_with(|| {
		for i in 1..=20u8 {
			let family = account(16, i);
			// Every second family is dust below the floor.
			let bytes = if i % 2 == 0 { 1 } else { 4 * TIB };
			add_child_with_quality(&family, &account(116, i), quality(bytes, 0, 1000));
		}
		for (f, w) in Arion::<Runtime>::active_family_weights() {
			assert!(w > 0, "family {f:?} returned with zero weight");
		}
	});
}

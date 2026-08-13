//! Tests for the aggregate family weight formula.
//!
//! The family score is computed from the AGGREGATE bytes of its fresh Active
//! children (not a sum of per-node scores), normalized over the useful range
//! [FamilyScoreFloorBytes, FamilyScoreCeilBytes] and capped at MaxFamilyScore
//! (< MaxFamilyWeight). Assertions read `FamilyWeightRaw` — the raw computed
//! value — because `FamilyWeight` is EMA-smoothed and delta-clamped.

use frame_support::assert_ok;
use hippius_mainnet_runtime::{AccountId, Arion, Runtime, RuntimeOrigin, System};
use pallet_arion::{
	ChildRegistration, ChildRegistrations, ChildStatus, CurrentWeightBucket, FamilyActiveChildren,
	FamilyChildren, FamilyWeightRaw, NodeQuality, NodeQualityByChild, NodeWeightLastBucket,
};
use sp_core::crypto::Ss58Codec;
use sp_runtime::{AccountId32, BuildStorage};

const BUCKET: u32 = 100;
const TIB: u128 = 1024 * 1024 * 1024 * 1024;

/// Whitelisted `ArionAdminMembers` account (WeightAuthorityOrigin).
fn admin() -> AccountId {
	AccountId32::from_ss58check("5CVXqxb7mhFTtZVw5BJ8M2ujND9PFymSDxF8bkod6Sm4XJTW").unwrap()
}

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

/// Register a child under a family and add it to the family index.
fn add_child(family: &AccountId, child: &AccountId) {
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
		v.try_push(child.clone()).expect("under MaxChildrenPerFamily");
	});
	FamilyActiveChildren::<Runtime>::mutate(family, |n| *n += 1);
}

/// Submit quality for the given (child, quality) pairs and trigger the recompute.
fn submit(pairs: Vec<(AccountId, NodeQuality)>) {
	let updates: Vec<(AccountId, NodeQuality)> = pairs;
	assert_ok!(Arion::submit_node_quality(
		RuntimeOrigin::signed(admin()),
		BUCKET,
		updates.try_into().expect("under MaxNodeWeightUpdates"),
	));
}

fn raw(family: &AccountId) -> u16 {
	FamilyWeightRaw::<Runtime>::get(family)
}

#[test]
fn fragmentation_is_neutral() {
	new_test_ext().execute_with(|| {
		// Family A: one node with 4 TiB. Family B: four nodes with 1 TiB each.
		let fam_a = account(1, 0);
		let fam_b = account(2, 0);
		let a1 = account(1, 10);
		add_child(&fam_a, &a1);
		let mut pairs = vec![(a1, quality(4 * TIB, 0, 900))];
		for i in 0..4u8 {
			let c = account(2, 10 + i);
			add_child(&fam_b, &c);
			pairs.push((c, quality(TIB, 0, 900)));
		}
		submit(pairs);

		assert_eq!(raw(&fam_a), raw(&fam_b), "same total bytes must score identically");
		assert!(raw(&fam_a) > 0);
	});
}

#[test]
fn strictly_monotonic_in_bytes() {
	new_test_ext().execute_with(|| {
		let sizes = [TIB, 10 * TIB, 100 * TIB];
		let mut pairs = Vec::new();
		let mut fams = Vec::new();
		for (i, sz) in sizes.iter().enumerate() {
			let fam = account(10 + i as u8, 0);
			let c = account(10 + i as u8, 1);
			add_child(&fam, &c);
			pairs.push((c, quality(*sz, 0, 1000)));
			fams.push(fam);
		}
		submit(pairs);

		let (w1, w10, w100) = (raw(&fams[0]), raw(&fams[1]), raw(&fams[2]));
		assert!(w1 < w10 && w10 < w100, "ranking must exist: {w1} < {w10} < {w100}");
	});
}

#[test]
fn never_reaches_the_type_ceiling() {
	new_test_ext().execute_with(|| {
		// 10 children of 100 TiB (~1 PiB aggregate = the calibration ceiling),
		// maximum bandwidth, perfect uptime.
		let fam = account(20, 0);
		let mut pairs = Vec::new();
		for i in 0..10u8 {
			let c = account(20, 10 + i);
			add_child(&fam, &c);
			pairs.push((c, quality(100 * TIB, 100 * TIB, 1000)));
		}
		submit(pairs);

		let w = raw(&fam);
		let max_score = 60_000u16; // runtime MaxFamilyScore
		assert!(w <= max_score, "raw {w} must not exceed MaxFamilyScore");
		assert!(w < u16::MAX, "families must never sit at the u16 ceiling");
		assert!(w > 50_000, "a ceiling-range family should still score near the top: {w}");
	});
}

#[test]
fn absurd_inputs_do_not_overflow() {
	new_test_ext().execute_with(|| {
		// u128::MAX everywhere: must not panic, must stay bounded.
		let fam = account(30, 0);
		let mut pairs = Vec::new();
		for i in 0..3u8 {
			let c = account(30, 10 + i);
			add_child(&fam, &c);
			pairs.push((
				c,
				NodeQuality {
					shard_data_bytes: u128::MAX,
					bandwidth_bytes: u128::MAX,
					uptime_permille: u16::MAX, // > 1000, must be clamped
					strikes: 0,
					integrity_fails: 0,
				},
			));
		}
		submit(pairs);
		assert!(raw(&fam) <= 60_000);

		// Max penalties: must saturate to zero, not underflow.
		let fam2 = account(31, 0);
		let c2 = account(31, 10);
		add_child(&fam2, &c2);
		submit(vec![(
			c2,
			NodeQuality {
				shard_data_bytes: 10 * TIB,
				bandwidth_bytes: 0,
				uptime_permille: 1000,
				strikes: u32::MAX,
				integrity_fails: u32::MAX,
			},
		)]);
		assert_eq!(raw(&fam2), 0);
	});
}

#[test]
fn dust_children_add_nothing() {
	new_test_ext().execute_with(|| {
		let fam_solo = account(40, 0);
		let solo = account(40, 10);
		add_child(&fam_solo, &solo);
		let mut pairs = vec![(solo, quality(10 * TIB, 0, 900))];

		// Same bytes plus 30 dust children of 1 byte each.
		let fam_dust = account(41, 0);
		let big = account(41, 10);
		add_child(&fam_dust, &big);
		pairs.push((big, quality(10 * TIB, 0, 900)));
		for i in 0..30u8 {
			let c = account(41, 50 + i);
			add_child(&fam_dust, &c);
			pairs.push((c, quality(1, 0, 900)));
		}
		submit(pairs);

		let diff = raw(&fam_dust).abs_diff(raw(&fam_solo));
		assert!(diff <= 1, "30 dust children changed the score by {diff}");
	});
}

#[test]
fn stale_children_stop_counting() {
	new_test_ext().execute_with(|| {
		// Family with one fresh and one stale child of equal size: the stale one
		// must contribute nothing, so the family scores like the fresh child alone.
		let fam = account(50, 0);
		let fresh = account(50, 10);
		let stale = account(50, 11);
		add_child(&fam, &fresh);
		add_child(&fam, &stale);
		// Stale child: quality present but last refreshed 10 buckets ago (> StaleChildBuckets=4).
		NodeQualityByChild::<Runtime>::insert(&stale, quality(10 * TIB, 0, 1000));
		NodeWeightLastBucket::<Runtime>::insert(&stale, BUCKET - 10);

		let fam_ref = account(51, 0);
		let only = account(51, 10);
		add_child(&fam_ref, &only);

		submit(vec![
			(fresh, quality(10 * TIB, 0, 1000)),
			(only, quality(10 * TIB, 0, 1000)),
		]);

		assert_eq!(raw(&fam), raw(&fam_ref), "stale child must contribute zero");

		// Family whose ONLY child went stale: raw collapses to 0.
		let fam_dead = account(52, 0);
		let dead = account(52, 10);
		add_child(&fam_dead, &dead);
		NodeQualityByChild::<Runtime>::insert(&dead, quality(10 * TIB, 0, 1000));
		NodeWeightLastBucket::<Runtime>::insert(&dead, BUCKET - 10);
		// Trigger a recompute via an unrelated submission.
		submit(vec![(account(51, 10), quality(10 * TIB, 0, 1000))]);
		assert_eq!(raw(&fam_dead), 0);
	});
}

#[test]
fn uptime_scales_the_score() {
	new_test_ext().execute_with(|| {
		let fam_hi = account(60, 0);
		let hi = account(60, 10);
		add_child(&fam_hi, &hi);
		let fam_lo = account(61, 0);
		let lo = account(61, 10);
		add_child(&fam_lo, &lo);
		submit(vec![
			(hi, quality(10 * TIB, 0, 1000)),
			(lo, quality(10 * TIB, 0, 500)),
		]);
		let (whi, wlo) = (raw(&fam_hi) as u32, raw(&fam_lo) as u32);
		assert!(wlo * 2 >= whi.saturating_sub(2) && wlo * 2 <= whi + 2,
			"uptime 500 should halve the score: hi={whi} lo={wlo}");
	});
}

#[test]
fn below_floor_scores_zero() {
	new_test_ext().execute_with(|| {
		// 10 GiB total is below the 100 GiB floor: no score.
		let fam = account(70, 0);
		let c = account(70, 10);
		add_child(&fam, &c);
		submit(vec![(c, quality(10 * 1024 * 1024 * 1024, 0, 1000))]);
		assert_eq!(raw(&fam), 0);
	});
}

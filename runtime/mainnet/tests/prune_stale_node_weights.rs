//! Tests for `Arion::prune_stale_node_weights`.
//!
//! The extrinsic removes per-child weight/quality entries left behind by
//! children that are no longer `Active` (legacy deregistrations predating
//! `remove_child_node_weight_entries`, expired unbondings), plus family
//! weight entries of families with no active children left.

use frame_support::{assert_noop, assert_ok};
use hippius_mainnet_runtime::{AccountId, Arion, Runtime, RuntimeEvent, RuntimeOrigin, System};
use pallet_arion::{
	ChildRegistration, ChildRegistrations, ChildStatus, CurrentWeightBucket, FamilyActiveChildren,
	FamilyFirstSeenBucket, FamilyWeight, FamilyWeightRaw, NodeQuality, NodeQualityByChild,
	NodeWeightByChild, NodeWeightLastBucket,
};
use sp_core::crypto::Ss58Codec;
use sp_runtime::{AccountId32, BuildStorage};

const CURRENT_BUCKET: u32 = 23_000;

/// Whitelisted `ArionAdminMembers` account (WeightAuthorityOrigin).
fn admin() -> AccountId {
	AccountId32::from_ss58check("5CVXqxb7mhFTtZVw5BJ8M2ujND9PFymSDxF8bkod6Sm4XJTW").unwrap()
}

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| {
		System::set_block_number(1);
		CurrentWeightBucket::<Runtime>::put(CURRENT_BUCKET);
	});
	ext
}

fn quality(uptime: u16) -> NodeQuality {
	NodeQuality {
		shard_data_bytes: 1,
		bandwidth_bytes: 1,
		uptime_permille: uptime,
		strikes: 0,
		integrity_fails: 0,
	}
}

/// Seed the weight trio for a child, `age` buckets behind the current bucket.
fn seed_weight(child: &AccountId, weight: u16, age: u32) {
	NodeWeightByChild::<Runtime>::insert(child, weight);
	NodeWeightLastBucket::<Runtime>::insert(child, CURRENT_BUCKET.saturating_sub(age));
	NodeQualityByChild::<Runtime>::insert(child, quality(900));
}

fn register(child: &AccountId, family: &AccountId, status: ChildStatus) {
	let node_id: [u8; 32] = child.clone().into();
	ChildRegistrations::<Runtime>::insert(
		child,
		ChildRegistration {
			family: family.clone(),
			node_id,
			status,
			deposit: 0u128,
			unbonding_end: 0u32.into(),
		},
	);
}

fn last_prune_event() -> Option<(u32, u32)> {
	System::events().iter().rev().find_map(|r| match &r.event {
		RuntimeEvent::Arion(pallet_arion::Event::StaleNodeWeightsPruned {
			children_pruned,
			families_pruned,
		}) => Some((*children_pruned, *families_pruned)),
		_ => None,
	})
}

#[test]
fn prunes_stale_children_without_active_registration() {
	new_test_ext().execute_with(|| {
		let family = account(1);
		// Legacy fossil: weight entries but no registration at all.
		let ghost = account(10);
		seed_weight(&ghost, 400, 500);
		// Expired unbonding: registration present but not Active.
		let unbonding = account(11);
		seed_weight(&unbonding, 300, 500);
		register(&unbonding, &family, ChildStatus::Unbonding);

		assert_ok!(Arion::prune_stale_node_weights(RuntimeOrigin::signed(admin()), 4, 100, 1000));

		for child in [&ghost, &unbonding] {
			assert!(!NodeWeightByChild::<Runtime>::contains_key(child));
			assert!(!NodeWeightLastBucket::<Runtime>::contains_key(child));
			assert!(!NodeQualityByChild::<Runtime>::contains_key(child));
		}
		assert_eq!(last_prune_event(), Some((2, 1)));
	});
}

#[test]
fn keeps_live_and_active_children() {
	new_test_ext().execute_with(|| {
		let family = account(1);
		// Live child: refreshed this bucket (no registration needed to be safe).
		let live = account(20);
		seed_weight(&live, 500, 0);
		// Stale but still Active: quality path owns it, prune must not touch it.
		let stale_active = account(21);
		seed_weight(&stale_active, 200, 500);
		register(&stale_active, &family, ChildStatus::Active);
		FamilyActiveChildren::<Runtime>::insert(&family, 1u32);

		assert_ok!(Arion::prune_stale_node_weights(RuntimeOrigin::signed(admin()), 4, 100, 1000));

		assert_eq!(NodeWeightByChild::<Runtime>::get(&live), 500);
		assert_eq!(NodeWeightByChild::<Runtime>::get(&stale_active), 200);
		assert!(last_prune_event().is_none());
	});
}

#[test]
fn missing_last_bucket_counts_as_stale() {
	new_test_ext().execute_with(|| {
		let orphan = account(30);
		// Weight + quality but no NodeWeightLastBucket entry (decodes as 0).
		NodeWeightByChild::<Runtime>::insert(&orphan, 123u16);
		NodeQualityByChild::<Runtime>::insert(&orphan, quality(500));

		assert_ok!(Arion::prune_stale_node_weights(RuntimeOrigin::signed(admin()), 4, 100, 1000));

		assert!(!NodeWeightByChild::<Runtime>::contains_key(&orphan));
		assert!(!NodeQualityByChild::<Runtime>::contains_key(&orphan));
		assert_eq!(last_prune_event(), Some((1, 0)));
	});
}

#[test]
fn family_sweep_only_when_no_active_children() {
	new_test_ext().execute_with(|| {
		// Family A: its only child is Unbonding -> family weights swept.
		let fam_a = account(1);
		let child_a = account(40);
		seed_weight(&child_a, 300, 500);
		register(&child_a, &fam_a, ChildStatus::Unbonding);
		FamilyActiveChildren::<Runtime>::insert(&fam_a, 0u32);
		FamilyWeight::<Runtime>::insert(&fam_a, 50u16);
		FamilyWeightRaw::<Runtime>::insert(&fam_a, 50u16);
		FamilyFirstSeenBucket::<Runtime>::insert(&fam_a, 1u32);

		// Family B: one stale Unbonding child pruned, but another Active child
		// remains -> family weights must be kept.
		let fam_b = account(2);
		let child_b_dead = account(41);
		seed_weight(&child_b_dead, 300, 500);
		register(&child_b_dead, &fam_b, ChildStatus::Unbonding);
		FamilyActiveChildren::<Runtime>::insert(&fam_b, 1u32);
		FamilyWeight::<Runtime>::insert(&fam_b, 60u16);
		FamilyWeightRaw::<Runtime>::insert(&fam_b, 60u16);

		assert_ok!(Arion::prune_stale_node_weights(RuntimeOrigin::signed(admin()), 4, 100, 1000));

		assert!(!FamilyWeight::<Runtime>::contains_key(&fam_a));
		assert!(!FamilyWeightRaw::<Runtime>::contains_key(&fam_a));
		assert!(!FamilyFirstSeenBucket::<Runtime>::contains_key(&fam_a));
		assert_eq!(FamilyWeight::<Runtime>::get(&fam_b), 60);
		assert_eq!(FamilyWeightRaw::<Runtime>::get(&fam_b), 60);
		assert_eq!(last_prune_event(), Some((2, 1)));
	});
}

#[test]
fn respects_max_children_cap() {
	new_test_ext().execute_with(|| {
		for seed in 50u8..60u8 {
			seed_weight(&account(seed), 100, 500);
		}

		assert_ok!(Arion::prune_stale_node_weights(RuntimeOrigin::signed(admin()), 4, 3, 1000));
		assert_eq!(last_prune_event(), Some((3, 0)));

		let remaining =
			(50u8..60u8).filter(|s| NodeWeightByChild::<Runtime>::contains_key(&account(*s))).count();
		assert_eq!(remaining, 7);

		// A second call keeps draining the backlog.
		assert_ok!(Arion::prune_stale_node_weights(RuntimeOrigin::signed(admin()), 4, 100, 1000));
		assert_eq!(last_prune_event(), Some((7, 0)));
	});
}

#[test]
fn rejects_non_authority_origin() {
	new_test_ext().execute_with(|| {
		seed_weight(&account(70), 100, 500);
		assert_noop!(
			Arion::prune_stale_node_weights(RuntimeOrigin::signed(account(99)), 4, 100, 1000),
			sp_runtime::DispatchError::BadOrigin
		);
		assert_noop!(
			Arion::prune_stale_node_weights(RuntimeOrigin::root(), 4, 100, 1000),
			sp_runtime::DispatchError::BadOrigin
		);
	});
}

#[test]
fn validates_batch_and_staleness_params() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Arion::prune_stale_node_weights(RuntimeOrigin::signed(admin()), 4, 0, 1000),
			pallet_arion::Error::<Runtime>::InvalidNodeWeightPruneBatch
		);
		// MaxNodeWeightPrunePerCall is 200 on mainnet.
		assert_noop!(
			Arion::prune_stale_node_weights(RuntimeOrigin::signed(admin()), 4, 201, 1000),
			pallet_arion::Error::<Runtime>::InvalidNodeWeightPruneBatch
		);
		// Staleness floor of 2 buckets protects live children.
		assert_noop!(
			Arion::prune_stale_node_weights(RuntimeOrigin::signed(admin()), 1, 100, 1000),
			pallet_arion::Error::<Runtime>::InvalidNodeWeightPruneBatch
		);
		// max_scan must cover max_children and respect MaxNodeWeightScanPerCall (2000).
		assert_noop!(
			Arion::prune_stale_node_weights(RuntimeOrigin::signed(admin()), 4, 100, 50),
			pallet_arion::Error::<Runtime>::InvalidNodeWeightPruneBatch
		);
		assert_noop!(
			Arion::prune_stale_node_weights(RuntimeOrigin::signed(admin()), 4, 100, 2001),
			pallet_arion::Error::<Runtime>::InvalidNodeWeightPruneBatch
		);
	});
}

#[test]
fn bounded_scan_pages_through_live_entries() {
	new_test_ext().execute_with(|| {
		use pallet_arion::NodeWeightPruneCursor;
		// 12 LIVE children (fresh last_bucket, Active): nothing prunable, but the
		// walk must stay bounded by max_scan and resume via the cursor.
		let fam = account(80);
		for seed in 0u8..12u8 {
			let c = AccountId32::new([seed.wrapping_add(100); 32]);
			seed_weight(&c, 100, 0);
			ChildRegistrations::<Runtime>::insert(
				&c,
				ChildRegistration {
					family: fam.clone(),
					node_id: [seed; 32],
					status: ChildStatus::Active,
					deposit: 0u128,
					unbonding_end: 0u32.into(),
				},
			);
		}
		// One stale ghost parked at the end of the walk (position depends on key
		// hash; the three paged calls below must find it wherever it lands).
		let ghost = account(81);
		seed_weight(&ghost, 400, 500);

		assert!(NodeWeightPruneCursor::<Runtime>::get().is_none());
		// Page 1: scan 5 of 13 entries.
		assert_ok!(Arion::prune_stale_node_weights(RuntimeOrigin::signed(admin()), 4, 4, 5));
		assert!(NodeWeightPruneCursor::<Runtime>::get().is_some(), "cursor must park mid-map");
		// Pages 2..: finish the map; cursor resets at the end.
		for _ in 0..4 {
			assert_ok!(Arion::prune_stale_node_weights(RuntimeOrigin::signed(admin()), 4, 4, 5));
			if NodeWeightPruneCursor::<Runtime>::get().is_none() {
				break;
			}
		}
		assert!(NodeWeightPruneCursor::<Runtime>::get().is_none(), "cursor must reset after full pass");
		assert!(!NodeWeightByChild::<Runtime>::contains_key(&ghost), "ghost pruned during paging");
		// Live children untouched.
		let live_left = (0u8..12u8)
			.filter(|s| NodeWeightByChild::<Runtime>::contains_key(&AccountId32::new([s.wrapping_add(100); 32])))
			.count();
		assert_eq!(live_left, 12);
	});
}

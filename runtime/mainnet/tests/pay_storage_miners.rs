//! End-to-end: emission deposited into the bank is distributed to **Arion
//! families** by `pay_storage_miners`, each family's weight being the sum of
//! the node weights of its eligible children; and the emission compartment is
//! invisible to the arion settlement headroom.
//!
//! These tests read real Arion storage through the runtime's
//! `ArionFamilyWeightsSource` adapter — the payout set is `FamilyChildren` +
//! `NodeWeightByChild`, NOT `RankingStorage`. Paying from the ranking pallet
//! was the production bug: Arion operators never appear in that leaderboard,
//! so registered families carrying real weight received nothing.

use frame_support::traits::{Currency, Get};
use frame_support::{assert_noop, assert_ok};
use hippius_mainnet_runtime::{
	AccountId, ArionPayoutSource, Balances, Hippocampus, Runtime, RuntimeOrigin, System,
};
use pallet_arion::{
	ChildRegistration, ChildRegistrations, ChildStatus, CurrentWeightBucket, FamilyActiveChildren,
	FamilyChildren, NodeWeightByChild, NodeWeightLastBucket, PayoutSource,
};
use pallet_hippocampus::DepositType;
use pallet_metagraph::{Role, UID};
use sp_core::crypto::Ss58Codec;
use sp_runtime::{AccountId32, BuildStorage};

/// Must match `ExistentialDeposit` in the runtime.
const ED: u128 = 500;

/// Bucket every fresh weight is reported in.
const BUCKET: u32 = 100;

/// The hardcoded `ArionAdminMembers` account (all arion/bank admin origins).
fn admin() -> AccountId {
	AccountId32::from_ss58check("5CVXqxb7mhFTtZVw5BJ8M2ujND9PFymSDxF8bkod6Sm4XJTW").unwrap()
}

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

/// A child account distinct from any family account seed.
fn child(seed: u8) -> AccountId {
	let mut raw = [seed; 32];
	raw[0] = 0xC0;
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

/// Register `child` under `family` with the given status, and give it a node
/// weight last reported `age` buckets ago. Seeds storage directly, the same
/// way family_weight_aggregate.rs and miner_payments.rs do.
fn seed_child(family: &AccountId, child: &AccountId, weight: u16, status: ChildStatus, age: u32) {
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
	FamilyChildren::<Runtime>::mutate(family, |v| {
		v.try_push(child.clone()).expect("under MaxChildrenPerFamily");
	});
	if status == ChildStatus::Active {
		FamilyActiveChildren::<Runtime>::mutate(family, |n| *n += 1);
	}
	NodeWeightByChild::<Runtime>::insert(child, weight);
	NodeWeightLastBucket::<Runtime>::insert(child, BUCKET.saturating_sub(age));
}

/// The common case: an Active child whose weight was reported this bucket.
fn seed_active_child(family: &AccountId, child: &AccountId, weight: u16) {
	seed_child(family, child, weight, ChildStatus::Active, 0);
}

/// Fund the bank with an ED cushion plus `emission` tagged as Emission, and
/// whitelist the admin account as the payout caller.
fn fund_bank_and_whitelist_admin(emission: u128) {
	let funder = account(1);
	Balances::make_free_balance_be(&funder, emission + 1_000_000);

	assert_ok!(Hippocampus::deposit(
		RuntimeOrigin::signed(funder.clone()),
		ED * 2,
		DepositType::Grant
	));
	assert_ok!(Hippocampus::deposit(
		RuntimeOrigin::signed(funder),
		emission,
		DepositType::Emission
	));
	// AdminOrigin is EnsureSignedBy<ArionAdminMembers>, not root.
	assert_ok!(Hippocampus::add_miner_payment_caller(RuntimeOrigin::signed(admin()), admin()));
}

#[test]
fn emission_compartment_invisible_to_arion_settlement() {
	new_test_ext().execute_with(|| {
		let funder = account(1);
		Balances::make_free_balance_be(&funder, 1_000_000);

		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(funder.clone()),
			ED * 2,
			DepositType::Grant
		));
		let arion_headroom_before = ArionPayoutSource::available();
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(funder),
			100_000,
			DepositType::Emission
		));

		// Wall: emission is invisible to the arion settlement headroom.
		assert_eq!(ArionPayoutSource::available(), arion_headroom_before);
		assert_eq!(Hippocampus::emission_available(), 100_000);
	});
}

#[test]
fn pay_storage_miners_with_no_registered_families() {
	new_test_ext().execute_with(|| {
		fund_bank_and_whitelist_admin(100_000);

		assert_noop!(
			Hippocampus::pay_storage_miners(RuntimeOrigin::signed(admin()), 100_000),
			pallet_hippocampus::Error::<Runtime>::NoEligibleMiners
		);
		assert_eq!(Hippocampus::emission_available(), 100_000);
	});
}

#[test]
fn pay_storage_miners_distributes_to_arion_families() {
	// The regression test for the production bug: a family registered in Arion
	// with weight on its children gets paid, and gets paid in proportion.
	new_test_ext().execute_with(|| {
		let (fam_a, fam_b) = (account(2), account(3));
		fund_bank_and_whitelist_admin(100_000);

		seed_active_child(&fam_a, &child(10), 100);
		seed_active_child(&fam_b, &child(11), 300);

		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(admin()), 100_000));

		assert_eq!(Balances::free_balance(&fam_a), 25_000);
		assert_eq!(Balances::free_balance(&fam_b), 75_000);
		assert_eq!(Hippocampus::emission_available(), 0);
	});
}

#[test]
fn a_families_share_is_the_sum_of_its_childrens_weights() {
	// fam_a runs three nodes, fam_b one. The split follows summed weight
	// (300 vs 100), and fam_a receives ONE transfer, not three.
	new_test_ext().execute_with(|| {
		let (fam_a, fam_b) = (account(2), account(3));
		fund_bank_and_whitelist_admin(100_000);

		seed_active_child(&fam_a, &child(10), 100);
		seed_active_child(&fam_a, &child(11), 150);
		seed_active_child(&fam_a, &child(12), 50);
		seed_active_child(&fam_b, &child(13), 100);

		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(admin()), 100_000));

		assert_eq!(Balances::free_balance(&fam_a), 75_000);
		assert_eq!(Balances::free_balance(&fam_b), 25_000);
		// Children are not payees — only the family account that put up the
		// deposits receives emission.
		for seed in [10u8, 11, 12, 13] {
			assert_eq!(Balances::free_balance(&child(seed)), 0, "child {seed} was paid");
		}
	});
}

#[test]
fn the_heaviest_family_takes_the_largest_share() {
	// Three families, strictly ordered by summed child weight: the payout
	// must preserve that order.
	new_test_ext().execute_with(|| {
		let (heavy, middle, light) = (account(2), account(3), account(4));
		fund_bank_and_whitelist_admin(100_000);

		// 30_000 / 15_000 / 5_000 — family totals well past u16::MAX territory
		// once summed, which is what the u128 weight width is for.
		seed_active_child(&heavy, &child(10), 15_000);
		seed_active_child(&heavy, &child(11), 15_000);
		seed_active_child(&middle, &child(12), 15_000);
		seed_active_child(&light, &child(13), 5_000);

		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(admin()), 100_000));

		let (h, m, l) = (
			Balances::free_balance(&heavy),
			Balances::free_balance(&middle),
			Balances::free_balance(&light),
		);
		assert_eq!(h, 60_000);
		assert_eq!(m, 30_000);
		assert_eq!(l, 10_000);
		assert!(h > m && m > l);
	});
}

#[test]
fn family_weights_beyond_u16_are_not_flattened() {
	// 35 children (MaxChildrenPerFamily) at MaxNodeWeight sums to 1_750_000 —
	// 26x u16::MAX. A u16 accumulator would have clamped this family to 65_535
	// and handed it a 1:1 split against a family a quarter its size instead of
	// the 4:1 it earned.
	new_test_ext().execute_with(|| {
		let (big, small) = (account(2), account(3));
		fund_bank_and_whitelist_admin(100_000);

		let max_node_weight: u16 = <Runtime as pallet_arion::Config>::MaxNodeWeight::get();
		for i in 0..35u8 {
			seed_active_child(&big, &child(10 + i), max_node_weight);
		}
		for i in 0..9u8 {
			seed_active_child(&small, &child(100 + i), max_node_weight);
		}
		// 35 : 9 — deliberately not a round ratio, so a clamped weight cannot
		// coincidentally produce the same split.
		let weights = pallet_arion::Pallet::<Runtime>::active_family_weights();
		let of = |f: &AccountId| weights.iter().find(|(a, _)| a == f).unwrap().1;
		assert_eq!(of(&big), 35 * u128::from(max_node_weight));
		assert!(of(&big) > u128::from(u16::MAX));

		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(admin()), 100_000));

		assert_eq!(Balances::free_balance(&big), 100_000 * 35 / 44);
		assert_eq!(Balances::free_balance(&small), 100_000 * 9 / 44);
	});
}

#[test]
fn unbonding_children_neither_earn_nor_dilute() {
	// A child that deregistered is in Unbonding with its deposit on the way
	// out: it has nothing at risk, so its weight must not be paid for — and
	// must not shrink everyone else's share either.
	new_test_ext().execute_with(|| {
		let (fam_a, exiting) = (account(2), account(3));
		fund_bank_and_whitelist_admin(100_000);

		seed_active_child(&fam_a, &child(10), 100);
		seed_child(&exiting, &child(11), 300, ChildStatus::Unbonding, 0);

		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(admin()), 100_000));

		assert_eq!(Balances::free_balance(&exiting), 0);
		// The whole payout, not 25% of it: the excluded weight is out of the
		// denominator too.
		assert_eq!(Balances::free_balance(&fam_a), 100_000);
	});
}

#[test]
fn a_family_whose_children_all_went_stale_is_excluded() {
	// `NodeWeightByChild` keeps a node's last value until pruning gets to it.
	// Without the freshness filter an offline node would keep collecting on
	// its final good weight forever.
	new_test_ext().execute_with(|| {
		let (fresh, stale) = (account(2), account(3));
		fund_bank_and_whitelist_admin(100_000);

		let stale_after: u32 = <Runtime as pallet_arion::Config>::StaleChildBuckets::get();
		seed_active_child(&fresh, &child(10), 100);
		// One bucket past the staleness horizon.
		seed_child(&stale, &child(11), 300, ChildStatus::Active, stale_after + 1);

		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(admin()), 100_000));

		assert_eq!(Balances::free_balance(&stale), 0);
		assert_eq!(Balances::free_balance(&fresh), 100_000);
	});
}

#[test]
fn a_child_exactly_at_the_staleness_horizon_still_earns() {
	// The boundary is inclusive: `age == StaleChildBuckets` is still fresh,
	// matching the family-scoring filter. An off-by-one here would silently
	// stop paying every node reporting on the slowest allowed cadence.
	new_test_ext().execute_with(|| {
		let (borderline, fresh) = (account(2), account(3));
		fund_bank_and_whitelist_admin(100_000);

		let stale_after: u32 = <Runtime as pallet_arion::Config>::StaleChildBuckets::get();
		seed_child(&borderline, &child(10), 100, ChildStatus::Active, stale_after);
		seed_active_child(&fresh, &child(11), 100);

		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(admin()), 100_000));

		assert_eq!(Balances::free_balance(&borderline), 50_000);
		assert_eq!(Balances::free_balance(&fresh), 50_000);
	});
}

#[test]
fn unweighted_families_are_dropped_not_paid_zero() {
	// A registered family whose children have never been scored contributes
	// nothing, and must not occupy one of the `MaxMinersPerPayout` slots.
	new_test_ext().execute_with(|| {
		let (earning, unscored) = (account(2), account(3));
		fund_bank_and_whitelist_admin(100_000);

		seed_active_child(&earning, &child(10), 100);
		seed_active_child(&unscored, &child(11), 0);

		let weights = pallet_arion::Pallet::<Runtime>::active_family_weights();
		assert_eq!(weights.len(), 1, "zero-weight family should not occupy a payout slot");

		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(admin()), 100_000));
		assert_eq!(Balances::free_balance(&earning), 100_000);
		assert_eq!(Balances::free_balance(&unscored), 0);
	});
}

#[test]
fn a_family_is_not_paid_for_a_child_registered_to_someone_else() {
	// `FamilyChildren` and `ChildRegistrations` are two indexes of one fact.
	// The registration is authoritative: if a family's child list names a
	// child whose registration points at a different family, that child's
	// weight belongs to the other family and must not be credited here.
	new_test_ext().execute_with(|| {
		let (thief, owner) = (account(2), account(3));
		fund_bank_and_whitelist_admin(100_000);

		seed_active_child(&thief, &child(10), 100);
		seed_active_child(&owner, &child(11), 300);
		// Splice the owner's child into the thief's list without touching the
		// child's registration.
		pallet_arion::FamilyChildren::<Runtime>::mutate(&thief, |v| {
			v.try_push(child(11)).expect("under MaxChildrenPerFamily");
		});

		let weights = pallet_arion::Pallet::<Runtime>::active_family_weights();
		let of = |f: &AccountId| weights.iter().find(|(a, _)| a == f).map(|(_, w)| *w);
		assert_eq!(of(&thief), Some(100), "thief was credited for a child it does not own");
		assert_eq!(of(&owner), Some(300));

		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(admin()), 100_000));

		assert_eq!(Balances::free_balance(&thief), 25_000);
		assert_eq!(Balances::free_balance(&owner), 75_000);
	});
}

#[test]
fn pay_storage_miners_excludes_uid_238_account() {
	new_test_ext().execute_with(|| {
		let (fam_a, capture) = (account(2), account(5));
		fund_bank_and_whitelist_admin(100_000);

		seed_active_child(&fam_a, &child(10), 100);
		seed_active_child(&capture, &child(13), 300);

		// Mark `capture` as the uid-238 (emission capture) account on the
		// metagraph: it must be excluded and not dilute the family split.
		pallet_metagraph::UIDs::<Runtime>::put(vec![UID {
			address: sp_core::sr25519::Public::from_raw([0u8; 32]),
			id: 238,
			role: Role::Validator,
			substrate_address: capture.clone(),
		}]);

		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(admin()), 100_000));

		assert_eq!(Balances::free_balance(&capture), 0);
		assert_eq!(Balances::free_balance(&fam_a), 100_000);
		assert_eq!(Hippocampus::emission_available(), 0);
	});
}

#[test]
fn the_ranking_pallet_no_longer_decides_who_gets_paid() {
	// Explicit guard against a regression back to `RankingStorage`: a node
	// sitting at the top of the storage ranking that is not an Arion family
	// receives nothing, and an Arion family absent from the ranking is paid
	// in full. If someone re-points the adapter at the ranking pallet, this
	// test — not testnet — is where it fails.
	new_test_ext().execute_with(|| {
		let (family, ranked_only) = (account(2), account(6));
		fund_bank_and_whitelist_admin(100_000);

		seed_active_child(&family, &child(10), 100);

		let node_id = vec![14u8; 32];
		pallet_registration::ColdkeyNodeRegistrationV2::<Runtime>::insert(
			node_id.clone(),
			Some(pallet_registration::ColdkeyNodeInfoLite {
				node_id: node_id.clone(),
				node_type: pallet_registration::NodeType::StorageMiner,
				status: pallet_registration::Status::Online,
				registered_at: 0u32.into(),
				owner: ranked_only.clone(),
			}),
		);
		let mut list = pallet_rankings::RankedList::<Runtime>::get();
		list.push(pallet_rankings::NodeRankings {
			rank: 1,
			node_id,
			node_ss58_address: ranked_only.to_ss58check().into_bytes(),
			node_type: pallet_registration::NodeType::StorageMiner,
			weight: 60_000,
			last_updated: 0u32.into(),
			is_active: true,
		});
		pallet_rankings::RankedList::<Runtime>::put(list);

		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(admin()), 100_000));

		assert_eq!(Balances::free_balance(&ranked_only), 0);
		assert_eq!(Balances::free_balance(&family), 100_000);
	});
}

#[test]
fn the_payout_slot_bound_covers_every_family_that_can_exist() {
	// `pay_storage_miners` rejects the WHOLE call when the payee set exceeds
	// `MaxMinersPerPayout`, so a bound below the reachable family count is not
	// a degraded payout — it is a total, silent halt of storage emission, with
	// every missed day unrecoverable under the 24-hour cap.
	//
	// The reachable count is `MaxChildrenTotal`, not `MaxFamilies`: a family
	// needs one Active child to carry weight, and `register_child` caps
	// `TotalActiveChildren` unconditionally, whereas `MaxFamilies` is checked
	// only on a family's first fee-free slot.
	let max_payees: u32 = <Runtime as pallet_hippocampus::Config>::MaxMinersPerPayout::get();
	let max_families: u32 = <Runtime as pallet_arion::Config>::MaxChildrenTotal::get();
	assert!(
		max_payees >= max_families,
		"MaxMinersPerPayout ({max_payees}) must cover every family that can hold \
		 an active child ({max_families}), or pay_storage_miners bricks entirely"
	);
}

#[test]
fn daily_miner_payout_cap_is_3500_alpha() {
	// Guards the 18-decimals scaling of the cap: the constant originally
	// shipped 1000x too small (3 alpha instead of 3000). If the intended
	// daily cap changes, change this test together with the runtime value.
	//
	// 3500 is the COMBINED ceiling for `pay_storage_miners` +
	// `pay_compute_miners`, not a per-payout allowance. Adding a consumer to
	// the shared counter is not a reason to raise it.
	const UNIT: u128 = 1_000_000_000_000_000_000;
	assert_eq!(<Runtime as pallet_hippocampus::Config>::Max24HourMinerPayout::get(), 3_500 * UNIT);
}

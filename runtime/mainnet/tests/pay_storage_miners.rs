//! End-to-end: emission deposited into the bank is distributed to ranked
//! storage-miner owners by `pay_storage_miners`, the runtime's ranking
//! source resolves owners and applies the uid-238/activity filters, and the
//! emission compartment is invisible to the arion settlement headroom.

use frame_support::traits::{Currency, Get};
use frame_support::{assert_noop, assert_ok};
use hippius_mainnet_runtime::{
	AccountId, ArionPayoutSource, Balances, Hippocampus, Runtime, RuntimeOrigin, System,
};
use pallet_arion::PayoutSource;
use pallet_hippocampus::DepositType;
use pallet_metagraph::{Role, UID};
use pallet_registration::{ColdkeyNodeInfoLite, NodeType, Status};
use sp_core::crypto::Ss58Codec;
use sp_runtime::{AccountId32, BuildStorage};

/// Must match `ExistentialDeposit` in the runtime.
const ED: u128 = 500;

/// The hardcoded `ArionAdminMembers` account (all arion/bank admin origins).
fn admin() -> AccountId {
	AccountId32::from_ss58check("5CVXqxb7mhFTtZVw5BJ8M2ujND9PFymSDxF8bkod6Sm4XJTW").unwrap()
}

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| System::set_block_number(1));
	ext
}

/// Register a storage-miner node and append it to the ranked list. Seeds
/// storage directly, the same way miner_payments.rs seeds arion children.
fn seed_ranked_storage_miner(node_seed: u8, owner: &AccountId, weight: u16, is_active: bool) {
	let node_id = vec![node_seed; 32];
	pallet_registration::ColdkeyNodeRegistrationV2::<Runtime>::insert(
		node_id.clone(),
		Some(ColdkeyNodeInfoLite {
			node_id: node_id.clone(),
			node_type: NodeType::StorageMiner,
			status: Status::Online,
			registered_at: 0u32.into(),
			owner: owner.clone(),
		}),
	);
	let mut list = pallet_rankings::RankedList::<Runtime>::get();
	list.push(pallet_rankings::NodeRankings {
		rank: u32::from(node_seed),
		node_id,
		node_ss58_address: owner.to_ss58check().into_bytes(),
		node_type: NodeType::StorageMiner,
		weight,
		last_updated: 0u32.into(),
		is_active,
	});
	pallet_rankings::RankedList::<Runtime>::put(list);
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
fn pay_storage_miners_with_empty_ranking() {
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
fn pay_storage_miners_distributes_to_ranked_miners() {
	new_test_ext().execute_with(|| {
		let (miner_a, miner_b, inactive) = (account(2), account(3), account(4));
		fund_bank_and_whitelist_admin(100_000);

		seed_ranked_storage_miner(10, &miner_a, 100, true);
		seed_ranked_storage_miner(11, &miner_b, 300, true);
		// Inactive nodes must not receive a share or dilute the split.
		seed_ranked_storage_miner(12, &inactive, 600, false);

		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(admin()), 100_000));

		assert_eq!(Balances::free_balance(&miner_a), 25_000);
		assert_eq!(Balances::free_balance(&miner_b), 75_000);
		assert_eq!(Balances::free_balance(&inactive), 0);
		assert_eq!(Hippocampus::emission_available(), 0);
	});
}

#[test]
fn pay_storage_miners_excludes_uid_238_account() {
	new_test_ext().execute_with(|| {
		let (miner_a, capture) = (account(2), account(5));
		fund_bank_and_whitelist_admin(100_000);

		seed_ranked_storage_miner(10, &miner_a, 100, true);
		seed_ranked_storage_miner(13, &capture, 300, true);

		// Mark `capture` as the uid-238 (emission capture) account on the
		// metagraph: it must be excluded and not dilute the miner split.
		pallet_metagraph::UIDs::<Runtime>::put(vec![UID {
			address: sp_core::sr25519::Public::from_raw([0u8; 32]),
			id: 238,
			role: Role::Validator,
			substrate_address: capture.clone(),
		}]);

		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(admin()), 100_000));

		assert_eq!(Balances::free_balance(&capture), 0);
		assert_eq!(Balances::free_balance(&miner_a), 100_000);
		assert_eq!(Hippocampus::emission_available(), 0);
	});
}

#[test]
fn unregistered_or_degraded_ranked_nodes_are_excluded() {
	new_test_ext().execute_with(|| {
		let (miner_a, ghost, degraded) = (account(2), account(6), account(7));
		fund_bank_and_whitelist_admin(100_000);

		seed_ranked_storage_miner(10, &miner_a, 100, true);

		// Ranked but never registered: the owner cannot be resolved.
		let mut list = pallet_rankings::RankedList::<Runtime>::get();
		list.push(pallet_rankings::NodeRankings {
			rank: 14,
			node_id: vec![14u8; 32],
			node_ss58_address: ghost.to_ss58check().into_bytes(),
			node_type: NodeType::StorageMiner,
			weight: 300,
			last_updated: 0u32.into(),
			is_active: true,
		});
		pallet_rankings::RankedList::<Runtime>::put(list);

		// Registered but degraded: registration refuses to resolve it.
		seed_ranked_storage_miner(15, &degraded, 300, true);
		pallet_registration::ColdkeyNodeRegistrationV2::<Runtime>::insert(
			vec![15u8; 32],
			Some(ColdkeyNodeInfoLite {
				node_id: vec![15u8; 32],
				node_type: NodeType::StorageMiner,
				status: Status::Degraded,
				registered_at: 0u32.into(),
				owner: degraded.clone(),
			}),
		);

		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(admin()), 100_000));

		// Excluded nodes neither receive nor dilute the split.
		assert_eq!(Balances::free_balance(&ghost), 0);
		assert_eq!(Balances::free_balance(&degraded), 0);
		assert_eq!(Balances::free_balance(&miner_a), 100_000);
	});
}

#[test]
fn daily_miner_payout_cap_is_1500_alpha() {
	// Guards the 18-decimals scaling of the cap: the constant originally
	// shipped 1000x too small (3 alpha instead of 3000). If the intended
	// daily cap changes, change this test together with the runtime value.
	const UNIT: u128 = 1_000_000_000_000_000_000;
	assert_eq!(<Runtime as pallet_hippocampus::Config>::Max24HourMinerPayout::get(), 3_500 * UNIT);
}

//! End-to-end: compute emission deposited into the bank is distributed to
//! compute-miner *families* by `pay_compute_miners`, the runtime's
//! `ComputeMinerWeightsSource` resolves node → child → family off the
//! compute-scoring pallet's last-closed-epoch weights, and the compute
//! emission compartment is invisible to the arion settlement headroom.

use frame_support::traits::{Currency, Get};
use frame_support::{assert_noop, assert_ok};
use hippius_mainnet_runtime::{
	AccountId, ArionPayoutSource, Balances, ComputeMinerWeightsSource, Hippocampus, Runtime,
	RuntimeOrigin, System,
};
use pallet_arion::PayoutSource;
use pallet_compute_scoring::{ChildRegistration, ChildStatus};
use pallet_hippocampus::{ComputeMinerWeights, DepositType};
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

/// Seed a compute child owned by `family`, and the epoch weight the validator
/// reported for it in `epoch`. Mirrors what `register_child` +
/// `vali_submit_epoch_close` write.
fn seed_compute_child(
	node_seed: u8,
	family: &AccountId,
	child: &AccountId,
	epoch: u64,
	weight: u128,
	status: ChildStatus,
) {
	let node_id = [node_seed; 32];
	pallet_compute_scoring::NodeIdToChild::<Runtime>::insert(node_id, child.clone());
	pallet_compute_scoring::ChildRegistrations::<Runtime>::insert(
		child.clone(),
		ChildRegistration {
			family: family.clone(),
			node_id,
			status,
			deposit: 0u128,
			unbonding_end: 0u64,
		},
	);
	pallet_compute_scoring::EpochWeights::<Runtime>::insert(epoch, node_id, weight);
}

/// Register a storage-miner node and append it to the ranked list — needed
/// only to prove the shared 24-hour budget spans both payouts.
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

/// Close of `epoch`: `vali_submit_epoch_close` sets `CurrentEpoch = epoch`
/// after writing that epoch's weights, so `CurrentEpoch` *is* the last closed
/// epoch — the adapter reads exactly this.
fn close_epoch(epoch: u64) {
	pallet_compute_scoring::CurrentEpoch::<Runtime>::put(epoch);
}

/// Fund the bank with an ED cushion plus `emission` tagged as ComputeEmission,
/// and whitelist the admin account as the payout caller.
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
		DepositType::ComputeEmission
	));
	// AdminOrigin is EnsureSignedBy<ArionAdminMembers>, not root.
	assert_ok!(Hippocampus::add_miner_payment_caller(RuntimeOrigin::signed(admin()), admin()));
}

#[test]
fn compute_emission_compartment_invisible_to_arion_settlement() {
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
			DepositType::ComputeEmission
		));

		// Wall: compute emission is invisible to the arion settlement headroom.
		assert_eq!(ArionPayoutSource::available(), arion_headroom_before);
		assert_eq!(Hippocampus::compute_emission_available(), 100_000);
		// And it is not the storage compartment either.
		assert_eq!(Hippocampus::emission_available(), 0);
	});
}

#[test]
fn pay_compute_miners_with_no_closed_epoch() {
	new_test_ext().execute_with(|| {
		fund_bank_and_whitelist_admin(100_000);

		assert_noop!(
			Hippocampus::pay_compute_miners(RuntimeOrigin::signed(admin()), 100_000),
			pallet_hippocampus::Error::<Runtime>::NoEligibleComputeMiners
		);
		assert_eq!(Hippocampus::compute_emission_available(), 100_000);
	});
}

#[test]
fn pay_compute_miners_distributes_to_families() {
	new_test_ext().execute_with(|| {
		let (family_a, family_b) = (account(2), account(3));
		fund_bank_and_whitelist_admin(100_000);

		seed_compute_child(10, &family_a, &account(20), 7, 100, ChildStatus::Active);
		seed_compute_child(11, &family_b, &account(21), 7, 300, ChildStatus::Active);
		close_epoch(7);

		assert_ok!(Hippocampus::pay_compute_miners(RuntimeOrigin::signed(admin()), 100_000));

		assert_eq!(Balances::free_balance(&family_a), 25_000);
		assert_eq!(Balances::free_balance(&family_b), 75_000);
		// The child accounts themselves are never paid.
		assert_eq!(Balances::free_balance(account(20)), 0);
		assert_eq!(Balances::free_balance(account(21)), 0);
		assert_eq!(Hippocampus::compute_emission_available(), 0);
	});
}

#[test]
fn a_family_running_several_nodes_gets_one_summed_transfer() {
	new_test_ext().execute_with(|| {
		let (family_a, family_b) = (account(2), account(3));
		fund_bank_and_whitelist_admin(100_000);

		// family_a runs three nodes summing to 300; family_b runs one at 100.
		seed_compute_child(10, &family_a, &account(20), 7, 100, ChildStatus::Active);
		seed_compute_child(11, &family_a, &account(21), 7, 100, ChildStatus::Active);
		seed_compute_child(12, &family_a, &account(22), 7, 100, ChildStatus::Active);
		seed_compute_child(13, &family_b, &account(23), 7, 100, ChildStatus::Active);
		close_epoch(7);

		// One entry per family, not per node.
		let miners = ComputeMinerWeightsSource::active_compute_miners();
		assert_eq!(miners.len(), 2);
		assert_eq!(
			miners.iter().find(|(who, _)| who == &family_a).map(|(_, w)| *w),
			Some(300)
		);

		assert_ok!(Hippocampus::pay_compute_miners(RuntimeOrigin::signed(admin()), 100_000));

		assert_eq!(Balances::free_balance(&family_a), 75_000);
		assert_eq!(Balances::free_balance(&family_b), 25_000);
	});
}

#[test]
fn unbonding_unmapped_and_zero_weight_nodes_are_excluded() {
	new_test_ext().execute_with(|| {
		let (family_a, unbonding, quarantined) = (account(2), account(3), account(4));
		fund_bank_and_whitelist_admin(100_000);

		seed_compute_child(10, &family_a, &account(20), 7, 100, ChildStatus::Active);
		// Deregistered mid-epoch: weight was reported, but the operator is gone.
		seed_compute_child(11, &unbonding, &account(21), 7, 300, ChildStatus::Unbonding);
		// The validator reported an explicit zero-reward verdict.
		seed_compute_child(12, &quarantined, &account(22), 7, 0, ChildStatus::Active);
		// Weight reported for a node id that no longer maps to any child.
		pallet_compute_scoring::EpochWeights::<Runtime>::insert(7u64, [13u8; 32], 300u128);
		close_epoch(7);

		assert_ok!(Hippocampus::pay_compute_miners(RuntimeOrigin::signed(admin()), 100_000));

		// Excluded nodes neither receive nor dilute the split.
		assert_eq!(Balances::free_balance(&unbonding), 0);
		assert_eq!(Balances::free_balance(&quarantined), 0);
		assert_eq!(Balances::free_balance(&family_a), 100_000);
	});
}

#[test]
fn only_the_last_closed_epoch_is_paid() {
	new_test_ext().execute_with(|| {
		let (stale, current) = (account(2), account(3));
		fund_bank_and_whitelist_admin(100_000);

		// A node that earned in epoch 6 but was not reported in epoch 7.
		seed_compute_child(10, &stale, &account(20), 6, 900, ChildStatus::Active);
		seed_compute_child(11, &current, &account(21), 7, 100, ChildStatus::Active);
		close_epoch(7);

		assert_ok!(Hippocampus::pay_compute_miners(RuntimeOrigin::signed(admin()), 100_000));

		assert_eq!(Balances::free_balance(&stale), 0);
		assert_eq!(Balances::free_balance(&current), 100_000);
	});
}

#[test]
fn the_two_emission_compartments_do_not_mix() {
	new_test_ext().execute_with(|| {
		let family_a = account(2);
		let funder = account(1);
		Balances::make_free_balance_be(&funder, 10_000_000);

		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(funder.clone()),
			ED * 2,
			DepositType::Grant
		));
		// Storage emission only — nothing for compute to spend.
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(funder),
			100_000,
			DepositType::Emission
		));
		assert_ok!(Hippocampus::add_miner_payment_caller(RuntimeOrigin::signed(admin()), admin()));

		seed_compute_child(10, &family_a, &account(20), 7, 100, ChildStatus::Active);
		close_epoch(7);

		assert_noop!(
			Hippocampus::pay_compute_miners(RuntimeOrigin::signed(admin()), 100_000),
			pallet_hippocampus::Error::<Runtime>::InsufficientComputeEmissionFunds
		);
		assert_eq!(Hippocampus::emission_available(), 100_000);
	});
}

#[test]
fn the_3500_alpha_daily_cap_is_shared_with_storage_payouts() {
	// The 3500 alpha/24h budget is a TOTAL across both payouts, not one each:
	// spending it on storage miners leaves nothing for compute miners in the
	// same period, however full the compute compartment is.
	new_test_ext().execute_with(|| {
		let cap = <Runtime as pallet_hippocampus::Config>::Max24HourMinerPayout::get();
		let family_a = account(2);
		let storage_miner = account(9);
		let funder = account(1);
		Balances::make_free_balance_be(&funder, cap.saturating_mul(4));

		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(funder.clone()),
			ED * 2,
			DepositType::Grant
		));
		// Each compartment is funded with the full daily cap on its own.
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(funder.clone()),
			cap,
			DepositType::Emission
		));
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(funder),
			cap,
			DepositType::ComputeEmission
		));
		assert_ok!(Hippocampus::add_miner_payment_caller(RuntimeOrigin::signed(admin()), admin()));

		seed_ranked_storage_miner(30, &storage_miner, 100, true);
		seed_compute_child(10, &family_a, &account(20), 7, 100, ChildStatus::Active);
		close_epoch(7);

		// Storage takes the whole day's budget.
		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(admin()), cap));
		assert_eq!(Balances::free_balance(&storage_miner), cap);

		// Compute is rate-limited out even though its compartment is full.
		assert_noop!(
			Hippocampus::pay_compute_miners(RuntimeOrigin::signed(admin()), 1),
			pallet_hippocampus::Error::<Runtime>::ExceedsDaily24HourMinerPayoutLimit
		);
		assert_eq!(Hippocampus::compute_emission_available(), cap);

		// Next period, the shared budget refills once and compute can draw it.
		System::set_block_number(
			<Runtime as pallet_hippocampus::Config>::BlocksPer24Hours::get() + 1,
		);
		assert_ok!(Hippocampus::pay_compute_miners(RuntimeOrigin::signed(admin()), cap));
		assert_eq!(Balances::free_balance(&family_a), cap);
	});
}

#[test]
fn compute_payout_slot_bound_covers_every_family() {
	// The payee is a family, so the bank's per-call bound only has to cover
	// `MaxFamilies` — this pins that relationship so raising MaxFamilies
	// without raising the bound fails loudly here rather than as a runtime
	// `TooManyComputeMiners` in production.
	let max_families: u32 = <Runtime as pallet_compute_scoring::Config>::MaxFamilies::get();
	let max_payees: u32 =
		<Runtime as pallet_hippocampus::Config>::MaxComputeMinersPerPayout::get();
	assert!(
		max_payees >= max_families,
		"MaxComputeMinersPerPayout ({max_payees}) must cover MaxFamilies ({max_families})"
	);
}

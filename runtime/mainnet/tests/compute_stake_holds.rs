//! End-to-end: compute stake lives on the `ComputeScoring(ComputeStake)`
//! fungible hold, and a stake slash is recycled into the Hippocampus pot's
//! compute compartment (`DepositType::SlashRecycled`) through the runtime's
//! `SlashToHippocampus` sink — payable by `pay_compute_miners`, out of reach
//! of `request_payment`, never burned, never reported as emission.

use frame_support::traits::{
	fungible::InspectHold, Currency, Get, GetStorageVersion, NamedReservableCurrency,
	OnRuntimeUpgrade, StorageVersion,
};
use frame_support::{assert_noop, assert_ok};
use hippius_mainnet_runtime::{
	AccountId, ArionPayoutSource, Balances, ComputeScoring, Hippocampus, Runtime, RuntimeEvent,
	RuntimeHoldReason, RuntimeOrigin, System,
};
use pallet_arion::PayoutSource;
use pallet_compute_scoring::{
	ChildRegistration, ChildStatus, HoldReason, SlashReason, SlashRecord, StakeUnbonding,
	StakedAmount, STAKE_RESERVE_ID,
};
use pallet_hippocampus::{ComputeEmissionPaidOut, DepositType, TotalDeposited};
use sp_core::crypto::Ss58Codec;
use sp_runtime::{AccountId32, BuildStorage};

/// Must match `ExistentialDeposit` in the runtime.
const ED: u128 = 500;

/// The hardcoded `ArionAdminMembers` account (bank admin origins).
fn bank_admin() -> AccountId {
	AccountId32::from_ss58check("5CVXqxb7mhFTtZVw5BJ8M2ujND9PFymSDxF8bkod6Sm4XJTW").unwrap()
}

/// The hardcoded `ComputeScoringAuthorityMembers` account
/// (`ComputeScoringAdminOrigin`: `slash_stake`, `set_slashing_enabled`).
fn scoring_admin() -> AccountId {
	AccountId32::from_ss58check("5CP9wzk9G3kMdJmNyAsWGWVWDQE7Goe1dUvKtMv51EoXs563").unwrap()
}

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

fn stake_reason() -> RuntimeHoldReason {
	RuntimeHoldReason::ComputeScoring(HoldReason::ComputeStake)
}

fn on_hold(who: &AccountId) -> u128 {
	<Balances as InspectHold<AccountId>>::balance_on_hold(&stake_reason(), who)
}

fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| System::set_block_number(1));
	ext
}

/// Bring the bank pot into existence (ED cushion), the way the live pot is.
fn open_bank() {
	let funder = account(1);
	Balances::make_free_balance_be(&funder, 10_000_000);
	assert_ok!(Hippocampus::deposit(RuntimeOrigin::signed(funder), ED * 2, DepositType::Grant));
}

fn stake(who: &AccountId, amount: u128) {
	Balances::make_free_balance_be(who, amount * 2);
	assert_ok!(ComputeScoring::top_up_stake(RuntimeOrigin::signed(who.clone()), amount));
}

fn enable_slashing() {
	assert_ok!(ComputeScoring::set_slashing_enabled(RuntimeOrigin::signed(scoring_admin()), true));
}

fn slash(who: &AccountId, amount: u128, reason: SlashReason) {
	assert_ok!(ComputeScoring::slash_stake(
		RuntimeOrigin::signed(scoring_admin()),
		who.clone(),
		amount,
		reason
	));
}

fn hippocampus_events() -> Vec<pallet_hippocampus::Event<Runtime>> {
	System::events()
		.into_iter()
		.filter_map(|r| match r.event {
			RuntimeEvent::Hippocampus(e) => Some(e),
			_ => None,
		})
		.collect()
}

fn unbonding_period() -> u64 {
	<Runtime as pallet_compute_scoring::Config>::UnbondingPeriodBlocks::get()
}

#[test]
fn stake_unstake_and_claim_run_on_the_compute_stake_hold() {
	new_test_ext().execute_with(|| {
		let who = account(2);
		stake(&who, 10_000);

		assert_eq!(on_hold(&who), 10_000);
		assert_eq!(Balances::reserved_balance_named(&STAKE_RESERVE_ID, &who), 0);
		assert_eq!(Balances::free_balance(&who), 10_000);
		assert_eq!(StakedAmount::<Runtime>::get(&who), 10_000);

		// Unbonding moves no balance: same hold, ledger split.
		assert_ok!(ComputeScoring::request_unstake(RuntimeOrigin::signed(who.clone()), 4_000));
		assert_eq!(on_hold(&who), 10_000);
		assert_eq!(StakedAmount::<Runtime>::get(&who), 6_000);
		let (pending, end) = StakeUnbonding::<Runtime>::get(&who).unwrap();
		assert_eq!(pending, 4_000);
		assert_eq!(end, 1 + unbonding_period());

		// Not before the end of the unbonding period.
		System::set_block_number(end - 1);
		assert_noop!(
			ComputeScoring::claim_unstaked(RuntimeOrigin::signed(who.clone())),
			pallet_compute_scoring::Error::<Runtime>::UnbondingNotReady
		);

		System::set_block_number(end);
		assert_ok!(ComputeScoring::claim_unstaked(RuntimeOrigin::signed(who.clone())));
		assert_eq!(on_hold(&who), 6_000);
		assert_eq!(Balances::free_balance(&who), 14_000);
		assert_eq!(StakeUnbonding::<Runtime>::get(&who), None);
	});
}

#[test]
fn slash_of_active_stake_lands_in_the_pot_as_slash_recycled() {
	new_test_ext().execute_with(|| {
		open_bank();
		let who = account(2);
		stake(&who, 10_000);
		enable_slashing();
		let pot_before = Hippocampus::balance();
		let issuance_before = Balances::total_issuance();
		let emission_before = TotalDeposited::<Runtime>::get(DepositType::ComputeEmission);
		System::reset_events();

		slash(&who, 3_000, SlashReason::Sla);

		assert_eq!(on_hold(&who), 7_000);
		assert_eq!(StakedAmount::<Runtime>::get(&who), 7_000);
		assert_eq!(SlashRecord::<Runtime>::get(&who), 3_000);
		// Exactly the slashed amount reached the pot; nothing was burned.
		assert_eq!(Hippocampus::balance(), pot_before + 3_000);
		assert_eq!(Balances::total_issuance(), issuance_before);
		assert_eq!(TotalDeposited::<Runtime>::get(DepositType::SlashRecycled), 3_000);
		assert_eq!(TotalDeposited::<Runtime>::get(DepositType::ComputeEmission), emission_before);

		let events = hippocampus_events();
		assert_eq!(
			events,
			vec![pallet_hippocampus::Event::SlashRecycled {
				from: who.clone(),
				amount: 3_000,
				reason: SlashReason::Sla
			}],
			"exactly one SlashRecycled and no Deposited event"
		);
	});
}

#[test]
fn slash_of_unbonding_stake_lands_in_the_pot_as_slash_recycled() {
	new_test_ext().execute_with(|| {
		open_bank();
		let who = account(2);
		stake(&who, 10_000);
		// Everything into unbonding: still held, still slashable.
		assert_ok!(ComputeScoring::request_unstake(RuntimeOrigin::signed(who.clone()), 10_000));
		enable_slashing();
		let pot_before = Hippocampus::balance();
		System::reset_events();

		slash(&who, 2_500, SlashReason::Liveness);

		assert_eq!(on_hold(&who), 7_500);
		assert_eq!(StakeUnbonding::<Runtime>::get(&who).map(|(a, _)| a), Some(7_500));
		assert_eq!(Hippocampus::balance(), pot_before + 2_500);
		assert_eq!(TotalDeposited::<Runtime>::get(DepositType::SlashRecycled), 2_500);
		assert!(hippocampus_events()
			.iter()
			.all(|e| !matches!(e, pallet_hippocampus::Event::Deposited { .. })));
		assert!(hippocampus_events().contains(&pallet_hippocampus::Event::SlashRecycled {
			from: who.clone(),
			amount: 2_500,
			reason: SlashReason::Liveness
		}));

		// The rest is still claimable after the period.
		System::set_block_number(1 + unbonding_period());
		assert_ok!(ComputeScoring::claim_unstaked(RuntimeOrigin::signed(who.clone())));
		assert_eq!(on_hold(&who), 0);
		assert_eq!(Balances::free_balance(&who), 17_500);
	});
}

#[test]
fn slash_beyond_the_stake_recycles_only_what_was_held() {
	new_test_ext().execute_with(|| {
		open_bank();
		let who = account(2);
		stake(&who, 1_000);
		enable_slashing();
		let pot_before = Hippocampus::balance();

		slash(&who, 50_000, SlashReason::Manual);

		assert_eq!(on_hold(&who), 0);
		assert_eq!(Hippocampus::balance(), pot_before + 1_000);
		assert_eq!(TotalDeposited::<Runtime>::get(DepositType::SlashRecycled), 1_000);
		assert_eq!(SlashRecord::<Runtime>::get(&who), 1_000);
	});
}

#[test]
fn slashing_disabled_moves_nothing() {
	// Live state: `SlashingEnabled` is false.
	new_test_ext().execute_with(|| {
		open_bank();
		let who = account(2);
		stake(&who, 1_000);
		let pot_before = Hippocampus::balance();

		slash(&who, 500, SlashReason::Manual);

		assert_eq!(on_hold(&who), 1_000);
		assert_eq!(Hippocampus::balance(), pot_before);
		assert_eq!(TotalDeposited::<Runtime>::get(DepositType::SlashRecycled), 0);
	});
}

#[test]
fn compute_payable_compartment_is_emission_plus_recycled_minus_paid() {
	new_test_ext().execute_with(|| {
		open_bank();
		let funder = account(1);
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(funder),
			4_000,
			DepositType::ComputeEmission
		));
		let who = account(2);
		stake(&who, 10_000);
		enable_slashing();
		slash(&who, 6_000, SlashReason::Sla);

		assert_eq!(Hippocampus::compute_payable_available(), 10_000);
		assert_eq!(Hippocampus::compute_emission_available(), 4_000);
		assert_eq!(Hippocampus::emission_available(), 0);

		// Pay 5 000 out of it: emission is counted as spent first.
		seed_compute_child(10, &account(3), &account(30), 7, 100);
		pallet_compute_scoring::CurrentEpoch::<Runtime>::put(7);
		assert_ok!(Hippocampus::add_miner_payment_caller(
			RuntimeOrigin::signed(bank_admin()),
			bank_admin()
		));
		assert_ok!(Hippocampus::pay_compute_miners(RuntimeOrigin::signed(bank_admin()), 5_000));

		assert_eq!(ComputeEmissionPaidOut::<Runtime>::get(), 5_000);
		assert_eq!(Hippocampus::compute_payable_available(), 5_000);
		assert_eq!(Hippocampus::compute_emission_available(), 0);
	});
}

#[test]
fn request_payment_and_arion_cannot_spend_recycled_slashes() {
	new_test_ext().execute_with(|| {
		open_bank();
		let arion_before = ArionPayoutSource::available();
		let unreserved_before = Hippocampus::unreserved_for_payout();
		let who = account(2);
		stake(&who, 10_000);
		enable_slashing();
		slash(&who, 10_000, SlashReason::Sla);

		// 10 000 more in the pot, none of it spendable outside compute payouts.
		assert_eq!(ArionPayoutSource::available(), arion_before);
		assert_eq!(Hippocampus::unreserved_for_payout(), unreserved_before);

		let requester = account(9);
		assert_ok!(Hippocampus::add_requester(
			RuntimeOrigin::signed(bank_admin()),
			requester.clone()
		));
		let paid = Hippocampus::request_payment(&requester, &account(8), 10_000).unwrap();
		assert_eq!(paid, unreserved_before);
		assert_eq!(Hippocampus::compute_payable_available(), 10_000);
		assert!(Hippocampus::available_for_payout() >= 10_000);
	});
}

#[test]
fn pay_compute_miners_can_spend_recycled_slashes_alone() {
	new_test_ext().execute_with(|| {
		open_bank();
		let who = account(2);
		stake(&who, 10_000);
		enable_slashing();
		slash(&who, 8_000, SlashReason::Sla);
		// No compute emission at all: the payout is funded only by the slash.
		assert_eq!(Hippocampus::compute_emission_available(), 0);

		let (honest_a, honest_b) = (account(3), account(4));
		seed_compute_child(10, &honest_a, &account(30), 7, 100);
		seed_compute_child(11, &honest_b, &account(31), 7, 300);
		pallet_compute_scoring::CurrentEpoch::<Runtime>::put(7);
		assert_ok!(Hippocampus::add_miner_payment_caller(
			RuntimeOrigin::signed(bank_admin()),
			bank_admin()
		));

		assert_ok!(Hippocampus::pay_compute_miners(RuntimeOrigin::signed(bank_admin()), 8_000));
		assert_eq!(Balances::free_balance(&honest_a), 2_000);
		assert_eq!(Balances::free_balance(&honest_b), 6_000);
		assert_eq!(Hippocampus::compute_payable_available(), 0);
	});
}

#[test]
fn set_slash_beneficiary_is_refused() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			ComputeScoring::set_slash_beneficiary(
				RuntimeOrigin::signed(scoring_admin()),
				Some(account(9))
			),
			pallet_compute_scoring::Error::<Runtime>::SlashBeneficiaryDeprecated
		);
	});
}

#[test]
fn migration_turns_an_injected_stake_reserve_into_a_hold() {
	new_test_ext().execute_with(|| {
		type Migration = pallet_compute_scoring::migrations::v1::MigrateStakeReserveToHold<Runtime>;
		StorageVersion::new(0).put::<ComputeScoring>();

		// Pre-upgrade shape: what the old `top_up_stake` + `request_unstake`
		// left behind, plus a child deposit and a legacy beneficiary.
		let who = account(2);
		Balances::make_free_balance_be(&who, 100_000);
		assert_ok!(Balances::reserve_named(&STAKE_RESERVE_ID, &who, 30_000));
		assert_ok!(Balances::reserve_named(
			&pallet_compute_scoring::DEPOSIT_RESERVE_ID,
			&who,
			5_000
		));
		StakedAmount::<Runtime>::insert(&who, 20_000);
		StakeUnbonding::<Runtime>::insert(&who, (10_000u128, 99u64));
		pallet_compute_scoring::SlashBeneficiary::<Runtime>::put(account(9));

		Migration::on_runtime_upgrade();

		assert_eq!(on_hold(&who), 30_000);
		assert_eq!(Balances::reserved_balance_named(&STAKE_RESERVE_ID, &who), 0);
		assert_eq!(
			Balances::reserved_balance_named(&pallet_compute_scoring::DEPOSIT_RESERVE_ID, &who),
			5_000
		);
		assert_eq!(Balances::free_balance(&who), 65_000);
		assert!(!pallet_compute_scoring::SlashBeneficiary::<Runtime>::exists());
		assert_eq!(ComputeScoring::on_chain_storage_version(), 1);

		// Converted stake behaves like native hold stake: claimable after the
		// recorded end, slashable into the pot.
		System::set_block_number(99);
		assert_ok!(ComputeScoring::claim_unstaked(RuntimeOrigin::signed(who.clone())));
		assert_eq!(on_hold(&who), 20_000);

		// Idempotent.
		Migration::on_runtime_upgrade();
		assert_eq!(on_hold(&who), 20_000);
		assert_eq!(Balances::free_balance(&who), 75_000);
	});
}

#[test]
fn migration_on_the_live_shape_is_a_version_bump() {
	// Mainnet at spec 92018: StakedAmount / StakeUnbonding empty,
	// SlashBeneficiary unset, compute-scoring at storage version 0.
	new_test_ext().execute_with(|| {
		StorageVersion::new(0).put::<ComputeScoring>();
		pallet_compute_scoring::migrations::v1::MigrateStakeReserveToHold::<Runtime>::on_runtime_upgrade();
		assert_eq!(ComputeScoring::on_chain_storage_version(), 1);
	});
}

/// Documents EXISTING behaviour (no change in this PR): a node whose epoch
/// weight is 0 — e.g. a forfeited / quarantined-for-cause verdict at epoch
/// close — is dropped by `ComputeMinerWeightsSource` before the sum, so its
/// share of the same epoch's payout is re-normalised onto the other miners.
/// Nothing is withheld and nothing stays in the pot.
#[test]
fn a_zero_weight_node_share_goes_to_the_other_miners() {
	new_test_ext().execute_with(|| {
		open_bank();
		let funder = account(1);
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(funder),
			100_000,
			DepositType::ComputeEmission
		));
		let (a, forfeited, c) = (account(3), account(4), account(5));
		seed_compute_child(10, &a, &account(30), 7, 100);
		seed_compute_child(11, &forfeited, &account(31), 7, 0);
		seed_compute_child(12, &c, &account(32), 7, 300);
		pallet_compute_scoring::CurrentEpoch::<Runtime>::put(7);
		assert_ok!(Hippocampus::add_miner_payment_caller(
			RuntimeOrigin::signed(bank_admin()),
			bank_admin()
		));

		assert_ok!(Hippocampus::pay_compute_miners(RuntimeOrigin::signed(bank_admin()), 100_000));

		assert_eq!(Balances::free_balance(&forfeited), 0);
		assert_eq!(Balances::free_balance(&a), 25_000);
		assert_eq!(Balances::free_balance(&c), 75_000);
		assert_eq!(ComputeEmissionPaidOut::<Runtime>::get(), 100_000);
	});
}

/// Seed an Active compute child owned by `family` with `weight` in `epoch`.
fn seed_compute_child(
	node_seed: u8,
	family: &AccountId,
	child: &AccountId,
	epoch: u64,
	weight: u128,
) {
	let node_id = [node_seed; 32];
	pallet_compute_scoring::NodeIdToChild::<Runtime>::insert(node_id, child.clone());
	pallet_compute_scoring::ChildRegistrations::<Runtime>::insert(
		child.clone(),
		ChildRegistration {
			family: family.clone(),
			node_id,
			status: ChildStatus::Active,
			deposit: 0u128,
			unbonding_end: 0u64,
		},
	);
	pallet_compute_scoring::EpochWeights::<Runtime>::insert(epoch, node_id, weight);
}

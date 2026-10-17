//! End-to-end: an operator with no `pallet_registration` entry (no sudo, no
//! Bittensor UID) becomes a compute family through
//! `ComputeScoring.register_family`, registers a child node via its proxy,
//! and leaves again — through the runtime's real `ComputeFamilyRegistry`,
//! `pallet_proxy` and `Registration.BannedAccounts`.

use frame_support::traits::{fungible::InspectHold, Currency, Get};
use frame_support::{assert_noop, assert_ok};
use hippius_mainnet_runtime::{
	AccountId, Balances, ComputeFamilyRegistry, ComputeScoring, ProxyType, Runtime,
	RuntimeHoldReason, RuntimeOrigin, System,
};
use pallet_compute_scoring::{
	ComputeFamilies, ComputeFamiliesCount, Error, FamilyRegistry, HoldReason,
};
use sp_core::{ed25519, Pair};
use sp_runtime::codec::Encode;
use sp_runtime::{AccountId32, BuildStorage};

const UNIT: u128 = 1_000_000_000_000_000_000;
const DEPOSIT: u128 = 10 * UNIT;

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| System::set_block_number(1));
	ext
}

fn family_hold(who: &AccountId) -> u128 {
	<Balances as InspectHold<AccountId>>::balance_on_hold(
		&RuntimeHoldReason::ComputeScoring(HoldReason::ComputeFamilyDeposit),
		who,
	)
}

fn enable(deposit: u128) {
	assert_ok!(ComputeScoring::set_family_deposit(RuntimeOrigin::root(), deposit));
}

fn register_child(family: &AccountId, child: &AccountId, seed: u8) {
	assert_ok!(pallet_proxy::Pallet::<Runtime>::add_proxy(
		RuntimeOrigin::signed(family.clone()),
		child.clone().into(),
		ProxyType::NonTransfer,
		0,
	));
	let node = ed25519::Pair::from_seed(&[seed; 32]);
	let node_id = node.public().0;
	let msg = (b"HIPPIUS_COMPUTE_NODE_REG_V1", family, child, &node_id, 0u64).encode();
	assert_ok!(ComputeScoring::register_child(
		RuntimeOrigin::signed(family.clone()),
		family.clone(),
		child.clone(),
		node_id,
		node.sign(&msg).0,
	));
}

#[test]
fn disabled_by_default() {
	new_test_ext().execute_with(|| {
		let op = account(1);
		let _ = Balances::deposit_creating(&op, 100 * UNIT);
		assert_noop!(
			ComputeScoring::register_family(RuntimeOrigin::signed(op)),
			Error::<Runtime>::FamilyRegistrationDisabled
		);
	});
}

#[test]
fn set_family_deposit_needs_root() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			ComputeScoring::set_family_deposit(RuntimeOrigin::signed(account(1)), DEPOSIT),
			sp_runtime::DispatchError::BadOrigin
		);
	});
}

#[test]
fn an_unregistered_operator_becomes_a_family_registers_a_node_and_leaves() {
	new_test_ext().execute_with(|| {
		enable(DEPOSIT);
		let (op, child) = (account(1), account(2));
		let _ = Balances::deposit_creating(&op, 100 * UNIT);
		assert!(!pallet_registration::Pallet::<Runtime>::is_owner_node_registered(&op));
		assert!(!ComputeFamilyRegistry::is_registered_family(&op));

		assert_ok!(ComputeScoring::register_family(RuntimeOrigin::signed(op.clone())));
		assert_eq!(family_hold(&op), DEPOSIT);
		assert_eq!(ComputeFamiliesCount::<Runtime>::get(), 1);
		assert!(ComputeFamilyRegistry::is_registered_family(&op));
		// Not a validator: no force-deregister rights.
		assert!(!ComputeFamilyRegistry::owner_has_validator_node(&op));

		register_child(&op, &child, 0x41);
		assert_eq!(pallet_compute_scoring::FamilyChildren::<Runtime>::get(&op).len(), 1);

		assert_noop!(
			ComputeScoring::unregister_family(RuntimeOrigin::signed(op.clone())),
			Error::<Runtime>::FamilyHasChildren
		);
		assert_ok!(ComputeScoring::deregister_child(
			RuntimeOrigin::signed(op.clone()),
			child.clone()
		));
		let reg = pallet_compute_scoring::ChildRegistrations::<Runtime>::get(&child).unwrap();
		System::set_block_number(reg.unbonding_end);
		assert_ok!(ComputeScoring::claim_unbonded(RuntimeOrigin::signed(op.clone()), child));

		let free_before = Balances::free_balance(&op);
		assert_ok!(ComputeScoring::unregister_family(RuntimeOrigin::signed(op.clone())));
		assert_eq!(family_hold(&op), 0);
		assert_eq!(Balances::free_balance(&op), free_before + DEPOSIT);
		assert!(!ComputeFamilies::<Runtime>::contains_key(&op));
		assert!(!ComputeFamilyRegistry::is_registered_family(&op));
	});
}

#[test]
fn a_registration_owner_cannot_also_register_permissionlessly() {
	new_test_ext().execute_with(|| {
		enable(DEPOSIT);
		let owner = account(3);
		let _ = Balances::deposit_creating(&owner, 100 * UNIT);
		pallet_registration::OwnerToNode::<Runtime>::insert(&owner, vec![b"node".to_vec()]);
		assert_noop!(
			ComputeScoring::register_family(RuntimeOrigin::signed(owner)),
			Error::<Runtime>::AlreadyAFamily
		);
	});
}

#[test]
fn a_ban_blocks_registration_and_switches_an_existing_family_off() {
	new_test_ext().execute_with(|| {
		enable(DEPOSIT);
		let (banned, fam) = (account(4), account(5));
		for who in [&banned, &fam] {
			let _ = Balances::deposit_creating(who, 100 * UNIT);
		}
		assert_ok!(pallet_registration::Pallet::<Runtime>::set_account_ban_status(
			RuntimeOrigin::root(),
			banned.clone(),
			true
		));
		assert_noop!(
			ComputeScoring::register_family(RuntimeOrigin::signed(banned)),
			Error::<Runtime>::AccountBanned
		);

		assert_ok!(ComputeScoring::register_family(RuntimeOrigin::signed(fam.clone())));
		assert!(ComputeFamilyRegistry::is_registered_family(&fam));
		assert_ok!(pallet_registration::Pallet::<Runtime>::set_account_ban_status(
			RuntimeOrigin::root(),
			fam.clone(),
			true
		));
		// Banned after the fact: no longer a family for register_child, but
		// the deposit stays its own and can still be withdrawn.
		assert!(!ComputeFamilyRegistry::is_registered_family(&fam));
		assert_ok!(ComputeScoring::unregister_family(RuntimeOrigin::signed(fam.clone())));
		assert_eq!(family_hold(&fam), 0);
	});
}

#[test]
fn the_cap_is_max_families() {
	new_test_ext().execute_with(|| {
		enable(DEPOSIT);
		let max: u32 = <Runtime as pallet_compute_scoring::Config>::MaxFamilies::get();
		assert_eq!(max, 600);
		ComputeFamiliesCount::<Runtime>::put(max);
		let op = account(6);
		let _ = Balances::deposit_creating(&op, 100 * UNIT);
		assert_noop!(
			ComputeScoring::register_family(RuntimeOrigin::signed(op)),
			Error::<Runtime>::TooManyComputeFamilies
		);
	});
}

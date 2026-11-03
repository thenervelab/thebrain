//! `Arion.FamilyCount` must equal the number of families with at least one
//! active child, on every register / deregister / claim path, and the v3
//! migration must restore that from a drifted counter.
//!
//! `claim_unbonded` / `deregister_family` must release the deposit recorded in
//! the registration, whatever `LockupEnabled` says at release time.

use frame_support::{
	assert_noop, assert_ok,
	traits::{Currency, GetStorageVersion, Hooks, ReservableCurrency, StorageVersion},
};
use hippius_mainnet_runtime::{
	AccountId, Arion, Balances, BlockNumber, ProxyType, Runtime, RuntimeOrigin, System,
};
use pallet_arion::{ChildRegistrations, FamilyActiveChildren, FamilyCount};
use parity_scale_codec::Encode;
use sp_core::{crypto::Ss58Codec, ed25519, Get, Pair};
use sp_runtime::{AccountId32, BuildStorage};

const UNIT: u128 = 1_000_000_000_000_000_000;
/// Past both `UnregisterCooldownBlocks` (7_200) and `UnbondingPeriodBlocks` (100_800).
const PAST_UNBONDING: BlockNumber = 200_000;

fn admin() -> AccountId {
	AccountId32::from_ss58check("5CVXqxb7mhFTtZVw5BJ8M2ujND9PFymSDxF8bkod6Sm4XJTW").unwrap()
}

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

fn node(seed: u8) -> ed25519::Pair {
	ed25519::Pair::from_seed(&[seed; 32])
}

fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| System::set_block_number(1));
	ext
}

fn make_family(family: &AccountId) {
	let _ = Balances::deposit_creating(family, 10_000 * UNIT);
	pallet_registration::OwnerToNode::<Runtime>::insert(family, vec![b"node".to_vec()]);
}

fn add_child_proxy(family: &AccountId, child: &AccountId) {
	assert_ok!(pallet_proxy::Pallet::<Runtime>::add_proxy(
		RuntimeOrigin::signed(family.clone()),
		child.clone().into(),
		ProxyType::Any,
		0,
	));
}

fn try_register(
	family: &AccountId,
	child: &AccountId,
	node: &ed25519::Pair,
) -> sp_runtime::DispatchResult {
	let nonce = pallet_arion::NodeIdNonce::<Runtime>::get(node.public().0);
	let msg = (b"ARION_NODE_REG_V1", family, child, &node.public().0, nonce).encode();
	Arion::register_child(
		RuntimeOrigin::signed(family.clone()),
		family.clone(),
		child.clone(),
		node.public().0,
		node.sign(&msg).0,
	)
}

fn register(family: &AccountId, child: &AccountId, node: &ed25519::Pair) {
	add_child_proxy(family, child);
	assert_ok!(try_register(family, child, node));
}

fn deregister(family: &AccountId, child: &AccountId) {
	assert_ok!(Arion::deregister_child(RuntimeOrigin::signed(family.clone()), child.clone()));
}

fn claim(family: &AccountId, child: &AccountId) {
	assert_ok!(Arion::claim_unbonded(RuntimeOrigin::signed(family.clone()), child.clone()));
}

fn set_lockup(enabled: bool) {
	assert_ok!(Arion::set_lockup_enabled(RuntimeOrigin::signed(admin()), enabled));
}

/// Families with >= 1 active child, recomputed from scratch.
fn current_families() -> u32 {
	FamilyActiveChildren::<Runtime>::iter().filter(|(_, n)| *n > 0).count() as u32
}

fn assert_count(expected: u32) {
	assert_eq!(FamilyCount::<Runtime>::get(), expected, "FamilyCount");
	assert_eq!(current_families(), expected, "families with an active child");
}

#[test]
fn family_count_stays_consistent_across_register_deregister_claim_cycles() {
	new_test_ext().execute_with(|| {
		let (fam_a, fam_b) = (account(1), account(2));
		make_family(&fam_a);
		make_family(&fam_b);

		register(&fam_a, &account(10), &node(10));
		assert_count(1);
		register(&fam_a, &account(11), &node(11));
		assert_count(1);
		register(&fam_b, &account(20), &node(20));
		assert_count(2);

		deregister(&fam_a, &account(10));
		assert_count(2);
		deregister(&fam_a, &account(11));
		assert_count(1);
		claim(&fam_a, &account(10));
		claim(&fam_a, &account(11));
		assert_count(1);

		// A family that dropped out and comes back is a current family again.
		// The old code only counted a family on its first-ever fee-free slot,
		// so this re-entry was invisible to the counter while the next drop to
		// zero still decremented it.
		System::set_block_number(PAST_UNBONDING);
		register(&fam_a, &account(12), &node(12));
		assert_count(2);
		deregister(&fam_a, &account(12));
		assert_count(1);
		register(&fam_a, &account(13), &node(13));
		assert_count(2);

		// `deregister_family` (execution-unit's miner purge) drops a current family.
		assert_ok!(Arion::deregister_family(fam_b.clone()));
		assert_count(1);
	});
}

#[test]
fn max_families_gates_every_family_becoming_current() {
	new_test_ext().execute_with(|| {
		let max: u32 = <Runtime as pallet_arion::Config>::MaxFamilies::get();
		let (fam_a, fam_new) = (account(1), account(2));
		make_family(&fam_a);
		make_family(&fam_new);

		register(&fam_a, &account(10), &node(10));
		deregister(&fam_a, &account(10));
		assert_count(0);

		// At the cap: no family may become current — brand new or returning.
		FamilyCount::<Runtime>::put(max);
		System::set_block_number(PAST_UNBONDING);
		add_child_proxy(&fam_new, &account(20));
		assert_noop!(
			try_register(&fam_new, &account(20), &node(20)),
			pallet_arion::Error::<Runtime>::TooManyFamilies
		);
		add_child_proxy(&fam_a, &account(11));
		assert_noop!(
			try_register(&fam_a, &account(11), &node(11)),
			pallet_arion::Error::<Runtime>::TooManyFamilies
		);

		// Below the cap a returning family gets in, and a current family can
		// still add children at the cap.
		FamilyCount::<Runtime>::put(max - 1);
		assert_ok!(try_register(&fam_a, &account(11), &node(11)));
		assert_eq!(FamilyCount::<Runtime>::get(), max);
		register(&fam_a, &account(12), &node(12));
		assert_eq!(FamilyCount::<Runtime>::get(), max);
	});
}

#[test]
fn migration_v3_recomputes_a_drifted_family_count() {
	new_test_ext().execute_with(|| {
		let (fam_a, fam_b) = (account(1), account(2));
		make_family(&fam_a);
		make_family(&fam_b);
		register(&fam_a, &account(10), &node(10));
		register(&fam_b, &account(20), &node(20));
		// Legacy row: a family whose last child left before the drop-to-zero
		// decrement existed. Still keyed, value 0, never decremented.
		FamilyActiveChildren::<Runtime>::insert(account(3), 0);

		// The live state: counter pinned at the cap.
		FamilyCount::<Runtime>::put(600);
		StorageVersion::new(2).put::<Arion>();

		<Arion as Hooks<BlockNumber>>::on_runtime_upgrade();
		assert_eq!(FamilyCount::<Runtime>::get(), 2);
		assert_eq!(Arion::on_chain_storage_version(), StorageVersion::new(4));

		// Idempotent, and gated: a second upgrade at v3 is a no-op.
		FamilyCount::<Runtime>::put(7);
		<Arion as Hooks<BlockNumber>>::on_runtime_upgrade();
		assert_eq!(FamilyCount::<Runtime>::get(), 7, "v3 must not re-run");
		StorageVersion::new(2).put::<Arion>();
		<Arion as Hooks<BlockNumber>>::on_runtime_upgrade();
		assert_eq!(FamilyCount::<Runtime>::get(), 2);

		// After the fix a new family can register again.
		make_family(&account(4));
		register(&account(4), &account(40), &node(40));
		assert_count(3);
	});
}

#[test]
fn deregister_family_does_not_decrement_for_a_legacy_zero_row() {
	new_test_ext().execute_with(|| {
		make_family(&account(1));
		register(&account(1), &account(10), &node(10));
		FamilyActiveChildren::<Runtime>::insert(account(3), 0);
		assert_count(1);

		assert_ok!(Arion::deregister_family(account(3)));
		assert_count(1);
	});
}

/// Register a free child then a paid one with lockup on; returns the paid deposit.
fn register_paid_child(family: &AccountId, paid_child: &AccountId) -> u128 {
	set_lockup(true);
	make_family(family);
	register(family, &account(10), &node(10));
	register(family, paid_child, &node(11));
	let deposit = ChildRegistrations::<Runtime>::get(paid_child).expect("registered").deposit;
	assert!(deposit > 0, "second child must be paid");
	assert_eq!(Balances::reserved_balance(family), deposit);
	deposit
}

fn claim_after_unbonding(lockup_at_claim: bool) {
	new_test_ext().execute_with(|| {
		let (family, paid) = (account(1), account(11));
		let deposit = register_paid_child(&family, &paid);
		let free_before = Balances::free_balance(&family);

		deregister(&family, &paid);
		set_lockup(lockup_at_claim);
		System::set_block_number(PAST_UNBONDING);
		claim(&family, &paid);

		assert_eq!(Balances::reserved_balance(&family), 0, "deposit must be released");
		assert_eq!(Balances::free_balance(&family), free_before + deposit);
		assert!(ChildRegistrations::<Runtime>::get(&paid).is_none());
	});
}

#[test]
fn claim_unbonded_refunds_recorded_deposit_with_lockup_on() {
	claim_after_unbonding(true);
}

#[test]
fn claim_unbonded_refunds_recorded_deposit_after_lockup_turned_off() {
	claim_after_unbonding(false);
}

#[test]
fn deregister_family_refunds_recorded_deposit_after_lockup_turned_off() {
	new_test_ext().execute_with(|| {
		let family = account(1);
		let deposit = register_paid_child(&family, &account(11));
		let free_before = Balances::free_balance(&family);

		set_lockup(false);
		assert_ok!(Arion::deregister_family(family.clone()));

		assert_eq!(Balances::reserved_balance(&family), 0, "deposit must be released");
		assert_eq!(Balances::free_balance(&family), free_before + deposit);
		assert_count(0);
	});
}

#[test]
fn free_child_reserves_nothing_and_claims_nothing() {
	new_test_ext().execute_with(|| {
		set_lockup(true);
		make_family(&account(1));
		register(&account(1), &account(10), &node(10));
		assert_eq!(ChildRegistrations::<Runtime>::get(account(10)).unwrap().deposit, 0);
		assert_eq!(Balances::reserved_balance(account(1)), 0);
		deregister(&account(1), &account(10));
		claim(&account(1), &account(10));
		assert_eq!(Balances::reserved_balance(account(1)), 0);
	});
}

/// The whole trie, so "nothing changed" means nothing at all.
fn state_root() -> Vec<u8> {
	sp_io::storage::root(sp_runtime::StateVersion::V0)
}

/// #75: an error in the middle of `deregister_family` must leave no trace.
/// The first child is fully processed (indices removed, cooldowns set, its
/// registration deleted) before the second one fails.
#[test]
fn deregister_family_rolls_back_when_a_child_has_no_registration() {
	new_test_ext().execute_with(|| {
		let family = account(1);
		make_family(&family);
		register(&family, &account(10), &node(10));
		register(&family, &account(20), &node(20));
		// Corrupt the second child: listed under the family, no registration.
		ChildRegistrations::<Runtime>::remove(account(20));
		let before = state_root();

		assert_eq!(
			Arion::deregister_family(family.clone()),
			Err(pallet_arion::Error::<Runtime>::ChildNotRegistered.into())
		);
		assert_eq!(state_root(), before, "a failed deregister_family must change nothing");
		assert!(ChildRegistrations::<Runtime>::get(account(10)).is_some());
	});
}

/// #75: same, when the failure is a short reserve on a paid child after a
/// free child was already released.
#[test]
fn deregister_family_rolls_back_on_a_partial_unreserve() {
	new_test_ext().execute_with(|| {
		let family = account(1);
		let deposit = register_paid_child(&family, &account(11));
		// Out-of-band drift: half the paid child's deposit is no longer reserved.
		assert_eq!(
			<Balances as frame_support::traits::ReservableCurrency<AccountId>>::unreserve(
				&family,
				deposit / 2
			),
			0
		);
		let before = state_root();

		assert_eq!(
			Arion::deregister_family(family.clone()),
			Err(pallet_arion::Error::<Runtime>::PartialUnreserve.into())
		);
		assert_eq!(state_root(), before, "a failed deregister_family must change nothing");
		assert_eq!(Balances::reserved_balance(&family), deposit - deposit / 2);
		assert!(ChildRegistrations::<Runtime>::get(account(10)).is_some());
	});
}

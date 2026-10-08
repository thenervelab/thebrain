//! Occupancy-priced deposit for creating an Arion family.
//!
//! A family is meant to be one operator and one failure domain: CRUSH spreads
//! the shards of a stripe across families. Creating a family used to cost
//! nothing, so a single operator could register many families and hold many
//! shard slots of the same stripe. These tests pin the deposit that now prices
//! that: reserved when a family becomes active (0 -> 1 active children),
//! growing with `FamilyCount / MaxFamilies`, returned exactly when the family
//! has no active child left, and never charged to families that predate it.

use frame_support::{
	assert_noop, assert_ok,
	dispatch::GetDispatchInfo,
	traits::{Currency, ReservableCurrency, UnfilteredDispatchable},
};
use hippius_mainnet_runtime::{
	AccountId, Arion, Balances, ProxyType, Runtime, RuntimeEvent, RuntimeOrigin, System,
};
use pallet_arion::{
	Error, Event, FamilyCount, FamilyDepositBase, FamilyDepositOccupancyFactor, FamilyDeposits,
	MAX_FAMILY_DEPOSIT_OCCUPANCY_FACTOR,
};
use parity_scale_codec::Encode;
use sp_core::{crypto::Ss58Codec, ed25519, Pair};
use sp_runtime::{AccountId32, BuildStorage, DispatchError};

const UNIT: u128 = 1_000_000_000_000_000_000;
const BASE: u128 = 100 * UNIT;
const MAX_FAMILIES: u32 = 600;
const START_BALANCE: u128 = 10_000 * UNIT;

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

fn register_main_node(owner: &AccountId, node_id: &[u8], node_type: pallet_registration::NodeType) {
	pallet_registration::OwnerToNode::<Runtime>::insert(owner, vec![node_id.to_vec()]);
	pallet_registration::ColdkeyNodeRegistrationV2::<Runtime>::insert(
		node_id.to_vec(),
		Some(pallet_registration::ColdkeyNodeInfoLite {
			node_id: node_id.to_vec(),
			node_type,
			status: pallet_registration::Status::Online,
			registered_at: 1,
			owner: owner.clone(),
		}),
	);
}

/// A family registered in pallet-registration, funded with `balance`.
fn make_family(family: &AccountId, balance: u128) {
	let _ = Balances::deposit_creating(family, balance);
	let mut node_id = b"family-".to_vec();
	node_id.extend_from_slice(&family.encode()[..4]);
	register_main_node(family, &node_id, pallet_registration::NodeType::StorageMiner);
}

fn allow_child(family: &AccountId, child: &AccountId) {
	pallet_proxy::Pallet::<Runtime>::add_proxy(
		RuntimeOrigin::signed(family.clone()),
		child.clone().into(),
		ProxyType::Any,
		0,
	)
	.expect("add_proxy");
}

fn register_call(
	family: &AccountId,
	child: &AccountId,
	node: &ed25519::Pair,
) -> pallet_arion::Call<Runtime> {
	let nonce = pallet_arion::NodeIdNonce::<Runtime>::get(node.public().0);
	let msg = (b"ARION_NODE_REG_V1", family, child, &node.public().0, nonce).encode();
	pallet_arion::Call::<Runtime>::register_child {
		family: family.clone(),
		child: child.clone(),
		node_id: node.public().0,
		node_sig: node.sign(&msg).0,
	}
}

/// Dispatch through the call (transactional), as a real extrinsic would.
/// The family must already have proxied `child` (see [`setup_family`]).
fn try_register(
	family: &AccountId,
	child: &AccountId,
	node: &ed25519::Pair,
) -> Result<(), DispatchError> {
	register_call(family, child, node)
		.dispatch_bypass_filter(RuntimeOrigin::signed(family.clone()))
		.map(|_| ())
		.map_err(|e| e.error)
}

/// Registers and returns how much more the family has reserved afterwards.
fn register(family: &AccountId, child: &AccountId, node: &ed25519::Pair) -> u128 {
	let before = Balances::reserved_balance(family);
	try_register(family, child, node).expect("register_child");
	Balances::reserved_balance(family) - before
}

/// Fund + register the family's main node + proxy its children (the proxy
/// deposits are reserved here). Returns `(free, reserved)` after setup, the
/// baseline every family-deposit assertion is measured against.
fn setup_family(family: &AccountId, balance: u128, children: &[AccountId]) -> (u128, u128) {
	make_family(family, balance);
	for c in children {
		allow_child(family, c);
	}
	(Balances::free_balance(family), Balances::reserved_balance(family))
}

fn set_params(base: u128, k: u32) {
	assert_ok!(Arion::set_family_deposit_params(RuntimeOrigin::signed(admin()), base, k));
}

/// `base * (1 + k*n/M)^2` computed independently of the pallet.
fn expected_deposit(base: u128, k: u32, n: u32) -> u128 {
	let m = MAX_FAMILIES as u128;
	let scale = m + k as u128 * n as u128;
	base * scale / m * scale / m
}

fn arion_events() -> Vec<Event<Runtime>> {
	System::events()
		.into_iter()
		.filter_map(|r| match r.event {
			RuntimeEvent::Arion(e) => Some(e),
			_ => None,
		})
		.collect()
}

#[test]
fn deposit_grows_with_family_count() {
	new_test_ext().execute_with(|| {
		set_params(BASE, 3);

		// First family on an empty network pays exactly the base.
		let (fam_a, fam_b) = (account(1), account(2));
		setup_family(&fam_a, START_BALANCE, &[account(11)]);
		assert_eq!(register(&fam_a, &account(11), &node(11)), BASE);
		assert_eq!(FamilyDeposits::<Runtime>::get(&fam_a), Some(BASE));
		assert_eq!(FamilyCount::<Runtime>::get(), 1);
		assert!(arion_events().contains(&Event::FamilyDepositReserved {
			family: fam_a.clone(),
			amount: BASE,
			family_count: 0,
		}));

		// Half-full network: (1 + 3 * 300/600)^2 = 6.25x.
		FamilyCount::<Runtime>::put(300);
		setup_family(&fam_b, START_BALANCE, &[account(12)]);
		assert_eq!(expected_deposit(BASE, 3, 300), 625 * UNIT);
		assert_eq!(register(&fam_b, &account(12), &node(12)), 625 * UNIT);
		assert_eq!(FamilyDeposits::<Runtime>::get(&fam_b), Some(625 * UNIT));
		assert_eq!(FamilyCount::<Runtime>::get(), 301);
		assert_eq!(Arion::next_family_deposit(), expected_deposit(BASE, 3, 301));

		// The curve is monotonic and saturating-safe over the whole range.
		let mut prev = 0u128;
		for n in 0..=MAX_FAMILIES {
			let d = Arion::family_deposit_for(n);
			assert_eq!(d, expected_deposit(BASE, 3, n));
			assert!(d >= prev, "deposit must not decrease with occupancy");
			prev = d;
		}
		assert_eq!(Arion::family_deposit_for(MAX_FAMILIES), 16 * BASE);
		// Above the cap the occupancy is clamped, never extrapolated.
		assert_eq!(Arion::family_deposit_for(u32::MAX), 16 * BASE);
		// Huge base saturates instead of panicking.
		set_params(u128::MAX, MAX_FAMILY_DEPOSIT_OCCUPANCY_FACTOR);
		assert_eq!(Arion::family_deposit_for(MAX_FAMILIES - 1), u128::MAX);
	});
}

#[test]
fn second_child_of_an_active_family_pays_no_family_deposit() {
	new_test_ext().execute_with(|| {
		set_params(BASE, 3);
		let fam = account(1);
		setup_family(&fam, START_BALANCE, &[account(11), account(12)]);
		assert_eq!(register(&fam, &account(11), &node(11)), BASE);
		assert_eq!(register(&fam, &account(12), &node(12)), 0);
		assert_eq!(FamilyCount::<Runtime>::get(), 1);
	});
}

#[test]
fn insufficient_balance_fails_and_changes_nothing() {
	new_test_ext().execute_with(|| {
		set_params(BASE, 0);
		let fam = account(1);
		let child = account(11);
		let (free0, reserved0) = setup_family(&fam, BASE / 2, &[child.clone()]);
		assert!(free0 < BASE);

		assert_eq!(
			try_register(&fam, &child, &node(11)),
			Err(Error::<Runtime>::InsufficientFamilyDeposit.into())
		);
		assert_eq!(Balances::reserved_balance(&fam), reserved0);
		assert_eq!(Balances::free_balance(&fam), free0);
		assert_eq!(FamilyCount::<Runtime>::get(), 0);
		assert!(FamilyDeposits::<Runtime>::get(&fam).is_none());
		assert!(pallet_arion::ChildRegistrations::<Runtime>::get(&child).is_none());
		assert_eq!(pallet_arion::FamilyActiveChildren::<Runtime>::get(&fam), 0);
	});
}

#[test]
fn deregistering_last_child_unreserves_exactly_what_was_reserved() {
	new_test_ext().execute_with(|| {
		set_params(BASE, 3);
		FamilyCount::<Runtime>::put(150);
		let fam = account(1);
		let (c1, c2) = (account(11), account(12));
		let (free0, reserved0) = setup_family(&fam, START_BALANCE, &[c1.clone(), c2.clone()]);
		let paid = expected_deposit(BASE, 3, 150);
		assert_eq!(register(&fam, &c1, &node(11)), paid);
		assert_eq!(register(&fam, &c2, &node(12)), 0);

		// Re-pricing after the fact must not change what comes back.
		set_params(7 * BASE, 50);

		assert_ok!(Arion::deregister_child(RuntimeOrigin::signed(fam.clone()), c1.clone()));
		// Still one active child: the family slot is still held.
		assert_eq!(Balances::reserved_balance(&fam), reserved0 + paid);
		assert_eq!(FamilyDeposits::<Runtime>::get(&fam), Some(paid));

		assert_ok!(Arion::deregister_child(RuntimeOrigin::signed(fam.clone()), c2.clone()));
		assert_eq!(Balances::reserved_balance(&fam), reserved0);
		assert_eq!(Balances::free_balance(&fam), free0);
		assert!(FamilyDeposits::<Runtime>::get(&fam).is_none());
		assert_eq!(FamilyCount::<Runtime>::get(), 150);
		assert!(arion_events().contains(&Event::FamilyDepositUnreserved {
			family: fam.clone(),
			amount: paid,
			missing: 0,
		}));
	});
}

#[test]
fn force_deregister_and_deregister_family_also_unreserve() {
	new_test_ext().execute_with(|| {
		set_params(BASE, 0);

		// force_deregister_child by a validator-node owner.
		let validator = account(90);
		register_main_node(&validator, b"validator", pallet_registration::NodeType::Validator);
		let fam_a = account(1);
		let child_a = account(11);
		let (free_a, reserved_a) = setup_family(&fam_a, START_BALANCE, &[child_a.clone()]);
		assert_eq!(register(&fam_a, &child_a, &node(11)), BASE);
		assert_ok!(Arion::force_deregister_child(RuntimeOrigin::signed(validator), child_a));
		assert_eq!(Balances::reserved_balance(&fam_a), reserved_a);
		assert_eq!(Balances::free_balance(&fam_a), free_a);
		assert!(FamilyDeposits::<Runtime>::get(&fam_a).is_none());

		// deregister_family (called by execution-unit's purge).
		let fam_b = account(2);
		let (free_b, reserved_b) = setup_family(&fam_b, START_BALANCE, &[account(12)]);
		assert_eq!(register(&fam_b, &account(12), &node(12)), BASE);
		assert_ok!(Arion::deregister_family(fam_b.clone()));
		assert_eq!(Balances::reserved_balance(&fam_b), reserved_b);
		assert_eq!(Balances::free_balance(&fam_b), free_b);
		assert!(FamilyDeposits::<Runtime>::get(&fam_b).is_none());
		assert_eq!(FamilyCount::<Runtime>::get(), 0);
	});
}

#[test]
fn reactivated_family_is_counted_and_pays_again() {
	new_test_ext().execute_with(|| {
		set_params(BASE, 0);
		let fam = account(1);
		let (_, reserved0) = setup_family(&fam, START_BALANCE, &[account(11), account(12)]);
		assert_eq!(register(&fam, &account(11), &node(11)), BASE);
		assert_ok!(Arion::deregister_child(RuntimeOrigin::signed(fam.clone()), account(11)));
		assert_eq!(FamilyCount::<Runtime>::get(), 0);
		assert_eq!(Balances::reserved_balance(&fam), reserved0);

		// Coming back after dropping to zero creates the family again: it is
		// counted against MaxFamilies (it used to come back uncounted) and pays.
		assert_eq!(register(&fam, &account(12), &node(12)), BASE);
		assert_eq!(FamilyCount::<Runtime>::get(), 1);
		assert_eq!(FamilyDeposits::<Runtime>::get(&fam), Some(BASE));
	});
}

#[test]
fn reactivation_respects_max_families() {
	new_test_ext().execute_with(|| {
		let fam = account(1);
		setup_family(&fam, START_BALANCE, &[account(11), account(12)]);
		register(&fam, &account(11), &node(11));
		assert_ok!(Arion::deregister_child(RuntimeOrigin::signed(fam.clone()), account(11)));
		FamilyCount::<Runtime>::put(MAX_FAMILIES);
		assert_eq!(
			try_register(&fam, &account(12), &node(12)),
			Err(Error::<Runtime>::TooManyFamilies.into())
		);
	});
}

#[test]
fn families_created_before_the_deposit_are_not_charged() {
	new_test_ext().execute_with(|| {
		// Deposit disabled (default): no reserve, no record.
		assert_eq!(FamilyDepositBase::<Runtime>::get(), 0);
		let fam = account(1);
		let (_, reserved0) = setup_family(&fam, START_BALANCE, &[account(11), account(12)]);
		assert_eq!(register(&fam, &account(11), &node(11)), 0);
		assert!(FamilyDeposits::<Runtime>::get(&fam).is_none());

		// Turning it on is not retroactive, not even when the existing family grows.
		set_params(BASE, 3);
		assert_eq!(register(&fam, &account(12), &node(12)), 0);

		// Leaving releases nothing and emits no unreserve event. An unrelated
		// reserve on the account proves it is left alone.
		assert_ok!(Balances::reserve(&fam, 5 * UNIT));
		assert_ok!(Arion::deregister_child(RuntimeOrigin::signed(fam.clone()), account(11)));
		assert_ok!(Arion::deregister_child(RuntimeOrigin::signed(fam.clone()), account(12)));
		assert_eq!(Balances::reserved_balance(&fam), reserved0 + 5 * UNIT);
		assert!(!arion_events()
			.iter()
			.any(|e| matches!(e, Event::FamilyDepositUnreserved { .. })));
	});
}

#[test]
fn only_admin_can_set_params_and_factor_is_capped() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Arion::set_family_deposit_params(RuntimeOrigin::signed(account(7)), BASE, 3),
			DispatchError::BadOrigin
		);
		assert_noop!(
			Arion::set_family_deposit_params(RuntimeOrigin::root(), BASE, 3),
			DispatchError::BadOrigin
		);
		assert_noop!(
			Arion::set_family_deposit_params(
				RuntimeOrigin::signed(admin()),
				BASE,
				MAX_FAMILY_DEPOSIT_OCCUPANCY_FACTOR + 1
			),
			Error::<Runtime>::FamilyDepositOccupancyFactorTooLarge
		);

		set_params(BASE, 3);
		assert_eq!(FamilyDepositBase::<Runtime>::get(), BASE);
		assert_eq!(FamilyDepositOccupancyFactor::<Runtime>::get(), 3);
		assert!(arion_events()
			.contains(&Event::FamilyDepositParamsSet { base: BASE, occupancy_factor: 3 }));

		let info = pallet_arion::Call::<Runtime>::set_family_deposit_params {
			base: BASE,
			occupancy_factor: 3,
		}
		.get_dispatch_info();
		assert!(info.weight.ref_time() > 0);
	});
}

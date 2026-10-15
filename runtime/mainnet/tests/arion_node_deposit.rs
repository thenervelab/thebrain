//! Burst-priced child (node) deposit for Arion.
//!
//! A family is meant to be one operator and one failure domain, and every node
//! is a shard slot in CRUSH. These tests pin the price paid by each paid child
//! registration:
//!
//! - floor = the child deposit (`BaseChildDepositValue`), there is no second base;
//! - EIP-1559 style: each paid registration multiplies the network-wide price,
//!   idle time halves it (linear in between), never above `cap`;
//! - `fee_share` of the price goes to the bank (Hippocampus) as
//!   `DepositType::NodeRegistrationFee`, the rest is the child's deposit,
//!   reserved on the family and released by `claim_unbonded` `refund_delay`
//!   blocks after deregistration;
//! - the same node (family, child account, node id) coming back while its
//!   deposit is still unbonding reuses it;
//! - with `NodeDepositConfig` unset the pre-upgrade behaviour is kept (×2 per
//!   paid registration, 1-day halving, no cap, no fee, 7-day unbonding), and
//!   with lockup off nothing is charged.

use frame_support::{
	assert_noop, assert_ok,
	dispatch::GetDispatchInfo,
	traits::{Currency, ReservableCurrency, UnfilteredDispatchable},
};
use hippius_mainnet_runtime::{
	AccountId, Arion, ArionPayoutSource, Balances, BlockNumber, ProxyType, Runtime, RuntimeEvent,
	RuntimeOrigin, System,
};
use pallet_arion::{
	ChildRegistrations, ChildStatus, Error, Event, FamilyCount, GlobalLastPaidRegistrationBlock,
	GlobalNextDeposit, NodeDepositConfig, NodeDepositParams, PayoutSource,
	LEGACY_NODE_PRICE_MULTIPLIER_PERMILL, MAX_NODE_PRICE_HALF_LIFE_BLOCKS,
	MAX_NODE_PRICE_MULTIPLIER_PERMILL, MAX_NODE_REFUND_DELAY_BLOCKS,
	MAX_NODE_REGISTRATION_FEE_SHARE_PERCENT,
};
use pallet_hippocampus::{DepositType, TotalDeposited};
use parity_scale_codec::Encode;
use sp_core::{crypto::Ss58Codec, ed25519, Pair};
use sp_runtime::{AccountId32, BuildStorage, DispatchError, Perbill};

const UNIT: u128 = 1_000_000_000_000_000_000;
/// `Config::BaseChildDeposit` of the mainnet runtime (10 DOLLAR = 100 UNIT).
const BASE: u128 = 100 * UNIT;
const MAX_FAMILIES: u32 = 600;
const START_BALANCE: u128 = 10_000 * UNIT;
/// Proposed half-life: 3 hours at 6 s blocks.
const HALF_LIFE: BlockNumber = 1_800;
/// `Config::GlobalDepositHalvingPeriodBlocks` (1 day).
const LEGACY_HALF_LIFE: BlockNumber = 14_400;
/// `Config::UnbondingPeriodBlocks` (7 days).
const UNBONDING: BlockNumber = 100_800;
/// `Config::UnregisterCooldownBlocks` (12 hours).
const COOLDOWN: BlockNumber = 7_200;
/// Refund delay used by the tests with explicit parameters (3 days).
const REFUND_DELAY: BlockNumber = 43_200;
const DOUBLE: u32 = 2_000_000;

type Params = NodeDepositParams<u128, BlockNumber>;

fn admin() -> AccountId {
	AccountId32::from_ss58check("5CVXqxb7mhFTtZVw5BJ8M2ujND9PFymSDxF8bkod6Sm4XJTW").unwrap()
}

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

fn node(seed: u8) -> ed25519::Pair {
	ed25519::Pair::from_seed(&[seed; 32])
}

fn bank() -> AccountId {
	pallet_hippocampus::Pallet::<Runtime>::account_id()
}

fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| {
		System::set_block_number(1);
		// A live bank account, so fee deltas are exact (no account creation).
		let _ = Balances::deposit_creating(&bank(), 1_000 * UNIT);
	});
	ext
}

/// Mainnet today: lockup on, one free child per family (the default).
fn lockup_on() {
	assert_ok!(Arion::set_lockup_enabled(RuntimeOrigin::signed(admin()), true));
}

/// Every registration pays (no free slot), lockup on.
fn every_node_pays() {
	lockup_on();
	assert_ok!(Arion::set_free_child_slots_per_family(RuntimeOrigin::signed(admin()), 0));
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

fn allow_child(family: &AccountId, child: &AccountId) {
	pallet_proxy::Pallet::<Runtime>::add_proxy(
		RuntimeOrigin::signed(family.clone()),
		child.clone().into(),
		ProxyType::Any,
		0,
	)
	.expect("add_proxy");
}

/// Fund + register the family's main node + proxy its children (the proxy
/// deposits are reserved here). Returns `(free, reserved)` after setup, the
/// baseline every deposit assertion is measured against.
fn setup_family(family: &AccountId, balance: u128, children: &[AccountId]) -> (u128, u128) {
	let _ = Balances::deposit_creating(family, balance);
	let mut node_id = b"family-".to_vec();
	node_id.extend_from_slice(&family.encode()[..4]);
	register_main_node(family, &node_id, pallet_registration::NodeType::StorageMiner);
	for c in children {
		allow_child(family, c);
	}
	(Balances::free_balance(family), Balances::reserved_balance(family))
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

/// What one registration cost the family.
#[derive(Debug, PartialEq, Eq)]
struct Cost {
	/// Left the family for good (the bank fee).
	paid: u128,
	/// Newly reserved on the family account (the child's deposit).
	reserved: u128,
}

fn register(family: &AccountId, child: &AccountId, node: &ed25519::Pair) -> Cost {
	let total_before = Balances::total_balance(family);
	let reserved_before = Balances::reserved_balance(family);
	try_register(family, child, node).expect("register_child");
	Cost {
		paid: total_before - Balances::total_balance(family),
		reserved: Balances::reserved_balance(family) - reserved_before,
	}
}

fn params() -> Params {
	NodeDepositParams {
		multiplier_permill: DOUBLE,
		half_life: HALF_LIFE,
		cap: BASE.saturating_mul(1 << 30),
		fee_share: Perbill::from_percent(10),
		refund_delay: REFUND_DELAY,
	}
}

fn set_params(p: Params) {
	assert_ok!(Arion::set_node_deposit_params(RuntimeOrigin::signed(admin()), p));
}

fn fee_of(price: u128) -> u128 {
	Perbill::from_percent(10).mul_floor(price)
}

fn held(price: u128) -> u128 {
	price - fee_of(price)
}

fn deregister(family: &AccountId, child: &AccountId) {
	assert_ok!(Arion::deregister_child(RuntimeOrigin::signed(family.clone()), child.clone()));
}

fn claim(family: &AccountId, child: &AccountId) -> Result<(), DispatchError> {
	Arion::claim_unbonded(RuntimeOrigin::signed(family.clone()), child.clone())
}

fn deposit_of(child: &AccountId) -> u128 {
	ChildRegistrations::<Runtime>::get(child).expect("registered").deposit
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

fn fee_deposits_to_bank() -> usize {
	System::events()
		.into_iter()
		.filter(|r| {
			matches!(
				r.event,
				RuntimeEvent::Hippocampus(pallet_hippocampus::Event::Deposited {
					deposit_type: DepositType::NodeRegistrationFee,
					..
				})
			)
		})
		.count()
}

#[test]
fn neutral_defaults_keep_the_pre_upgrade_child_deposit() {
	new_test_ext().execute_with(|| {
		lockup_on();
		// Unset config = the pre-existing curve.
		assert!(NodeDepositConfig::<Runtime>::get().is_none());
		assert_eq!(
			Arion::node_deposit_params(),
			NodeDepositParams {
				multiplier_permill: LEGACY_NODE_PRICE_MULTIPLIER_PERMILL,
				half_life: LEGACY_HALF_LIFE,
				cap: u128::MAX,
				fee_share: Perbill::zero(),
				refund_delay: UNBONDING,
			}
		);
		assert_eq!(Arion::current_node_price(), BASE);

		let fam = account(1);
		let kids: Vec<AccountId> = (11..15).map(account).collect();
		setup_family(&fam, START_BALANCE, &kids);
		let bank0 = Balances::free_balance(bank());

		// First child: the family's free slot — nothing charged, price untouched.
		assert_eq!(register(&fam, &kids[0], &node(11)), Cost { paid: 0, reserved: 0 });
		assert_eq!(GlobalLastPaidRegistrationBlock::<Runtime>::get(), 0);
		// Then the doubling curve, fully reserved, no fee.
		assert_eq!(register(&fam, &kids[1], &node(12)), Cost { paid: 0, reserved: BASE });
		assert_eq!(register(&fam, &kids[2], &node(13)), Cost { paid: 0, reserved: 2 * BASE });
		assert_eq!(GlobalNextDeposit::<Runtime>::get(), 4 * BASE);
		assert_eq!(Balances::free_balance(bank()), bank0);
		assert_eq!(fee_deposits_to_bank(), 0);
		assert_eq!(deposit_of(&kids[2]), 2 * BASE);
		assert!(arion_events().contains(&Event::ChildRegistered {
			family: fam.clone(),
			child: kids[2].clone(),
			node_id: node(13).public().0,
			deposit: 2 * BASE,
		}));

		// Halves every day (same values as before at every day boundary).
		System::set_block_number(1 + LEGACY_HALF_LIFE);
		assert_eq!(Arion::current_node_price(), 2 * BASE);
		System::set_block_number(1 + 3 * LEGACY_HALF_LIFE);
		assert_eq!(Arion::current_node_price(), BASE);

		// Unbonding: 7 days, claimed by the family, full amount back.
		System::set_block_number(1);
		deregister(&fam, &kids[2]);
		let reg = ChildRegistrations::<Runtime>::get(&kids[2]).unwrap();
		assert_eq!(reg.status, ChildStatus::Unbonding);
		assert_eq!(reg.unbonding_end, 1 + UNBONDING);
		System::set_block_number(UNBONDING);
		assert_noop!(claim(&fam, &kids[2]), Error::<Runtime>::UnbondingNotReady);
		System::set_block_number(1 + UNBONDING);
		let reserved = Balances::reserved_balance(&fam);
		assert_ok!(claim(&fam, &kids[2]));
		assert_eq!(reserved - Balances::reserved_balance(&fam), 2 * BASE);
		// A free child unbonds immediately, as before.
		deregister(&fam, &kids[0]);
		assert_ok!(claim(&fam, &kids[0]));
	});
}

#[test]
fn mainnet_state_prices_the_next_paid_node_at_the_child_deposit() {
	new_test_ext().execute_with(|| {
		// Mainnet at spec 92017: GlobalNextDeposit = child deposit, never paid (block 0).
		lockup_on();
		let floor = 50_000 * UNIT;
		assert_ok!(Arion::set_base_child_deposit(RuntimeOrigin::signed(admin()), floor));
		GlobalNextDeposit::<Runtime>::put(floor);
		System::set_block_number(7_964_633);
		assert_eq!(Arion::current_node_price(), floor);
	});
}

#[test]
fn lockup_off_charges_nothing() {
	new_test_ext().execute_with(|| {
		set_params(params());
		assert_ok!(Arion::set_free_child_slots_per_family(RuntimeOrigin::signed(admin()), 0));
		let fam = account(1);
		let kids: Vec<AccountId> = (11..14).map(account).collect();
		setup_family(&fam, START_BALANCE, &kids);
		for (i, k) in kids.iter().enumerate() {
			assert_eq!(register(&fam, k, &node(11 + i as u8)), Cost { paid: 0, reserved: 0 });
		}
		assert_eq!(GlobalNextDeposit::<Runtime>::get(), 0);
		assert!(!arion_events().iter().any(|e| matches!(e, Event::NodeDepositCharged { .. })));
		deregister(&fam, &kids[0]);
		assert_eq!(ChildRegistrations::<Runtime>::get(&kids[0]).unwrap().unbonding_end, 1);
	});
}

#[test]
fn burst_of_nodes_pays_a_geometric_sum_with_ten_percent_to_the_bank() {
	new_test_ext().execute_with(|| {
		every_node_pays();
		let base = UNIT;
		assert_ok!(Arion::set_base_child_deposit(RuntimeOrigin::signed(admin()), base));
		set_params(params());
		let bank0 = Balances::free_balance(bank());
		let available0 = ArionPayoutSource::available();

		// 20 nodes in one block, spread over two families: the counter is network-wide.
		let mut total_paid = 0u128;
		let mut total_reserved = 0u128;
		for f in 0..2u8 {
			let fam = account(f + 1);
			let kids: Vec<AccountId> = (0..10u8).map(|i| account(100 + 10 * f + i)).collect();
			setup_family(&fam, 10_000_000 * UNIT, &kids);
			for (i, k) in kids.iter().enumerate() {
				let n = 10 * f as u32 + i as u32;
				let price = base << n;
				assert_eq!(Arion::current_node_price(), price);
				let cost = register(&fam, k, &node(100 + n as u8));
				assert_eq!(cost, Cost { paid: fee_of(price), reserved: held(price) });
				assert_eq!(deposit_of(k), held(price));
				total_paid += cost.paid;
				total_reserved += cost.reserved;
			}
		}
		// base * (2^20 - 1): the 20th node alone costs 524_288x the floor.
		assert_eq!(total_paid + total_reserved, base * ((1 << 20) - 1));
		assert_eq!(total_paid, (0..20).map(|i| fee_of(base << i)).sum::<u128>());
		assert_eq!(GlobalNextDeposit::<Runtime>::get(), base << 20);

		// The 10 % reached the bank, tagged, and is spendable payout money.
		assert_eq!(Balances::free_balance(bank()) - bank0, total_paid);
		assert_eq!(TotalDeposited::<Runtime>::get(DepositType::NodeRegistrationFee), total_paid);
		assert_eq!(ArionPayoutSource::available() - available0, total_paid);
		assert_eq!(pallet_hippocampus::Pallet::<Runtime>::emission_available(), 0);
		assert_eq!(pallet_hippocampus::Pallet::<Runtime>::compute_emission_available(), 0);
		assert_eq!(fee_deposits_to_bank(), 20);
		assert!(arion_events().contains(&Event::NodeDepositCharged {
			family: account(1),
			child: account(100),
			price: base,
			fee: fee_of(base),
			reserved: held(base),
			next_price: 2 * base,
		}));
	});
}

#[test]
fn spread_out_registrations_pay_the_floor() {
	new_test_ext().execute_with(|| {
		every_node_pays();
		set_params(params());
		let fam = account(1);
		let kids: Vec<AccountId> = (11..15).map(account).collect();
		setup_family(&fam, START_BALANCE, &kids);
		for (i, k) in kids.iter().enumerate() {
			let cost = register(&fam, k, &node(11 + i as u8));
			assert_eq!(cost.paid + cost.reserved, BASE);
			// Two days later the doubled price is back at the floor.
			System::set_block_number(System::block_number() + 16 * HALF_LIFE);
		}
	});
}

#[test]
fn price_halves_every_half_life_linear_in_between_never_below_the_floor() {
	new_test_ext().execute_with(|| {
		every_node_pays();
		set_params(params());
		let fam = account(1);
		let kids: Vec<AccountId> = (11..15).map(account).collect();
		setup_family(&fam, START_BALANCE, &kids);
		for i in 0..3u8 {
			register(&fam, &kids[i as usize], &node(11 + i));
		}
		assert_eq!(GlobalNextDeposit::<Runtime>::get(), 8 * BASE);
		assert_eq!(GlobalLastPaidRegistrationBlock::<Runtime>::get(), 1);

		let at = |blocks: BlockNumber| {
			System::set_block_number(1 + blocks);
			Arion::current_node_price()
		};
		assert_eq!(at(0), 8 * BASE);
		assert_eq!(at(HALF_LIFE / 2), 6 * BASE); // linear in between: 8 * 3/4
		assert_eq!(at(HALF_LIFE), 4 * BASE);
		assert_eq!(at(2 * HALF_LIFE), 2 * BASE);
		assert_eq!(at(3 * HALF_LIFE), BASE);
		assert_eq!(at(10 * HALF_LIFE), BASE); // floor
		assert_eq!(at(BlockNumber::MAX - 1), BASE);

		// Continuous at a half-life boundary and monotonic while idle.
		let mut prev = u128::MAX;
		for b in (0..=3 * HALF_LIFE).step_by(97) {
			let p = at(b);
			assert!(p <= prev, "price must not increase while idle");
			prev = p;
		}
		let just_before = at(HALF_LIFE - 1);
		let at_boundary = at(HALF_LIFE);
		assert!(just_before - at_boundary <= 8 * BASE / (2 * HALF_LIFE as u128) + 1);

		// A registration after decay restarts from the decayed price.
		System::set_block_number(1 + HALF_LIFE);
		let cost = register(&fam, &kids[3], &node(14));
		assert_eq!(cost.paid + cost.reserved, 4 * BASE);
		assert_eq!(GlobalNextDeposit::<Runtime>::get(), 8 * BASE);
		assert_eq!(GlobalLastPaidRegistrationBlock::<Runtime>::get(), 1 + HALF_LIFE);
	});
}

#[test]
fn price_is_capped_and_saturates() {
	new_test_ext().execute_with(|| {
		every_node_pays();
		set_params(NodeDepositParams { cap: 5 * BASE, ..params() });
		let fam = account(1);
		let kids: Vec<AccountId> = (11..17).map(account).collect();
		setup_family(&fam, START_BALANCE, &kids);
		let prices: Vec<u128> = (0..6u8)
			.map(|i| {
				let c = register(&fam, &kids[i as usize], &node(11 + i));
				c.paid + c.reserved
			})
			.collect();
		assert_eq!(prices, vec![BASE, 2 * BASE, 4 * BASE, 5 * BASE, 5 * BASE, 5 * BASE]);
		assert_eq!(GlobalNextDeposit::<Runtime>::get(), 5 * BASE);

		// Lowering the cap applies immediately to the stored price.
		set_params(NodeDepositParams { cap: 3 * BASE, ..params() });
		assert_eq!(Arion::current_node_price(), 3 * BASE);

		// A floor raised above the cap wins: flat price.
		assert_ok!(Arion::set_base_child_deposit(RuntimeOrigin::signed(admin()), 4 * BASE));
		assert_eq!(Arion::current_node_price(), 4 * BASE);

		// Extreme values saturate instead of overflowing.
		assert_ok!(Arion::set_base_child_deposit(RuntimeOrigin::signed(admin()), u128::MAX / 2));
		set_params(NodeDepositParams {
			cap: u128::MAX,
			multiplier_permill: MAX_NODE_PRICE_MULTIPLIER_PERMILL,
			..params()
		});
		GlobalNextDeposit::<Runtime>::put(u128::MAX);
		GlobalLastPaidRegistrationBlock::<Runtime>::put(System::block_number());
		assert_eq!(Arion::current_node_price(), u128::MAX);
		// And a registration at that price fails cleanly instead of overflowing.
		let k = account(30);
		allow_child(&fam, &k);
		assert_noop!(try_register(&fam, &k, &node(30)), Error::<Runtime>::InsufficientDeposit);
	});
}

#[test]
fn free_slot_registrations_neither_pay_nor_move_the_price() {
	new_test_ext().execute_with(|| {
		lockup_on();
		set_params(params());
		let bank0 = Balances::free_balance(bank());
		for f in 0..3u8 {
			let fam = account(f + 1);
			setup_family(&fam, START_BALANCE, &[account(f + 11)]);
			assert_eq!(
				register(&fam, &account(f + 11), &node(f + 11)),
				Cost { paid: 0, reserved: 0 }
			);
		}
		assert_eq!(Arion::current_node_price(), BASE);
		assert_eq!(GlobalLastPaidRegistrationBlock::<Runtime>::get(), 0);
		assert_eq!(Balances::free_balance(bank()), bank0);
		assert_eq!(FamilyCount::<Runtime>::get(), 3);
		// The second child of a family pays the floor.
		allow_child(&account(1), &account(21));
		let c = register(&account(1), &account(21), &node(21));
		assert_eq!(c, Cost { paid: fee_of(BASE), reserved: held(BASE) });
	});
}

#[test]
fn insufficient_funds_fail_and_change_nothing() {
	new_test_ext().execute_with(|| {
		every_node_pays();
		set_params(params());
		let fam = account(1);
		let child = account(11);
		let (free0, reserved0) = setup_family(&fam, BASE / 2, &[child.clone()]);
		assert!(free0 < BASE);
		let bank0 = Balances::free_balance(bank());
		let events0 = System::events().len();

		assert_noop!(try_register(&fam, &child, &node(11)), Error::<Runtime>::InsufficientDeposit);
		assert_eq!(Balances::free_balance(&fam), free0);
		assert_eq!(Balances::reserved_balance(&fam), reserved0);
		assert_eq!(Balances::free_balance(bank()), bank0);
		assert_eq!(GlobalNextDeposit::<Runtime>::get(), 0);
		assert_eq!(GlobalLastPaidRegistrationBlock::<Runtime>::get(), 0);
		assert!(ChildRegistrations::<Runtime>::get(&child).is_none());
		assert_eq!(FamilyCount::<Runtime>::get(), 0);
		assert_eq!(System::events().len(), events0);

		// Exactly the price is not enough when there is a fee: the payer must stay alive.
		let _ = Balances::deposit_creating(&fam, BASE - free0);
		assert_eq!(Balances::free_balance(&fam), BASE);
		assert_noop!(try_register(&fam, &child, &node(11)), Error::<Runtime>::InsufficientDeposit);
		let _ = Balances::deposit_creating(&fam, Balances::minimum_balance());
		assert_ok!(try_register(&fam, &child, &node(11)));
	});
}

#[test]
fn deposit_is_returned_after_the_refund_delay_and_only_once() {
	new_test_ext().execute_with(|| {
		every_node_pays();
		set_params(params());
		let fam = account(1);
		let child = account(11);
		let (free0, reserved0) = setup_family(&fam, START_BALANCE, &[child.clone()]);
		register(&fam, &child, &node(11));
		deregister(&fam, &child);
		assert_eq!(
			ChildRegistrations::<Runtime>::get(&child).unwrap().unbonding_end,
			1 + REFUND_DELAY
		);
		System::set_block_number(REFUND_DELAY);
		assert_noop!(claim(&fam, &child), Error::<Runtime>::UnbondingNotReady);
		// Only the family claims.
		System::set_block_number(1 + REFUND_DELAY);
		assert_noop!(
			Arion::claim_unbonded(RuntimeOrigin::signed(account(200)), child.clone()),
			DispatchError::BadOrigin
		);
		assert_ok!(claim(&fam, &child));
		assert_eq!(Balances::reserved_balance(&fam), reserved0);
		// The fee is gone for good, the rest is back.
		assert_eq!(Balances::free_balance(&fam), free0 - fee_of(BASE));
		assert!(arion_events().contains(&Event::ChildUnbonded {
			family: fam.clone(),
			child: child.clone(),
			node_id: node(11).public().0,
			amount: held(BASE),
		}));
		assert_noop!(claim(&fam, &child), Error::<Runtime>::ChildNotRegistered);
	});
}

#[test]
fn force_deregister_and_deregister_family_follow_the_same_unbonding() {
	new_test_ext().execute_with(|| {
		every_node_pays();
		set_params(params());

		// force_deregister_child by a validator-node owner.
		let validator = account(90);
		register_main_node(&validator, b"validator", pallet_registration::NodeType::Validator);
		let fam_a = account(1);
		let child_a = account(11);
		let (_, reserved_a) = setup_family(&fam_a, START_BALANCE, &[child_a.clone()]);
		register(&fam_a, &child_a, &node(11));
		assert_ok!(Arion::force_deregister_child(
			RuntimeOrigin::signed(validator),
			child_a.clone()
		));
		let reg = ChildRegistrations::<Runtime>::get(&child_a).unwrap();
		assert_eq!((reg.status, reg.unbonding_end), (ChildStatus::Unbonding, 1 + REFUND_DELAY));

		// deregister_family (called by execution-unit's purge) no longer refunds on the spot.
		let fam_b = account(2);
		let kids_b = [account(12), account(13)];
		let (_, reserved_b) = setup_family(&fam_b, START_BALANCE, &kids_b);
		register(&fam_b, &kids_b[0], &node(12));
		register(&fam_b, &kids_b[1], &node(13));
		let held_b = held(2 * BASE) + held(4 * BASE);
		assert_eq!(Balances::reserved_balance(&fam_b), reserved_b + held_b);
		assert_ok!(Arion::deregister_family(fam_b.clone()));
		assert_eq!(Balances::reserved_balance(&fam_b), reserved_b + held_b);
		for k in &kids_b {
			let reg = ChildRegistrations::<Runtime>::get(k).unwrap();
			assert_eq!((reg.status, reg.unbonding_end), (ChildStatus::Unbonding, 1 + REFUND_DELAY));
		}
		assert_eq!(FamilyCount::<Runtime>::get(), 0);

		System::set_block_number(1 + REFUND_DELAY);
		assert_ok!(claim(&fam_a, &child_a));
		for k in &kids_b {
			assert_ok!(claim(&fam_b, k));
		}
		assert_eq!(Balances::reserved_balance(&fam_a), reserved_a);
		assert_eq!(Balances::reserved_balance(&fam_b), reserved_b);
	});
}

#[test]
fn same_node_coming_back_before_the_delay_reuses_its_deposit() {
	new_test_ext().execute_with(|| {
		every_node_pays();
		set_params(params());
		let fam = account(1);
		let child = account(11);
		let (_, reserved0) = setup_family(&fam, START_BALANCE, &[child.clone()]);
		register(&fam, &child, &node(11));
		deregister(&fam, &child);

		// Still in cooldown: refused as before.
		assert_noop!(try_register(&fam, &child, &node(11)), Error::<Runtime>::ChildInCooldown);

		System::set_block_number(1 + COOLDOWN);
		let price_before = Arion::current_node_price();
		let stored_before = (
			GlobalNextDeposit::<Runtime>::get(),
			GlobalLastPaidRegistrationBlock::<Runtime>::get(),
		);
		let bank0 = Balances::free_balance(bank());
		assert_eq!(register(&fam, &child, &node(11)), Cost { paid: 0, reserved: 0 });
		assert_eq!(Balances::reserved_balance(&fam), reserved0 + held(BASE));
		assert_eq!(Balances::free_balance(bank()), bank0);
		assert_eq!(Arion::current_node_price(), price_before);
		assert_eq!(
			(
				GlobalNextDeposit::<Runtime>::get(),
				GlobalLastPaidRegistrationBlock::<Runtime>::get()
			),
			stored_before
		);
		let reg = ChildRegistrations::<Runtime>::get(&child).unwrap();
		assert_eq!((reg.status, reg.deposit), (ChildStatus::Active, held(BASE)));
		assert!(arion_events().contains(&Event::ChildDepositReused {
			family: fam.clone(),
			child: child.clone(),
			amount: held(BASE),
		}));

		// Leaving again restarts the delay; the deposit comes back exactly once.
		let now = System::block_number();
		deregister(&fam, &child);
		System::set_block_number(now + REFUND_DELAY);
		assert_ok!(claim(&fam, &child));
		assert_eq!(Balances::reserved_balance(&fam), reserved0);
		assert_noop!(claim(&fam, &child), Error::<Runtime>::ChildNotRegistered);
	});
}

#[test]
fn a_new_key_is_a_new_node() {
	new_test_ext().execute_with(|| {
		every_node_pays();
		set_params(params());
		let fam = account(1);
		let other = account(2);
		let child = account(11);
		let new_child = account(12);
		let (_, reserved0) = setup_family(&fam, START_BALANCE, &[child.clone(), new_child.clone()]);
		setup_family(&other, START_BALANCE, &[child.clone()]);
		register(&fam, &child, &node(11));
		deregister(&fam, &child);
		System::set_block_number(1 + COOLDOWN);

		// Same child account, other node id: refused while the old record unbonds.
		assert_noop!(
			try_register(&fam, &child, &node(21)),
			Error::<Runtime>::ChildAlreadyRegistered
		);
		// Same child account and node id under another family: refused too.
		assert_noop!(
			try_register(&other, &child, &node(11)),
			Error::<Runtime>::ChildAlreadyRegistered
		);
		// Same node id, new child account: a new node, it pays the current price.
		let price = Arion::current_node_price();
		assert_eq!(
			register(&fam, &new_child, &node(11)),
			Cost { paid: fee_of(price), reserved: held(price) }
		);
		// Both deposits are held: the old one is still unbonding.
		assert_eq!(Balances::reserved_balance(&fam), reserved0 + held(BASE) + held(price));
		System::set_block_number(1 + REFUND_DELAY);
		assert_ok!(claim(&fam, &child));
		assert_eq!(Balances::reserved_balance(&fam), reserved0 + held(price));
	});
}

#[test]
fn coming_back_after_the_delay_claims_first_then_pays_again() {
	new_test_ext().execute_with(|| {
		every_node_pays();
		set_params(params());
		let fam = account(1);
		let child = account(11);
		let (_, reserved0) = setup_family(&fam, START_BALANCE, &[child.clone()]);
		register(&fam, &child, &node(11));
		deregister(&fam, &child);
		System::set_block_number(1 + REFUND_DELAY);
		assert_noop!(
			try_register(&fam, &child, &node(11)),
			Error::<Runtime>::ChildAlreadyRegistered
		);
		assert_ok!(claim(&fam, &child));
		assert_eq!(Balances::reserved_balance(&fam), reserved0);
		let price = Arion::current_node_price();
		assert_eq!(price, BASE);
		assert_eq!(
			register(&fam, &child, &node(11)),
			Cost { paid: fee_of(BASE), reserved: held(BASE) }
		);
	});
}

#[test]
fn turning_lockup_off_still_returns_held_deposits() {
	new_test_ext().execute_with(|| {
		every_node_pays();
		set_params(params());
		let fam = account(1);
		let child = account(11);
		let (_, reserved0) = setup_family(&fam, START_BALANCE, &[child.clone()]);
		register(&fam, &child, &node(11));
		assert_ok!(Arion::set_lockup_enabled(RuntimeOrigin::signed(admin()), false));
		deregister(&fam, &child);
		// Lockup off: immediate unbonding, and the reserved deposit is released (it used to be
		// stranded because the release was gated on the switch).
		assert_ok!(claim(&fam, &child));
		assert_eq!(Balances::reserved_balance(&fam), reserved0);
	});
}

#[test]
fn reactivation_respects_max_families() {
	new_test_ext().execute_with(|| {
		let fam = account(1);
		setup_family(&fam, START_BALANCE, &[account(11), account(12)]);
		register(&fam, &account(11), &node(11));
		deregister(&fam, &account(11));
		FamilyCount::<Runtime>::put(MAX_FAMILIES);
		assert_eq!(
			try_register(&fam, &account(12), &node(12)),
			Err(Error::<Runtime>::TooManyFamilies.into())
		);
	});
}

#[test]
fn unrelated_reserves_are_left_alone() {
	new_test_ext().execute_with(|| {
		every_node_pays();
		set_params(params());
		let fam = account(1);
		let child = account(11);
		let (_, reserved0) = setup_family(&fam, START_BALANCE, &[child.clone()]);
		register(&fam, &child, &node(11));
		assert_ok!(Balances::reserve(&fam, 5 * UNIT));
		deregister(&fam, &child);
		System::set_block_number(1 + REFUND_DELAY);
		assert_ok!(claim(&fam, &child));
		assert_eq!(Balances::reserved_balance(&fam), reserved0 + 5 * UNIT);
	});
}

#[test]
fn only_admin_sets_params_within_bounds() {
	new_test_ext().execute_with(|| {
		let set = |origin: RuntimeOrigin, p: Params| Arion::set_node_deposit_params(origin, p);
		let as_admin = || RuntimeOrigin::signed(admin());

		assert_noop!(set(RuntimeOrigin::signed(account(7)), params()), DispatchError::BadOrigin);
		assert_noop!(set(RuntimeOrigin::root(), params()), DispatchError::BadOrigin);

		let rejected: Vec<(Params, Error<Runtime>)> = vec![
			(
				NodeDepositParams { multiplier_permill: 999_999, ..params() },
				Error::NodePriceMultiplierOutOfRange,
			),
			(
				NodeDepositParams {
					multiplier_permill: MAX_NODE_PRICE_MULTIPLIER_PERMILL + 1,
					..params()
				},
				Error::NodePriceMultiplierOutOfRange,
			),
			(NodeDepositParams { half_life: 0, ..params() }, Error::NodePriceHalfLifeOutOfRange),
			(
				NodeDepositParams {
					half_life: BlockNumber::from(MAX_NODE_PRICE_HALF_LIFE_BLOCKS) + 1,
					..params()
				},
				Error::NodePriceHalfLifeOutOfRange,
			),
			(NodeDepositParams { cap: BASE - 1, ..params() }, Error::NodePriceCapBelowBase),
			(
				NodeDepositParams {
					fee_share: Perbill::from_percent(MAX_NODE_REGISTRATION_FEE_SHARE_PERCENT)
						+ Perbill::from_parts(1),
					..params()
				},
				Error::NodeRegistrationFeeShareTooLarge,
			),
			(
				NodeDepositParams {
					refund_delay: BlockNumber::from(MAX_NODE_REFUND_DELAY_BLOCKS) + 1,
					..params()
				},
				Error::NodeRefundDelayTooLong,
			),
		];
		for (p, err) in rejected {
			assert_eq!(set(as_admin(), p), Err(err.into()), "{p:?}");
			assert!(NodeDepositConfig::<Runtime>::get().is_none());
		}

		// The cap is checked against the live child deposit.
		assert_ok!(Arion::set_base_child_deposit(as_admin(), 2 * BASE));
		assert_noop!(
			set(as_admin(), NodeDepositParams { cap: 2 * BASE - 1, ..params() }),
			Error::<Runtime>::NodePriceCapBelowBase
		);
		assert_ok!(Arion::set_base_child_deposit(as_admin(), BASE));

		// Edges are accepted, including the proposed 3-hour half-life.
		let edge = NodeDepositParams {
			multiplier_permill: MAX_NODE_PRICE_MULTIPLIER_PERMILL,
			half_life: MAX_NODE_PRICE_HALF_LIFE_BLOCKS.into(),
			cap: BASE,
			fee_share: Perbill::from_percent(MAX_NODE_REGISTRATION_FEE_SHARE_PERCENT),
			refund_delay: MAX_NODE_REFUND_DELAY_BLOCKS.into(),
		};
		assert_ok!(set(as_admin(), edge));
		assert_ok!(set(as_admin(), NodeDepositParams { multiplier_permill: 1_000_000, ..edge }));
		assert_ok!(set(as_admin(), NodeDepositParams { half_life: 1, ..edge }));
		assert_ok!(set(as_admin(), NodeDepositParams { refund_delay: 0, ..edge }));

		set_params(params());
		assert_eq!(NodeDepositConfig::<Runtime>::get(), Some(params()));
		assert!(arion_events().contains(&Event::NodeDepositParamsSet { params: params() }));

		// ×1 without fee: a flat price equal to the child deposit, nothing left the family.
		set_params(NodeDepositParams {
			multiplier_permill: 1_000_000,
			fee_share: Perbill::zero(),
			..params()
		});
		every_node_pays();
		let fam = account(1);
		let kids: Vec<AccountId> = (11..14).map(account).collect();
		setup_family(&fam, START_BALANCE, &kids);
		for (i, k) in kids.iter().enumerate() {
			assert_eq!(register(&fam, k, &node(11 + i as u8)), Cost { paid: 0, reserved: BASE });
		}

		let info = pallet_arion::Call::<Runtime>::set_node_deposit_params { params: params() }
			.get_dispatch_info();
		assert!(info.weight.ref_time() > 0);
	});
}

#[test]
fn a_failure_after_the_fee_was_paid_rolls_everything_back() {
	new_test_ext().execute_with(|| {
		every_node_pays();
		set_params(params());
		let fam = account(1);
		let child = account(11);
		let (free0, reserved0) = setup_family(&fam, START_BALANCE, &[child.clone()]);
		// Corrupt the family index so the registration fails *after* the charge (fee
		// transferred, deposit reserved, price bumped): the push into the full index fails.
		let full: Vec<AccountId> = (0..35u8).map(|i| account(150 + i)).collect();
		pallet_arion::FamilyChildren::<Runtime>::insert(
			&fam,
			frame_support::BoundedVec::truncate_from(full),
		);
		let bank0 = Balances::free_balance(bank());

		assert_noop!(
			try_register(&fam, &child, &node(11)),
			Error::<Runtime>::TooManyChildrenInFamily
		);
		assert_eq!(Balances::free_balance(&fam), free0);
		assert_eq!(Balances::reserved_balance(&fam), reserved0);
		assert_eq!(Balances::free_balance(bank()), bank0);
		assert_eq!(TotalDeposited::<Runtime>::get(DepositType::NodeRegistrationFee), 0);
		assert_eq!(GlobalLastPaidRegistrationBlock::<Runtime>::get(), 0);
		assert!(ChildRegistrations::<Runtime>::get(&child).is_none());
	});
}

#[test]
fn a_reserve_reduced_elsewhere_is_not_released_from_other_reserves() {
	new_test_ext().execute_with(|| {
		every_node_pays();
		set_params(params());
		let fam = account(1);
		let child = account(11);
		setup_family(&fam, START_BALANCE, &[child.clone()]);
		register(&fam, &child, &node(11));
		deregister(&fam, &child);
		System::set_block_number(1 + REFUND_DELAY);
		// Something outside the pallet released every reserve of the account.
		let all = Balances::reserved_balance(&fam);
		<Balances as ReservableCurrency<AccountId>>::unreserve(&fam, all);
		assert_noop!(claim(&fam, &child), Error::<Runtime>::PartialUnreserve);
		assert_eq!(ChildRegistrations::<Runtime>::get(&child).unwrap().deposit, held(BASE));
	});
}

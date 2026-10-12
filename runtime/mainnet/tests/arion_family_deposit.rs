//! Burst-priced deposit for creating an Arion family.
//!
//! A family is meant to be one operator and one failure domain: CRUSH spreads
//! the shards of a stripe across families. Creating a family used to cost
//! nothing, so a single operator could register many families and hold many
//! shard slots of the same stripe. These tests pin the price that now makes
//! that expensive:
//!
//! - charged when a family becomes active (0 -> 1 active children);
//! - EIP-1559 style: each creation multiplies the price, idle time halves it,
//!   never below `base`, never above `cap`;
//! - `fee_share` of the price goes to the bank (Hippocampus) as
//!   `DepositType::FamilyCreationFee`, the rest is reserved on the family;
//! - the reserved part is claimable `refund_delay` blocks after the family has
//!   no active child left; coming back before that reuses it;
//! - `base == 0` (default) disables everything.

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
	Error, Event, FamilyCount, FamilyDepositConfig, FamilyDepositParams, FamilyDepositRecord,
	FamilyDeposits, FamilyPrice, FamilyPriceUpdatedAt, PayoutSource,
	MAX_FAMILY_CREATION_FEE_SHARE_PERCENT, MAX_FAMILY_PRICE_HALF_LIFE_BLOCKS,
	MAX_FAMILY_PRICE_MULTIPLIER_PERMILL, MAX_FAMILY_REFUND_DELAY_BLOCKS,
};
use pallet_hippocampus::{DepositType, TotalDeposited};
use parity_scale_codec::Encode;
use sp_core::{crypto::Ss58Codec, ed25519, Pair};
use sp_runtime::{AccountId32, BuildStorage, DispatchError, Perbill};

const UNIT: u128 = 1_000_000_000_000_000_000;
const BASE: u128 = 100 * UNIT;
const MAX_FAMILIES: u32 = 600;
const START_BALANCE: u128 = 10_000 * UNIT;
/// One day at 6 s blocks.
const HALF_LIFE: BlockNumber = 14_400;
/// Seven days at 6 s blocks.
const REFUND_DELAY: BlockNumber = 100_800;
const DOUBLE: u32 = 2_000_000;

type Params = FamilyDepositParams<u128, BlockNumber>;

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

/// What one registration cost the family.
#[derive(Debug, PartialEq, Eq)]
struct Cost {
	/// Left the family for good (the bank fee).
	paid: u128,
	/// Newly reserved on the family account.
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

fn params(base: u128) -> Params {
	FamilyDepositParams {
		base,
		multiplier_permill: DOUBLE,
		half_life: HALF_LIFE,
		cap: base.saturating_mul(1 << 30),
		fee_share: Perbill::from_percent(10),
		refund_delay: REFUND_DELAY,
	}
}

fn set_params(p: Params) {
	assert_ok!(Arion::set_family_deposit_params(RuntimeOrigin::signed(admin()), p));
}

fn fee_of(price: u128) -> u128 {
	Perbill::from_percent(10).mul_floor(price)
}

fn deregister(family: &AccountId, child: &AccountId) {
	assert_ok!(Arion::deregister_child(RuntimeOrigin::signed(family.clone()), child.clone()));
}

fn claim(family: &AccountId) -> Result<(), DispatchError> {
	// Permissionless: claimed by an unrelated account on purpose.
	Arion::claim_family_refund(RuntimeOrigin::signed(account(200)), family.clone())
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
fn disabled_by_default_charges_nothing() {
	new_test_ext().execute_with(|| {
		assert_eq!(FamilyDepositConfig::<Runtime>::get().base, 0);
		assert_eq!(Arion::current_family_price(), 0);
		let fam = account(1);
		let bank_before = Balances::free_balance(bank());
		setup_family(&fam, START_BALANCE, &[account(11)]);
		assert_eq!(register(&fam, &account(11), &node(11)), Cost { paid: 0, reserved: 0 });
		assert!(FamilyDeposits::<Runtime>::get(&fam).is_none());
		assert_eq!(FamilyPrice::<Runtime>::get(), 0);
		assert_eq!(FamilyPriceUpdatedAt::<Runtime>::get(), 0);
		assert_eq!(Balances::free_balance(bank()), bank_before);
		assert_eq!(FamilyCount::<Runtime>::get(), 1);

		deregister(&fam, &account(11));
		assert!(!arion_events().iter().any(|e| matches!(
			e,
			Event::FamilyDepositCharged { .. } | Event::FamilyRefundScheduled { .. }
		)));
		assert_eq!(claim(&fam), Err(Error::<Runtime>::NoFamilyRefundScheduled.into()));
	});
}

#[test]
fn burst_in_one_block_pays_a_geometric_sum() {
	new_test_ext().execute_with(|| {
		let base = UNIT;
		set_params(params(base));
		let bank_before = Balances::free_balance(bank());
		let available_before = ArionPayoutSource::available();

		let mut total_paid = 0u128;
		let mut total_reserved = 0u128;
		for i in 0..20u8 {
			let fam = account(i + 1);
			let child = account(i + 101);
			setup_family(&fam, 10_000_000 * UNIT, &[child.clone()]);
			let price = base << i;
			assert_eq!(Arion::current_family_price(), price);
			let cost = register(&fam, &child, &node(i + 101));
			assert_eq!(cost, Cost { paid: fee_of(price), reserved: price - fee_of(price) });
			assert_eq!(
				FamilyDeposits::<Runtime>::get(&fam),
				Some(FamilyDepositRecord { amount: price - fee_of(price), refund_at: None })
			);
			total_paid += cost.paid;
			total_reserved += cost.reserved;
		}
		// base * (2^20 - 1): the 20th family alone costs 524_288x the base.
		assert_eq!(total_paid + total_reserved, base * ((1 << 20) - 1));
		assert_eq!(total_paid, (0..20).map(|i| fee_of(base << i)).sum::<u128>());
		assert_eq!(FamilyCount::<Runtime>::get(), 20);
		assert_eq!(FamilyPrice::<Runtime>::get(), base << 20);

		// The 10 % reached the bank, tagged, and is spendable payout money.
		assert_eq!(Balances::free_balance(bank()) - bank_before, total_paid);
		assert_eq!(TotalDeposited::<Runtime>::get(DepositType::FamilyCreationFee), total_paid);
		assert_eq!(ArionPayoutSource::available() - available_before, total_paid);
		assert_eq!(pallet_hippocampus::Pallet::<Runtime>::emission_available(), 0);
		let tagged = System::events()
			.into_iter()
			.filter(|r| {
				matches!(
					r.event,
					RuntimeEvent::Hippocampus(pallet_hippocampus::Event::Deposited {
						deposit_type: DepositType::FamilyCreationFee,
						..
					})
				)
			})
			.count();
		assert_eq!(tagged, 20);
		assert!(arion_events().contains(&Event::FamilyDepositCharged {
			family: account(1),
			price: base,
			fee: fee_of(base),
			reserved: base - fee_of(base),
			next_price: 2 * base,
		}));
	});
}

#[test]
fn honest_operator_pays_base_when_creations_are_spread_out() {
	new_test_ext().execute_with(|| {
		set_params(params(BASE));
		for i in 0..3u8 {
			let fam = account(i + 1);
			setup_family(&fam, START_BALANCE, &[account(i + 11)]);
			let cost = register(&fam, &account(i + 11), &node(i + 11));
			assert_eq!(cost.paid + cost.reserved, BASE);
			// Two weeks later the doubled price has decayed back to the floor.
			System::set_block_number(System::block_number() + 14 * HALF_LIFE);
		}
	});
}

#[test]
fn price_halves_every_half_life_and_never_drops_below_base() {
	new_test_ext().execute_with(|| {
		set_params(params(BASE));
		// Three creations at block 1: stored price 8 * BASE.
		for i in 0..3u8 {
			setup_family(&account(i + 1), START_BALANCE, &[account(i + 11)]);
			register(&account(i + 1), &account(i + 11), &node(i + 11));
		}
		assert_eq!(FamilyPrice::<Runtime>::get(), 8 * BASE);
		assert_eq!(FamilyPriceUpdatedAt::<Runtime>::get(), 1);

		let at = |blocks: BlockNumber| {
			System::set_block_number(1 + blocks);
			Arion::current_family_price()
		};
		assert_eq!(at(0), 8 * BASE);
		assert_eq!(at(HALF_LIFE / 2), 6 * BASE); // linear in between: 8 * 3/4
		assert_eq!(at(HALF_LIFE), 4 * BASE);
		assert_eq!(at(2 * HALF_LIFE), 2 * BASE);
		assert_eq!(at(3 * HALF_LIFE), BASE);
		assert_eq!(at(10 * HALF_LIFE), BASE); // floor
		assert_eq!(at(BlockNumber::MAX - 1), BASE);

		// Continuous at a half-life boundary and monotonic.
		let mut prev = u128::MAX;
		for b in (0..=3 * HALF_LIFE).step_by(97) {
			let p = at(b);
			assert!(p <= prev, "price must not increase while idle");
			prev = p;
		}
		let just_before = at(HALF_LIFE - 1);
		let at_boundary = at(HALF_LIFE);
		assert!(just_before - at_boundary <= 8 * BASE / (2 * HALF_LIFE as u128) + 1);

		// A creation after decay restarts from the decayed price.
		System::set_block_number(1 + HALF_LIFE);
		setup_family(&account(4), START_BALANCE, &[account(14)]);
		let cost = register(&account(4), &account(14), &node(14));
		assert_eq!(cost.paid + cost.reserved, 4 * BASE);
		assert_eq!(FamilyPrice::<Runtime>::get(), 8 * BASE);
		assert_eq!(FamilyPriceUpdatedAt::<Runtime>::get(), 1 + HALF_LIFE);
	});
}

#[test]
fn price_is_capped() {
	new_test_ext().execute_with(|| {
		set_params(FamilyDepositParams { cap: 5 * BASE, ..params(BASE) });
		let prices: Vec<u128> = (0..6u8)
			.map(|i| {
				setup_family(&account(i + 1), START_BALANCE, &[account(i + 11)]);
				let c = register(&account(i + 1), &account(i + 11), &node(i + 11));
				c.paid + c.reserved
			})
			.collect();
		assert_eq!(prices, vec![BASE, 2 * BASE, 4 * BASE, 5 * BASE, 5 * BASE, 5 * BASE]);
		assert_eq!(FamilyPrice::<Runtime>::get(), 5 * BASE);

		// Lowering the cap applies immediately to the stored price.
		set_params(FamilyDepositParams { cap: 3 * BASE, ..params(BASE) });
		assert_eq!(Arion::current_family_price(), 3 * BASE);

		// Extreme values saturate instead of overflowing.
		set_params(FamilyDepositParams {
			base: u128::MAX / 2,
			cap: u128::MAX,
			multiplier_permill: MAX_FAMILY_PRICE_MULTIPLIER_PERMILL,
			..params(BASE)
		});
		FamilyPrice::<Runtime>::put(u128::MAX);
		FamilyPriceUpdatedAt::<Runtime>::put(System::block_number());
		assert_eq!(Arion::current_family_price(), u128::MAX);
	});
}

#[test]
fn second_child_of_an_active_family_pays_nothing() {
	new_test_ext().execute_with(|| {
		set_params(params(BASE));
		let fam = account(1);
		setup_family(&fam, START_BALANCE, &[account(11), account(12)]);
		assert_eq!(register(&fam, &account(11), &node(11)).paid, fee_of(BASE));
		assert_eq!(register(&fam, &account(12), &node(12)), Cost { paid: 0, reserved: 0 });
		assert_eq!(FamilyCount::<Runtime>::get(), 1);
		assert_eq!(FamilyPrice::<Runtime>::get(), 2 * BASE);
	});
}

#[test]
fn insufficient_balance_fails_and_changes_nothing() {
	new_test_ext().execute_with(|| {
		set_params(params(BASE));
		let fam = account(1);
		let child = account(11);
		let (free0, reserved0) = setup_family(&fam, BASE / 2, &[child.clone()]);
		assert!(free0 < BASE);
		let bank0 = Balances::free_balance(bank());

		assert_eq!(
			try_register(&fam, &child, &node(11)),
			Err(Error::<Runtime>::InsufficientFamilyDeposit.into())
		);
		assert_eq!(Balances::reserved_balance(&fam), reserved0);
		assert_eq!(Balances::free_balance(&fam), free0);
		assert_eq!(Balances::free_balance(bank()), bank0);
		assert_eq!(TotalDeposited::<Runtime>::get(DepositType::FamilyCreationFee), 0);
		assert_eq!(FamilyPrice::<Runtime>::get(), 0);
		assert_eq!(FamilyCount::<Runtime>::get(), 0);
		assert!(FamilyDeposits::<Runtime>::get(&fam).is_none());
		assert!(pallet_arion::ChildRegistrations::<Runtime>::get(&child).is_none());
		assert_eq!(pallet_arion::FamilyActiveChildren::<Runtime>::get(&fam), 0);

		// Exactly the price is not enough either: the account must stay alive.
		let fam2 = account(2);
		setup_family(&fam2, START_BALANCE, &[account(12)]);
		let spare = Balances::free_balance(&fam2) - BASE;
		assert_ok!(Balances::reserve(&fam2, spare));
		assert_eq!(
			try_register(&fam2, &account(12), &node(12)),
			Err(Error::<Runtime>::InsufficientFamilyDeposit.into())
		);
		assert_eq!(FamilyPrice::<Runtime>::get(), 0);
	});
}

#[test]
fn refund_only_after_the_delay() {
	new_test_ext().execute_with(|| {
		set_params(params(BASE));
		let fam = account(1);
		let (c1, c2) = (account(11), account(12));
		let (free0, reserved0) = setup_family(&fam, START_BALANCE, &[c1.clone(), c2.clone()]);
		let held = BASE - fee_of(BASE);
		register(&fam, &c1, &node(11));
		register(&fam, &c2, &node(12));

		deregister(&fam, &c1);
		assert_eq!(FamilyDeposits::<Runtime>::get(&fam).unwrap().refund_at, None);
		assert_eq!(claim(&fam), Err(Error::<Runtime>::NoFamilyRefundScheduled.into()));

		// Re-pricing after the fact must not change what comes back.
		set_params(params(7 * BASE));
		// Last child leaves at block 10: refund scheduled with the delay in force then.
		System::set_block_number(10);
		deregister(&fam, &c2);
		let due = 10 + REFUND_DELAY;
		assert_eq!(
			FamilyDeposits::<Runtime>::get(&fam),
			Some(FamilyDepositRecord { amount: held, refund_at: Some(due) })
		);
		assert!(arion_events().contains(&Event::FamilyRefundScheduled {
			family: fam.clone(),
			amount: held,
			refund_at: due,
		}));
		assert_eq!(Balances::reserved_balance(&fam), reserved0 + held);
		assert_eq!(FamilyCount::<Runtime>::get(), 0);

		System::set_block_number(due - 1);
		assert_eq!(claim(&fam), Err(Error::<Runtime>::FamilyRefundNotDue.into()));
		assert_eq!(Balances::reserved_balance(&fam), reserved0 + held);

		System::set_block_number(due);
		assert_ok!(claim(&fam));
		assert_eq!(Balances::reserved_balance(&fam), reserved0);
		// Only the bank fee is gone for good.
		assert_eq!(Balances::free_balance(&fam), free0 - fee_of(BASE));
		assert!(FamilyDeposits::<Runtime>::get(&fam).is_none());
		assert!(arion_events().contains(&Event::FamilyDepositRefunded {
			family: fam.clone(),
			amount: held,
			missing: 0,
		}));
		assert_eq!(claim(&fam), Err(Error::<Runtime>::NoFamilyRefundScheduled.into()));
	});
}

#[test]
fn reactivation_before_the_delay_reuses_the_deposit() {
	new_test_ext().execute_with(|| {
		set_params(params(BASE));
		let fam = account(1);
		let (_, reserved0) =
			setup_family(&fam, START_BALANCE, &[account(11), account(12), account(13)]);
		let held = BASE - fee_of(BASE);
		register(&fam, &account(11), &node(11));
		deregister(&fam, &account(11));

		System::set_block_number(1 + REFUND_DELAY - 1);
		let price_before = FamilyPrice::<Runtime>::get();
		let bank_before = Balances::free_balance(bank());
		assert_eq!(register(&fam, &account(12), &node(12)), Cost { paid: 0, reserved: 0 });
		assert_eq!(
			FamilyDeposits::<Runtime>::get(&fam),
			Some(FamilyDepositRecord { amount: held, refund_at: None })
		);
		assert!(arion_events()
			.contains(&Event::FamilyDepositReused { family: fam.clone(), amount: held }));
		// Same family coming back: no fee, no double reserve, price unchanged, still counted.
		assert_eq!(Balances::free_balance(bank()), bank_before);
		assert_eq!(Balances::reserved_balance(&fam), reserved0 + held);
		assert_eq!(FamilyPrice::<Runtime>::get(), price_before);
		assert_eq!(FamilyCount::<Runtime>::get(), 1);

		// The old due date is gone: nothing claimable while active.
		System::set_block_number(1 + 2 * REFUND_DELAY);
		assert_eq!(claim(&fam), Err(Error::<Runtime>::NoFamilyRefundScheduled.into()));

		// Leaving again schedules a fresh delay; the same deposit is returned once.
		deregister(&fam, &account(12));
		let due = 1 + 3 * REFUND_DELAY;
		assert_eq!(FamilyDeposits::<Runtime>::get(&fam).unwrap().refund_at, Some(due));
		System::set_block_number(due);
		assert_ok!(claim(&fam));
		assert_eq!(Balances::reserved_balance(&fam), reserved0);
	});
}

#[test]
fn reactivation_after_an_unclaimed_due_refund_pays_again() {
	new_test_ext().execute_with(|| {
		set_params(params(BASE));
		let fam = account(1);
		let (free0, reserved0) = setup_family(&fam, START_BALANCE, &[account(11), account(12)]);
		register(&fam, &account(11), &node(11));
		deregister(&fam, &account(11));

		// Long after the due date and the price decay, nobody claimed.
		System::set_block_number(1 + REFUND_DELAY + 30 * HALF_LIFE);
		let cost = register(&fam, &account(12), &node(12));
		// The old deposit was refunded and a new one charged at the current price.
		assert_eq!(cost.paid, fee_of(BASE));
		assert_eq!(Balances::reserved_balance(&fam), reserved0 + BASE - fee_of(BASE));
		assert_eq!(Balances::free_balance(&fam), free0 - 2 * BASE + (BASE - fee_of(BASE)));
		assert_eq!(
			FamilyDeposits::<Runtime>::get(&fam),
			Some(FamilyDepositRecord { amount: BASE - fee_of(BASE), refund_at: None })
		);
		let events = arion_events();
		assert!(events.iter().any(|e| matches!(e, Event::FamilyDepositRefunded { .. })));
		assert_eq!(
			events
				.iter()
				.filter(|e| matches!(e, Event::FamilyDepositCharged { .. }))
				.count(),
			2
		);
	});
}

#[test]
fn force_deregister_and_deregister_family_also_schedule_the_refund() {
	new_test_ext().execute_with(|| {
		set_params(params(BASE));
		let held = |p: u128| p - fee_of(p);

		// force_deregister_child by a validator-node owner.
		let validator = account(90);
		register_main_node(&validator, b"validator", pallet_registration::NodeType::Validator);
		let fam_a = account(1);
		let child_a = account(11);
		let (_, reserved_a) = setup_family(&fam_a, START_BALANCE, &[child_a.clone()]);
		register(&fam_a, &child_a, &node(11));
		assert_ok!(Arion::force_deregister_child(RuntimeOrigin::signed(validator), child_a));
		assert_eq!(
			FamilyDeposits::<Runtime>::get(&fam_a),
			Some(FamilyDepositRecord { amount: held(BASE), refund_at: Some(1 + REFUND_DELAY) })
		);

		// deregister_family (called by execution-unit's purge).
		let fam_b = account(2);
		let (_, reserved_b) = setup_family(&fam_b, START_BALANCE, &[account(12)]);
		register(&fam_b, &account(12), &node(12));
		assert_ok!(Arion::deregister_family(fam_b.clone()));
		assert_eq!(
			FamilyDeposits::<Runtime>::get(&fam_b),
			Some(FamilyDepositRecord { amount: held(2 * BASE), refund_at: Some(1 + REFUND_DELAY) })
		);
		assert_eq!(FamilyCount::<Runtime>::get(), 0);

		System::set_block_number(1 + REFUND_DELAY);
		assert_ok!(claim(&fam_a));
		assert_ok!(claim(&fam_b));
		assert_eq!(Balances::reserved_balance(&fam_a), reserved_a);
		assert_eq!(Balances::reserved_balance(&fam_b), reserved_b);
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
fn families_created_before_enabling_are_not_charged() {
	new_test_ext().execute_with(|| {
		let fam = account(1);
		let (_, reserved0) = setup_family(&fam, START_BALANCE, &[account(11), account(12)]);
		assert_eq!(register(&fam, &account(11), &node(11)), Cost { paid: 0, reserved: 0 });

		// Turning it on is not retroactive, not even when the existing family grows.
		set_params(params(BASE));
		assert_eq!(register(&fam, &account(12), &node(12)), Cost { paid: 0, reserved: 0 });

		// Leaving schedules nothing. An unrelated reserve on the account is left alone.
		assert_ok!(Balances::reserve(&fam, 5 * UNIT));
		deregister(&fam, &account(11));
		deregister(&fam, &account(12));
		assert_eq!(Balances::reserved_balance(&fam), reserved0 + 5 * UNIT);
		assert!(!arion_events().iter().any(|e| matches!(e, Event::FamilyRefundScheduled { .. })));
		assert_eq!(claim(&fam), Err(Error::<Runtime>::NoFamilyRefundScheduled.into()));
	});
}

#[test]
fn only_admin_sets_params_within_bounds() {
	new_test_ext().execute_with(|| {
		let set = |origin: RuntimeOrigin, p: Params| Arion::set_family_deposit_params(origin, p);
		let as_admin = || RuntimeOrigin::signed(admin());

		assert_noop!(
			Arion::set_family_deposit_params(RuntimeOrigin::signed(account(7)), params(BASE)),
			DispatchError::BadOrigin
		);
		assert_noop!(
			Arion::set_family_deposit_params(RuntimeOrigin::root(), params(BASE)),
			DispatchError::BadOrigin
		);

		let rejected: Vec<(Params, Error<Runtime>)> = vec![
			(
				FamilyDepositParams { multiplier_permill: 999_999, ..params(BASE) },
				Error::FamilyPriceMultiplierOutOfRange,
			),
			(
				FamilyDepositParams {
					multiplier_permill: MAX_FAMILY_PRICE_MULTIPLIER_PERMILL + 1,
					..params(BASE)
				},
				Error::FamilyPriceMultiplierOutOfRange,
			),
			(
				FamilyDepositParams { half_life: 0, ..params(BASE) },
				Error::FamilyPriceHalfLifeOutOfRange,
			),
			(
				FamilyDepositParams {
					half_life: BlockNumber::from(MAX_FAMILY_PRICE_HALF_LIFE_BLOCKS) + 1,
					..params(BASE)
				},
				Error::FamilyPriceHalfLifeOutOfRange,
			),
			(FamilyDepositParams { cap: BASE - 1, ..params(BASE) }, Error::FamilyPriceCapBelowBase),
			(
				FamilyDepositParams {
					fee_share: Perbill::from_percent(MAX_FAMILY_CREATION_FEE_SHARE_PERCENT)
						+ Perbill::from_parts(1),
					..params(BASE)
				},
				Error::FamilyCreationFeeShareTooLarge,
			),
			(
				FamilyDepositParams {
					refund_delay: BlockNumber::from(MAX_FAMILY_REFUND_DELAY_BLOCKS) + 1,
					..params(BASE)
				},
				Error::FamilyRefundDelayTooLong,
			),
		];
		for (p, err) in rejected {
			assert_eq!(set(as_admin(), p), Err(err.into()), "{p:?}");
			assert_eq!(FamilyDepositConfig::<Runtime>::get(), Params::default());
		}

		// Edges are accepted.
		let edge = FamilyDepositParams {
			base: BASE,
			multiplier_permill: MAX_FAMILY_PRICE_MULTIPLIER_PERMILL,
			half_life: MAX_FAMILY_PRICE_HALF_LIFE_BLOCKS.into(),
			cap: BASE,
			fee_share: Perbill::from_percent(MAX_FAMILY_CREATION_FEE_SHARE_PERCENT),
			refund_delay: MAX_FAMILY_REFUND_DELAY_BLOCKS.into(),
		};
		assert_ok!(set(as_admin(), edge));
		assert_ok!(set(as_admin(), FamilyDepositParams { multiplier_permill: 1_000_000, ..edge }));
		assert_ok!(set(as_admin(), FamilyDepositParams { refund_delay: 0, ..edge }));

		set_params(params(BASE));
		assert_eq!(FamilyDepositConfig::<Runtime>::get(), params(BASE));
		assert!(arion_events().contains(&Event::FamilyDepositParamsSet { params: params(BASE) }));

		// base = 0 switches it off again, whatever the other values.
		set_params(params(0));
		assert_eq!(Arion::current_family_price(), 0);

		let info =
			pallet_arion::Call::<Runtime>::set_family_deposit_params { params: params(BASE) }
				.get_dispatch_info();
		assert!(info.weight.ref_time() > 0);
		let info = pallet_arion::Call::<Runtime>::claim_family_refund { family: account(1) }
			.get_dispatch_info();
		assert!(info.weight.ref_time() > 0);
		assert_eq!(info.pays_fee, frame_support::dispatch::Pays::Yes);
	});
}

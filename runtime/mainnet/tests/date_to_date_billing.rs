//! Date-to-date billing: the due-day index, the anniversary cycle, and the
//! bounded drain that replaced the full-map sweep.
//!
//! These tests are written against the invariants in the design note rather
//! than against the implementation, so every expected date and amount below is
//! hand-computed from the calendar. A regression that changes the schedule will
//! disagree with an arithmetic constant here, not with a re-derived expression
//! that would drift along with the bug.
//!
//! The invariants under test, named as the design names them:
//!
//! - `INDEX-COMPLETE` — every active subscription's account is indexed at its
//!   due day. Violated in the "missing entry" direction it is silent free
//!   service, which is why it gets the most coverage here.
//! - `INDEX-ADVISORY` — an index entry is a hint; the subscription is the
//!   truth. A stale entry costs a wasted read, never a phantom charge.
//! - `ANCHOR-DERIVABLE` — absence from `SubscriptionAnchorDay` means "derive
//!   the anchor from the due date", so anchors 1–28 must never write an entry.
//! - `CURSOR-MONOTONIC` — the day cursor advances only on an empty prefix and
//!   never past today.
//! - `WEIGHT-DECLARED` — work per tick is bounded by the cap before it runs.
//! - `NO-ORPHANS` — no active subscription holds a `None` due date once the
//!   backfill completes.
//! - `STORAGE-RECLAIMED` — a cancelled subscription leaves no trace.
//! - `ARREARS-BOUNDED` — a subscription in arrears is charged at most
//!   `max_catchup_months` cycles.

use frame_support::{
	assert_ok,
	traits::{Currency, Hooks},
};
use hippius_mainnet_runtime::{
	AccountId, Balances, Credits, Hippocampus, Marketplace, Runtime, RuntimeOrigin, System,
};
use sp_core::crypto::Ss58Codec;
use sp_runtime::{traits::Hash, AccountId32, BuildStorage};

type Hashed = <Runtime as frame_system::Config>::Hash;

// ── Calendar constants, all hand-computed ────────────────────────────────

/// 2026-01-01T00:00:00Z.
const JAN1_2026_MS: u64 = 1_767_225_600_000;
/// Unix day of 2026-01-01. `1_767_225_600 / 86_400`.
const JAN1_2026_DAY: u32 = 20_454;

const DAY_MS: u64 = 86_400_000;

/// Unix day of the given 2026 date, from `JAN1_2026_DAY` plus the day-of-year
/// offset. Written out rather than computed from a date library so the
/// expectations stay independent of the code under test.
fn day_2026(month: u8, day: u8) -> u32 {
	// Cumulative days before each month in a non-leap year.
	const BEFORE: [u32; 13] = [0, 0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
	JAN1_2026_DAY + BEFORE[month as usize] + day as u32 - 1
}

/// Same, for 2027 — `JAN1_2026_DAY + 365`.
fn day_2027(month: u8, day: u8) -> u32 {
	day_2026(month, day) + 365
}

/// Chain timestamp (ms) for midday on a 2026 date. Midday rather than midnight
/// so a test never sits exactly on a day boundary by accident.
fn ms_2026(month: u8, day: u8) -> u64 {
	u64::from(day_2026(month, day)) * DAY_MS + 12 * 3_600_000
}

// ── Economics ────────────────────────────────────────────────────────────

const PLAN_PRICE: u128 = 1_000;
/// Cheaper second plan, for the partial-payment ordering test.
const CHEAP_PRICE: u128 = 400;
const BANK_FUND: u128 = 100_000_000;

/// `BlockChargeCheckInterval` — the hook only does billing work on multiples
/// of this, so every tick in these tests is a multiple of 8.
const TICK: u64 = 8;

/// `MaxSubscriptionChargesPerRun` in the mainnet runtime.
const CAP: u32 = 128;

// ── Harness ──────────────────────────────────────────────────────────────

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

/// Distinct accounts for the bulk tests, which need more than 256 of them.
fn bulk_account(n: u32) -> AccountId {
	let mut raw = [0u8; 32];
	raw[0] = 0xB0;
	raw[1..5].copy_from_slice(&n.to_le_bytes());
	AccountId32::new(raw)
}

fn authority() -> AccountId {
	account(1)
}

fn backend() -> AccountId {
	account(2)
}

fn admin() -> AccountId {
	AccountId32::from_ss58check("5CVXqxb7mhFTtZVw5BJ8M2ujND9PFymSDxF8bkod6Sm4XJTW").unwrap()
}

fn new_test_ext_at(now_ms: u64) -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| {
		System::set_block_number(1);
		pallet_timestamp::Now::<Runtime>::put(now_ms);
		assert_ok!(Credits::add_authority(RuntimeOrigin::root(), authority()));
		assert_ok!(Marketplace::sudo_set_whitelist_canceller(RuntimeOrigin::root(), backend()));
		assert_ok!(Marketplace::sudo_set_purchase_plan_enabled(RuntimeOrigin::root(), true));
		let _ = Balances::deposit_creating(&Hippocampus::account_id(), BANK_FUND);
		assert_ok!(Hippocampus::add_requester(
			RuntimeOrigin::signed(admin()),
			Marketplace::account_id(),
		));
	});
	ext
}

fn add_plan(name: &[u8], price: u128) -> Hashed {
	assert_ok!(Marketplace::add_new_plan(
		RuntimeOrigin::root(),
		name.to_vec(),
		b"{}".to_vec(),
		b"{}".to_vec(),
		price,
		true,
		false,
		Some(1_000_000),
	));
	<Runtime as frame_system::Config>::Hashing::hash_of(&name.to_vec())
}

/// A compute plan — occupies no storage slot, so an account may hold one
/// alongside a Drive plan and have two subscriptions due on the same day.
fn add_compute_plan(name: &[u8], price: u128) -> Hashed {
	assert_ok!(Marketplace::add_new_plan(
		RuntimeOrigin::root(),
		name.to_vec(),
		b"{}".to_vec(),
		b"{}".to_vec(),
		price,
		false,
		false,
		None,
	));
	<Runtime as frame_system::Config>::Hashing::hash_of(&name.to_vec())
}

fn deposit_credits(who: &AccountId, amount: u128) {
	assert_ok!(Marketplace::deposit(
		RuntimeOrigin::signed(authority()),
		who.clone(),
		amount,
		0,
		false,
		None,
	));
}

fn purchase(owner: &AccountId, plan_id: Hashed) {
	assert_ok!(Marketplace::purchase_plan(
		RuntimeOrigin::signed(backend()),
		owner.clone(),
		vec![plan_id],
		None,
		None,
		None,
		None,
	));
}

fn purchase_upfront(owner: &AccountId, plan_id: Hashed, months: u128) {
	assert_ok!(Marketplace::purchase_plan(
		RuntimeOrigin::signed(backend()),
		owner.clone(),
		vec![plan_id],
		None,
		None,
		None,
		Some(months),
	));
}

fn cancel(owner: &AccountId, id: u32) {
	assert_ok!(Marketplace::cancel_user_subscription(
		RuntimeOrigin::signed(backend()),
		owner.clone(),
		Some(id),
	));
}

fn subs_of(owner: &AccountId) -> Vec<pallet_marketplace::UserPlanSubscription<Runtime>> {
	Marketplace::user_all_subscription_plans(owner)
}

/// The single subscription an account holds, for the many tests that keep one.
fn only_sub(owner: &AccountId) -> pallet_marketplace::UserPlanSubscription<Runtime> {
	let mut subs = subs_of(owner);
	assert_eq!(subs.len(), 1, "expected exactly one subscription");
	subs.pop().unwrap()
}

fn due_day(owner: &AccountId) -> u32 {
	only_sub(owner).next_charge_unix_day.expect("an active subscription has a due date")
}

fn is_indexed_at(day: u32, owner: &AccountId) -> bool {
	pallet_marketplace::DueAccounts::<Runtime>::contains_key(day, owner)
}

/// Every `(day, account)` pair currently in the index.
fn index_entries() -> Vec<(u32, AccountId)> {
	pallet_marketplace::DueAccounts::<Runtime>::iter().map(|(d, a, _)| (d, a)).collect()
}

/// The next block number that is a multiple of `BlockChargeCheckInterval`.
///
/// The hook only does billing work on those blocks, so a tick that lands
/// anywhere else is a silent no-op — which reads in a test exactly like a
/// subscription that was not due.
fn next_tick_block() -> u64 {
	(System::block_number() / TICK + 1) * TICK
}

/// Set the chain clock to a 2026 date and run one billing tick.
fn tick_on(month: u8, day: u8) {
	pallet_timestamp::Now::<Runtime>::put(ms_2026(month, day));
	tick_again();
}

/// Run a tick without moving the clock — for draining a day that needs more
/// than one pass to clear the cap.
fn tick_again() {
	let next = next_tick_block();
	System::set_block_number(next);
	Marketplace::on_initialize(next);
}

/// Advance to the next block that clears `UserRequestsCount`.
///
/// `MaxRequestsPerBlock` is 5 and the counter is only reset on multiples of
/// 15, so any test making more than five purchases has to step over one of
/// those blocks or it hits `TooManyRequests`.
fn clear_request_budget() {
	let next = (System::block_number() / 15 + 1) * 15;
	System::set_block_number(next);
	Marketplace::on_initialize(next);
}

/// Drive the backfill to completion so the index path is live.
///
/// On a fresh ext with no subscriptions this finishes on the first tick, but
/// the tests that seed storage first need it driven explicitly.
fn finish_backfill() {
	for _ in 0..64 {
		if pallet_marketplace::BackfillDone::<Runtime>::get() {
			return;
		}
		tick_again();
	}
	panic!("backfill did not complete");
}

// ── ANCHOR-DERIVABLE ─────────────────────────────────────────────────────

/// The sparse anchor map is only correct if absence means "derive it". An
/// anchor of 1–28 is recoverable from the due date in every month of the year,
/// so writing one would be redundant — and a test that let it be written would
/// stop catching the inverse bug, a missing entry for a 29–31 anchor.
#[test]
fn anchors_up_to_28_are_never_stored() {
	for day in [1u8, 5, 15, 28] {
		new_test_ext_at(ms_2026(1, day)).execute_with(|| {
			let plan = add_plan(b"drive", PLAN_PRICE);
			let user = account(11);
			deposit_credits(&user, 100_000);
			purchase(&user, plan);

			let sub = only_sub(&user);
			assert_eq!(
				pallet_marketplace::SubscriptionAnchorDay::<Runtime>::get(sub.id),
				None,
				"anchor {day} is derivable from the due date and must not be stored",
			);
			assert_eq!(sub.next_charge_unix_day, Some(day_2026(2, day)));
		});
	}
}

/// Anchors that cannot be derived — 29, 30 and 31 — must be stored, because a
/// clamped due date has lost them. Without the entry a 31st subscriber would
/// read back as anchored to whatever short month last clipped them.
#[test]
fn anchors_past_28_are_stored() {
	for day in [29u8, 30, 31] {
		new_test_ext_at(ms_2026(1, day)).execute_with(|| {
			let plan = add_plan(b"drive", PLAN_PRICE);
			let user = account(11);
			deposit_credits(&user, 100_000);
			purchase(&user, plan);

			let sub = only_sub(&user);
			assert_eq!(
				pallet_marketplace::SubscriptionAnchorDay::<Runtime>::get(sub.id),
				Some(day),
				"anchor {day} is not derivable and must be stored",
			);
		});
	}
}

/// The case the whole clamp exists for. A subscriber anchored to the 31st is
/// clipped by February and April, and must come *back* to the 31st rather than
/// sticking on the short month's last day — which is what would happen if the
/// anchor were re-derived from the clamped date instead of remembered.
#[test]
fn a_31st_subscriber_returns_to_the_31st_after_every_short_month() {
	new_test_ext_at(ms_2026(1, 31)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);
		purchase(&user, plan);
		finish_backfill();

		assert_eq!(due_day(&user), day_2026(2, 28), "Feb 2026 has 28 days");

		// (charge month, charge day, expected next due) — hand-computed.
		let schedule = [
			(2u8, 28u8, day_2026(3, 31)),
			(3, 31, day_2026(4, 30)),
			(4, 30, day_2026(5, 31)),
			(5, 31, day_2026(6, 30)),
			(6, 30, day_2026(7, 31)),
			(7, 31, day_2026(8, 31)),
			(8, 31, day_2026(9, 30)),
			(9, 30, day_2026(10, 31)),
			(10, 31, day_2026(11, 30)),
			(11, 30, day_2026(12, 31)),
		];
		for (month, day, expected) in schedule {
			tick_on(month, day);
			assert_eq!(
				due_day(&user),
				expected,
				"charging on 2026-{month}-{day} must schedule the next anniversary",
			);
		}
	});
}

// ── Legacy parity ────────────────────────────────────────────────────────

/// A subscription anchored to the 1st must bill on exactly the days it would
/// have under the 1st-of-month code, for twelve consecutive months. This is the
/// promise made to every account that predates the change: their schedule is
/// bit-for-bit what it was.
#[test]
fn a_1st_anchored_subscription_bills_on_the_1st_for_a_full_year() {
	new_test_ext_at(ms_2026(1, 1)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);
		purchase(&user, plan);
		finish_backfill();

		assert_eq!(due_day(&user), day_2026(2, 1));

		for month in 2u8..=12 {
			tick_on(month, 1);
			let expected = if month == 12 { day_2027(1, 1) } else { day_2026(month + 1, 1) };
			assert_eq!(due_day(&user), expected, "billing on 2026-{month}-01");
			assert_eq!(
				pallet_marketplace::SubscriptionAnchorDay::<Runtime>::get(only_sub(&user).id),
				None,
				"anchor 1 stays derivable all year",
			);
		}
	});
}

/// Twelve charges a year, not the 12.17 a fixed 30-day cycle would produce.
/// A 30-day cycle also walks the date backwards ~5 days annually; the
/// anniversary rule holds the date still.
#[test]
fn a_year_of_anniversaries_is_exactly_twelve_charges() {
	new_test_ext_at(ms_2026(1, 15)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);
		purchase(&user, plan);
		finish_backfill();

		let after_purchase = Credits::get_free_credits(&user);
		for month in 2u8..=12 {
			tick_on(month, 15);
		}
		tick_on(12, 31);

		// Eleven renewals between Feb and Dec; the twelfth charge of the year
		// was the purchase itself.
		assert_eq!(
			after_purchase - Credits::get_free_credits(&user),
			11 * PLAN_PRICE,
			"exactly eleven renewals in the eleven months after purchase",
		);
		assert_eq!(due_day(&user), day_2027(1, 15), "the date never drifts off the 15th");
	});
}

// ── INDEX-COMPLETE ───────────────────────────────────────────────────────

/// The invariant that fails silently: a due subscription with no index entry is
/// never charged, and nothing surfaces it. Purchase must index the account at
/// the day it will actually be due.
#[test]
fn purchase_indexes_the_account_at_its_due_day() {
	new_test_ext_at(ms_2026(3, 9)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);
		purchase(&user, plan);

		let due = day_2026(4, 9);
		assert_eq!(due_day(&user), due);
		assert!(is_indexed_at(due, &user), "INDEX-COMPLETE: purchase must index the account");
		assert_eq!(index_entries().len(), 1, "and index it exactly once");
	});
}

/// Renewal must re-file the account forward. Left at the old day the entry
/// would be drained and dropped, and the subscription would never be charged
/// again — the free-service failure.
#[test]
fn renewal_refiles_the_account_onto_the_next_due_day() {
	new_test_ext_at(ms_2026(3, 9)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);
		purchase(&user, plan);
		finish_backfill();

		let first_due = day_2026(4, 9);
		tick_on(4, 9);

		let second_due = day_2026(5, 9);
		assert_eq!(due_day(&user), second_due);
		assert!(!is_indexed_at(first_due, &user), "the drained day must not keep the entry");
		assert!(is_indexed_at(second_due, &user), "INDEX-COMPLETE after renewal");
		assert_eq!(index_entries(), vec![(second_due, user.clone())]);
	});
}

/// An account holding two plans due on different days holds an entry under
/// each, and draining one day leaves the other alone.
#[test]
fn an_account_due_on_two_days_is_indexed_under_both() {
	new_test_ext_at(ms_2026(3, 9)).execute_with(|| {
		let drive = add_plan(b"drive", PLAN_PRICE);
		let compute = add_compute_plan(b"compute", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);
		purchase(&user, drive);

		// Second plan bought a day later, so it falls due a day later too.
		pallet_timestamp::Now::<Runtime>::put(ms_2026(3, 10));
		purchase(&user, compute);

		let drive_due = day_2026(4, 9);
		let compute_due = day_2026(4, 10);
		assert!(is_indexed_at(drive_due, &user));
		assert!(is_indexed_at(compute_due, &user));

		finish_backfill();
		tick_on(4, 9);

		// The Drive plan renewed to May 9; the compute plan is still due Apr 10.
		assert!(is_indexed_at(day_2026(5, 9), &user), "renewed Drive re-filed");
		assert!(is_indexed_at(compute_due, &user), "the other day's entry survives the drain");
	});
}

/// Cancelling must remove the index entry along with the subscription, or the
/// drain keeps finding an account with nothing to charge.
#[test]
fn cancelling_removes_the_index_entry() {
	new_test_ext_at(ms_2026(3, 9)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);
		purchase(&user, plan);

		let due = day_2026(4, 9);
		let id = only_sub(&user).id;
		assert!(is_indexed_at(due, &user));

		cancel(&user, id);
		assert!(!is_indexed_at(due, &user), "INDEX-COMPLETE has no entry for a dead subscription");
		assert!(index_entries().is_empty());
	});
}

// ── STORAGE-RECLAIMED ────────────────────────────────────────────────────

/// Nothing may be left behind by a cancellation: not the vector entry, not the
/// index key, not the anchor. Dead rows are decoded on every tick forever, so
/// this is a weight leak as much as a space one.
#[test]
fn churn_leaves_nothing_behind() {
	new_test_ext_at(ms_2026(1, 31)).execute_with(|| {
		// Anchored to the 31st so every cycle also writes an anchor entry —
		// the map that would otherwise silently accumulate.
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000_000);

		for _ in 0..100 {
			purchase(&user, plan);
			let id = only_sub(&user).id;
			cancel(&user, id);
			// Two budgets stand between iterations: cancelling to zero active
			// subscriptions arms the `MinSubscriptionBlocks` resubscribe
			// cooldown, and each purchase spends one of the five per-block
			// requests. Stepping to the next multiple of 15 clears both.
			clear_request_budget();
		}

		assert!(subs_of(&user).is_empty(), "the vector is empty");
		assert!(
			!pallet_marketplace::UserAllSubscriptionPlans::<Runtime>::contains_key(&user),
			"STORAGE-RECLAIMED: the map entry itself is gone at zero subscriptions",
		);
		assert!(index_entries().is_empty(), "no index keys survive");
		assert_eq!(
			pallet_marketplace::SubscriptionAnchorDay::<Runtime>::iter().count(),
			0,
			// Not because an id could be reused — `NextSubscriptionId` only ever
			// counts up, so a stale anchor could never be inherited. It is that
			// an anchor is only written for the 29th–31st, so a churning
			// month-end subscriber would leave one entry per cancelled
			// subscription in a map nothing prunes and nothing reads.
			"no anchor entries survive — the map must not grow with churn",
		);
	});
}

// ── INDEX-ADVISORY ───────────────────────────────────────────────────────

/// A drain spans blocks, so a user can cancel between their day opening and
/// their entry being reached. The index entry alone must never authorise a
/// charge — the subscription is re-read and re-checked.
#[test]
fn a_subscription_cancelled_mid_drain_is_not_charged() {
	new_test_ext_at(ms_2026(3, 9)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);
		purchase(&user, plan);
		finish_backfill();

		let due = day_2026(4, 9);
		let id = only_sub(&user).id;

		// The day opens…
		pallet_timestamp::Now::<Runtime>::put(ms_2026(4, 9));
		// …and the user cancels before the drain reaches them. Cancelling
		// refunds nothing here: the cycle in progress is not refundable.
		cancel(&user, id);
		let credits_after_cancel = Credits::get_free_credits(&user);

		tick_again();

		assert_eq!(
			Credits::get_free_credits(&user),
			credits_after_cancel,
			"INDEX-ADVISORY: a cancelled subscription must not be charged",
		);
		assert!(subs_of(&user).is_empty());
		assert!(!is_indexed_at(due, &user));
	});
}

/// A stale index entry — one pointing at an account whose subscription is no
/// longer due — must be dropped rather than wedging the day. If the drain left
/// it in place the prefix would never empty and the cursor would stall
/// forever, which is `CURSOR-MONOTONIC`'s failure mode.
#[test]
fn a_stale_index_entry_is_dropped_and_does_not_wedge_the_cursor() {
	new_test_ext_at(ms_2026(3, 9)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);
		purchase(&user, plan);
		finish_backfill();

		// Forge an entry on a day the subscription is not due on.
		let bogus_day = day_2026(3, 20);
		pallet_marketplace::DueAccounts::<Runtime>::insert(bogus_day, &user, ());

		let before = Credits::get_free_credits(&user);
		tick_on(3, 21);

		assert_eq!(before, Credits::get_free_credits(&user), "not due, so not charged");
		assert!(!is_indexed_at(bogus_day, &user), "the stale entry is dropped");
		assert!(
			pallet_marketplace::DueDayCursor::<Runtime>::get().unwrap() > bogus_day,
			"CURSOR-MONOTONIC: the cursor moves past a day it has emptied",
		);
	});
}

// ── CURSOR-MONOTONIC / catch-up ──────────────────────────────────────────

/// Reading only *today's* prefix would silently drop anyone due on a day the
/// chain did not drain — downtime, or a day that overran the cap. The cursor
/// walks forward from the oldest undrained day instead.
#[test]
fn a_day_skipped_by_downtime_is_still_charged() {
	new_test_ext_at(ms_2026(3, 9)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);
		purchase(&user, plan);
		finish_backfill();

		let before = Credits::get_free_credits(&user);

		// The chain goes away for 40 days, straight past the Apr 9 due date,
		// and comes back on May 19.
		tick_on(5, 19);

		assert_eq!(
			before - Credits::get_free_credits(&user),
			2 * PLAN_PRICE,
			"the missed Apr 9 cycle and the May 9 one are both collected",
		);
		assert_eq!(due_day(&user), day_2026(6, 9), "and the anniversary is not dragged later");
	});
}

/// `ARREARS-BOUNDED`: a subscription that has been in arrears for a year is
/// charged `max_catchup_months` cycles, not twelve. Twelve charges in one block
/// is both a weight problem and a product one.
#[test]
fn arrears_are_capped_at_three_cycles() {
	new_test_ext_at(ms_2026(1, 9)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);
		purchase(&user, plan);
		finish_backfill();

		let before = Credits::get_free_credits(&user);
		// Come back a year later, in one tick.
		tick_on(12, 20);

		assert_eq!(
			before - Credits::get_free_credits(&user),
			3 * PLAN_PRICE,
			"ARREARS-BOUNDED: at most three cycles in one pass",
		);
	});
}

// ── WEIGHT-DECLARED ──────────────────────────────────────────────────────

/// A run is bounded before any of its work starts, and nothing due is lost —
/// whatever a run cannot reach is charged by later runs.
///
/// The bound that binds is the *weight meter*, not the account count. At
/// `RenewalWeightBudget` (5% of a 2000ms block = 100ms) and the pallet's own
/// ~1.4ms per charged account, a run affords ~70 accounts, comfortably under
/// `MaxSubscriptionChargesPerRun` (128). That ordering is deliberate and worth
/// asserting: the count rests on an *estimate* of what an account costs, while
/// the meter measures what was actually spent, so the meter is the one that
/// should run out first. If a future weight change quietly made the count
/// binding again, the hook would be back to trusting an estimate — and this
/// test is what says so.
#[test]
fn a_run_is_bounded_by_the_weight_meter_and_loses_nothing() {
	new_test_ext_at(ms_2026(3, 9)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let total = CAP + 1;

		for n in 0..total {
			let user = bulk_account(n);
			deposit_credits(&user, 100_000);
			purchase(&user, plan);
		}
		finish_backfill();

		let due = day_2026(4, 9);
		assert_eq!(index_entries().len(), total as usize, "all are indexed on the same day");

		let remaining = || pallet_marketplace::DueAccounts::<Runtime>::iter_key_prefix(due).count();

		pallet_timestamp::Now::<Runtime>::put(ms_2026(4, 9));
		tick_again();

		let drained = total as usize - remaining();
		assert!(drained > 0, "a run must make progress");
		assert!(
			drained < CAP as usize,
			"the weight meter must run out before the account count does: drained {drained} \
			 of a {CAP} cap",
		);

		// Keep ticking until the day is empty. Bounded runs mean more than one
		// tick, which is the point — the work is spread, not dropped.
		let mut ticks = 1;
		while remaining() > 0 {
			tick_again();
			ticks += 1;
			assert!(ticks < 16, "the day should drain in a handful of bounded runs");
		}
		assert!(ticks >= 2, "{total} accounts cannot fit in one metered run");

		// Every one of them advanced a cycle — nobody was skipped.
		for n in 0..total {
			let user = bulk_account(n);
			assert_eq!(due_day(&user), day_2026(5, 9), "account {n} renewed");
		}
	});
}

/// The weight the drain reports is the weight it metered, and it never exceeds
/// the budget it was given.
///
/// This is the whole point of metering rather than tallying: `on_initialize` is
/// not rejected for being overweight, so a hook that measures itself after the
/// fact can only report the overrun, never prevent it.
#[test]
fn the_drain_never_returns_more_weight_than_its_budget() {
	use frame_support::traits::Hooks;

	new_test_ext_at(ms_2026(3, 9)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		// Far more due at once than a single run can afford.
		for n in 0..(CAP * 3) {
			let user = bulk_account(n);
			deposit_credits(&user, 100_000);
			purchase(&user, plan);
		}
		finish_backfill();

		let budget = <Runtime as pallet_marketplace::Config>::RenewalWeightBudget::get()
			* <Runtime as frame_system::Config>::BlockWeights::get().max_block;

		pallet_timestamp::Now::<Runtime>::put(ms_2026(4, 9));
		let block = System::block_number() + TICK - (System::block_number() % TICK);
		System::set_block_number(block);
		let consumed = <Marketplace as Hooks<u64>>::on_initialize(block);

		// The hook also runs hourly billing and the backfill probe, so its
		// total is not the drain's alone — but the drain is the only unbounded
		// part, and it must not have spent more than it was allotted.
		assert!(
			consumed.ref_time() <= budget.ref_time() * 2,
			"on_initialize spent {} against a drain budget of {}",
			consumed.ref_time(),
			budget.ref_time(),
		);
	});
}

/// The cursor must never move backwards, and must not outrun today by more
/// than the one day it needs to mark today as drained.
///
/// The drain leaves the cursor at `today + 1` once today's prefix is empty,
/// which is harmless on its own — the next tick simply finds nothing to do
/// until tomorrow. It matters only in combination with an entry filed *behind*
/// it, which is what the arrears test below pins down.
#[test]
fn the_cursor_advances_monotonically_and_tracks_today() {
	new_test_ext_at(ms_2026(3, 9)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);
		purchase(&user, plan);
		finish_backfill();

		let mut previous = pallet_marketplace::DueDayCursor::<Runtime>::get().unwrap();
		for day in [10u8, 11, 15, 20] {
			tick_on(3, day);
			let cursor = pallet_marketplace::DueDayCursor::<Runtime>::get().unwrap();
			assert!(cursor >= previous, "CURSOR-MONOTONIC: cursor went backwards");
			assert!(
				cursor <= day_2026(3, day) + 1,
				"cursor {cursor} ran more than one day past today {}",
				day_2026(3, day),
			);
			previous = cursor;
		}
	});
}

/// A subscription whose arrears exceed `max_catchup_months` is charged three
/// cycles and re-filed at a due date that is *itself* still in the past —
/// potentially behind the day cursor, where nothing would reach it again.
///
/// It converges rather than stranding, and the margin is what makes it safe:
/// the cursor advances at most `MAX_DAY_PROBES` (64) empty days per tick, while
/// three catch-up cycles move the due date forward by at least 84, so the
/// subscription outruns the cursor instead of falling behind it. This test
/// guards that margin — narrowing the catch-up cap or widening the day probe
/// budget without re-checking it would reintroduce the stranding it rules out.
#[test]
fn a_subscription_still_in_arrears_is_not_stranded_behind_the_cursor() {
	new_test_ext_at(ms_2026(1, 9)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);
		purchase(&user, plan);
		finish_backfill();

		// Eleven months of arrears, so three catch-up cycles leave the due date
		// (Apr 9) still far in the past.
		tick_on(12, 20);
		let after_first_pass = Credits::get_free_credits(&user);
		let due_after_catchup = due_day(&user);
		assert!(
			due_after_catchup < day_2026(12, 20),
			"the catch-up cap leaves this subscription still in arrears",
		);

		// The next tick must keep collecting the arrears rather than losing the
		// subscription behind the cursor.
		tick_again();
		assert!(
			Credits::get_free_credits(&user) < after_first_pass,
			"a subscription still in arrears must keep being charged, not be \
			 stranded at day {due_after_catchup} behind the cursor",
		);
	});
}

// ── Partial payment ──────────────────────────────────────────────────────

/// An account with two due plans and credits for only one keeps the lower
/// subscription id, deterministically. Charging is ordered by ascending id
/// precisely so this is not decided by hash order.
#[test]
fn a_short_funded_account_keeps_its_oldest_subscription() {
	new_test_ext_at(ms_2026(3, 9)).execute_with(|| {
		let drive = add_plan(b"drive", PLAN_PRICE);
		let compute = add_compute_plan(b"compute", CHEAP_PRICE);
		let user = account(11);

		deposit_credits(&user, PLAN_PRICE + CHEAP_PRICE);
		purchase(&user, drive);
		purchase(&user, compute);
		finish_backfill();

		let first_id = subs_of(&user).iter().map(|s| s.id).min().unwrap();

		// Fund exactly one Drive cycle for the renewal — not both.
		deposit_credits(&user, PLAN_PRICE);
		tick_on(4, 9);

		let survivors = subs_of(&user);
		assert_eq!(survivors.len(), 1, "the unaffordable subscription is dropped");
		assert_eq!(
			survivors[0].id, first_id,
			"the oldest subscription is the one that survives, every time",
		);
	});
}

/// `INDEX-COMPLETE` across a partial payment — the one path that both moves a
/// subscription forward and drops a sibling in the same write.
///
/// Two subscriptions share a due day, so the account holds a single index key
/// for it. Charging deactivates one and advances the other, and
/// `commit_subscriptions` has to reconcile that from the diff: the shared key
/// must go and the survivor's new day must appear. Getting it wrong in the
/// missing-entry direction leaves a paid-up subscription that nothing ever
/// charges again — silent free service, with a successful charge in the same
/// block to make it look healthy.
#[test]
fn a_partial_payment_reindexes_onto_the_surviving_subscriptions_day() {
	new_test_ext_at(ms_2026(3, 9)).execute_with(|| {
		let drive = add_plan(b"drive", PLAN_PRICE);
		let compute = add_compute_plan(b"compute", CHEAP_PRICE);
		let user = account(11);

		deposit_credits(&user, PLAN_PRICE + CHEAP_PRICE);
		purchase(&user, drive);
		purchase(&user, compute);
		finish_backfill();

		// Both bought on the 9th, so both fall due on the same day and the
		// account holds exactly one key for it.
		assert!(is_indexed_at(day_2026(4, 9), &user), "both subscriptions share one due day");
		assert_eq!(index_entries().len(), 1, "one account-day key, not one per subscription");

		// Fund exactly one Drive cycle, so the compute side cannot be paid.
		deposit_credits(&user, PLAN_PRICE);
		tick_on(4, 9);

		let survivors = subs_of(&user);
		assert_eq!(survivors.len(), 1, "the unaffordable subscription is dropped");
		assert!(survivors[0].package.is_storage_plan, "Drive is the one that survived");

		assert!(
			is_indexed_at(day_2026(5, 9), &user),
			"INDEX-COMPLETE: the survivor is re-filed onto its next anniversary",
		);
		assert!(
			!is_indexed_at(day_2026(4, 9), &user),
			"and the day it was just charged for is released",
		);
		assert_eq!(
			index_entries(),
			vec![(day_2026(5, 9), user.clone())],
			"the deactivated sibling leaves no key of its own behind",
		);

		// The survivor must still bill on that day, which is the guarantee the
		// index exists to provide.
		let before = Credits::get_free_credits(&user);
		deposit_credits(&user, PLAN_PRICE);
		tick_on(5, 9);
		assert_eq!(
			Credits::get_free_credits(&user),
			before + PLAN_PRICE - PLAN_PRICE,
			"the survivor is charged on its next anniversary, not skipped",
		);
	});
}

// ── Plan-change carry credit ─────────────────────────────────────────────

/// Switching plans mid-cycle credits exactly the unused remainder of the old
/// plan, measured against *its own* due date rather than the calendar month.
/// The expected figure is hand-computed: getting this wrong silently over- or
/// under-credits every plan change.
#[test]
fn a_plan_change_credits_the_unused_remainder_of_the_old_cycle() {
	new_test_ext_at(ms_2026(1, 10)).execute_with(|| {
		let solo = add_plan(b"solo", 1_000);
		let max = add_plan(b"max", 3_000);
		let user = account(11);
		deposit_credits(&user, 100_000);

		purchase(&user, solo);
		let start = Credits::get_free_credits(&user);
		assert_eq!(due_day(&user), day_2026(2, 10));

		// Move 10 days into a 31-day cycle (Jan 10 → Feb 10). 21 of 31 days
		// remain, so the carry credit is ⌈1_000 × 21/31⌉ = 678 and the new Max
		// cycle costs 3_000 − 678 = 2_322.
		pallet_timestamp::Now::<Runtime>::put(ms_2026(1, 20));
		assert_ok!(Marketplace::change_storage_plan(
			RuntimeOrigin::signed(backend()),
			user.clone(),
			max,
			None,
			None,
			None,
		));

		assert_eq!(
			start - Credits::get_free_credits(&user),
			2_322,
			"3_000 for the new cycle less the 678 of unused Solo cycle",
		);
		assert_eq!(due_day(&user), day_2026(2, 20), "the new plan starts a fresh cycle today");
	});
}

// ── Upfront purchases ────────────────────────────────────────────────────

/// Upfront months are whole cycles at full price, and the due date is the
/// anchor advanced once per cycle — each step clamped. Three months from Jan 31
/// must land on Apr 30, not on the Apr 3 a naive day-count would produce.
#[test]
fn an_upfront_purchase_buys_whole_clamped_cycles() {
	new_test_ext_at(ms_2026(1, 31)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);

		let before = Credits::get_free_credits(&user);
		purchase_upfront(&user, plan, 3);

		assert_eq!(
			before - Credits::get_free_credits(&user),
			3 * PLAN_PRICE,
			"three whole cycles at full price, no prorated stub",
		);
		assert_eq!(
			due_day(&user),
			day_2026(4, 30),
			"clamped at each step: Jan 31 → Feb 28 → Mar 31 → Apr 30",
		);
		assert!(is_indexed_at(day_2026(4, 30), &user), "INDEX-COMPLETE for upfront purchases");
	});
}

// ── Prepaid span accounting ──────────────────────────────────────────────

/// The conservation rule for a plan change on a prepaid subscription: the
/// whole-cycle refund and the carry credit split the *same* prepaid span, so
/// together they can never hand back more than was paid, and never more than
/// the days the user has not yet used.
///
/// This is the invariant a real over-credit bug broke. `remaining_cycle_value`
/// measured the remaining days against the *due date* rather than against the
/// cycle containing today. On a subscription that prepaid `n` cycles the due
/// date is `n` anniversaries out, so the "days remaining" spanned every prepaid
/// cycle at once while the divisor was a single cycle — and the proration
/// clamps at one whole month, so the carry silently paid out a full month's
/// price no matter how much of the current cycle had already been consumed.
/// Stacked on the correct `n-1` cycle refund, that returned the entire amount
/// paid while the user had already used part of cycle one and was starting a
/// fresh cycle on the new plan, minting the difference as credits.
#[test]
fn a_plan_change_never_credits_back_more_than_the_unused_prepaid_span() {
	// Two same-priced plans, so the change is pure accounting: whatever the
	// user gets back beyond the unused days is credit conjured out of nothing.
	new_test_ext_at(ms_2026(1, 1)).execute_with(|| {
		let solo = add_plan(b"solo", PLAN_PRICE);
		let other = add_plan(b"other", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);

		let start = Credits::get_free_credits(&user);
		purchase_upfront(&user, solo, 2);
		assert_eq!(start - Credits::get_free_credits(&user), 2 * PLAN_PRICE);
		assert_eq!(due_day(&user), day_2026(3, 1), "two whole cycles bought");

		// Day 31 of a 31-day first cycle: 1 of 31 days is unused, so the carry
		// is ceil(1_000 x 1/31) = 33, and the untouched February cycle refunds
		// 1_000. The new Other cycle costs 1_000, so the net movement is
		// 1_000 + 33 - 1_000 = 33 back, leaving 2_000 - 33 spent.
		pallet_timestamp::Now::<Runtime>::put(ms_2026(1, 31));
		assert_ok!(Marketplace::change_storage_plan(
			RuntimeOrigin::signed(backend()),
			user.clone(),
			other,
			None,
			None,
			None,
		));

		let spent = start - Credits::get_free_credits(&user);
		assert_eq!(
			spent,
			2 * PLAN_PRICE - 33,
			"only the unused 1 of 31 days carries over, not a whole month",
		);
		// Conservation, stated as value held: 30 consumed days of the old cycle
		// (967) plus one fresh cycle on the new plan (1_000) is exactly the
		// 1_967 spent. Under the bug the carry paid out a whole month instead
		// of one day, so `spent` came to 1_000 and 967 credits were minted.
		assert!(
			spent >= PLAN_PRICE,
			"a plan change must never refund the days already consumed: spent {spent}",
		);
		// Anchored to the 31st, so the first anniversary clamps to Feb 28.
		assert_eq!(due_day(&user), day_2026(2, 28), "the new plan starts a cycle today");
	});
}

/// The same conservation rule across the whole `pay_upfront` range and at
/// several points within the first cycle. Anything a user can reach through
/// the ordinary buy-then-upgrade flow has to hold the line.
#[test]
fn no_upfront_length_lets_a_plan_change_return_more_than_was_paid() {
	for cycles in [1u128, 2, 3, 12, 24] {
		for change_day in [1u8, 10, 20, 31] {
			new_test_ext_at(ms_2026(1, 1)).execute_with(|| {
				let solo = add_plan(b"solo", PLAN_PRICE);
				let other = add_plan(b"other", PLAN_PRICE);
				let user = account(11);
				deposit_credits(&user, 1_000_000);

				let start = Credits::get_free_credits(&user);
				purchase_upfront(&user, solo, cycles);
				assert_eq!(start - Credits::get_free_credits(&user), cycles * PLAN_PRICE);

				pallet_timestamp::Now::<Runtime>::put(ms_2026(1, change_day));
				assert_ok!(Marketplace::change_storage_plan(
					RuntimeOrigin::signed(backend()),
					user.clone(),
					other,
					None,
					None,
					None,
				));

				// Days consumed on the old plan are gone; one fresh cycle on
				// the new plan is owed in full. So out-of-pocket can never drop
				// below the single cycle the user is now holding.
				let spent = start - Credits::get_free_credits(&user);
				assert!(
					spent >= PLAN_PRICE,
					"cycles={cycles} change_day={change_day}: spent {spent} is less than \
					 the one cycle the user now holds — credits were minted",
				);
				// And bounded above independently of how many cycles were
				// prepaid: every untouched cycle is refunded, so the most a
				// user can be out of pocket is the old cycle they consumed
				// plus the one new cycle they now hold. A bound that grew with
				// `cycles` would let an unrefunded cycle hide inside it.
				assert!(
					spent <= 2 * PLAN_PRICE,
					"cycles={cycles} change_day={change_day}: spent {spent} exceeds one \
					 consumed cycle plus one new cycle — a prepaid cycle went unrefunded",
				);
			});
		}
	}
}

// ── Refunds on cancel ────────────────────────────────────────────────────

/// Cancelling refunds the whole cycles that have not started, counted along
/// the subscription's *own* anniversaries — and never the cycle in progress,
/// which the holder is using.
///
/// This is the path the refund rewrite exists for, and the one an anchored-to-
/// the-1st test cannot distinguish: for a 1st-anchored subscription the
/// anniversary boundaries and the calendar-month boundaries are the same dates,
/// so the old 1st-of-month walk and the new `add_months_clamped` walk agree by
/// coincidence. Anchored to the 10th they diverge, and only then does the test
/// actually pin which rule is in force.
#[test]
fn cancelling_refunds_whole_cycles_along_its_own_anniversaries() {
	new_test_ext_at(ms_2026(1, 10)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);

		let start = Credits::get_free_credits(&user);
		purchase_upfront(&user, plan, 3);
		assert_eq!(start - Credits::get_free_credits(&user), 3 * PLAN_PRICE);
		assert_eq!(due_day(&user), day_2026(4, 10), "three cycles from the 10th");

		// Feb 20 sits inside the second cycle (Feb 10 -> Mar 10). Walking back
		// from Apr 10: Mar 10 is still ahead of today and counts; Feb 10 is
		// behind it and stops the walk. So exactly one whole cycle is unused.
		//
		// Under the old 1st-of-month rule the boundaries would have been Mar 1
		// and Apr 1 — two of them ahead of Feb 20 — and the refund would have
		// been 2_000 for a subscription that has one unused cycle.
		pallet_timestamp::Now::<Runtime>::put(ms_2026(2, 20));
		let id = only_sub(&user).id;
		cancel(&user, id);

		assert_eq!(
			start - Credits::get_free_credits(&user),
			2 * PLAN_PRICE,
			"the used cycle and the one in progress are kept, the third refunded",
		);
		assert!(subs_of(&user).is_empty(), "and the subscription is gone");
		assert!(index_entries().is_empty(), "with no index entry left behind");
	});
}

/// The cycle in progress is never refunded, even on its very last day — the
/// holder had the service for all of it.
#[test]
fn cancelling_never_refunds_the_cycle_in_progress() {
	new_test_ext_at(ms_2026(1, 10)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);

		let start = Credits::get_free_credits(&user);
		purchase(&user, plan);

		// One day before the cycle ends.
		pallet_timestamp::Now::<Runtime>::put(ms_2026(2, 9));
		let id = only_sub(&user).id;
		cancel(&user, id);

		assert_eq!(
			start - Credits::get_free_credits(&user),
			PLAN_PRICE,
			"a single-cycle subscription refunds nothing on cancel",
		);
	});
}

// ── Backfill ─────────────────────────────────────────────────────────────

/// The backfill must be resumable: interrupting and resuming the cursor at any
/// point yields the same index, and no existing subscription's charge day
/// moves. It runs as ordinary paginated hook work precisely so it can be
/// interrupted by a block boundary.
#[test]
fn the_backfill_is_resumable_and_preserves_every_due_date() {
	new_test_ext_at(ms_2026(3, 9)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		// More than `MaxBackfillAccountsPerRun` (256) so it genuinely spans ticks.
		let total = 300u32;
		for n in 0..total {
			let user = bulk_account(n);
			deposit_credits(&user, 100_000);
			purchase(&user, plan);
		}

		let due_before: Vec<u32> = (0..total).map(|n| due_day(&bulk_account(n))).collect();

		// Wipe the index that `purchase` built, to force the backfill to
		// rebuild it from the subscriptions alone.
		let _ = pallet_marketplace::DueAccounts::<Runtime>::clear(u32::MAX, None);
		pallet_marketplace::BackfillDone::<Runtime>::put(false);
		pallet_marketplace::BackfillCursor::<Runtime>::kill();
		assert!(index_entries().is_empty());

		// Drive it one tick at a time — the interruption is the block boundary.
		let mut ticks = 0;
		while !pallet_marketplace::BackfillDone::<Runtime>::get() {
			tick_again();
			ticks += 1;
			assert!(ticks < 64, "backfill should finish in a handful of ticks");
		}
		assert!(ticks >= 2, "300 accounts at 256 per tick must span more than one tick");

		let mut rebuilt = index_entries();
		rebuilt.sort();
		assert_eq!(rebuilt.len(), total as usize, "every account is indexed");

		for (n, expected) in due_before.iter().enumerate() {
			let user = bulk_account(n as u32);
			assert_eq!(due_day(&user), *expected, "account {n} keeps its exact charge day");
			assert!(is_indexed_at(*expected, &user), "INDEX-COMPLETE after backfill");
		}
	});
}

/// `NO-ORPHANS`: a legacy `None` due date reads as "always due" through
/// `map_or(true, ..)`, and an index has no key for it — so any survivor would
/// silently stop being billed once the fallback scan is gone. The backfill
/// writes it out as the 1st of the current month, which is exactly what the old
/// sweep already read it to mean.
#[test]
fn the_backfill_normalises_legacy_none_due_dates() {
	new_test_ext_at(ms_2026(3, 9)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);
		purchase(&user, plan);

		// Re-create the legacy shape: an active subscription with no due date.
		let mut subs = subs_of(&user);
		subs[0].next_charge_unix_day = None;
		pallet_marketplace::UserAllSubscriptionPlans::<Runtime>::insert(&user, subs);
		let _ = pallet_marketplace::DueAccounts::<Runtime>::clear(u32::MAX, None);
		pallet_marketplace::BackfillDone::<Runtime>::put(false);
		pallet_marketplace::BackfillCursor::<Runtime>::kill();

		let before = Credits::get_free_credits(&user);
		finish_backfill();

		// The backfill normalises the `None` to the 1st of the current month —
		// exactly what the old sweep already read it to mean — which makes the
		// subscription immediately due, so the same tick also charges it. The
		// observable guarantee is therefore not the intermediate value but that
		// this account keeps billing, on precisely the legacy 1st-of-month
		// schedule it would have had.
		assert_eq!(
			before - Credits::get_free_credits(&user),
			PLAN_PRICE,
			"NO-ORPHANS: a legacy None is picked up and billed, not silently skipped",
		);
		let sub = only_sub(&user);
		assert_eq!(
			sub.next_charge_unix_day,
			Some(day_2026(4, 1)),
			"and lands back on the 1st-of-month schedule",
		);
		assert!(
			is_indexed_at(day_2026(4, 1), &user),
			"and the account is represented in the index",
		);
	});
}

/// `NO-ORPHANS` as a global sweep rather than a single-account case.
///
/// The other tests each assert the invariant for the one account they set up.
/// This one poses a mixed population — legacy `None` rows, ordinary dated rows,
/// an inactive row, and an account that churns during the run — then walks the
/// *entire* map afterwards and asserts that every active subscription both
/// carries a due date and is reachable through the index.
///
/// The reachability half is the point. A due date with no matching index key is
/// still silent free service: the charge path reads it as due, but the drain
/// only ever visits accounts it finds through keys, so nothing would ever ask.
#[test]
fn no_active_subscription_is_left_unreachable_after_the_backfill() {
	new_test_ext_at(ms_2026(3, 9)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let compute = add_compute_plan(b"compute", CHEAP_PRICE);

		// A spread of shapes, all in one map.
		for n in 0..12u32 {
			let user = bulk_account(n);
			deposit_credits(&user, 100_000);
			purchase(&user, plan);
			if n % 3 == 0 {
				purchase(&user, compute);
			}
		}

		// Legacy `None` on a third of them, and one fully dead row.
		for n in (0..12u32).step_by(4) {
			let user = bulk_account(n);
			let mut subs = subs_of(&user);
			subs[0].next_charge_unix_day = None;
			pallet_marketplace::UserAllSubscriptionPlans::<Runtime>::insert(&user, subs);
		}
		let corpse = bulk_account(7);
		let mut dead = subs_of(&corpse);
		dead[0].active = false;
		pallet_marketplace::UserAllSubscriptionPlans::<Runtime>::insert(&corpse, dead);

		let _ = pallet_marketplace::DueAccounts::<Runtime>::clear(u32::MAX, None);
		pallet_marketplace::BackfillDone::<Runtime>::put(false);
		pallet_marketplace::BackfillCursor::<Runtime>::kill();
		finish_backfill();

		// Sweep the whole map, not one account.
		let mut checked = 0;
		for (who, subs) in pallet_marketplace::UserAllSubscriptionPlans::<Runtime>::iter() {
			for sub in subs.iter().filter(|s| s.active) {
				let due = sub.next_charge_unix_day.unwrap_or_else(|| {
					panic!("NO-ORPHANS: {who:?} sub {} is active with no due date", sub.id)
				});
				assert!(
					is_indexed_at(due, &who),
					"NO-ORPHANS: {who:?} sub {} is due on day {due} with no index key — \
					 the drain would never reach it",
					sub.id,
				);
				checked += 1;
			}
		}
		assert!(checked >= 12, "the sweep must actually have inspected the population");
	});
}

/// An *inactive* legacy row carrying a `None` due date is left entirely alone.
///
/// This shape is what the backfill actually meets on chain: the v2 migration
/// carries `active` and `next_charge_unix_day` through verbatim, and the
/// "drop deactivated rows" filter in `commit_subscriptions` only arrived with
/// date-to-date billing — so cancelled subscriptions from before it are still
/// sitting in stored vectors with whatever due date they had.
///
/// Two things must hold, and neither is implied by the active-row test above.
/// A `None` on a dead row must not be normalised into a real due date, and it
/// must not be filed in the index — either would resurrect a cancelled
/// subscription into the charging path. The active account alongside it is
/// deliberate: it shares the backfill batch, so anything that lets one
/// account's normalisation leak into the decision made for another shows up
/// here.
#[test]
fn the_backfill_leaves_an_inactive_legacy_row_alone() {
	new_test_ext_at(ms_2026(3, 9)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);

		let live = account(11);
		deposit_credits(&live, 100_000);
		purchase(&live, plan);

		// Two dead rows, carrying the two due dates a cancelled subscription can
		// be left holding. The `None` one probes normalisation; the *future*
		// one probes indexing, and it has to be in the future to probe anything
		// at all — a dead row filed on a past day would be swept straight back
		// out by the drain in the same tick, erasing the evidence before the
		// assertion runs.
		let compute = add_compute_plan(b"compute", CHEAP_PRICE);
		let dead = account(12);
		deposit_credits(&dead, 100_000);
		purchase(&dead, plan);
		purchase(&dead, compute);

		// Re-create the two legacy shapes side by side: an active row with no
		// due date, and deactivated ones the old code left in the vector.
		let mut live_subs = subs_of(&live);
		live_subs[0].next_charge_unix_day = None;
		pallet_marketplace::UserAllSubscriptionPlans::<Runtime>::insert(&live, live_subs);

		let mut dead_subs = subs_of(&dead);
		assert_eq!(dead_subs.len(), 2);
		dead_subs[0].active = false;
		dead_subs[0].next_charge_unix_day = None;
		dead_subs[1].active = false;
		dead_subs[1].next_charge_unix_day = Some(day_2026(4, 9));
		pallet_marketplace::UserAllSubscriptionPlans::<Runtime>::insert(&dead, dead_subs);

		let _ = pallet_marketplace::DueAccounts::<Runtime>::clear(u32::MAX, None);
		pallet_marketplace::BackfillDone::<Runtime>::put(false);
		pallet_marketplace::BackfillCursor::<Runtime>::kill();

		let dead_credits_before = Credits::get_free_credits(&dead);
		finish_backfill();

		// The dead rows are untouched: same `active`, same due dates.
		let after = pallet_marketplace::UserAllSubscriptionPlans::<Runtime>::get(&dead);
		assert_eq!(after.len(), 2, "the backfill does not prune, it only indexes");
		assert!(after.iter().all(|s| !s.active), "an inactive row stays inactive");
		assert_eq!(
			after[0].next_charge_unix_day, None,
			"a dead row's None is not normalised — only active rows get a due date",
		);

		// Nothing will ever try to charge them. The future-dated row is the
		// load-bearing half: the cursor is nowhere near April, so if the
		// backfill had filed it the key would still be sitting there.
		assert!(
			!is_indexed_at(day_2026(4, 9), &dead),
			"INDEX-COMPLETE covers active subscriptions only: a dead row is never filed",
		);
		assert!(
			!pallet_marketplace::DueAccounts::<Runtime>::iter().any(|(_, who, _)| who == dead),
			"and the account holds no index key at all",
		);
		assert_eq!(
			Credits::get_free_credits(&dead),
			dead_credits_before,
			"and it is never charged",
		);

		// The active account in the same batch is still normalised and billed,
		// so this is not passing because the backfill did nothing at all.
		assert_eq!(
			only_sub(&live).next_charge_unix_day,
			Some(day_2026(4, 1)),
			"the live row alongside it is still picked up",
		);
		assert!(is_indexed_at(day_2026(4, 1), &live), "and indexed");
	});
}

/// The transitional full scan is bounded and round-robins, so the upgrade
/// window cannot produce a heavy block.
///
/// This is the branch's worst moment by construction: the backfill takes
/// `accounts / MaxBackfillAccountsPerRun` ticks to finish, and the fallback is
/// live on every one of them. Unbounded, that is a full walk of every account
/// on every tick for hours, starting the instant the runtime upgrade lands —
/// the point at which a chain can least afford one. Bounded, each tick charges
/// what it can afford and the cursor brings it back to the rest.
#[test]
fn the_pre_index_fallback_is_bounded_and_reaches_everyone() {
	new_test_ext_at(ms_2026(3, 9)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		// More than `MaxBackfillAccountsPerRun` (256), so the backfill genuinely
		// spans ticks and the fallback is live for more than one of them — which
		// is the situation being tested.
		let total = 300u32;
		for n in 0..total {
			let user = bulk_account(n);
			deposit_credits(&user, 100_000);
			purchase(&user, plan);
		}

		// Pose the upgrade window: index empty, backfill not done.
		let _ = pallet_marketplace::DueAccounts::<Runtime>::clear(u32::MAX, None);
		pallet_marketplace::BackfillDone::<Runtime>::put(false);
		pallet_marketplace::BackfillCursor::<Runtime>::kill();
		pallet_marketplace::FullScanCursor::<Runtime>::kill();

		let budget = <Runtime as pallet_marketplace::Config>::RenewalWeightBudget::get()
			* <Runtime as frame_system::Config>::BlockWeights::get().max_block;

		// One tick, still inside the window, must not spend more than its
		// budget however many accounts exist.
		pallet_timestamp::Now::<Runtime>::put(ms_2026(4, 9));
		let block = next_tick_block();
		System::set_block_number(block);
		let consumed = <Marketplace as frame_support::traits::Hooks<u64>>::on_initialize(block);

		// The backfill and the fallback share one budget and the backfill draws
		// first, so on this tick it may take the whole of it and leave the scan
		// nothing — that is the intended priority, not a failure. What must hold
		// is the sum.
		assert!(
			!pallet_marketplace::BackfillDone::<Runtime>::get(),
			"300 accounts cannot be indexed in one metered tick",
		);
		assert!(
			consumed.ref_time() <= budget.ref_time() * 2,
			"the upgrade-window tick spent {} against a shared budget of {}",
			consumed.ref_time(),
			budget.ref_time(),
		);

		// Every later tick stays inside the budget too, and between them the
		// backfill finishes and the charges all land.
		for _ in 0..40 {
			let next = next_tick_block();
			System::set_block_number(next);
			let spent = <Marketplace as frame_support::traits::Hooks<u64>>::on_initialize(next);
			assert!(
				spent.ref_time() <= budget.ref_time() * 2,
				"a later upgrade-window tick spent {}",
				spent.ref_time(),
			);
		}
		for n in 0..total {
			let user = bulk_account(n);
			assert_eq!(
				due_day(&user),
				day_2026(5, 9),
				"account {n} was reached by the paged fallback",
			);
		}
	});
}

/// Until the backfill finishes, an account in its untouched tail has no index
/// entry — so the sweep must fall back to the full scan rather than skip them.
/// Dropping the fallback early is silent free service for the tail.
#[test]
fn charging_falls_back_to_the_full_scan_until_the_backfill_completes() {
	new_test_ext_at(ms_2026(3, 9)).execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 100_000);
		purchase(&user, plan);

		// A due subscription with no index entry at all, and the backfill
		// deliberately not done.
		let _ = pallet_marketplace::DueAccounts::<Runtime>::clear(u32::MAX, None);
		pallet_marketplace::BackfillDone::<Runtime>::put(false);
		pallet_marketplace::BackfillCursor::<Runtime>::kill();

		let before = Credits::get_free_credits(&user);
		pallet_timestamp::Now::<Runtime>::put(ms_2026(4, 9));
		tick_again();

		assert_eq!(
			before - Credits::get_free_credits(&user),
			PLAN_PRICE,
			"the pre-index full scan still charges the tail",
		);
	});
}

// ── INDEX-COMPLETE, for cycles that collect nothing ──────────────────────

/// A cycle that collects nothing still has to advance, or the account leaves
/// the index and never comes back.
///
/// `INDEX-COMPLETE` in the "missing entry" direction, reached without any
/// failure: the drain removes an account's entry *before* charging and leaves
/// `commit_subscriptions` to re-file it, but that only writes days that
/// changed. A due day that does not move is therefore re-filed by nobody, the
/// cursor walks past, and the account is stranded — active, due, and
/// unreachable. A plan priced at zero is the way in, and root sets one up
/// whenever it runs a free tier.
#[test]
fn a_zero_priced_cycle_advances_and_stays_indexed() {
	new_test_ext_at(ms_2026(1, 15)).execute_with(|| {
		let plan = add_plan(b"promo", 0);
		let user = account(11);
		deposit_credits(&user, 100_000);
		purchase(&user, plan);
		finish_backfill();

		let first_due = due_day(&user);
		assert_eq!(first_due, day_2026(2, 15));
		assert!(is_indexed_at(first_due, &user), "a free plan is still indexed");

		tick_on(2, 15);

		let next_due = due_day(&user);
		assert_eq!(next_due, day_2026(3, 15), "a free cycle advances like a paid one");
		assert!(is_indexed_at(next_due, &user), "and is re-filed at its new day");
		assert!(!is_indexed_at(first_due, &user), "leaving nothing behind at the old one");
	});
}

/// Ending a free promotion must not have written off everyone who took it.
///
/// This is the whole point of the previous test, played out: the stranding is
/// silent while the plan is free — nobody notices an account not being charged
/// zero — and only shows up as permanent free service once the price goes back
/// up. Repricing rewrites the snapshot but touches neither the index nor the
/// cursor, so an account that fell out while the plan was free never returns on
/// its own.
#[test]
fn ending_a_free_promo_bills_its_holders_again() {
	new_test_ext_at(ms_2026(1, 15)).execute_with(|| {
		let plan = add_plan(b"promo", 0);
		let user = account(11);
		deposit_credits(&user, 100_000);
		purchase(&user, plan);
		finish_backfill();

		// Two free cycles — enough that a stranded account would have been
		// dropped by the cursor well before the promotion ends.
		tick_on(2, 15);
		tick_on(3, 15);
		assert_eq!(due_day(&user), day_2026(4, 15));

		// The promotion ends. The walk rewrites the price this account renews on.
		assert_ok!(Marketplace::set_plan_price(RuntimeOrigin::root(), plan, PLAN_PRICE));
		for _ in 0..8 {
			tick_again();
		}
		assert_eq!(only_sub(&user).package.price, PLAN_PRICE, "the reprice reached the snapshot");

		let before = Credits::get_free_credits(&user);
		tick_on(4, 15);
		assert_eq!(
			before.saturating_sub(Credits::get_free_credits(&user)),
			PLAN_PRICE,
			"a holder of an ended promotion is charged on the next anniversary",
		);
	});
}

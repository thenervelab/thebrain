//! `set_plan_price` and the paged walk that carries a reprice into the
//! subscriptions already holding the plan.
//!
//! Repricing is two writes, not one. `Plans` is what a *future* purchase reads,
//! but every live subscription carries its own copy of the plan taken at
//! purchase time and the monthly charge bills that copy — so a price change
//! only reaches existing subscribers once their snapshots are rewritten. The
//! rewrite is an unbounded walk, so it lives in `on_initialize` behind a cursor
//! rather than in the extrinsic.
//!
//! The invariants under test:
//!
//! - `REPRICE-REACHES-SNAPSHOTS` — when the walk retires, every subscription on
//!   the plan carries the price in `Plans`. A split between the two is silent:
//!   nothing on chain says which accounts are on the superseded figure.
//! - `REPRICE-CONVERGES` — a plan repriced again mid-walk finishes on the
//!   latest price, for every holder, not just the ones the cursor had yet to
//!   reach.
//! - `REPRICE-RETIRES` — the job leaves the queue and the cursor clean, and
//!   reports what it rewrote.
//! - `PREPAID-UNTOUCHED` — `paid_per_month` records what a holder actually paid
//!   for months already prepaid, and a reprice is not a retroactive rebill.

use frame_support::{
	assert_noop, assert_ok,
	traits::{Currency, Hooks},
};
use hippius_mainnet_runtime::{
	AccountId, Balances, Credits, Hippocampus, Marketplace, Runtime, RuntimeOrigin, System,
};
use sp_core::crypto::Ss58Codec;
use sp_runtime::{traits::Hash, AccountId32, BuildStorage};

type Hashed = <Runtime as frame_system::Config>::Hash;

/// 2026-01-15T12:00:00Z — mid-month, so no test sits on a boundary.
const JAN15_2026_MS: u64 = 1_768_478_400_000;

const PLAN_PRICE: u128 = 1_000;
const BANK_FUND: u128 = 100_000_000;

/// `MaxRepricedAccountsPerBlock` in the mainnet runtime.
const BATCH: u32 = 250;

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

/// Distinct accounts for the paging tests, which need more than a batch of them.
fn bulk_account(n: u32) -> AccountId {
	let mut raw = [0u8; 32];
	raw[0] = 0xA0;
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

fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| {
		System::set_block_number(1);
		pallet_timestamp::Now::<Runtime>::put(JAN15_2026_MS);
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

/// A compute plan — occupies no storage slot, so an account can hold one
/// alongside a Drive plan.
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

/// A holder of `plan`, funded and subscribed.
fn holder(who: &AccountId, plan: Hashed) {
	deposit_credits(who, 1_000_000);
	purchase(who, plan);
}

fn subs_of(owner: &AccountId) -> Vec<pallet_marketplace::UserPlanSubscription<Runtime>> {
	Marketplace::user_all_subscription_plans(owner)
}

/// The price carried by `owner`'s snapshot of `plan` — what the monthly charge
/// will actually bill, as opposed to what `Plans` says.
fn snapshot_price(owner: &AccountId, plan: Hashed) -> u128 {
	subs_of(owner)
		.into_iter()
		.find(|s| s.package.id == plan)
		.expect("holder has a subscription on the plan")
		.package
		.price
}

fn plan_price(plan: Hashed) -> u128 {
	pallet_marketplace::Plans::<Runtime>::get(plan).expect("plan exists").price
}

/// Run one block of the hook — the repricing walk is ungated, so every block
/// drains a batch.
fn step() {
	let next = System::block_number() + 1;
	System::set_block_number(next);
	Marketplace::on_initialize(next);
}

/// Run blocks until the repricing queue is empty, up to a bound.
fn drain_repricing() {
	for _ in 0..200 {
		if pallet_marketplace::RepricingQueue::<Runtime>::get().is_empty() {
			return;
		}
		step();
	}
	panic!("repricing queue did not drain");
}

fn queue() -> Vec<Hashed> {
	pallet_marketplace::RepricingQueue::<Runtime>::get()
}

fn cursor_parked() -> bool {
	pallet_marketplace::RepricingCursor::<Runtime>::get().is_some()
}

// ── REPRICE-REACHES-SNAPSHOTS ────────────────────────────────────────────

/// The base case, which nothing asserted before: a reprice must actually reach
/// the subscriptions holding the plan.
///
/// Writing `Plans` alone would leave every existing subscriber renewing at the
/// old price forever, since the monthly charge bills the snapshot. That is the
/// failure the walk exists to prevent, and it is invisible from the extrinsic's
/// own result — `set_plan_price` returns `Ok` either way.
#[test]
fn a_reprice_reaches_existing_subscriptions() {
	new_test_ext().execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		holder(&user, plan);

		assert_eq!(snapshot_price(&user, plan), PLAN_PRICE);

		assert_ok!(Marketplace::set_plan_price(RuntimeOrigin::root(), plan, 2_500));
		// `Plans` moves immediately; the snapshot does not.
		assert_eq!(plan_price(plan), 2_500);
		assert_eq!(snapshot_price(&user, plan), PLAN_PRICE, "the walk has not run yet");

		drain_repricing();
		assert_eq!(snapshot_price(&user, plan), 2_500, "the walk carried it to the snapshot");
	});
}

/// Only the repriced plan is rewritten. An account holding two plans keeps the
/// other one's price, which is what makes the per-subscription `package.id`
/// match load-bearing rather than incidental.
#[test]
fn a_reprice_leaves_other_plans_alone() {
	new_test_ext().execute_with(|| {
		let drive = add_plan(b"drive", PLAN_PRICE);
		let compute = add_compute_plan(b"compute", 700);
		let user = account(11);
		deposit_credits(&user, 1_000_000);
		purchase(&user, drive);
		purchase(&user, compute);

		assert_ok!(Marketplace::set_plan_price(RuntimeOrigin::root(), drive, 2_500));
		drain_repricing();

		assert_eq!(snapshot_price(&user, drive), 2_500);
		assert_eq!(snapshot_price(&user, compute), 700, "an unrelated plan is untouched");
	});
}

/// The walk spans blocks and must reach every holder, including the ones past
/// the first batch. A cursor that failed to resume would leave the tail on the
/// old price permanently — and, like the split-price case, silently.
#[test]
fn a_paged_walk_reaches_every_holder() {
	new_test_ext().execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		// More than one batch, so the walk cannot finish in a single block.
		let total = BATCH + 40;
		for n in 0..total {
			holder(&bulk_account(n), plan);
		}

		assert_ok!(Marketplace::set_plan_price(RuntimeOrigin::root(), plan, 2_500));

		// One block cannot have finished: a cursor must be parked.
		step();
		assert!(cursor_parked(), "{total} holders cannot fit in one batch of {BATCH}");

		drain_repricing();
		for n in 0..total {
			assert_eq!(
				snapshot_price(&bulk_account(n), plan),
				2_500,
				"holder {n} was reached by the paged walk",
			);
		}
	});
}

// ── REPRICE-CONVERGES ────────────────────────────────────────────────────

/// A plan repriced again mid-walk must converge on the latest price for *every*
/// holder — including the ones the first pass already rewrote.
///
/// Re-reading the price each block converges only the accounts ahead of the
/// cursor. The prefix behind it already carries the superseded figure and no
/// later pass revisits it, so without resetting the cursor the map ends split
/// between two prices while `Plans` holds one, the queue drains, and
/// `PlanRepricingCompleted` reports the reprice as fully applied. Nothing on
/// chain would say which half is on the wrong price.
#[test]
fn a_reprice_mid_walk_converges_for_holders_already_rewritten() {
	new_test_ext().execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let total = BATCH + 40;
		for n in 0..total {
			holder(&bulk_account(n), plan);
		}

		// First reprice, then exactly one block — enough to rewrite the first
		// batch and park the cursor, not enough to finish.
		assert_ok!(Marketplace::set_plan_price(RuntimeOrigin::root(), plan, 2_000));
		step();
		assert!(cursor_parked(), "the walk must be mid-flight for this test to mean anything");
		let rewritten_first_pass = (0..total)
			.filter(|n| snapshot_price(&bulk_account(*n), plan) == 2_000)
			.count();
		assert!(
			rewritten_first_pass > 0 && rewritten_first_pass < total as usize,
			"expected a partial first pass, got {rewritten_first_pass} of {total}",
		);

		// Corrected while the walk is still in flight.
		assert_ok!(Marketplace::set_plan_price(RuntimeOrigin::root(), plan, 3_000));
		drain_repricing();

		for n in 0..total {
			assert_eq!(
				snapshot_price(&bulk_account(n), plan),
				3_000,
				"holder {n} must end on the latest price, not the superseded one",
			);
		}
		assert_eq!(plan_price(plan), 3_000, "and the plan agrees with every snapshot");
	});
}

/// Repricing a plan that is queued *behind* another must not disturb the walk
/// in progress. The cursor belongs to the plan at the head of the queue, so
/// resetting it on an unrelated reprice would restart that plan's walk from the
/// top for no reason.
#[test]
fn repricing_a_queued_plan_does_not_disturb_the_walk_in_flight() {
	new_test_ext().execute_with(|| {
		let drive = add_plan(b"drive", PLAN_PRICE);
		let compute = add_compute_plan(b"compute", 700);
		let total = BATCH + 40;
		for n in 0..total {
			let who = bulk_account(n);
			deposit_credits(&who, 1_000_000);
			purchase(&who, drive);
			purchase(&who, compute);
		}

		assert_ok!(Marketplace::set_plan_price(RuntimeOrigin::root(), drive, 2_000));
		assert_ok!(Marketplace::set_plan_price(RuntimeOrigin::root(), compute, 900));
		step();
		let parked = pallet_marketplace::RepricingCursor::<Runtime>::get();
		assert!(parked.is_some());
		assert_eq!(queue(), vec![drive, compute], "drive is at the head, compute waits");

		// Reprice the *waiting* plan. The head's cursor must survive.
		assert_ok!(Marketplace::set_plan_price(RuntimeOrigin::root(), compute, 950));
		assert_eq!(
			pallet_marketplace::RepricingCursor::<Runtime>::get(),
			parked,
			"a reprice of a queued plan must not reset the head's cursor",
		);

		drain_repricing();
		for n in 0..total {
			assert_eq!(snapshot_price(&bulk_account(n), drive), 2_000);
			assert_eq!(snapshot_price(&bulk_account(n), compute), 950);
		}
	});
}

// ── REPRICE-RETIRES ──────────────────────────────────────────────────────

/// The job leaves no state behind, and reports what it rewrote across every
/// block it spanned — the per-block count would under-report a paged walk.
#[test]
fn a_finished_walk_retires_and_reports_its_total() {
	new_test_ext().execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let total = BATCH + 40;
		for n in 0..total {
			holder(&bulk_account(n), plan);
		}

		assert_ok!(Marketplace::set_plan_price(RuntimeOrigin::root(), plan, 2_500));
		drain_repricing();

		assert!(queue().is_empty(), "the job leaves the queue");
		assert!(!cursor_parked(), "and the cursor is cleared");
		assert_eq!(
			pallet_marketplace::RepricedSoFar::<Runtime>::get(),
			0,
			"the running tally is reset for the next job",
		);

		let reported = System::events()
			.into_iter()
			.filter_map(|r| match r.event {
				hippius_mainnet_runtime::RuntimeEvent::Marketplace(
					pallet_marketplace::Event::PlanRepricingCompleted {
						plan_id,
						new_price,
						subscriptions_updated,
					},
				) if plan_id == plan => Some((new_price, subscriptions_updated)),
				_ => None,
			})
			.last()
			.expect("a finished walk reports completion");
		assert_eq!(
			reported,
			(2_500, total),
			"the tally must span every block the walk took, not just the last",
		);
	});
}

/// A price that is already current rewrites nothing. The guard that makes this
/// true is what lets a restarted walk re-cover its own prefix for free.
#[test]
fn repricing_to_the_same_price_rewrites_nothing() {
	new_test_ext().execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		holder(&user, plan);

		assert_ok!(Marketplace::set_plan_price(RuntimeOrigin::root(), plan, PLAN_PRICE));
		drain_repricing();

		let reported = System::events()
			.into_iter()
			.filter_map(|r| match r.event {
				hippius_mainnet_runtime::RuntimeEvent::Marketplace(
					pallet_marketplace::Event::PlanRepricingCompleted {
						subscriptions_updated,
						..
					},
				) => Some(subscriptions_updated),
				_ => None,
			})
			.last()
			.expect("the job still runs and retires");
		assert_eq!(reported, 0, "a snapshot that already agrees is left alone");
	});
}

// ── Origin and inputs ────────────────────────────────────────────────────

/// Repricing is sudo-only. It rewrites what every holder of the plan pays, so
/// a signed origin reaching it would be a direct economic escalation.
#[test]
fn set_plan_price_is_root_only() {
	new_test_ext().execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		holder(&user, plan);

		assert_noop!(
			Marketplace::set_plan_price(RuntimeOrigin::signed(user.clone()), plan, 1),
			sp_runtime::DispatchError::BadOrigin,
		);
		// Not even the whitelisted backend, which may buy and cancel on a
		// user's behalf, may set what a plan costs.
		assert_noop!(
			Marketplace::set_plan_price(RuntimeOrigin::signed(backend()), plan, 1),
			sp_runtime::DispatchError::BadOrigin,
		);

		assert_eq!(plan_price(plan), PLAN_PRICE);
		assert!(queue().is_empty(), "a rejected call queues no walk");
	});
}

/// An unknown plan is rejected rather than queued, so the walk never starts a
/// job it would have to abandon.
#[test]
fn repricing_an_unknown_plan_fails_and_queues_nothing() {
	new_test_ext().execute_with(|| {
		let missing = <Runtime as frame_system::Config>::Hashing::hash_of(&b"nope".to_vec());
		assert_noop!(
			Marketplace::set_plan_price(RuntimeOrigin::root(), missing, 1_000),
			pallet_marketplace::Error::<Runtime>::PlanNotFound,
		);
		assert!(queue().is_empty());
	});
}

// ── PREPAID-UNTOUCHED ────────────────────────────────────────────────────

/// A reprice changes the going rate, not what a holder already paid.
///
/// `paid_per_month` is what the refund and carry-credit paths value a prepaid
/// cycle from. Rewriting it here would retroactively revalue months already
/// bought — refunding a holder more than they paid after a price rise, or less
/// after a cut. The two fields are meant to differ after a reprice.
#[test]
fn a_reprice_does_not_revalue_months_already_paid_for() {
	new_test_ext().execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		holder(&user, plan);

		let paid_before = subs_of(&user)[0].paid_per_month;

		assert_ok!(Marketplace::set_plan_price(RuntimeOrigin::root(), plan, 5_000));
		drain_repricing();

		assert_eq!(snapshot_price(&user, plan), 5_000, "the going rate moved");
		assert_eq!(
			subs_of(&user)[0].paid_per_month,
			paid_before,
			"what was already paid did not",
		);
	});
}

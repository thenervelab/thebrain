//! `update_plan` and `remove_plan` — sudo edits to the plan catalogue.
//!
//! `Plans` is the catalogue: the only thing a purchase or a plan change reads
//! to find a plan. It is *not* what a live subscription bills from — every
//! subscription carries its own copy of the plan taken at purchase time, and
//! the monthly charge reads that copy. Both of these calls touch the catalogue
//! and nothing else, and that split is what the tests below pin down.
//!
//! The invariants under test:
//!
//! - `REMOVE-DELISTS` — a removed plan cannot be bought or switched onto.
//! - `REMOVE-SPARES-HOLDERS` — a removal is not a cancellation. Existing
//!   holders keep their subscription and their snapshot, because nothing on the
//!   billing path reads `Plans`.
//! - `REMOVE-CLEARS-REPRICE` — removing a plan with a queued reprice leaves the
//!   queue and the cursor in a state the next job can start from. A stale head,
//!   or a cursor inherited from a dead job, would strand or half-walk it.
//! - `UPDATE-IS-PARTIAL` — a `None` field is left alone, and the id never moves,
//!   including across a rename.
//! - `CATALOGUE-IS-SUDO` — neither call is reachable without root.

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

/// 2026-01-15T12:00:00Z — mid-month, so no test sits on a billing boundary.
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

fn plan_of(plan: Hashed) -> pallet_marketplace::Plan<Hashed> {
	pallet_marketplace::Plans::<Runtime>::get(plan).expect("plan exists")
}

fn subs_of(owner: &AccountId) -> Vec<pallet_marketplace::UserPlanSubscription<Runtime>> {
	Marketplace::user_all_subscription_plans(owner)
}

fn queue() -> Vec<Hashed> {
	pallet_marketplace::RepricingQueue::<Runtime>::get()
}

fn cursor_parked() -> bool {
	pallet_marketplace::RepricingCursor::<Runtime>::get().is_some()
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
		if queue().is_empty() {
			return;
		}
		step();
	}
	panic!("repricing queue did not drain");
}

// ── REMOVE-DELISTS ───────────────────────────────────────────────────────

/// The point of the call: a removed plan is gone from the catalogue, so nothing
/// can be bought into it.
#[test]
fn a_removed_plan_cannot_be_purchased() {
	new_test_ext().execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		deposit_credits(&user, 1_000_000);

		assert_ok!(Marketplace::remove_plan(RuntimeOrigin::root(), plan));
		assert!(pallet_marketplace::Plans::<Runtime>::get(plan).is_none());

		assert_noop!(
			Marketplace::purchase_plan(
				RuntimeOrigin::signed(backend()),
				user.clone(),
				vec![plan],
				None,
				None,
				None,
				None,
			),
			pallet_marketplace::Error::<Runtime>::PlanNotFound,
		);
	});
}

/// Removing a plan nobody ever created is an error rather than a silent no-op:
/// a sudo call that reports success on a typo'd id is worse than one that
/// fails.
#[test]
fn removing_an_unknown_plan_fails() {
	new_test_ext().execute_with(|| {
		let ghost = <Runtime as frame_system::Config>::Hashing::hash_of(&b"nothing".to_vec());
		assert_noop!(
			Marketplace::remove_plan(RuntimeOrigin::root(), ghost),
			pallet_marketplace::Error::<Runtime>::PlanNotFound,
		);
	});
}

/// A rebuy of a removed name is allowed, because the id was freed with it.
/// `add_new_plan` derives the id from the name and refuses a collision, so this
/// is the one thing that would break if `remove_plan` left the key behind.
#[test]
fn a_removed_name_can_be_recreated() {
	new_test_ext().execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		assert_ok!(Marketplace::remove_plan(RuntimeOrigin::root(), plan));

		let again = add_plan(b"drive", 2_000);
		assert_eq!(again, plan, "the id is derived from the name, so it comes back the same");
		assert_eq!(plan_of(again).price, 2_000);
	});
}

// ── REMOVE-SPARES-HOLDERS ────────────────────────────────────────────────

/// Delisting is not cancelling. The monthly charge bills the subscription's own
/// copy of the plan and never reads `Plans`, so an existing holder is untouched
/// — and must be, or retiring a plan would silently terminate everyone paying
/// for it.
#[test]
fn removing_a_plan_leaves_its_holders_subscribed() {
	new_test_ext().execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		holder(&user, plan);

		assert_ok!(Marketplace::remove_plan(RuntimeOrigin::root(), plan));

		// Several blocks of the hook, in case anything sweeps orphaned plans.
		for _ in 0..5 {
			step();
		}

		let subs = subs_of(&user);
		assert_eq!(subs.len(), 1, "the holder keeps the subscription");
		assert!(subs[0].active, "and it stays active");
		assert_eq!(
			subs[0].package.price, PLAN_PRICE,
			"and keeps billing from its own snapshot of the plan",
		);
	});
}

// ── REMOVE-CLEARS-REPRICE ────────────────────────────────────────────────

/// A reprice queued for a plan that is then removed has no price left to copy.
/// Leaving it at the head would stall every job behind it until a block reached
/// and retired it, so the removal clears it — along with the cursor and tally
/// that belonged to it.
#[test]
fn removing_the_plan_being_repriced_clears_the_job() {
	new_test_ext().execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let total = BATCH + 40;
		for n in 0..total {
			holder(&bulk_account(n), plan);
		}

		assert_ok!(Marketplace::set_plan_price(RuntimeOrigin::root(), plan, 2_500));
		step();
		assert!(cursor_parked(), "{total} holders cannot fit in one batch of {BATCH}");

		assert_ok!(Marketplace::remove_plan(RuntimeOrigin::root(), plan));

		assert!(queue().is_empty(), "the dead job leaves the queue");
		assert!(!cursor_parked(), "and takes its cursor with it");
		assert_eq!(
			pallet_marketplace::RepricedSoFar::<Runtime>::get(),
			0,
			"and its running tally, which belongs to no job now",
		);
	});
}

/// The half-repriced holders of a removed plan keep whichever price their
/// snapshot happened to reach. That is not a bug to fix here — the plan is
/// gone, there is no authoritative price to converge on — but it is the reason
/// the safe retirement order is suspend, drain, then remove.
#[test]
fn a_removal_mid_walk_stops_rewriting_snapshots() {
	new_test_ext().execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let total = BATCH + 40;
		for n in 0..total {
			holder(&bulk_account(n), plan);
		}

		assert_ok!(Marketplace::set_plan_price(RuntimeOrigin::root(), plan, 2_500));
		step();
		assert_ok!(Marketplace::remove_plan(RuntimeOrigin::root(), plan));
		for _ in 0..5 {
			step();
		}

		let untouched = (0..total)
			.filter(|n| {
				subs_of(&bulk_account(*n))
					.iter()
					.any(|s| s.package.id == plan && s.package.price == PLAN_PRICE)
			})
			.count();
		assert!(
			untouched > 0,
			"the walk stopped where the removal caught it; nothing repriced the rest",
		);
	});
}

/// Removing a plan queued *behind* the one being walked must not disturb the
/// job in flight — its cursor is mid-map and restarting it would re-walk the
/// prefix and double-count the total it reports.
#[test]
fn removing_a_queued_plan_leaves_the_head_walking() {
	new_test_ext().execute_with(|| {
		let drive = add_plan(b"drive", PLAN_PRICE);
		let compute = add_compute_plan(b"compute", 500);
		let total = BATCH + 40;
		for n in 0..total {
			let who = bulk_account(n);
			deposit_credits(&who, 2_000_000);
			purchase(&who, drive);
			purchase(&who, compute);
		}

		assert_ok!(Marketplace::set_plan_price(RuntimeOrigin::root(), drive, 2_000));
		assert_ok!(Marketplace::set_plan_price(RuntimeOrigin::root(), compute, 900));
		step();
		let parked = pallet_marketplace::RepricingCursor::<Runtime>::get();
		assert!(parked.is_some(), "the drive walk is mid-map");

		assert_ok!(Marketplace::remove_plan(RuntimeOrigin::root(), compute));

		assert_eq!(queue(), vec![drive], "only the queued job is dropped");
		assert_eq!(
			pallet_marketplace::RepricingCursor::<Runtime>::get(),
			parked,
			"the head keeps its cursor",
		);

		drain_repricing();
		for n in 0..total {
			let subs = subs_of(&bulk_account(n));
			let drive_price =
				subs.iter().find(|s| s.package.id == drive).expect("holds drive").package.price;
			assert_eq!(drive_price, 2_000, "the head's walk still finished for every holder");
		}
	});
}

// ── UPDATE-IS-PARTIAL ────────────────────────────────────────────────────

/// A `None` field means "leave it alone", so an edit to one field cannot blank
/// the others — the failure mode of a call that took every field unconditionally.
#[test]
fn an_update_touches_only_the_fields_it_names() {
	new_test_ext().execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);

		assert_ok!(Marketplace::update_plan(
			RuntimeOrigin::root(),
			plan,
			None,
			Some(br#"{"blurb":"now with more bytes"}"#.to_vec()),
			None,
			None,
		));

		let after = plan_of(plan);
		assert_eq!(after.plan_description, br#"{"blurb":"now with more bytes"}"#.to_vec());
		assert_eq!(after.plan_name, b"drive".to_vec(), "the name is untouched");
		assert_eq!(after.plan_technical_description, b"{}".to_vec(), "so is the technical blob");
		assert_eq!(after.storage_limit, Some(1_000_000), "so is the limit");
		assert_eq!(after.price, PLAN_PRICE, "and the price, which this call cannot move");
		assert!(!after.is_suspended);
	});
}

/// Every editable field at once, including clearing the limit — `Some(None)`
/// has to mean "clear it", or an optional field could be set but never unset.
#[test]
fn an_update_can_rewrite_every_editable_field() {
	new_test_ext().execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);

		assert_ok!(Marketplace::update_plan(
			RuntimeOrigin::root(),
			plan,
			Some(b"drive-pro".to_vec()),
			Some(br#"{"a":1}"#.to_vec()),
			Some(br#"{"b":2}"#.to_vec()),
			Some(None),
		));

		let after = plan_of(plan);
		assert_eq!(after.plan_name, b"drive-pro".to_vec());
		assert_eq!(after.plan_description, br#"{"a":1}"#.to_vec());
		assert_eq!(after.plan_technical_description, br#"{"b":2}"#.to_vec());
		assert_eq!(after.storage_limit, None, "Some(None) clears the limit");
	});
}

/// The id is the storage key every subscription points at, so a rename must not
/// move it. Rewriting the key to follow the new name would orphan every holder.
#[test]
fn a_rename_leaves_the_id_and_its_holders_in_place() {
	new_test_ext().execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		let user = account(11);
		holder(&user, plan);

		assert_ok!(Marketplace::update_plan(
			RuntimeOrigin::root(),
			plan,
			Some(b"drive-renamed".to_vec()),
			None,
			None,
			None,
		));

		let after = plan_of(plan);
		assert_eq!(after.plan_name, b"drive-renamed".to_vec());
		assert_eq!(after.id, plan, "the id field still agrees with the key");
		assert_ne!(
			plan,
			<Runtime as frame_system::Config>::Hashing::hash_of(&b"drive-renamed".to_vec()),
			"and no longer hashes to the name — the key is what is authoritative",
		);
		assert_eq!(subs_of(&user)[0].package.id, plan, "the holder still points at it");

		// And it is still buyable under its unchanged id.
		let other = account(12);
		deposit_credits(&other, 1_000_000);
		let compute = add_compute_plan(b"compute", 500);
		purchase(&other, compute);
		purchase(&other, plan);
		assert_eq!(subs_of(&other).len(), 2);
	});
}

/// An all-`None` call would emit `PlanUpdated` for an update that never
/// happened, which is worse than nothing for anyone following the event stream.
#[test]
fn an_empty_update_is_rejected() {
	new_test_ext().execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);
		assert_noop!(
			Marketplace::update_plan(RuntimeOrigin::root(), plan, None, None, None, None),
			pallet_marketplace::Error::<Runtime>::InvalidInput,
		);
	});
}

#[test]
fn updating_an_unknown_plan_fails() {
	new_test_ext().execute_with(|| {
		let ghost = <Runtime as frame_system::Config>::Hashing::hash_of(&b"nothing".to_vec());
		assert_noop!(
			Marketplace::update_plan(
				RuntimeOrigin::root(),
				ghost,
				Some(b"x".to_vec()),
				None,
				None,
				None,
			),
			sp_runtime::DispatchError::from(pallet_marketplace::Error::<Runtime>::PlanNotFound),
		);
	});
}

// ── CATALOGUE-IS-SUDO ────────────────────────────────────────────────────

/// Both calls rewrite what everyone can buy, so neither is reachable from a
/// signed origin — including the whitelisted backend, which is trusted to
/// purchase on a user's behalf and nothing more.
#[test]
fn catalogue_edits_require_root() {
	new_test_ext().execute_with(|| {
		let plan = add_plan(b"drive", PLAN_PRICE);

		assert_noop!(
			Marketplace::remove_plan(RuntimeOrigin::signed(backend()), plan),
			sp_runtime::DispatchError::BadOrigin,
		);
		assert_noop!(
			Marketplace::update_plan(
				RuntimeOrigin::signed(backend()),
				plan,
				Some(b"free".to_vec()),
				None,
				None,
				None,
			),
			sp_runtime::DispatchError::BadOrigin,
		);

		assert!(pallet_marketplace::Plans::<Runtime>::get(plan).is_some());
		assert_eq!(plan_of(plan).plan_name, b"drive".to_vec());
	});
}

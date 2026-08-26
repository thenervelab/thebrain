//! End-to-end behaviour of `Marketplace::change_storage_plan` — the atomic
//! Solo → Duo → Max swap the backend relayer drives on a user's behalf.
//!
//! The property under test is that a plan change is *not* a cancel plus a
//! re-purchase:
//! - the storage entitlement never lapses (the subscription slot is replaced
//!   in place, never emptied),
//! - `LastSubscriptionCancelledAt` is never written, so the
//!   `MinSubscriptionBlocks` resubscribe cooldown is never armed and the two
//!   calls never have to be sequenced across blocks,
//! - and the refund for the old plan and the charge for the new one collapse
//!   into a single net `FreeCredits` movement, so the month the user already
//!   paid for is not billed twice.
//!
//! All expected credit amounts below are hand-computed from the ceil-rounded
//! `prorate_first_month` so a regression in the proration wiring fails loudly
//! instead of cancelling out against a re-derived expectation.

use frame_support::{
	assert_err, assert_noop, assert_ok,
	traits::{Currency, Hooks},
};
use hippius_mainnet_runtime::{
	AccountId, Balances, Credits, Hippocampus, Marketplace, Runtime, RuntimeCall, RuntimeOrigin,
	System,
};
use sp_core::crypto::Ss58Codec;
use sp_runtime::{
	traits::{Dispatchable, Hash},
	AccountId32, BuildStorage,
};

type Hashed = <Runtime as frame_system::Config>::Hash;

/// 2026-01-01T00:00:00Z. January has 31 days, so a purchase on the 1st
/// prorates to exactly one full month — the arithmetic-free baseline.
const JAN1_2026_MS: u64 = 1_767_225_600_000;
/// 2026-01-16T12:00:00Z — 16 of 31 days remain (inclusive of today), the
/// mid-month case where proration actually rounds.
const JAN16_2026_MS: u64 = JAN1_2026_MS + 15 * 86_400_000 + 12 * 3_600_000;

const FEB1_2026_DAY: u32 = 20_485;
const APR1_2026_DAY: u32 = 20_544;

/// Plan ladder. Prices are round so the netting assertions read directly.
const SOLO_PRICE: u128 = 1_000;
const MAX_PRICE: u128 = 3_000;

/// Mid-month (16/31) prorations, ceil-rounded: ⌈1_000×16/31⌉ = 517,
/// ⌈3_000×16/31⌉ = 1_549.
const SOLO_PRORATED: u128 = 517;
const MAX_PRORATED: u128 = 1_549;

const ED: u128 = 500;
const BANK_FUND: u128 = 1_000_000;

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

fn authority() -> AccountId {
	account(1)
}

/// Backend account whitelisted for `purchase_plan` / `change_storage_plan`.
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

fn new_test_ext() -> sp_io::TestExternalities {
	new_test_ext_at(JAN1_2026_MS)
}

fn add_plan(name: &[u8], price: u128, is_storage_plan: bool) -> Hashed {
	add_plan_of_flavour(name, price, is_storage_plan, false)
}

/// The three kinds are mutually exclusive: Drive `(true, false)`,
/// S3 `(false, true)`, compute `(false, false)`.
fn add_plan_of_flavour(
	name: &[u8],
	price: u128,
	is_storage_plan: bool,
	is_s3_plan: bool,
) -> Hashed {
	assert_ok!(Marketplace::add_new_plan(
		RuntimeOrigin::root(),
		name.to_vec(),
		b"{}".to_vec(),
		b"{}".to_vec(),
		price,
		is_storage_plan,
		is_s3_plan,
		Some(price * 1_000),
	));
	<Runtime as frame_system::Config>::Hashing::hash_of(&name.to_vec())
}

/// The Solo/Max storage ladder used by most tests.
fn ladder() -> (Hashed, Hashed) {
	(add_plan(b"solo", SOLO_PRICE, true), add_plan(b"max", MAX_PRICE, true))
}

fn deposit_credits(who: &AccountId, amount: u128, code: Option<Vec<u8>>) {
	assert_ok!(Marketplace::deposit(
		RuntimeOrigin::signed(authority()),
		who.clone(),
		amount,
		0,
		false,
		code,
	));
}

fn purchase(owner: &AccountId, plan_id: Hashed, pay_upfront: Option<u128>) {
	assert_ok!(Marketplace::purchase_plan(
		RuntimeOrigin::signed(backend()),
		owner.clone(),
		vec![plan_id],
		None,
		None,
		None,
		pay_upfront,
	));
}

fn change_plan(user: &AccountId, new_plan_id: Hashed) -> frame_support::dispatch::DispatchResult {
	Marketplace::change_storage_plan(
		RuntimeOrigin::signed(backend()),
		user.clone(),
		new_plan_id,
		None,
		None,
		None,
	)
}

/// Same change dispatched through `RuntimeCall` so extrinsic-level rollback
/// applies — required for the atomicity assertions on failure paths.
fn dispatch_change(
	user: &AccountId,
	new_plan_id: Hashed,
) -> frame_support::dispatch::DispatchResultWithPostInfo {
	RuntimeCall::Marketplace(pallet_marketplace::Call::change_storage_plan {
		user: user.clone(),
		new_plan_id,
		selected_image_name: None,
		location_id: None,
		cloud_init_cid: None,
	})
	.dispatch(RuntimeOrigin::signed(backend()))
}

fn storage_subscription(owner: &AccountId) -> pallet_marketplace::UserPlanSubscription<Runtime> {
	Marketplace::user_all_subscription_plans(owner)
		.into_iter()
		.find(|s| s.package.is_drive_plan())
		.expect("storage subscription exists")
}

fn new_referral_code(owner: &AccountId) -> Vec<u8> {
	assert_ok!(Credits::create_referral_code(RuntimeOrigin::signed(owner.clone())));
	Credits::get_referral_codes(owner.clone()).pop().expect("code was just created")
}

// ── Upgrades ─────────────────────────────────────────────────────────────

#[test]
fn upgrade_on_the_first_charges_only_the_price_difference() {
	new_test_ext().execute_with(|| {
		let (solo, max) = ladder();
		let user = account(11);

		// Enough for Solo plus the upgrade delta, and not a credit more —
		// paying for January twice would not fit.
		deposit_credits(&user, SOLO_PRICE + (MAX_PRICE - SOLO_PRICE), None);
		purchase(&user, solo, None);
		assert_eq!(Credits::get_free_credits(&user), MAX_PRICE - SOLO_PRICE);

		assert_ok!(change_plan(&user, max));

		assert_eq!(Credits::get_free_credits(&user), 0, "only the delta is charged");
		let sub = storage_subscription(&user);
		assert_eq!(sub.package.id, max);
		assert_eq!(sub.paid_per_month, MAX_PRICE);
		assert_eq!(sub.next_charge_unix_day, Some(FEB1_2026_DAY));
		assert!(sub.active);
	});
}

#[test]
fn mid_month_upgrade_never_bills_the_remaining_days_twice() {
	new_test_ext_at(JAN16_2026_MS).execute_with(|| {
		let (solo, max) = ladder();
		let user = account(11);

		deposit_credits(&user, 10_000, None);
		purchase(&user, solo, None);
		assert_eq!(Credits::get_free_credits(&user), 10_000 - SOLO_PRORATED);

		assert_ok!(change_plan(&user, max));

		// Total out of pocket for January is exactly one prorated Max month:
		// the 517 already paid for Solo carries over as a credit.
		assert_eq!(Credits::get_free_credits(&user), 10_000 - MAX_PRORATED);
	});
}

#[test]
fn upgrade_is_rejected_atomically_when_the_delta_is_unaffordable() {
	new_test_ext().execute_with(|| {
		let (solo, max) = ladder();
		let user = account(11);

		// One credit short of the upgrade delta.
		deposit_credits(&user, SOLO_PRICE + (MAX_PRICE - SOLO_PRICE) - 1, None);
		purchase(&user, solo, None);
		let before = Credits::get_free_credits(&user);
		let sub_before = storage_subscription(&user);

		assert_noop!(
			dispatch_change(&user, max),
			pallet_marketplace::Error::<Runtime>::InsufficientFreeCredits,
		);

		assert_eq!(Credits::get_free_credits(&user), before, "no credits moved");
		let sub_after = storage_subscription(&user);
		assert_eq!(sub_after.id, sub_before.id, "subscription untouched");
		assert_eq!(sub_after.package.id, solo);
	});
}

// ── Downgrades ───────────────────────────────────────────────────────────

#[test]
fn downgrade_costs_nothing_now_and_bills_the_cheaper_plan_next_month() {
	new_test_ext().execute_with(|| {
		let (solo, max) = ladder();
		let user = account(11);

		deposit_credits(&user, MAX_PRICE, None);
		purchase(&user, max, None);
		assert_eq!(Credits::get_free_credits(&user), 0);

		assert_ok!(change_plan(&user, solo));

		// The month already bought is not refunded — the same rule the cancel
		// path applies to the current month — but it fully covers the cheaper
		// plan, so nothing is charged either.
		assert_eq!(Credits::get_free_credits(&user), 0);
		let sub = storage_subscription(&user);
		assert_eq!(sub.package.id, solo);
		assert_eq!(sub.paid_per_month, SOLO_PRICE, "next month bills Solo");
		assert_eq!(sub.next_charge_unix_day, Some(FEB1_2026_DAY));
	});
}

#[test]
fn prepaid_months_pay_for_the_new_plan_before_free_credits_do() {
	new_test_ext().execute_with(|| {
		let (solo, max) = ladder();
		let user = account(11);

		// 3 upfront Solo months on the 1st = 3 full months.
		deposit_credits(&user, 3 * SOLO_PRICE, None);
		purchase(&user, solo, Some(3));
		assert_eq!(Credits::get_free_credits(&user), 0);
		assert_eq!(storage_subscription(&user).next_charge_unix_day, Some(APR1_2026_DAY));

		// Upgrading to Max: Feb + Mar (2_000) are refunded and January's Solo
		// month (1_000) carries over, which together cover Max's 3_000 exactly.
		// Net movement is zero and the user never needs spare credits.
		assert_ok!(change_plan(&user, max));

		assert_eq!(Credits::get_free_credits(&user), 0, "settled by netting alone");
		let sub = storage_subscription(&user);
		assert_eq!(sub.package.id, max);
		assert_eq!(sub.next_charge_unix_day, Some(FEB1_2026_DAY));
	});
}

#[test]
fn surplus_prepaid_months_are_refunded_as_one_net_movement() {
	new_test_ext().execute_with(|| {
		let (solo, max) = ladder();
		let user = account(11);

		// 3 upfront Max months (9_000), then downgrade to Solo on the 1st.
		deposit_credits(&user, 3 * MAX_PRICE, None);
		purchase(&user, max, Some(3));
		assert_eq!(Credits::get_free_credits(&user), 0);

		// Refund = Feb + Mar of Max = 6_000. Charge = Solo's January (1_000)
		// minus the Max January already paid ⇒ 0. Net refund = 6_000.
		assert_ok!(change_plan(&user, solo));

		assert_eq!(Credits::get_free_credits(&user), 6_000);
		assert_eq!(storage_subscription(&user).next_charge_unix_day, Some(FEB1_2026_DAY));
	});
}

// ── Not a cancel: no gap, no cooldown, no lost attribution ───────────────

#[test]
fn change_leaves_no_entitlement_gap_and_arms_no_resubscribe_cooldown() {
	new_test_ext().execute_with(|| {
		let (solo, max) = ladder();
		let user = account(11);

		deposit_credits(&user, 10_000, None);
		purchase(&user, solo, None);
		let old_id = storage_subscription(&user).id;

		assert_ok!(change_plan(&user, max));

		// Exactly one storage subscription throughout — the slot was replaced,
		// never emptied, so the user is never without a storage entitlement.
		let subs = Marketplace::user_all_subscription_plans(&user);
		assert_eq!(subs.iter().filter(|s| s.active && s.package.is_storage_plan).count(), 1);
		let sub = storage_subscription(&user);
		assert_ne!(sub.id, old_id, "a fresh subscription id is issued");
		assert_eq!(sub.package.storage_limit, Some(MAX_PRICE * 1_000));

		// The cooldown that forces cancel + re-purchase across blocks is never armed…
		assert_eq!(Marketplace::last_subscription_cancelled_at(&user), None);
		// …so a second change lands in the very same block.
		assert_ok!(change_plan(&user, solo));
		assert_eq!(storage_subscription(&user).package.id, solo);
	});
}

#[test]
fn referral_attribution_survives_and_commission_follows_the_net_charge() {
	new_test_ext().execute_with(|| {
		let (solo, max) = ladder();
		let referrer = account(10);
		let user = account(11);
		let code = new_referral_code(&referrer);
		let _ = Balances::deposit_creating(&referrer, ED);

		// Referred purchase: 5% off Solo (950 charged), 5% commission = 47.
		deposit_credits(&user, 10_000, Some(code.clone()));
		purchase(&user, solo, None);
		assert_eq!(Credits::get_free_credits(&user), 10_000 - 950);
		assert_eq!(Balances::free_balance(&referrer), ED + 47);

		// Upgrade: Max discounted to 2_850, less the 950 Solo month already
		// paid ⇒ 1_900 charged, commission ⌊1_900 × 5%⌋ = 95.
		assert_ok!(change_plan(&user, max));

		assert_eq!(Credits::get_free_credits(&user), 10_000 - 950 - 1_900);
		assert_eq!(Balances::free_balance(&referrer), ED + 47 + 95);
		assert_eq!(Credits::referred_users(&user), Some(code), "attribution untouched");
		assert_eq!(storage_subscription(&user).paid_per_month, 2_850);
	});
}

// ── Guards ───────────────────────────────────────────────────────────────

#[test]
fn only_whitelisted_callers_may_change_a_plan() {
	new_test_ext().execute_with(|| {
		let (solo, max) = ladder();
		let user = account(11);
		deposit_credits(&user, 10_000, None);
		purchase(&user, solo, None);

		assert_err!(
			Marketplace::change_storage_plan(
				RuntimeOrigin::signed(account(99)),
				user.clone(),
				max,
				None,
				None,
				None,
			),
			pallet_marketplace::Error::<Runtime>::WhitelistedCallerNotAuthorized,
		);
		// Not even the subscriber themselves.
		assert_err!(
			Marketplace::change_storage_plan(
				RuntimeOrigin::signed(user.clone()),
				user.clone(),
				max,
				None,
				None,
				None,
			),
			pallet_marketplace::Error::<Runtime>::WhitelistedCallerNotAuthorized,
		);
		assert_eq!(storage_subscription(&user).package.id, solo);
	});
}

#[test]
fn a_user_without_an_active_storage_subscription_cannot_change_plan() {
	new_test_ext().execute_with(|| {
		let (_solo, max) = ladder();
		let user = account(11);
		deposit_credits(&user, 10_000, None);

		assert_err!(
			change_plan(&user, max),
			pallet_marketplace::Error::<Runtime>::NoActiveSubscription,
		);

		// A compute-only subscriber is equally out of scope for this call.
		let compute = add_plan(b"compute", SOLO_PRICE, false);
		purchase(&user, compute, None);
		assert_err!(
			change_plan(&user, max),
			pallet_marketplace::Error::<Runtime>::NoActiveSubscription,
		);
	});
}

#[test]
fn the_target_plan_must_be_a_different_live_storage_plan() {
	new_test_ext().execute_with(|| {
		let (solo, max) = ladder();
		let user = account(11);
		deposit_credits(&user, 10_000, None);
		purchase(&user, solo, None);

		// Same plan is a no-op the backend should not be able to bill for.
		assert_err!(change_plan(&user, solo), pallet_marketplace::Error::<Runtime>::InvalidInput);

		// Unknown plan.
		assert_err!(
			change_plan(&user, <Runtime as frame_system::Config>::Hashing::hash_of(&b"nope")),
			pallet_marketplace::Error::<Runtime>::PlanNotFound,
		);

		// Compute plans are not a storage-plan target.
		let compute = add_plan(b"compute", MAX_PRICE, false);
		assert_err!(
			change_plan(&user, compute),
			pallet_marketplace::Error::<Runtime>::InvalidPlanType,
		);

		// Suspended target.
		assert_ok!(Marketplace::set_package_suspension(RuntimeOrigin::root(), max, true));
		assert_err!(change_plan(&user, max), pallet_marketplace::Error::<Runtime>::PlanSuspended);

		assert_eq!(storage_subscription(&user).package.id, solo);
	});
}

#[test]
fn the_purchase_plan_kill_switch_also_stops_plan_changes() {
	new_test_ext().execute_with(|| {
		let (solo, max) = ladder();
		let user = account(11);
		deposit_credits(&user, 10_000, None);
		purchase(&user, solo, None);

		assert_ok!(Marketplace::sudo_set_purchase_plan_enabled(RuntimeOrigin::root(), false));
		assert_err!(
			change_plan(&user, max),
			pallet_marketplace::Error::<Runtime>::PlanOperationDisabled,
		);
	});
}

#[test]
fn plan_changes_are_rate_limited_per_block_like_purchases() {
	new_test_ext().execute_with(|| {
		let (solo, max) = ladder();
		let user = account(11);
		deposit_credits(&user, 100_000, None);
		purchase(&user, solo, None);

		// `MaxRequestsPerBlock` is 5 and the purchase consumed one, so four
		// changes fit and the fifth is refused.
		for i in 0..4 {
			let target = if i % 2 == 0 { max } else { solo };
			assert_ok!(change_plan(&user, target));
		}
		assert_err!(change_plan(&user, max), pallet_marketplace::Error::<Runtime>::TooManyRequests);

		// The counter is cleared on the pallet's 15-block boundary.
		System::set_block_number(15);
		<Marketplace as Hooks<u64>>::on_initialize(15);
		assert_ok!(change_plan(&user, max));
	});
}

// ── Drive and S3 as two independent storage slots ────────────────────────
//
// `is_s3_plan` narrows a storage plan to the S3 flavour. An account holds at
// most one active plan of each flavour, and a change is routed by the flavour
// of the plan being moved onto, so the two slots never interfere.

/// An S3 plan priced like Max, so the netting assertions reuse the ladder's
/// arithmetic.
fn s3_ladder() -> (Hashed, Hashed) {
	(
		add_plan_of_flavour(b"s3-solo", SOLO_PRICE, false, true),
		add_plan_of_flavour(b"s3-max", MAX_PRICE, false, true),
	)
}

fn change_s3(user: &AccountId, new_plan_id: Hashed) -> frame_support::dispatch::DispatchResult {
	Marketplace::change_s3_plan(
		RuntimeOrigin::signed(backend()),
		user.clone(),
		new_plan_id,
		None,
		None,
		None,
	)
}

fn s3_subscription(owner: &AccountId) -> pallet_marketplace::UserPlanSubscription<Runtime> {
	Marketplace::user_all_subscription_plans(owner)
		.into_iter()
		.find(|s| s.package.is_s3_plan)
		.expect("s3 subscription exists")
}

#[test]
fn a_plan_may_not_claim_both_storage_flavours() {
	new_test_ext().execute_with(|| {
		// Drive and S3 are separate kinds with separate per-account slots, so a
		// plan claiming both would occupy two at once.
		assert_err!(
			Marketplace::add_new_plan(
				RuntimeOrigin::root(),
				b"both".to_vec(),
				b"{}".to_vec(),
				b"{}".to_vec(),
				SOLO_PRICE,
				true,
				true,
				None,
			),
			pallet_marketplace::Error::<Runtime>::InvalidPlanType,
		);

		// Claiming neither is simply a compute plan, which stays legal.
		assert_ok!(Marketplace::add_new_plan(
			RuntimeOrigin::root(),
			b"compute-ok".to_vec(),
			b"{}".to_vec(),
			b"{}".to_vec(),
			SOLO_PRICE,
			false,
			false,
			None,
		));
	});
}

#[test]
fn one_drive_plan_and_one_s3_plan_may_be_held_at_once() {
	new_test_ext().execute_with(|| {
		let (solo, _max) = ladder();
		let (s3_solo, _s3_max) = s3_ladder();
		let user = account(11);
		deposit_credits(&user, 10_000, None);

		// One of each flavour coexists — the S3 purchase is not blocked by the
		// Drive plan, whose bytes it does not cover.
		purchase(&user, solo, None);
		purchase(&user, s3_solo, None);

		let subs = Marketplace::user_all_subscription_plans(&user);
		assert_eq!(subs.iter().filter(|s| s.active && s.package.is_drive_plan()).count(), 1);
		assert_eq!(subs.iter().filter(|s| s.active && s.package.is_s3_plan).count(), 1);
	});
}

#[test]
fn a_second_plan_of_the_same_flavour_is_refused() {
	new_test_ext().execute_with(|| {
		let (solo, max) = ladder();
		let (s3_solo, s3_max) = s3_ladder();
		let user = account(11);
		deposit_credits(&user, 100_000, None);

		purchase(&user, solo, None);
		let after_drive = Credits::get_free_credits(&user);

		// A different Drive plan is still a Drive plan.
		assert_err!(
			Marketplace::purchase_plan(
				RuntimeOrigin::signed(backend()),
				user.clone(),
				vec![max],
				None,
				None,
				None,
				None,
			),
			pallet_marketplace::Error::<Runtime>::AlreadyHasActiveSubscription,
		);
		assert_eq!(Credits::get_free_credits(&user), after_drive, "no credits moved");

		purchase(&user, s3_solo, None);
		let after_s3 = Credits::get_free_credits(&user);

		// …and the same rule applies independently to the S3 slot.
		assert_err!(
			Marketplace::purchase_plan(
				RuntimeOrigin::signed(backend()),
				user.clone(),
				vec![s3_max],
				None,
				None,
				None,
				None,
			),
			pallet_marketplace::Error::<Runtime>::AlreadyHasActiveSubscription,
		);
		assert_eq!(Credits::get_free_credits(&user), after_s3, "no credits moved");
	});
}

#[test]
fn each_extrinsic_moves_only_its_own_slot() {
	new_test_ext().execute_with(|| {
		let (solo, max) = ladder();
		let (s3_solo, s3_max) = s3_ladder();
		let user = account(11);
		deposit_credits(&user, 100_000, None);

		purchase(&user, solo, None);
		purchase(&user, s3_solo, None);
		let drive_id = storage_subscription(&user).id;
		let s3_id = s3_subscription(&user).id;

		// `change_storage_plan` moves the Drive subscription…
		assert_ok!(change_plan(&user, max));
		assert_eq!(storage_subscription(&user).package.id, max);
		assert_eq!(s3_subscription(&user).package.id, s3_solo, "s3 slot untouched");
		assert_eq!(s3_subscription(&user).id, s3_id, "same s3 subscription, not replaced");

		// …and `change_s3_plan` moves the S3 one.
		assert_ok!(change_s3(&user, s3_max));
		assert_eq!(s3_subscription(&user).package.id, s3_max);
		assert_eq!(storage_subscription(&user).package.id, max, "drive slot untouched");

		// Neither change ever emptied a slot or armed the resubscribe cooldown.
		assert_ne!(storage_subscription(&user).id, drive_id, "drive slot was replaced in place");
		assert_eq!(Marketplace::last_subscription_cancelled_at(&user), None);
		assert_eq!(
			Marketplace::user_all_subscription_plans(&user)
				.iter()
				.filter(|s| s.active)
				.count(),
			2,
		);
	});
}

#[test]
fn changing_a_flavour_the_user_does_not_hold_is_refused() {
	new_test_ext().execute_with(|| {
		let (solo, _max) = ladder();
		let (_s3_solo, s3_max) = s3_ladder();
		let user = account(11);
		deposit_credits(&user, 10_000, None);
		purchase(&user, solo, None);
		let before = Credits::get_free_credits(&user);

		// Holding a Drive plan is not holding an S3 plan: `change_s3_plan` has no
		// subscription to move, and must not fall back to the Drive one.
		assert_err!(
			change_s3(&user, s3_max),
			pallet_marketplace::Error::<Runtime>::NoActiveSubscription,
		);
		assert_eq!(Credits::get_free_credits(&user), before, "no credits moved");
		assert_eq!(storage_subscription(&user).package.id, solo);
	});
}

#[test]
fn neither_change_accepts_a_plan_of_the_other_flavour() {
	new_test_ext().execute_with(|| {
		let (solo, max) = ladder();
		let (s3_solo, s3_max) = s3_ladder();
		let user = account(11);
		deposit_credits(&user, 100_000, None);
		purchase(&user, solo, None);
		purchase(&user, s3_solo, None);
		let before = Credits::get_free_credits(&user);

		// Crossing the flavours would leave the account holding two of one kind
		// and none of the other, so each call refuses the other's plans.
		assert_err!(
			change_plan(&user, s3_max),
			pallet_marketplace::Error::<Runtime>::InvalidPlanType,
		);
		assert_err!(change_s3(&user, max), pallet_marketplace::Error::<Runtime>::InvalidPlanType,);

		assert_eq!(Credits::get_free_credits(&user), before, "no credits moved");
		assert_eq!(storage_subscription(&user).package.id, solo);
		assert_eq!(s3_subscription(&user).package.id, s3_solo);
	});
}

#[test]
fn an_s3_upgrade_charges_only_the_price_difference() {
	new_test_ext().execute_with(|| {
		let (s3_solo, s3_max) = s3_ladder();
		let user = account(11);

		// The same netting `change_storage_plan` does, on the S3 slot: enough
		// for Solo plus the delta and not a credit more, so paying for January
		// twice would not fit.
		deposit_credits(&user, SOLO_PRICE + (MAX_PRICE - SOLO_PRICE), None);
		purchase(&user, s3_solo, None);
		assert_eq!(Credits::get_free_credits(&user), MAX_PRICE - SOLO_PRICE);

		assert_ok!(change_s3(&user, s3_max));

		assert_eq!(Credits::get_free_credits(&user), 0, "only the delta is charged");
		let sub = s3_subscription(&user);
		assert_eq!(sub.package.id, s3_max);
		assert_eq!(sub.paid_per_month, MAX_PRICE);
		assert_eq!(sub.next_charge_unix_day, Some(FEB1_2026_DAY));
	});
}

#[test]
fn an_s3_downgrade_refunds_surplus_prepaid_months_as_one_movement() {
	new_test_ext().execute_with(|| {
		let (s3_solo, s3_max) = s3_ladder();
		let user = account(11);
		deposit_credits(&user, 9_000, None);
		purchase(&user, s3_max, Some(3));
		assert_eq!(Credits::get_free_credits(&user), 0);

		// Refund = Feb + Mar of Max = 6_000. Charge = Solo's January (1_000)
		// minus the Max January already paid ⇒ 0. Net refund = 6_000.
		assert_ok!(change_s3(&user, s3_solo));

		assert_eq!(Credits::get_free_credits(&user), 6_000);
		assert_eq!(s3_subscription(&user).next_charge_unix_day, Some(FEB1_2026_DAY));
	});
}

#[test]
fn only_whitelisted_callers_may_change_an_s3_plan() {
	new_test_ext().execute_with(|| {
		let (s3_solo, s3_max) = s3_ladder();
		let user = account(11);
		deposit_credits(&user, 10_000, None);
		purchase(&user, s3_solo, None);

		// Same ACL as `change_storage_plan`: neither a stranger…
		assert_err!(
			Marketplace::change_s3_plan(
				RuntimeOrigin::signed(account(99)),
				user.clone(),
				s3_max,
				None,
				None,
				None,
			),
			pallet_marketplace::Error::<Runtime>::WhitelistedCallerNotAuthorized,
		);
		// …nor the subscriber themselves may relay it.
		assert_err!(
			Marketplace::change_s3_plan(
				RuntimeOrigin::signed(user.clone()),
				user.clone(),
				s3_max,
				None,
				None,
				None,
			),
			pallet_marketplace::Error::<Runtime>::WhitelistedCallerNotAuthorized,
		);

		assert_eq!(s3_subscription(&user).package.id, s3_solo);
	});
}

#[test]
fn both_change_calls_share_one_per_block_request_budget() {
	new_test_ext().execute_with(|| {
		let (solo, max) = ladder();
		let (s3_solo, s3_max) = s3_ladder();
		let user = account(11);
		deposit_credits(&user, 100_000, None);
		purchase(&user, solo, None);
		purchase(&user, s3_solo, None);

		// `MaxRequestsPerBlock` is 5 and the two purchases consumed two, so
		// three changes fit across the two calls and the fourth is refused —
		// alternating between them must not double the budget.
		assert_ok!(change_plan(&user, max));
		assert_ok!(change_s3(&user, s3_max));
		assert_ok!(change_plan(&user, solo));
		assert_err!(
			change_s3(&user, s3_solo),
			pallet_marketplace::Error::<Runtime>::TooManyRequests,
		);
	});
}

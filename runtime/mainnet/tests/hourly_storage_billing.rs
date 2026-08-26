//! Hourly pay-as-you-go billing, split per storage flavour.
//!
//! Drive bytes and S3 bytes are metered separately and billed as two line
//! items of one hourly charge. Each line item is exempted by its own plan:
//! an active Drive plan removes the Drive bytes from the bill and an active
//! S3 plan removes the S3 bytes, so holding one plan still leaves the other
//! side's usage billable and holding both leaves nothing to bill.
//!
//! Every expected amount is hand-computed from the per-line ceil-to-GiB
//! rounding, so a regression that merges the two counts before rounding —
//! which would under-bill a user storing a partial GiB on each side — fails
//! loudly rather than cancelling out against a re-derived expectation.

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

/// 2026-01-01T00:00:00Z.
const JAN1_2026_MS: u64 = 1_767_225_600_000;

/// Odd price so the two line items cannot be confused with one doubled charge.
const PRICE_PER_GB: u128 = 2_713;

/// 1 GiB + 1 byte on each side. Rounded per line item that is 2 GiB each, so
/// 4 GiB billed in total; rounded on the *sum* it would be 3 GiB — the
/// difference is what pins the per-line rounding.
const DRIVE_BYTES: u128 = payment_math::GIB + 1;
const S3_BYTES: u128 = payment_math::GIB + 1;
/// `2_713 × 2` — one hour of one side.
const ONE_SIDE_CHARGE: u128 = 5_426;
/// Both sides billed: `5_426 × 2`.
const BOTH_SIDES_CHARGE: u128 = 10_852;

const PLAN_PRICE: u128 = 1_000;
const BANK_FUND: u128 = 1_000_000;

/// `BlocksPerHour` is 600 and the charge check runs only on multiples of
/// `BlockChargeCheckInterval` (8), so the soonest block that can carry an
/// hourly charge after block 0 is 608.
const HOURLY_STEP: u64 = 608;

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
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
		pallet_timestamp::Now::<Runtime>::put(JAN1_2026_MS);
		assert_ok!(Credits::add_authority(RuntimeOrigin::root(), authority()));
		assert_ok!(Marketplace::sudo_set_whitelist_canceller(RuntimeOrigin::root(), backend()));
		assert_ok!(Marketplace::sudo_set_purchase_plan_enabled(RuntimeOrigin::root(), true));
		assert_ok!(Marketplace::set_price_per_gb(RuntimeOrigin::root(), PRICE_PER_GB));
		let _ = Balances::deposit_creating(&Hippocampus::account_id(), BANK_FUND);
		assert_ok!(Hippocampus::add_requester(
			RuntimeOrigin::signed(admin()),
			Marketplace::account_id(),
		));
	});
	ext
}

/// The two storage kinds are mutually exclusive: Drive `(true, false)`,
/// S3 `(false, true)`.
fn add_plan(name: &[u8], is_s3_plan: bool) -> Hashed {
	assert_ok!(Marketplace::add_new_plan(
		RuntimeOrigin::root(),
		name.to_vec(),
		b"{}".to_vec(),
		b"{}".to_vec(),
		PLAN_PRICE,
		!is_s3_plan,
		is_s3_plan,
		Some(1_000_000),
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

/// Report the validator metric the way the backend does.
fn report_usage(who: &AccountId, drive_bytes: u128, s3_bytes: u128) {
	assert_ok!(Marketplace::update_user_file_usage(
		RuntimeOrigin::signed(backend()),
		who.clone(),
		drive_bytes,
		1,
		s3_bytes,
		1,
	));
}

/// Run the charge tick for one hour.
fn run_one_hour() {
	System::set_block_number(HOURLY_STEP);
	Marketplace::on_initialize(HOURLY_STEP);
}

/// Credits spent by `who` over one hourly tick, with a float large enough that
/// nothing is refused for want of funds.
fn hourly_spend(setup: impl FnOnce(&AccountId)) -> u128 {
	let user = account(11);
	deposit_credits(&user, 100_000);
	setup(&user);
	// A plan purchase costs credits of its own; measure from after it.
	let before = Credits::get_free_credits(&user);
	report_usage(&user, DRIVE_BYTES, S3_BYTES);
	run_one_hour();
	before.saturating_sub(Credits::get_free_credits(&user))
}

#[test]
fn a_user_with_no_plan_pays_for_both_drive_and_s3() {
	new_test_ext().execute_with(|| {
		assert_eq!(hourly_spend(|_| {}), BOTH_SIDES_CHARGE);
	});
}

#[test]
fn a_drive_plan_exempts_only_the_drive_bytes() {
	new_test_ext().execute_with(|| {
		let spent = hourly_spend(|user| {
			let drive = add_plan(b"drive", false);
			purchase(user, drive);
		});
		assert_eq!(spent, ONE_SIDE_CHARGE, "S3 usage is still billed hourly");
	});
}

#[test]
fn an_s3_plan_exempts_only_the_s3_bytes() {
	new_test_ext().execute_with(|| {
		let spent = hourly_spend(|user| {
			let s3 = add_plan(b"s3", true);
			purchase(user, s3);
		});
		assert_eq!(spent, ONE_SIDE_CHARGE, "Drive usage is still billed hourly");
	});
}

#[test]
fn holding_both_plans_stops_hourly_billing_entirely() {
	new_test_ext().execute_with(|| {
		let spent = hourly_spend(|user| {
			let drive = add_plan(b"drive", false);
			let s3 = add_plan(b"s3", true);
			purchase(user, drive);
			purchase(user, s3);
		});
		assert_eq!(spent, 0);
	});
}

#[test]
fn each_side_rounds_up_to_a_whole_gib_on_its_own() {
	new_test_ext().execute_with(|| {
		let user = account(11);
		deposit_credits(&user, 100_000);
		let before = Credits::get_free_credits(&user);

		// One byte on each side: two separate line items, each rounding to a
		// full GiB. Summing the bytes first would round to a single GiB and
		// bill half as much.
		report_usage(&user, 1, 1);
		run_one_hour();

		assert_eq!(before - Credits::get_free_credits(&user), PRICE_PER_GB * 2);
	});
}

#[test]
fn an_inactive_plan_stops_exempting_its_side() {
	new_test_ext().execute_with(|| {
		let user = account(11);
		deposit_credits(&user, 100_000);
		let drive = add_plan(b"drive", false);
		purchase(&user, drive);

		let sub_id = Marketplace::user_all_subscription_plans(&user)
			.into_iter()
			.find(|s| s.package.is_drive_plan())
			.expect("drive subscription exists")
			.id;
		assert_ok!(Marketplace::cancel_user_subscription(
			RuntimeOrigin::signed(backend()),
			user.clone(),
			Some(sub_id),
		));

		let before = Credits::get_free_credits(&user);
		report_usage(&user, DRIVE_BYTES, S3_BYTES);
		run_one_hour();

		assert_eq!(
			before - Credits::get_free_credits(&user),
			BOTH_SIDES_CHARGE,
			"a cancelled plan covers nothing",
		);
	});
}

#[test]
fn an_s3_only_user_is_billed_even_with_no_drive_bytes() {
	new_test_ext().execute_with(|| {
		let user = account(11);
		deposit_credits(&user, 100_000);
		let before = Credits::get_free_credits(&user);

		// The sweep iterates the Drive map; both usage maps are written
		// together, so a zero Drive row still carries an S3-only user into it.
		report_usage(&user, 0, S3_BYTES);
		run_one_hour();

		assert_eq!(before - Credits::get_free_credits(&user), ONE_SIDE_CHARGE);
	});
}

#[test]
fn a_user_with_no_reported_usage_is_never_charged() {
	new_test_ext().execute_with(|| {
		let user = account(11);
		deposit_credits(&user, 100_000);
		let before = Credits::get_free_credits(&user);

		report_usage(&user, 0, 0);
		run_one_hour();

		assert_eq!(before, Credits::get_free_credits(&user));
	});
}

// ── Referral commission on the hourly charge ─────────────────────────────
//
// The hourly path follows the monthly-renewal rule, not the purchase rule:
// the referrer earns a commission on what was actually collected and the
// referred user gets no discount. Commissions accrue per referrer and are
// paid out by a separate sweep, so these tests run the sweep before reading
// the referrer's balance.

/// `ReferralPayoutInterval`: accrued commissions are swept every 300 blocks.
const SWEEP_INTERVAL: u64 = 300;

const ED: u128 = 500;

fn fund_ed(who: &AccountId) {
	let _ = Balances::deposit_creating(who, ED);
}

fn new_referral_code(owner: &AccountId) -> Vec<u8> {
	assert_ok!(Credits::create_referral_code(RuntimeOrigin::signed(owner.clone())));
	Credits::get_referral_codes(owner.clone()).pop().expect("code was just created")
}

/// Deposit credits redeeming `code`, which is what attributes the referral.
fn deposit_credits_with_code(who: &AccountId, amount: u128, code: Vec<u8>) {
	assert_ok!(Marketplace::deposit(
		RuntimeOrigin::signed(authority()),
		who.clone(),
		amount,
		0,
		false,
		Some(code),
	));
}

/// Run one commission sweep at the first interval boundary after now.
fn run_sweep() {
	let now: u64 = System::block_number();
	let block = (now / SWEEP_INTERVAL + 1) * SWEEP_INTERVAL;
	System::set_block_number(block);
	Marketplace::on_initialize(block);
}

/// 5% of the collected charge, floored — the same conserved split the monthly
/// path pays.
fn commission_on(charge: u128) -> u128 {
	charge * 500 / 10_000
}

/// Bill one hour for a referred user under `setup`, then sweep. Returns what
/// the referrer earned *from the hourly charge alone* — paid out plus whatever
/// is still accrued.
///
/// Summing the two keeps the assertion about *earnings* rather than about
/// payout timing: the bank pays what it can and leaves any shortfall accrued
/// for the next sweep, so reading the balance alone could report zero and look
/// identical to an exemption that wrongly applied.
///
/// The baseline is taken after `setup` on purpose: a plan purchase pays its own
/// referral commission immediately, straight to the referrer's balance rather
/// than through the sweep, so counting from before setup would fold that into
/// the hourly figure.
fn referrer_cut_for(setup: impl FnOnce(&AccountId)) -> u128 {
	let referrer = account(10);
	let user = account(11);
	let code = new_referral_code(&referrer);
	fund_ed(&referrer);
	deposit_credits_with_code(&user, 100_000, code);
	setup(&user);
	let before =
		Balances::free_balance(&referrer) + Marketplace::accrued_referral_commission(&referrer);

	report_usage(&user, DRIVE_BYTES, S3_BYTES);
	run_one_hour();
	run_sweep();

	Balances::free_balance(&referrer) + Marketplace::accrued_referral_commission(&referrer) - before
}

#[test]
fn the_referrer_earns_on_both_halves_of_an_unsubscribed_bill() {
	new_test_ext().execute_with(|| {
		// Nothing exempt, so the commission follows the full Drive + S3 charge.
		assert_eq!(referrer_cut_for(|_| {}), commission_on(BOTH_SIDES_CHARGE));
	});
}

#[test]
fn a_drive_plan_shrinks_the_commission_to_the_s3_half() {
	new_test_ext().execute_with(|| {
		let cut = referrer_cut_for(|user| {
			let drive = add_plan(b"drive", false);
			purchase(user, drive);
		});
		// Commission is on money actually collected, and the Drive half was
		// never collected — so it is the S3 half alone, not the full bill.
		assert_eq!(cut, commission_on(ONE_SIDE_CHARGE));
	});
}

#[test]
fn an_s3_plan_shrinks_the_commission_to_the_drive_half() {
	new_test_ext().execute_with(|| {
		let cut = referrer_cut_for(|user| {
			let s3 = add_plan(b"s3", true);
			purchase(user, s3);
		});
		assert_eq!(cut, commission_on(ONE_SIDE_CHARGE));
	});
}

#[test]
fn holding_both_plans_earns_the_referrer_nothing_hourly() {
	new_test_ext().execute_with(|| {
		let cut = referrer_cut_for(|user| {
			let drive = add_plan(b"drive", false);
			let s3 = add_plan(b"s3", true);
			purchase(user, drive);
			purchase(user, s3);
		});
		// No hourly charge means no hourly commission. The plan purchases paid
		// their own commission at purchase time, which the baseline excludes.
		assert_eq!(cut, 0);
	});
}

#[test]
fn the_referred_user_gets_no_hourly_discount() {
	new_test_ext().execute_with(|| {
		// A referred user and a plain user with identical usage pay the same:
		// the referral discount applies at purchase, never to hourly billing.
		let referrer = account(10);
		let referred = account(11);
		let plain = account(12);
		let code = new_referral_code(&referrer);
		fund_ed(&referrer);
		deposit_credits_with_code(&referred, 100_000, code);
		deposit_credits(&plain, 100_000);

		let referred_before = Credits::get_free_credits(&referred);
		let plain_before = Credits::get_free_credits(&plain);
		report_usage(&referred, DRIVE_BYTES, S3_BYTES);
		report_usage(&plain, DRIVE_BYTES, S3_BYTES);
		run_one_hour();

		let referred_paid = referred_before - Credits::get_free_credits(&referred);
		let plain_paid = plain_before - Credits::get_free_credits(&plain);
		assert_eq!(referred_paid, plain_paid);
		assert_eq!(referred_paid, BOTH_SIDES_CHARGE);
	});
}

#[test]
fn an_s3_plan_is_provisioned_as_storage_not_compute() {
	new_test_ext().execute_with(|| {
		// S3 plans now set `is_storage_plan: false`, so a routing site still
		// reading that flag alone would send them down the compute path. The
		// storage path provisions from the plan alone and drops any image
		// selection; the compute path would store it.
		let user = account(11);
		deposit_credits(&user, 100_000);
		let s3 = add_plan(b"s3", true);

		assert_ok!(Marketplace::purchase_plan(
			RuntimeOrigin::signed(backend()),
			user.clone(),
			vec![s3],
			None,
			Some(vec![Some(b"ubuntu".to_vec())]),
			None,
			None,
		));

		let sub = Marketplace::user_all_subscription_plans(&user)
			.into_iter()
			.find(|s| s.package.is_s3_plan)
			.expect("s3 subscription exists");
		assert_eq!(sub.selected_image_name, None, "storage plans carry no image");
		assert_eq!(sub.cdn_location_id, None);
	});
}

// ── Monthly renewal: a partial payment stays partial ─────────────────────
//
// An account can hold a Drive plan and an S3 plan at once, so the monthly
// sweep must charge them one at a time. Failing the storage side as a unit
// would take both plans from a user who could afford one — and drop the side
// they had covered onto hourly pay-as-you-go, which is precisely the billing
// this suite exists to guard.

/// 2026-02-01T12:00:00Z — the renewal tick, mid-day so the test does not
/// depend on unix-day boundary rounding.
const FEB1_2026_MS: u64 = 1_769_904_000_000 + 12 * 3_600 * 1_000;
/// A multiple of `BlockChargeCheckInterval` (8), so the tick actually runs.
const CHARGE_BLOCK: u64 = 800;

fn add_priced_plan(name: &[u8], price: u128, is_s3_plan: bool) -> Hashed {
	assert_ok!(Marketplace::add_new_plan(
		RuntimeOrigin::root(),
		name.to_vec(),
		b"{}".to_vec(),
		b"{}".to_vec(),
		price,
		!is_s3_plan,
		is_s3_plan,
		Some(1_000_000),
	));
	<Runtime as frame_system::Config>::Hashing::hash_of(&name.to_vec())
}

fn run_monthly_charge_at_feb1() {
	System::set_block_number(CHARGE_BLOCK);
	pallet_timestamp::Now::<Runtime>::put(FEB1_2026_MS);
	Marketplace::on_initialize(CHARGE_BLOCK);
}

#[test]
fn an_affordable_storage_plan_survives_when_its_sibling_cannot_be_paid() {
	new_test_ext().execute_with(|| {
		let user = account(11);
		// Drive is bought first, so it holds the lower subscription id and is
		// charged first at renewal.
		let drive = add_priced_plan(b"drive", 1_000, false);
		let s3 = add_priced_plan(b"s3", 3_000, true);

		// Exactly both first months, then only enough to renew the cheaper one.
		deposit_credits(&user, 1_000 + 3_000 + 1_000);
		purchase(&user, drive);
		purchase(&user, s3);
		assert_eq!(Credits::get_free_credits(&user), 1_000);

		run_monthly_charge_at_feb1();

		let subs = Marketplace::user_all_subscription_plans(&user);
		let drive_sub = subs.iter().find(|s| s.package.id == drive).expect("drive sub");
		let s3_sub = subs.iter().find(|s| s.package.id == s3).expect("s3 sub");

		assert!(drive_sub.active, "the plan the user could pay for is kept");
		assert!(!s3_sub.active, "only the unaffordable plan is deactivated");
		assert_eq!(Credits::get_free_credits(&user), 0, "the cheaper renewal was collected");
	});
}

#[test]
fn the_surviving_plan_still_suppresses_its_half_of_hourly_billing() {
	new_test_ext().execute_with(|| {
		let user = account(11);
		let drive = add_priced_plan(b"drive", 1_000, false);
		let s3 = add_priced_plan(b"s3", 3_000, true);

		deposit_credits(&user, 1_000 + 3_000 + 1_000);
		purchase(&user, drive);
		purchase(&user, s3);
		run_monthly_charge_at_feb1();

		// Drive survived the renewal, S3 did not. Top up and bill an hour: the
		// user must pay for S3 bytes only. Before the per-subscription fix both
		// plans would have lapsed here and the whole bill would be charged.
		deposit_credits(&user, 100_000);
		let before = Credits::get_free_credits(&user);
		report_usage(&user, DRIVE_BYTES, S3_BYTES);
		System::set_block_number(CHARGE_BLOCK + HOURLY_STEP);
		Marketplace::on_initialize(CHARGE_BLOCK + HOURLY_STEP);

		assert_eq!(
			before - Credits::get_free_credits(&user),
			ONE_SIDE_CHARGE,
			"the surviving Drive plan still covers the Drive bytes",
		);
	});
}

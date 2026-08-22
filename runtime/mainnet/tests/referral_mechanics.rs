//! End-to-end referral mechanics: code creation, redemption, and money flow.
//!
//! The live referral flow spans three pallets:
//! - `pallet-credits` owns the codes (`create_referral_code`) and the
//!   user → code link (`ReferredUsers`, written when credits are deposited
//!   with a code).
//! - `pallet-marketplace` computes the money flow: a fixed 5% buyer discount
//!   in credits at subscription purchase, plus a rate-configurable referrer
//!   commission on the credits actually collected — at purchase and on every
//!   successful recurring monthly charge.
//! - `pallet-hippocampus` (the bank) pays the commission in native tokens:
//!   the marketplace sovereign account must be a whitelisted requester, the
//!   bank never overdraws, and any shortfall is dropped, never owed.
//!
//! All percentages route through `payment-math::split`, which floors the part
//! and conserves the total exactly. The expected values below are hand-computed
//! (not re-derived with `split`) so a regression in the split wiring fails
//! loudly instead of cancelling out.

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

/// 2026-01-01T00:00:00Z. January 2026 has 31 days, so a purchase on the 1st
/// prorates to exactly one full month.
const JAN1_2026_MS: u64 = 1_767_225_600_000;
/// 2026-02-01T12:00:00Z — mid-day so the test does not depend on boundary
/// rounding of the unix-day conversion.
const FEB1_2026_MS: u64 = 1_769_904_000_000 + 12 * 3_600 * 1_000;

/// Unix day numbers of month boundaries around the test window.
const FEB1_2026_DAY: u32 = 20_485;
const MAR1_2026_DAY: u32 = 20_513;
const APR1_2026_DAY: u32 = 20_544;

/// Odd price so the 5% floor actually rounds: 5% of 9_999 is 499.95.
const PLAN_PRICE: u128 = 9_999;
/// `split(9_999, 500).0` = ⌊9_999 × 500 / 10_000⌋ — the buyer-side discount.
const DISCOUNT: u128 = 499;
/// What a referred buyer pays at purchase: face − discount.
const CHARGED: u128 = PLAN_PRICE - DISCOUNT;
/// `split(9_999, 9_500).0` — the discounted full-month price recorded on the
/// subscription for refunds. One unit below `CHARGED` because the 95% part is
/// floored independently of the 5% part.
const PAID_PER_MONTH: u128 = 9_499;

/// Commission tokens on a referred purchase: 5% of the 9_500 actually
/// collected (post-discount), credits → planck 1:1.
const PURCHASE_COMMISSION: u128 = 475;
/// Commission tokens on a renewal: 5% of the 9_999 face price collected.
const RENEWAL_COMMISSION: u128 = 499;

/// Runtime existential deposit; commission transfers below this cannot create
/// the referrer's account.
const ED: u128 = 500;
/// Generous bank funding for the happy-path tests.
const BANK_FUND: u128 = 1_000_000;

/// A block number divisible by the runtime's `BlockChargeCheckInterval` (8),
/// so `on_initialize` reaches the monthly charging path.
const CHARGE_BLOCK: u64 = 16;

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

fn authority() -> AccountId {
	account(1)
}

/// Backend account whitelisted for `purchase_plan` / `cancel_user_subscription`.
fn backend() -> AccountId {
	account(2)
}

/// The bank's `AdminOrigin` (same fixed admin the miner-payment tests use).
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
		// Bank side: fund it and whitelist the marketplace as a requester —
		// the same one-time operational steps mainnet needs after the upgrade.
		let _ = Balances::deposit_creating(&Hippocampus::account_id(), BANK_FUND);
		assert_ok!(Hippocampus::add_requester(
			RuntimeOrigin::signed(admin()),
			Marketplace::account_id(),
		));
	});
	ext
}

fn add_plan(name: &[u8], is_storage_plan: bool) -> <Runtime as frame_system::Config>::Hash {
	assert_ok!(Marketplace::add_new_plan(
		RuntimeOrigin::root(),
		name.to_vec(),
		b"{}".to_vec(),
		b"{}".to_vec(),
		PLAN_PRICE,
		is_storage_plan,
		Some(1_000_000),
	));
	<Runtime as frame_system::Config>::Hashing::hash_of(&name.to_vec())
}

/// Mint spendable credits for `who` the way the backend does, optionally
/// redeeming a referral code in the same deposit.
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

/// Same deposit, dispatched through `RuntimeCall` so extrinsic-level rollback
/// applies — required for the atomicity assertions on failure paths.
fn dispatch_deposit(
	who: &AccountId,
	amount: u128,
	code: Option<Vec<u8>>,
) -> frame_support::dispatch::DispatchResultWithPostInfo {
	RuntimeCall::Marketplace(pallet_marketplace::Call::deposit {
		account: who.clone(),
		credit_amount: amount,
		alpha_amount: 0,
		freeze_for_chargeback: false,
		code,
	})
	.dispatch(RuntimeOrigin::signed(authority()))
}

fn new_referral_code(owner: &AccountId) -> Vec<u8> {
	assert_ok!(Credits::create_referral_code(RuntimeOrigin::signed(owner.clone())));
	Credits::get_referral_codes(owner.clone()).pop().expect("code was just created")
}

/// Give a referrer the existential deposit so commission transfers can land.
/// A first commission below ED cannot create the account — covered by
/// `below_ed_first_commission_is_skipped`.
fn fund_ed(who: &AccountId) {
	let _ = Balances::deposit_creating(who, ED);
}

fn purchase(
	owner: &AccountId,
	plan_id: <Runtime as frame_system::Config>::Hash,
	pay_upfront: Option<u128>,
) {
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

fn storage_subscription(owner: &AccountId) -> pallet_marketplace::UserPlanSubscription<Runtime> {
	Marketplace::user_all_subscription_plans(owner)
		.into_iter()
		.find(|s| s.package.is_storage_plan)
		.expect("storage subscription exists")
}

/// A referred buyer with a storage subscription, ED-funded referrer.
/// Returns (referrer, buyer). `buyer_credits` is deposited with the code.
fn referred_storage_setup(buyer_credits: u128) -> (AccountId, AccountId) {
	let referrer = account(10);
	let buyer = account(11);
	let plan_id = add_plan(b"storage", true);
	let code = new_referral_code(&referrer);
	fund_ed(&referrer);
	deposit_credits(&buyer, buyer_credits, Some(code));
	purchase(&buyer, plan_id, None);
	(referrer, buyer)
}

// ── Code creation ────────────────────────────────────────────────────────

#[test]
fn referrer_gets_a_unique_owned_hippius_code() {
	new_test_ext().execute_with(|| {
		let referrer = account(10);
		let code = new_referral_code(&referrer);

		assert!(code.starts_with(b"HIPPIUS"), "codes carry the HIPPIUS prefix");
		assert_eq!(Credits::referral_codes(&code), Some(referrer.clone()));
		assert_eq!(Credits::get_referral_codes(referrer).len(), 1);
		assert_eq!(Credits::total_referral_codes(), 1);
	});
}

#[test]
fn referral_code_creation_enforces_cooldown() {
	new_test_ext().execute_with(|| {
		let referrer = account(10);
		let first = new_referral_code(&referrer);

		// Same block: still cooling down.
		assert_err!(
			Credits::create_referral_code(RuntimeOrigin::signed(referrer.clone())),
			pallet_credits::Error::<Runtime>::ReferralCodeCooldown,
		);

		// Cooldown is 200 blocks and strict: created at block 1 ⇒ block 201
		// still fails, block 202 succeeds.
		System::set_block_number(201);
		assert_err!(
			Credits::create_referral_code(RuntimeOrigin::signed(referrer.clone())),
			pallet_credits::Error::<Runtime>::ReferralCodeCooldown,
		);

		System::set_block_number(202);
		assert_ok!(Credits::create_referral_code(RuntimeOrigin::signed(referrer.clone())));

		// The old code must survive: deleting it would orphan referred users.
		assert_eq!(Credits::referral_codes(&first), Some(referrer.clone()));
		assert_eq!(Credits::get_referral_codes(referrer).len(), 2);
		assert_eq!(Credits::total_referral_codes(), 2);
	});
}

// ── Redemption (deposit with code) ───────────────────────────────────────

#[test]
fn deposit_with_unknown_code_is_rejected_atomically() {
	new_test_ext().execute_with(|| {
		let buyer = account(11);

		assert_noop!(
			dispatch_deposit(&buyer, 1_000, Some(b"HIPPIUS0".to_vec())),
			pallet_credits::Error::<Runtime>::InvalidReferralCode,
		);
		assert_eq!(Credits::get_free_credits(&buyer), 0, "no credits minted on failure");
		assert_eq!(Credits::referred_users(&buyer), None);
	});
}

#[test]
fn deposit_with_empty_code_is_rejected() {
	new_test_ext().execute_with(|| {
		let buyer = account(11);

		assert_noop!(
			dispatch_deposit(&buyer, 1_000, Some(Vec::new())),
			pallet_credits::Error::<Runtime>::InvalidReferralCode,
		);
	});
}

#[test]
fn deposit_with_own_code_is_rejected() {
	new_test_ext().execute_with(|| {
		let referrer = account(10);
		let code = new_referral_code(&referrer);

		assert_noop!(
			dispatch_deposit(&referrer, 1_000, Some(code)),
			pallet_credits::Error::<Runtime>::InvalidRefferalOwner,
		);
	});
}

#[test]
fn deposit_with_valid_code_links_user_to_referrer() {
	new_test_ext().execute_with(|| {
		let referrer = account(10);
		let buyer = account(11);
		let code = new_referral_code(&referrer);

		deposit_credits(&buyer, 1_000, Some(code.clone()));

		assert_eq!(Credits::get_free_credits(&buyer), 1_000);
		assert_eq!(Credits::referred_users(&buyer), Some(code));
		assert_eq!(Credits::get_referred_users(referrer), vec![buyer]);
	});
}

#[test]
fn later_deposit_with_other_code_switches_referrer() {
	// Documents current behavior: the link is last-write-wins. Every deposit
	// carrying a code silently re-points ALL future discounts and commissions
	// at the new code's owner. If first-referrer-wins is the intended product
	// rule, `insert_referral_code` must refuse to overwrite an existing link.
	new_test_ext().execute_with(|| {
		let first_referrer = account(10);
		let second_referrer = account(12);
		let buyer = account(11);
		let first_code = new_referral_code(&first_referrer);
		let second_code = new_referral_code(&second_referrer);

		deposit_credits(&buyer, 1_000, Some(first_code));
		deposit_credits(&buyer, 1_000, Some(second_code.clone()));

		assert_eq!(Credits::referred_users(&buyer), Some(second_code));
		assert_eq!(Credits::get_referred_users(first_referrer), Vec::<AccountId>::new());
	});
}

// ── Purchase-time discount (credits) and commission (tokens) ─────────────

#[test]
fn storage_purchase_discounts_buyer_and_pays_referrer_tokens() {
	new_test_ext().execute_with(|| {
		// Exactly the discounted price: proves the buyer is charged CHARGED,
		// not a unit more.
		let (referrer, buyer) = referred_storage_setup(CHARGED);

		assert_eq!(Credits::get_free_credits(&buyer), 0);
		// The buyer-side split conserves the face price exactly.
		assert_eq!(CHARGED + DISCOUNT, PLAN_PRICE);

		// Referrer is paid in native tokens from the bank — no credits.
		assert_eq!(Credits::get_free_credits(&referrer), 0);
		assert_eq!(Balances::free_balance(&referrer), ED + PURCHASE_COMMISSION);
		assert_eq!(
			Balances::free_balance(&Hippocampus::account_id()),
			BANK_FUND - PURCHASE_COMMISSION,
		);

		let sub = storage_subscription(&buyer);
		assert!(sub.active);
		assert_eq!(sub.paid_per_month, PAID_PER_MONTH);
		assert_eq!(sub.next_charge_unix_day, Some(FEB1_2026_DAY));
	});
}

#[test]
fn unreferred_purchase_pays_face_price_and_pays_nothing() {
	new_test_ext().execute_with(|| {
		let buyer = account(11);
		let plan_id = add_plan(b"storage", true);

		deposit_credits(&buyer, PLAN_PRICE, None);
		purchase(&buyer, plan_id, None);

		assert_eq!(Credits::get_free_credits(&buyer), 0);
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), BANK_FUND);
		let sub = storage_subscription(&buyer);
		assert_eq!(sub.paid_per_month, PLAN_PRICE);
	});
}

#[test]
fn compute_purchase_applies_same_referral_flow() {
	new_test_ext().execute_with(|| {
		let referrer = account(10);
		let buyer = account(11);
		let plan_id = add_plan(b"compute", false);
		let code = new_referral_code(&referrer);
		fund_ed(&referrer);

		deposit_credits(&buyer, CHARGED, Some(code));
		purchase(&buyer, plan_id, None);

		assert_eq!(Credits::get_free_credits(&buyer), 0);
		assert_eq!(Credits::get_free_credits(&referrer), 0);
		assert_eq!(Balances::free_balance(&referrer), ED + PURCHASE_COMMISSION);
		let sub = Marketplace::user_all_subscription_plans(&buyer)
			.into_iter()
			.find(|s| !s.package.is_storage_plan)
			.expect("compute subscription exists");
		assert_eq!(sub.paid_per_month, PAID_PER_MONTH);
	});
}

#[test]
fn upfront_purchase_discounts_and_commissions_all_prepaid_months() {
	new_test_ext().execute_with(|| {
		let referrer = account(10);
		let buyer = account(11);
		let plan_id = add_plan(b"storage", true);
		let code = new_referral_code(&referrer);
		fund_ed(&referrer);

		// 3 months on Jan 1 ⇒ 3 full months upfront: 29_997 face.
		// Discount = ⌊29_997 × 500 / 10_000⌋ = 1_499; charged = 28_498.
		// Commission = ⌊28_498 × 500 / 10_000⌋ = 1_424 tokens.
		deposit_credits(&buyer, 28_498, Some(code));
		purchase(&buyer, plan_id, Some(3));

		assert_eq!(Credits::get_free_credits(&buyer), 0);
		assert_eq!(Balances::free_balance(&referrer), ED + 1_424);

		let sub = storage_subscription(&buyer);
		assert_eq!(sub.next_charge_unix_day, Some(APR1_2026_DAY));
		assert_eq!(sub.paid_per_month, PAID_PER_MONTH);
	});
}

#[test]
fn cancel_after_upfront_refunds_buyer_but_referrer_keeps_commission() {
	new_test_ext().execute_with(|| {
		let referrer = account(10);
		let buyer = account(11);
		let plan_id = add_plan(b"storage", true);
		let code = new_referral_code(&referrer);
		fund_ed(&referrer);

		deposit_credits(&buyer, 28_498, Some(code));
		purchase(&buyer, plan_id, Some(3));

		// Cancel on Jan 1: the current month is never refunded; Feb and Mar
		// are whole future months, refunded at the discounted monthly rate.
		assert_ok!(Marketplace::cancel_user_subscription(
			RuntimeOrigin::signed(backend()),
			buyer.clone(),
			None,
		));
		assert_eq!(Credits::get_free_credits(&buyer), 2 * PAID_PER_MONTH);
		// No clawback: the tokens already left the bank — current behavior,
		// and a known abuse edge for refunded purchases.
		assert_eq!(Balances::free_balance(&referrer), ED + 1_424);
	});
}

// ── Commission rate governance ───────────────────────────────────────────

#[test]
fn commission_rate_is_sudo_settable_and_decoupled_from_discount() {
	new_test_ext().execute_with(|| {
		assert_err!(
			Marketplace::sudo_set_referral_commission_rate(RuntimeOrigin::root(), 10_001),
			pallet_marketplace::Error::<Runtime>::InvalidInput,
		);
		assert_ok!(Marketplace::sudo_set_referral_commission_rate(RuntimeOrigin::root(), 1_000));

		let (referrer, buyer) = referred_storage_setup(CHARGED);

		// Buyer discount is a fixed 5% regardless of the commission rate…
		assert_eq!(Credits::get_free_credits(&buyer), 0);
		// …while the referrer now earns 10% of the 9_500 collected.
		assert_eq!(Balances::free_balance(&referrer), ED + 950);
	});
}

// ── Bank solvency and misconfiguration backstops ─────────────────────────

#[test]
fn underfunded_bank_pays_partial_and_shortfall_is_dropped() {
	new_test_ext().execute_with(|| {
		// Bank can only release 100 tokens (everything above its ED).
		Balances::make_free_balance_be(&Hippocampus::account_id(), ED + 100);

		let (referrer, buyer) = referred_storage_setup(CHARGED + PLAN_PRICE);

		// Billing is untouched; the referrer gets what the bank could pay.
		assert_eq!(Balances::free_balance(&referrer), ED + 100);
		assert!(storage_subscription(&buyer).active);

		// Refill the bank and renew: the renewal commission is paid in full,
		// but the 375 missed at purchase stays dropped — skip, not arrears.
		let _ = Balances::deposit_creating(&Hippocampus::account_id(), BANK_FUND);
		run_monthly_charge_at_feb1();
		assert_eq!(Balances::free_balance(&referrer), ED + 100 + RENEWAL_COMMISSION);
	});
}

#[test]
fn bank_floor_caps_commission_and_is_never_breached() {
	new_test_ext().execute_with(|| {
		// Leave only 100 tokens of headroom above the root-set floor: the
		// commission (475) must be clipped to 100 and the floor untouched.
		let floor = BANK_FUND - ED - 100;
		assert_ok!(Marketplace::sudo_set_referral_bank_floor(RuntimeOrigin::root(), floor));

		let (referrer, buyer) = referred_storage_setup(CHARGED);

		assert_eq!(Balances::free_balance(&referrer), ED + 100);
		assert!(Balances::free_balance(&Hippocampus::account_id()) >= floor + ED);
		assert!(storage_subscription(&buyer).active, "billing unaffected");
	});
}

#[test]
fn bank_at_floor_pays_no_commission_but_billing_succeeds() {
	new_test_ext().execute_with(|| {
		// Floor above the whole bank balance: commissions stop entirely,
		// the bank keeps every token, and billing still works.
		assert_ok!(Marketplace::sudo_set_referral_bank_floor(RuntimeOrigin::root(), BANK_FUND));

		let (referrer, buyer) = referred_storage_setup(CHARGED);

		assert_eq!(Balances::free_balance(&referrer), ED, "no commission paid");
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), BANK_FUND);
		assert_eq!(Credits::get_free_credits(&buyer), 0, "buyer still charged");
		assert!(storage_subscription(&buyer).active);
	});
}

#[test]
fn commission_cannot_spend_reserved_bank_funds() {
	new_test_ext().execute_with(|| {
		// Wall off all of the bank's payable balance except 20: alpha backing
		// owed to depositors and refunds owed to the sudo account are not
		// spendable as commissions (same rule as ArionPayoutSource).
		let payable = BANK_FUND - ED;
		pallet_marketplace::TotalUndistributedBacking::<Runtime>::put(payable - 60);
		pallet_marketplace::PendingSudoRefunds::<Runtime>::put(40);

		let (referrer, buyer) = referred_storage_setup(CHARGED);

		// Of the bank's payable balance, only 60 is left outside the backing
		// reserve, and 40 of that is owed as sudo refunds — 20 remains.
		assert_eq!(Balances::free_balance(&referrer), ED + 20);
		// Reserved funds and the bank's own ED stay untouched.
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), BANK_FUND - 20);
		assert!(storage_subscription(&buyer).active, "billing unaffected");
	});
}

#[test]
fn commission_cannot_spend_emission_compartment() {
	new_test_ext().execute_with(|| {
		// Wall off all of the bank's payable balance except 20 as bridged
		// emission owed to storage miners: commissions must not touch the
		// compartment `pay_storage_miners` spends from.
		let payable = BANK_FUND - ED;
		pallet_hippocampus::TotalDeposited::<Runtime>::insert(
			pallet_hippocampus::DepositType::Emission,
			payable - 20,
		);

		let (referrer, buyer) = referred_storage_setup(CHARGED);

		assert_eq!(Balances::free_balance(&referrer), ED + 20);
		// The compartment is untouched and still fully backed by the bank.
		assert_eq!(Hippocampus::emission_available(), payable - 20);
		assert!(
			Balances::free_balance(&Hippocampus::account_id()) >= payable - 20 + ED,
			"bank still backs the emission compartment"
		);
		assert!(storage_subscription(&buyer).active, "billing unaffected");
	});
}

#[test]
fn unwhitelisted_marketplace_skips_commission_and_billing_succeeds() {
	new_test_ext().execute_with(|| {
		// Simulate the missed post-upgrade setup step.
		assert_ok!(Hippocampus::remove_requester(
			RuntimeOrigin::signed(admin()),
			Marketplace::account_id(),
		));

		let (referrer, buyer) = referred_storage_setup(CHARGED);

		assert_eq!(Balances::free_balance(&referrer), ED, "commission skipped");
		assert_eq!(Credits::get_free_credits(&buyer), 0, "buyer still charged");
		assert!(storage_subscription(&buyer).active);
	});
}

#[test]
fn below_ed_first_commission_is_skipped() {
	new_test_ext().execute_with(|| {
		let referrer = account(10);
		let buyer = account(11);
		let plan_id = add_plan(b"storage", true);
		let code = new_referral_code(&referrer);
		// Deliberately NOT funding the referrer: a 475-token transfer cannot
		// create an account under the 500 existential deposit, so the
		// commission is skipped. Referrers need an existing account (or a
		// first commission ≥ ED) to receive payouts.
		deposit_credits(&buyer, CHARGED, Some(code));
		purchase(&buyer, plan_id, None);

		assert_eq!(Balances::free_balance(&referrer), 0);
		assert!(storage_subscription(&buyer).active, "billing unaffected");
	});
}

#[test]
fn purchase_beyond_subscription_limit_rolls_back_atomically() {
	// Regression test for an external security-review claim: a purchase that
	// fails the late MaxActiveSubscriptions check supposedly leaves credits
	// consumed and the referral commission paid. FRAME wraps every
	// dispatchable in a storage layer, so the whole extrinsic must revert —
	// buyer credits, referrer tokens, bank balance, and events included.
	new_test_ext().execute_with(|| {
		let referrer = account(10);
		let buyer = account(11);
		let code = new_referral_code(&referrer);
		fund_ed(&referrer);

		// MaxActiveSubscriptions is 5: fill the limit with five compute plans
		// in a single purchase_plan call (also the MaxRequestsPerBlock cap).
		let plan_ids: Vec<_> =
			[&b"compute-a"[..], b"compute-b", b"compute-c", b"compute-d", b"compute-e"]
				.iter()
				.map(|name| add_plan(name, false))
				.collect();
		let sixth = add_plan(b"compute-f", false);

		deposit_credits(&buyer, 6 * CHARGED, Some(code));
		assert_ok!(Marketplace::purchase_plan(
			RuntimeOrigin::signed(backend()),
			buyer.clone(),
			plan_ids,
			None,
			None,
			None,
			None,
		));

		let credits_before = Credits::get_free_credits(&buyer);
		let referrer_before = Balances::free_balance(&referrer);
		let bank_before = Balances::free_balance(&Hippocampus::account_id());
		assert_eq!(credits_before, CHARGED, "five purchases charged");
		assert_eq!(referrer_before, ED + 5 * PURCHASE_COMMISSION);

		// New block so the per-block request counter clears (every 15 blocks).
		System::set_block_number(15);
		Marketplace::on_initialize(15);

		// The sixth purchase must fail the subscription limit AND leave no
		// trace: assert_noop checks the storage root is untouched.
		assert_noop!(
			RuntimeCall::Marketplace(pallet_marketplace::Call::purchase_plan {
				owner: buyer.clone(),
				plan_ids: vec![sixth],
				location_ids: None,
				selected_image_names: None,
				cloud_init_cids: None,
				pay_upfront: None,
			})
			.dispatch(RuntimeOrigin::signed(backend())),
			pallet_marketplace::Error::<Runtime>::TooManyActiveSubscriptions,
		);

		assert_eq!(Credits::get_free_credits(&buyer), credits_before, "no credits lost");
		assert_eq!(Balances::free_balance(&referrer), referrer_before, "no unearned commission");
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), bank_before);
		assert_eq!(Marketplace::user_all_subscription_plans(&buyer).len(), 5);
	});
}

#[test]
fn total_paid_by_requester_tracks_marketplace_outflow() {
	new_test_ext().execute_with(|| {
		let (_referrer, _buyer) = referred_storage_setup(CHARGED + PLAN_PRICE);
		run_monthly_charge_at_feb1();

		assert_eq!(
			pallet_hippocampus::TotalPaidByRequester::<Runtime>::get(Marketplace::account_id()),
			PURCHASE_COMMISSION + RENEWAL_COMMISSION,
		);
	});
}

// ── Recurring monthly commission ─────────────────────────────────────────

fn run_monthly_charge_at_feb1() {
	System::set_block_number(CHARGE_BLOCK);
	pallet_timestamp::Now::<Runtime>::put(FEB1_2026_MS);
	Marketplace::on_initialize(CHARGE_BLOCK);
}

#[test]
fn monthly_renewal_pays_commission_in_tokens() {
	new_test_ext().execute_with(|| {
		// Enough for the discounted first month plus one full-price renewal:
		// renewals are charged at face price, the discount is purchase-only.
		let (referrer, buyer) = referred_storage_setup(CHARGED + PLAN_PRICE);

		run_monthly_charge_at_feb1();

		assert_eq!(Credits::get_free_credits(&buyer), 0);
		assert_eq!(Credits::get_free_credits(&referrer), 0, "no credits ever minted");
		assert_eq!(
			Balances::free_balance(&referrer),
			ED + PURCHASE_COMMISSION + RENEWAL_COMMISSION,
		);

		let sub = storage_subscription(&buyer);
		assert!(sub.active);
		assert_eq!(sub.next_charge_unix_day, Some(MAR1_2026_DAY));
	});
}

#[test]
fn monthly_renewal_without_referrer_pays_no_commission() {
	new_test_ext().execute_with(|| {
		let buyer = account(11);
		let plan_id = add_plan(b"storage", true);

		deposit_credits(&buyer, 2 * PLAN_PRICE, None);
		purchase(&buyer, plan_id, None);

		let bank_before = Balances::free_balance(&Hippocampus::account_id());
		run_monthly_charge_at_feb1();

		assert_eq!(Credits::get_free_credits(&buyer), 0);
		// Nothing left the bank during the run.
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), bank_before);
	});
}

#[test]
fn failed_renewal_deactivates_subscription_and_pays_no_commission() {
	new_test_ext().execute_with(|| {
		// Only enough for the discounted first month — the renewal must fail.
		let (referrer, buyer) = referred_storage_setup(CHARGED);

		run_monthly_charge_at_feb1();

		let sub = storage_subscription(&buyer);
		assert!(!sub.active, "unpayable subscription is deactivated");
		// Commission accrues only on money actually collected.
		assert_eq!(Balances::free_balance(&referrer), ED + PURCHASE_COMMISSION);
	});
}

// ── Hourly pay-as-you-go commission ──────────────────────────────────────
//
// Users without an active storage plan are billed per GiB every hour by
// `handle_arion_storage_charging`. That path follows the monthly-renewal
// rule, not the purchase rule: the referrer earns a commission, the referred
// user gets no discount. Because an hour of per-GB billing is dust, the
// commission accrues per referrer and settles in one transfer once it clears
// `ReferralPayoutThreshold`.

/// Hourly price per GiB. Odd so the 5% commission actually floors.
const PRICE_PER_GB: u128 = 411;
/// 3 GiB + 1 byte — billed as 4 whole GiB, so `gibs_ceil` rounding is live.
const FILE_BYTES: u128 = 3 * payment_math::GIB + 1;
/// `411 × 4` — one hour of billing.
const HOURLY_CHARGE: u128 = 1_644;
/// `split(1_644, 500).0` = ⌊1_644 × 500 / 10_000⌋ = ⌊82.2⌋.
const HOURLY_COMMISSION: u128 = 82;

/// `BlocksPerHour` is 600 and the charge check only runs on multiples of
/// `BlockChargeCheckInterval` (8), so the soonest block that can carry the
/// next hourly charge is 608 after the previous one. Hour `h` is therefore
/// block `h × 608`, and consecutive hours are always far enough apart.
const HOURLY_STEP: u64 = 608;

/// Low threshold so a payout lands on the third hour (246 ≥ 200) instead of
/// the thirteenth, while staying above what a single hour earns.
const TEST_THRESHOLD: u128 = 200;

fn set_threshold(threshold: u128) {
	assert_ok!(Marketplace::sudo_set_referral_payout_threshold(RuntimeOrigin::root(), threshold));
}

/// Put `who` on the hourly path: files stored in arion, no storage plan.
fn store_files(who: &AccountId) {
	assert_ok!(Marketplace::set_price_per_gb(RuntimeOrigin::root(), PRICE_PER_GB));
	pallet_arion::UserTotalFilesSize::<Runtime>::insert(who, FILE_BYTES);
}

/// Run the hourly charge tick for each hour in `hours` (1-based). Ranges are
/// explicit rather than cumulative so a test that charges again after an
/// assertion says exactly which hours it is adding.
fn run_hours(hours: core::ops::RangeInclusive<u64>) {
	for h in hours {
		let block = h * HOURLY_STEP;
		System::set_block_number(block);
		Marketplace::on_initialize(block);
	}
}

/// A referred user billed hourly: ED-funded referrer, credits deposited with
/// the code, files stored, no subscription. Returns (referrer, user).
fn referred_hourly_setup(user_credits: u128) -> (AccountId, AccountId) {
	let referrer = account(10);
	let user = account(11);
	let code = new_referral_code(&referrer);
	fund_ed(&referrer);
	deposit_credits(&user, user_credits, Some(code));
	store_files(&user);
	(referrer, user)
}

#[test]
fn hourly_charge_below_threshold_accrues_without_paying() {
	new_test_ext().execute_with(|| {
		set_threshold(TEST_THRESHOLD);
		let (referrer, user) = referred_hourly_setup(10 * HOURLY_CHARGE);
		let bank_before = Balances::free_balance(&Hippocampus::account_id());

		run_hours(1..=1);

		// The user was billed one hour at the full per-GiB rate.
		assert_eq!(Credits::get_free_credits(&user), 9 * HOURLY_CHARGE);
		// Commission is held, not paid: nothing left the bank.
		assert_eq!(Marketplace::accrued_referral_commission(&referrer), HOURLY_COMMISSION);
		assert_eq!(Balances::free_balance(&referrer), ED);
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), bank_before);
	});
}

#[test]
fn hourly_accrual_pays_out_once_it_clears_the_threshold() {
	new_test_ext().execute_with(|| {
		set_threshold(TEST_THRESHOLD);
		let (referrer, user) = referred_hourly_setup(10 * HOURLY_CHARGE);
		let bank_before = Balances::free_balance(&Hippocampus::account_id());

		// 82, then 164 — both under 200 — then 246 clears it and settles all
		// three hours in one transfer.
		run_hours(1..=3);

		let payout = 3 * HOURLY_COMMISSION;
		assert_eq!(payout, 246);
		assert_eq!(Balances::free_balance(&referrer), ED + payout);
		assert_eq!(
			Marketplace::accrued_referral_commission(&referrer),
			0,
			"a fully paid accrual is cleared, not left as a zero row"
		);
		assert!(!pallet_marketplace::AccruedReferralCommission::<Runtime>::contains_key(&referrer));
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), bank_before - payout);
		assert_eq!(Credits::get_free_credits(&user), 7 * HOURLY_CHARGE);
	});
}

#[test]
fn hourly_accrual_restarts_after_a_payout() {
	new_test_ext().execute_with(|| {
		set_threshold(TEST_THRESHOLD);
		let (referrer, _user) = referred_hourly_setup(10 * HOURLY_CHARGE);

		run_hours(1..=3);
		assert_eq!(Balances::free_balance(&referrer), ED + 3 * HOURLY_COMMISSION);

		// Hour 4 opens a fresh accrual on top of the hour-3 payout.
		run_hours(4..=4);
		assert_eq!(Balances::free_balance(&referrer), ED + 3 * HOURLY_COMMISSION);
		assert_eq!(Marketplace::accrued_referral_commission(&referrer), HOURLY_COMMISSION);
	});
}

#[test]
fn default_threshold_batches_thirteen_hours_into_one_payout() {
	new_test_ext().execute_with(|| {
		// No sudo call: exercise the shipped default (1_000), which needs
		// ⌈1_000 / 82⌉ = 13 hours to clear.
		assert_eq!(Marketplace::referral_payout_threshold(), 1_000);
		let (referrer, _user) = referred_hourly_setup(20 * HOURLY_CHARGE);

		run_hours(1..=12);
		assert_eq!(12 * HOURLY_COMMISSION, 984);
		assert_eq!(Marketplace::accrued_referral_commission(&referrer), 984);
		assert_eq!(Balances::free_balance(&referrer), ED, "984 is still under 1_000");

		run_hours(13..=13);
		assert_eq!(13 * HOURLY_COMMISSION, 1_066);
		assert_eq!(Balances::free_balance(&referrer), ED + 1_066);
		assert_eq!(Marketplace::accrued_referral_commission(&referrer), 0);
	});
}

#[test]
fn hourly_charge_without_a_referrer_accrues_nothing() {
	new_test_ext().execute_with(|| {
		set_threshold(TEST_THRESHOLD);
		let user = account(11);
		deposit_credits(&user, 10 * HOURLY_CHARGE, None);
		store_files(&user);
		let bank_before = Balances::free_balance(&Hippocampus::account_id());

		run_hours(1..=3);

		assert_eq!(Credits::get_free_credits(&user), 7 * HOURLY_CHARGE);
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), bank_before);
	});
}

#[test]
fn hourly_billing_gives_the_referred_user_no_discount() {
	new_test_ext().execute_with(|| {
		set_threshold(TEST_THRESHOLD);
		let (_referrer, referred) = referred_hourly_setup(10 * HOURLY_CHARGE);
		let plain = account(12);
		deposit_credits(&plain, 10 * HOURLY_CHARGE, None);
		pallet_arion::UserTotalFilesSize::<Runtime>::insert(&plain, FILE_BYTES);

		run_hours(1..=1);

		// The 5% buyer discount is purchase-only; hourly usage is recurring,
		// so both users pay the identical per-GiB charge.
		assert_eq!(Credits::get_free_credits(&referred), Credits::get_free_credits(&plain));
		assert_eq!(Credits::get_free_credits(&referred), 9 * HOURLY_CHARGE);
	});
}

#[test]
fn subscriber_who_cancels_keeps_earning_commission_hourly() {
	new_test_ext().execute_with(|| {
		set_threshold(TEST_THRESHOLD);
		let referrer = account(10);
		let buyer = account(11);
		let plan_id = add_plan(b"storage", true);
		let code = new_referral_code(&referrer);
		fund_ed(&referrer);
		// Enough for the discounted first month plus several hourly charges.
		deposit_credits(&buyer, CHARGED + 10 * HOURLY_CHARGE, Some(code));
		purchase(&buyer, plan_id, None);

		// The purchase commission lands immediately, and the subscription
		// suppresses hourly billing entirely.
		assert_eq!(Balances::free_balance(&referrer), ED + PURCHASE_COMMISSION);
		store_files(&buyer);
		run_hours(1..=1);
		assert_eq!(
			Credits::get_free_credits(&buyer),
			10 * HOURLY_CHARGE,
			"an active storage plan is not also billed per GiB"
		);
		assert_eq!(Marketplace::accrued_referral_commission(&referrer), 0);

		// Cancel: a Jan-1 purchase has no whole future month to refund, so the
		// credit balance is untouched and the user drops onto the hourly path.
		assert_ok!(Marketplace::cancel_user_subscription(
			RuntimeOrigin::signed(backend()),
			buyer.clone(),
			None,
		));
		assert_eq!(Credits::get_free_credits(&buyer), 10 * HOURLY_CHARGE);

		run_hours(2..=5);

		assert_eq!(Credits::get_free_credits(&buyer), 6 * HOURLY_CHARGE);
		// The first three post-cancel hours settle at 246; the fourth accrues
		// 82 towards the next payout.
		assert_eq!(
			Balances::free_balance(&referrer),
			ED + PURCHASE_COMMISSION + 3 * HOURLY_COMMISSION,
		);
		assert_eq!(Marketplace::accrued_referral_commission(&referrer), HOURLY_COMMISSION);
	});
}

#[test]
fn accrual_survives_a_bank_at_its_floor_and_pays_later() {
	new_test_ext().execute_with(|| {
		set_threshold(TEST_THRESHOLD);
		// Floor above the whole bank balance: nothing is payable.
		assert_ok!(Marketplace::sudo_set_referral_bank_floor(RuntimeOrigin::root(), BANK_FUND));
		let (referrer, _user) = referred_hourly_setup(10 * HOURLY_CHARGE);

		run_hours(1..=3);

		// The payout attempt released nothing, so the balance stays owed —
		// unlike the one-shot purchase path, where a shortfall is dropped
		// because there is nowhere to record it.
		assert_eq!(Balances::free_balance(&referrer), ED);
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), BANK_FUND);
		assert_eq!(Marketplace::accrued_referral_commission(&referrer), 3 * HOURLY_COMMISSION);

		// Lift the floor: the next hour settles everything accrued so far.
		assert_ok!(Marketplace::sudo_set_referral_bank_floor(RuntimeOrigin::root(), 0));
		run_hours(4..=4);

		assert_eq!(Balances::free_balance(&referrer), ED + 4 * HOURLY_COMMISSION);
		assert_eq!(Marketplace::accrued_referral_commission(&referrer), 0);
	});
}

#[test]
fn partial_payout_leaves_the_exact_remainder_accrued() {
	new_test_ext().execute_with(|| {
		set_threshold(TEST_THRESHOLD);
		// Leave exactly 100 tokens of headroom above the floor.
		let floor = BANK_FUND - ED - 100;
		assert_ok!(Marketplace::sudo_set_referral_bank_floor(RuntimeOrigin::root(), floor));
		let (referrer, _user) = referred_hourly_setup(10 * HOURLY_CHARGE);

		run_hours(1..=3);

		// 246 requested, 100 released, 146 still owed — and the floor holds.
		assert_eq!(Balances::free_balance(&referrer), ED + 100);
		assert_eq!(
			Marketplace::accrued_referral_commission(&referrer),
			3 * HOURLY_COMMISSION - 100,
		);
		assert!(Balances::free_balance(&Hippocampus::account_id()) >= floor + ED);
	});
}

#[test]
fn accrual_is_retained_until_it_can_create_an_unfunded_referrers_account() {
	new_test_ext().execute_with(|| {
		// Threshold below ED with a referrer that was never funded: every
		// payout attempt from hour 2 on fails the ED check until the accrual
		// itself reaches 500.
		set_threshold(100);
		let referrer = account(10);
		let user = account(11);
		let code = new_referral_code(&referrer);
		deposit_credits(&user, 10 * HOURLY_CHARGE, Some(code));
		store_files(&user);
		assert_eq!(Balances::free_balance(&referrer), 0);

		// 82 · 164 · 246 · 328 · 410 · 492 — all under the 500 ED.
		run_hours(1..=6);
		assert_eq!(6 * HOURLY_COMMISSION, 492);
		assert_eq!(Balances::free_balance(&referrer), 0);
		assert_eq!(Marketplace::accrued_referral_commission(&referrer), 492);

		// Hour 7 reaches 574 ≥ ED, so the transfer can now create the account.
		run_hours(7..=7);
		assert_eq!(7 * HOURLY_COMMISSION, 574);
		assert_eq!(Balances::free_balance(&referrer), 574);
		assert_eq!(Marketplace::accrued_referral_commission(&referrer), 0);
	});
}

#[test]
fn hourly_commission_follows_the_sudo_set_rate() {
	new_test_ext().execute_with(|| {
		set_threshold(TEST_THRESHOLD);
		// 10% instead of the default 5%: ⌊1_644 × 1_000 / 10_000⌋ = 164.
		assert_ok!(Marketplace::sudo_set_referral_commission_rate(RuntimeOrigin::root(), 1_000));
		let (referrer, _user) = referred_hourly_setup(10 * HOURLY_CHARGE);

		run_hours(1..=1);
		assert_eq!(Marketplace::accrued_referral_commission(&referrer), 164);

		// 328 clears the threshold on hour 2 instead of hour 3.
		run_hours(2..=2);
		assert_eq!(Balances::free_balance(&referrer), ED + 328);
		assert_eq!(Marketplace::accrued_referral_commission(&referrer), 0);
	});
}

#[test]
fn a_failed_hourly_charge_earns_no_commission() {
	new_test_ext().execute_with(|| {
		set_threshold(TEST_THRESHOLD);
		// Credits for two hours only; the third charge finds an empty balance.
		let (referrer, user) = referred_hourly_setup(2 * HOURLY_CHARGE);

		run_hours(1..=3);

		assert_eq!(Credits::get_free_credits(&user), 0);
		// Two hours earned 164 — under the threshold, so nothing was paid, and
		// the uncollected third hour added nothing.
		assert_eq!(Marketplace::accrued_referral_commission(&referrer), 2 * HOURLY_COMMISSION);
		assert_eq!(Balances::free_balance(&referrer), ED);
	});
}

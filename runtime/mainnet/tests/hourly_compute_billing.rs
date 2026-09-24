//! Hourly usage-based compute billing: `submit_compute_usage`,
//! `settle_compute_arrears` and the caps and authorities around them.
//!
//! The backend closes each hour, sums every customer's compute usage off-chain
//! and submits one row per customer. What the chain owes that design is that a
//! row is charged **exactly once** however often it is resubmitted, that one
//! customer who cannot pay never costs the others their charge, and that a
//! stolen billing key can only move a bounded amount. Each test below pins one
//! of those.

use frame_support::{
	assert_noop, assert_ok,
	dispatch::{DispatchClass, GetDispatchInfo, Pays},
	traits::{Currency, Hooks},
	weights::Weight,
};
use hippius_mainnet_runtime::{
	AccountId, Balances, Credits, Hippocampus, Marketplace, Runtime, RuntimeCall, RuntimeEvent,
	RuntimeOrigin, System,
};
use pallet_marketplace::{
	ComputeArrears, ComputeChargeOutcome, ComputeUsageCharged, ComputeUsagePruneFrom,
	ComputeUsageRecord, ComputeUsageRefusal, Error, Event as MarketplaceEvent,
	LastComputeBillingPeriod, COMPUTE_BILLING_PERIOD_MS,
};
use sp_core::{crypto::Ss58Codec, H256};
use sp_runtime::{AccountId32, BuildStorage, DispatchError};

/// 2026-01-01T00:00:00Z, a whole hour.
const JAN1_2026_MS: u64 = 1_767_225_600_000;

/// The hour that ends at `JAN1_2026_MS`: the most recent closed period while
/// the clock sits there.
const PERIOD: u64 = JAN1_2026_MS / COMPUTE_BILLING_PERIOD_MS - 1;

/// `MaxComputeBillingLag` and `ComputeUsageRetention` in the runtime.
const LAG: u64 = 72;
const RETENTION: u64 = 168;

const PER_ACCOUNT_CAP: u128 = 1_000_000;
const PER_CALL_CAP: u128 = 10_000_000;

/// Deliberately odd, so a charge taken twice or folded wrongly cannot land on
/// an expected balance by accident.
const HOUR_COST: u128 = 1_237;

const BANK_FUND: u128 = 1_000_000;

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

/// Deposits credits (pallet-credits authority).
fn depositor() -> AccountId {
	account(1)
}

/// The compute billing key.
fn biller() -> AccountId {
	account(2)
}

fn admin() -> AccountId {
	AccountId32::from_ss58check("5CVXqxb7mhFTtZVw5BJ8M2ujND9PFymSDxF8bkod6Sm4XJTW").unwrap()
}

fn hash(n: u8) -> H256 {
	H256::repeat_byte(n)
}

fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| {
		System::set_block_number(1);
		pallet_timestamp::Now::<Runtime>::put(JAN1_2026_MS);
		assert_ok!(Credits::add_authority(RuntimeOrigin::root(), depositor()));
		assert_ok!(Marketplace::set_compute_billing_authorities(
			RuntimeOrigin::root(),
			vec![biller()],
		));
		assert_ok!(Marketplace::set_compute_charge_caps(
			RuntimeOrigin::root(),
			PER_ACCOUNT_CAP,
			PER_CALL_CAP,
		));
		let _ = Balances::deposit_creating(&Hippocampus::account_id(), BANK_FUND);
		assert_ok!(Hippocampus::add_requester(
			RuntimeOrigin::signed(admin()),
			Marketplace::account_id(),
		));
	});
	ext
}

fn deposit_credits(who: &AccountId, amount: u128) {
	assert_ok!(Marketplace::deposit(
		RuntimeOrigin::signed(depositor()),
		who.clone(),
		amount,
		0,
		false,
		None,
	));
}

fn credits(who: &AccountId) -> u128 {
	Credits::get_free_credits(who)
}

fn submit(period: u64, rows: Vec<(AccountId, u128, H256)>) {
	assert_ok!(Marketplace::submit_compute_usage(RuntimeOrigin::signed(biller()), period, rows));
}

/// Dispatch errors without the post-dispatch info, which the early exits set
/// to refund their unused weight.
fn submit_as(
	origin: RuntimeOrigin,
	period: u64,
	rows: Vec<(AccountId, u128, H256)>,
) -> Result<(), DispatchError> {
	Marketplace::submit_compute_usage(origin, period, rows)
		.map(|_| ())
		.map_err(|e| e.error)
}

fn settle_as(origin: RuntimeOrigin, accounts: Vec<AccountId>) -> Result<(), DispatchError> {
	Marketplace::settle_compute_arrears(origin, accounts)
		.map(|_| ())
		.map_err(|e| e.error)
}

fn record(period: u64, who: &AccountId) -> Option<ComputeUsageRecord> {
	ComputeUsageCharged::<Runtime>::get(period, who)
}

fn marketplace_events() -> Vec<MarketplaceEvent<Runtime>> {
	System::events()
		.into_iter()
		.filter_map(|r| match r.event {
			RuntimeEvent::Marketplace(e) => Some(e),
			_ => None,
		})
		.collect()
}

/// The `ComputeUsageSubmitted` summary of the most recent call.
fn last_summary() -> (u32, u32, u32, u32, u32, u128) {
	marketplace_events()
		.into_iter()
		.rev()
		.find_map(|e| match e {
			MarketplaceEvent::ComputeUsageSubmitted {
				charged,
				arrears,
				duplicates,
				conflicts,
				refused,
				total_accepted,
				..
			} => Some((charged, arrears, duplicates, conflicts, refused, total_accepted)),
			_ => None,
		})
		.expect("a submission summary was emitted")
}

/// Put the clock at the start of `period`, making it the current (open) one.
fn set_current_period(period: u64) {
	pallet_timestamp::Now::<Runtime>::put(period * COMPUTE_BILLING_PERIOD_MS);
}

// ── Charging ─────────────────────────────────────────────────────────────

#[test]
fn a_row_is_debited_and_recorded() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		deposit_credits(&user, 10_000);

		submit(PERIOD, vec![(user.clone(), HOUR_COST, hash(1))]);

		assert_eq!(credits(&user), 10_000 - HOUR_COST);
		assert_eq!(
			record(PERIOD, &user),
			Some(ComputeUsageRecord {
				amount: HOUR_COST,
				usage_hash: hash(1),
				outcome: ComputeChargeOutcome::Charged,
			})
		);
		assert_eq!(LastComputeBillingPeriod::<Runtime>::get(), Some(PERIOD));
		assert!(marketplace_events().contains(&MarketplaceEvent::ComputeUsageCharged {
			who: user,
			period: PERIOD,
			amount: HOUR_COST,
			arrears_collected: 0,
			usage_hash: hash(1),
		}));
		assert_eq!(last_summary(), (1, 0, 0, 0, 0, HOUR_COST));
	});
}

#[test]
fn an_exact_resubmit_is_a_duplicate_and_debits_nothing() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		deposit_credits(&user, 10_000);

		submit(PERIOD, vec![(user.clone(), HOUR_COST, hash(1))]);
		// The backend's retry after an expired era: same period, same row.
		submit(PERIOD, vec![(user.clone(), HOUR_COST, hash(1))]);

		assert_eq!(credits(&user), 10_000 - HOUR_COST);
		assert!(marketplace_events().contains(&MarketplaceEvent::ComputeUsageDuplicate {
			who: user,
			period: PERIOD,
			usage_hash: hash(1),
		}));
		assert_eq!(last_summary(), (0, 0, 1, 0, 0, 0));
	});
}

#[test]
fn a_different_row_for_a_charged_hour_is_a_conflict_and_the_first_stands() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		deposit_credits(&user, 10_000);
		submit(PERIOD, vec![(user.clone(), HOUR_COST, hash(1))]);

		// Different hash, same amount — and same hash, different amount. Either
		// half differing is enough: the record is of the pair.
		submit(PERIOD, vec![(user.clone(), HOUR_COST, hash(2))]);
		assert_eq!(last_summary(), (0, 0, 0, 1, 0, 0));
		submit(PERIOD, vec![(user.clone(), HOUR_COST + 1, hash(1))]);
		assert_eq!(last_summary(), (0, 0, 0, 1, 0, 0));

		assert_eq!(credits(&user), 10_000 - HOUR_COST);
		assert_eq!(record(PERIOD, &user).unwrap().usage_hash, hash(1));
		assert!(marketplace_events().contains(&MarketplaceEvent::ComputeUsageConflict {
			who: user,
			period: PERIOD,
			recorded_amount: HOUR_COST,
			recorded_hash: hash(1),
			submitted_amount: HOUR_COST,
			submitted_hash: hash(2),
		}));
	});
}

#[test]
fn an_account_repeated_within_one_call_is_charged_once() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		deposit_credits(&user, 10_000);

		submit(
			PERIOD,
			vec![
				(user.clone(), HOUR_COST, hash(1)),
				(user.clone(), HOUR_COST, hash(1)),
				(user.clone(), HOUR_COST * 2, hash(9)),
			],
		);

		assert_eq!(credits(&user), 10_000 - HOUR_COST);
		assert_eq!(last_summary(), (1, 0, 1, 1, 0, HOUR_COST));
	});
}

#[test]
fn the_same_account_is_charged_again_for_a_different_hour() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		deposit_credits(&user, 10_000);

		submit(PERIOD - 1, vec![(user.clone(), HOUR_COST, hash(1))]);
		submit(PERIOD, vec![(user.clone(), HOUR_COST, hash(1))]);

		assert_eq!(credits(&user), 10_000 - 2 * HOUR_COST);
		// Catch-up of an older hour never moves the high-water mark back.
		submit(PERIOD - 2, vec![(user.clone(), HOUR_COST, hash(1))]);
		assert_eq!(LastComputeBillingPeriod::<Runtime>::get(), Some(PERIOD));
		assert_eq!(credits(&user), 10_000 - 3 * HOUR_COST);
	});
}

#[test]
fn a_zero_row_is_recorded_without_touching_credits() {
	new_test_ext().execute_with(|| {
		let user = account(10);

		// No credits at all: a zero-price hour must still go through, which is
		// what lets the pipeline run end to end before prices are set.
		submit(PERIOD, vec![(user.clone(), 0, hash(1))]);

		assert_eq!(record(PERIOD, &user).unwrap().outcome, ComputeChargeOutcome::Charged);
		assert_eq!(ComputeArrears::<Runtime>::get(&user), 0);
		assert_eq!(last_summary(), (1, 0, 0, 0, 0, 0));
	});
}

#[test]
fn a_successful_charge_accrues_referral_commission() {
	new_test_ext().execute_with(|| {
		let referrer = account(20);
		let user = account(10);
		deposit_credits(&user, 1_000_000);
		pallet_credits::ReferralCodes::<Runtime>::insert(b"code".to_vec(), referrer.clone());
		pallet_credits::ReferredUsers::<Runtime>::insert(&user, b"code".to_vec());

		submit(PERIOD, vec![(user, 100_000, hash(1))]);

		// 5% default commission rate on what was actually collected.
		assert_eq!(Marketplace::accrued_referral_commission(&referrer), 5_000);
	});
}

// ── Arrears ──────────────────────────────────────────────────────────────

#[test]
fn a_broke_account_goes_to_arrears_without_reverting_the_others() {
	new_test_ext().execute_with(|| {
		let rich = account(10);
		let broke = account(11);
		let rich_too = account(12);
		deposit_credits(&rich, 10_000);
		deposit_credits(&broke, HOUR_COST - 1);
		deposit_credits(&rich_too, 10_000);

		submit(
			PERIOD,
			vec![
				(rich.clone(), HOUR_COST, hash(1)),
				(broke.clone(), HOUR_COST, hash(2)),
				(rich_too.clone(), HOUR_COST, hash(3)),
			],
		);

		assert_eq!(credits(&rich), 10_000 - HOUR_COST);
		assert_eq!(credits(&rich_too), 10_000 - HOUR_COST);
		// Nothing taken from the broke account — not even the part it had.
		assert_eq!(credits(&broke), HOUR_COST - 1);
		assert_eq!(ComputeArrears::<Runtime>::get(&broke), HOUR_COST);
		assert_eq!(record(PERIOD, &broke).unwrap().outcome, ComputeChargeOutcome::Arrears);
		assert!(marketplace_events().contains(&MarketplaceEvent::ComputeUsageChargeFailed {
			who: broke.clone(),
			period: PERIOD,
			amount: HOUR_COST,
			required: HOUR_COST,
			available: HOUR_COST - 1,
			usage_hash: hash(2),
		}));
		assert_eq!(last_summary(), (2, 1, 0, 0, 0, 3 * HOUR_COST));

		// A failed row is still a settled row: resubmitting it does not add the
		// hour to the arrears a second time.
		submit(PERIOD, vec![(broke.clone(), HOUR_COST, hash(2))]);
		assert_eq!(ComputeArrears::<Runtime>::get(&broke), HOUR_COST);
	});
}

#[test]
fn arrears_are_folded_into_the_next_charge() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		deposit_credits(&user, HOUR_COST - 1);
		submit(PERIOD - 1, vec![(user.clone(), HOUR_COST, hash(1))]);
		assert_eq!(ComputeArrears::<Runtime>::get(&user), HOUR_COST);

		// Enough for this hour but not for this hour plus the arrears: nothing
		// is debited, and the arrears grow by the hour.
		deposit_credits(&user, 1);
		submit(PERIOD, vec![(user.clone(), HOUR_COST, hash(2))]);
		assert_eq!(credits(&user), HOUR_COST);
		assert_eq!(ComputeArrears::<Runtime>::get(&user), 2 * HOUR_COST);

		// Funded for everything: the next hour collects all of it in one debit.
		deposit_credits(&user, 10_000);
		set_current_period(PERIOD + 2);
		submit(PERIOD + 1, vec![(user.clone(), HOUR_COST, hash(3))]);
		assert_eq!(credits(&user), HOUR_COST + 10_000 - 3 * HOUR_COST);
		assert_eq!(ComputeArrears::<Runtime>::get(&user), 0);
		assert!(marketplace_events().contains(&MarketplaceEvent::ComputeUsageCharged {
			who: user,
			period: PERIOD + 1,
			amount: HOUR_COST,
			arrears_collected: 2 * HOUR_COST,
			usage_hash: hash(3),
		}));
	});
}

#[test]
fn a_zero_row_leaves_existing_arrears_alone() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		submit(PERIOD - 1, vec![(user.clone(), HOUR_COST, hash(1))]);
		deposit_credits(&user, 10_000);

		submit(PERIOD, vec![(user.clone(), 0, hash(2))]);

		assert_eq!(ComputeArrears::<Runtime>::get(&user), HOUR_COST);
		assert_eq!(credits(&user), 10_000);
	});
}

#[test]
fn arrears_are_settled_after_a_top_up() {
	new_test_ext().execute_with(|| {
		let broke = account(10);
		let still_broke = account(11);
		let clear = account(12);
		submit(
			PERIOD,
			vec![(broke.clone(), HOUR_COST, hash(1)), (still_broke.clone(), HOUR_COST, hash(2))],
		);
		assert_eq!(ComputeArrears::<Runtime>::get(&broke), HOUR_COST);

		deposit_credits(&broke, 10_000);
		deposit_credits(&still_broke, HOUR_COST - 1);
		assert_ok!(Marketplace::settle_compute_arrears(
			RuntimeOrigin::signed(biller()),
			vec![broke.clone(), still_broke.clone(), clear.clone()],
		));

		assert_eq!(ComputeArrears::<Runtime>::get(&broke), 0);
		assert_eq!(credits(&broke), 10_000 - HOUR_COST);
		// The one that still cannot pay neither blocks the others nor loses
		// what it had.
		assert_eq!(ComputeArrears::<Runtime>::get(&still_broke), HOUR_COST);
		assert_eq!(credits(&still_broke), HOUR_COST - 1);

		let events = marketplace_events();
		assert!(events.contains(&MarketplaceEvent::ComputeArrearsSettled {
			who: broke.clone(),
			amount: HOUR_COST,
		}));
		assert!(events.contains(&MarketplaceEvent::ComputeArrearsSettleFailed {
			who: still_broke,
			required: HOUR_COST,
			available: HOUR_COST - 1,
		}));
		// An account with nothing owed is skipped silently.
		assert!(!events.iter().any(|e| matches!(
			e,
			MarketplaceEvent::ComputeArrearsSettled { who, .. }
				| MarketplaceEvent::ComputeArrearsSettleFailed { who, .. } if *who == clear
		)));

		// Settling twice takes nothing more.
		assert_ok!(Marketplace::settle_compute_arrears(
			RuntimeOrigin::signed(biller()),
			vec![broke.clone()],
		));
		assert_eq!(credits(&broke), 10_000 - HOUR_COST);
	});
}

#[test]
fn arrears_that_would_overflow_refuse_the_row_instead_of_forgiving_part_of_it() {
	new_test_ext().execute_with(|| {
		assert_ok!(Marketplace::set_compute_charge_caps(
			RuntimeOrigin::root(),
			u128::MAX,
			u128::MAX,
		));
		let user = account(10);
		ComputeArrears::<Runtime>::insert(&user, u128::MAX - 5);

		submit(PERIOD, vec![(user.clone(), 10, hash(1))]);

		assert_eq!(ComputeArrears::<Runtime>::get(&user), u128::MAX - 5);
		assert_eq!(record(PERIOD, &user), None);
		assert!(marketplace_events().contains(&MarketplaceEvent::ComputeUsageRefused {
			who: user,
			period: PERIOD,
			amount: 10,
			reason: ComputeUsageRefusal::ArrearsOverflow,
		}));
	});
}

#[test]
fn a_repeated_account_is_settled_once() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		submit(PERIOD, vec![(user.clone(), HOUR_COST, hash(1))]);
		deposit_credits(&user, HOUR_COST - 1);

		assert_ok!(Marketplace::settle_compute_arrears(
			RuntimeOrigin::signed(biller()),
			vec![user.clone(), user.clone(), user.clone()],
		));

		let attempts = marketplace_events()
			.into_iter()
			.filter(|e| matches!(e, MarketplaceEvent::ComputeArrearsSettleFailed { .. }))
			.count();
		assert_eq!(attempts, 1);
	});
}

// ── Caps ─────────────────────────────────────────────────────────────────

#[test]
fn unset_caps_refuse_every_non_zero_row() {
	new_test_ext().execute_with(|| {
		assert_ok!(Marketplace::set_compute_charge_caps(RuntimeOrigin::root(), 0, 0));
		let user = account(10);
		deposit_credits(&user, 10_000);

		submit(PERIOD, vec![(user.clone(), 1, hash(1)), (account(11), 0, hash(2))]);

		assert_eq!(credits(&user), 10_000);
		assert_eq!(record(PERIOD, &user), None);
		assert_eq!(last_summary(), (1, 0, 0, 0, 1, 0));
	});
}

#[test]
fn the_per_account_cap_refuses_the_row_and_leaves_it_resubmittable() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		deposit_credits(&user, 10 * PER_ACCOUNT_CAP);

		submit(PERIOD, vec![(user.clone(), PER_ACCOUNT_CAP + 1, hash(1))]);
		assert_eq!(credits(&user), 10 * PER_ACCOUNT_CAP);
		assert_eq!(record(PERIOD, &user), None);
		assert!(marketplace_events().contains(&MarketplaceEvent::ComputeUsageRefused {
			who: user.clone(),
			period: PERIOD,
			amount: PER_ACCOUNT_CAP + 1,
			reason: ComputeUsageRefusal::AccountCapExceeded,
		}));

		// Exactly at the cap is allowed.
		submit(PERIOD, vec![(account(11), PER_ACCOUNT_CAP, hash(3))]);
		assert_eq!(last_summary(), (0, 1, 0, 0, 0, PER_ACCOUNT_CAP));

		// Root raises the cap and the same row goes through: the refusal left
		// no record to conflict with.
		assert_ok!(Marketplace::set_compute_charge_caps(
			RuntimeOrigin::root(),
			2 * PER_ACCOUNT_CAP,
			PER_CALL_CAP,
		));
		submit(PERIOD, vec![(user.clone(), PER_ACCOUNT_CAP + 1, hash(1))]);
		assert_eq!(credits(&user), 10 * PER_ACCOUNT_CAP - PER_ACCOUNT_CAP - 1);
	});
}

#[test]
fn the_per_call_cap_bounds_the_running_total() {
	new_test_ext().execute_with(|| {
		assert_ok!(Marketplace::set_compute_charge_caps(RuntimeOrigin::root(), 1_000, 2_500));
		let users: Vec<AccountId> = (10..14).map(account).collect();
		for u in &users {
			deposit_credits(u, 10_000);
		}

		submit(PERIOD, users.iter().map(|u| (u.clone(), 1_000, hash(1))).collect());

		// 1_000 + 1_000 fit under 2_500; the third would make 3_000.
		assert_eq!(credits(&users[0]), 9_000);
		assert_eq!(credits(&users[1]), 9_000);
		assert_eq!(credits(&users[2]), 10_000);
		assert_eq!(credits(&users[3]), 10_000);
		assert!(marketplace_events().contains(&MarketplaceEvent::ComputeUsageRefused {
			who: users[2].clone(),
			period: PERIOD,
			amount: 1_000,
			reason: ComputeUsageRefusal::CallCapExceeded,
		}));
		assert_eq!(last_summary(), (2, 0, 0, 0, 2, 2_000));

		// A row that fits in the remaining headroom still goes through after a
		// refusal — the cap bounds the total, it does not end the call.
		submit(
			PERIOD,
			vec![
				(users[2].clone(), 1_000, hash(1)),
				(users[3].clone(), 1_000, hash(1)),
				(account(20), 500, hash(1)),
				(account(21), 1, hash(1)),
			],
		);
		assert_eq!(last_summary(), (2, 1, 0, 0, 1, 2_500));
	});
}

#[test]
fn a_duplicate_is_honoured_even_after_the_cap_is_lowered() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		deposit_credits(&user, 10_000);
		submit(PERIOD, vec![(user.clone(), HOUR_COST, hash(1))]);

		assert_ok!(Marketplace::set_compute_charge_caps(RuntimeOrigin::root(), 0, 0));
		submit(PERIOD, vec![(user.clone(), HOUR_COST, hash(1))]);

		// A duplicate, not a refusal the backend would read as "resubmit".
		assert_eq!(last_summary(), (0, 0, 1, 0, 0, 0));
	});
}

// ── Periods ──────────────────────────────────────────────────────────────

#[test]
fn only_closed_periods_within_the_lag_are_billable() {
	new_test_ext().execute_with(|| {
		let current = PERIOD + 1;
		let user = account(10);
		deposit_credits(&user, 1_000_000);
		let row = || vec![(user.clone(), HOUR_COST, hash(1))];

		assert_noop!(
			submit_as(RuntimeOrigin::signed(biller()), current, row()),
			Error::<Runtime>::ComputeBillingPeriodNotClosed
		);
		assert_noop!(
			submit_as(RuntimeOrigin::signed(biller()), u64::MAX, row()),
			Error::<Runtime>::ComputeBillingPeriodNotClosed
		);
		assert_noop!(
			submit_as(RuntimeOrigin::signed(biller()), current - LAG - 1, row()),
			Error::<Runtime>::ComputeBillingPeriodTooOld
		);

		submit(current - LAG, row());
		assert_eq!(credits(&user), 1_000_000 - HOUR_COST);
	});
}

#[test]
fn too_many_rows_are_rejected_whole() {
	new_test_ext().execute_with(|| {
		let rows: Vec<(AccountId, u128, H256)> = (0..251u32)
			.map(|n| {
				let mut raw = [0u8; 32];
				raw[..4].copy_from_slice(&n.to_le_bytes());
				(AccountId32::new(raw), 0, hash(1))
			})
			.collect();
		assert_noop!(
			submit_as(RuntimeOrigin::signed(biller()), PERIOD, rows),
			Error::<Runtime>::TooManyUpdates
		);
		assert_noop!(
			settle_as(RuntimeOrigin::signed(biller()), (0..251u8).map(account).collect()),
			Error::<Runtime>::TooManyUpdates
		);
	});
}

// ── Pruning ──────────────────────────────────────────────────────────────

#[test]
fn records_are_pruned_once_past_retention_and_not_before() {
	new_test_ext().execute_with(|| {
		let users: Vec<AccountId> = (10..15).map(account).collect();
		submit(PERIOD - 1, users.iter().map(|u| (u.clone(), 0, hash(1))).collect());
		submit(PERIOD, users.iter().map(|u| (u.clone(), 0, hash(1))).collect());
		assert_eq!(ComputeUsagePruneFrom::<Runtime>::get(), Some(PERIOD - 1));

		// Exactly `RETENTION` periods old: still kept.
		set_current_period(PERIOD - 1 + RETENTION);
		Marketplace::on_idle(System::block_number(), Weight::MAX);
		assert!(users.iter().all(|u| record(PERIOD - 1, u).is_some()));

		// One hour on, the older period goes and the newer one stays.
		set_current_period(PERIOD + RETENTION);
		Marketplace::on_idle(System::block_number(), Weight::MAX);
		assert!(users.iter().all(|u| record(PERIOD - 1, u).is_none()));
		assert!(users.iter().all(|u| record(PERIOD, u).is_some()));
		assert_eq!(ComputeUsagePruneFrom::<Runtime>::get(), Some(PERIOD));

		// Once everything is gone the cursor is cleared, so an idle chain pays
		// one read per block for it.
		set_current_period(PERIOD + RETENTION + 1);
		Marketplace::on_idle(System::block_number(), Weight::MAX);
		assert!(users.iter().all(|u| record(PERIOD, u).is_none()));
		assert_eq!(ComputeUsagePruneFrom::<Runtime>::get(), None);
	});
}

#[test]
fn pruning_stays_within_the_weight_it_is_given() {
	let users: Vec<AccountId> = (10..20).map(account).collect();
	let mut ext = new_test_ext();
	ext.execute_with(|| {
		submit(PERIOD, users.iter().map(|u| (u.clone(), 0, hash(1))).collect());
	});
	// The records were written in an earlier block. `clear_prefix` only counts
	// committed keys against its limit, so an uncommitted overlay would be
	// cleared whole and hide whether the limit holds.
	ext.commit_all().unwrap();
	ext.execute_with(|| {
		set_current_period(PERIOD + RETENTION + 1);

		let db = <Runtime as frame_system::Config>::DbWeight::get();
		// Overhead, the prefix probe, and three keys — bounded on time, with
		// proof size left unconstrained as it is on mainnet.
		let time = db.reads_writes(3, 1) + db.reads(1) + db.reads_writes(3, 3);
		let limit = Weight::from_parts(time.ref_time(), u64::MAX);
		let used = Marketplace::on_idle(System::block_number(), limit);

		assert!(used.all_lte(limit));
		let left = users.iter().filter(|u| record(PERIOD, u).is_some()).count();
		assert_eq!(left, 7);
		// Not finished, so the cursor still points at the period.
		assert_eq!(ComputeUsagePruneFrom::<Runtime>::get(), Some(PERIOD));

		// A budget too small for even the overhead does nothing at all.
		assert_eq!(Marketplace::on_idle(System::block_number(), Weight::zero()), Weight::zero());

		Marketplace::on_idle(System::block_number(), Weight::MAX);
		assert!(users.iter().all(|u| record(PERIOD, u).is_none()));
	});
}

#[test]
fn a_pruned_period_can_never_be_charged_again() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		deposit_credits(&user, 10_000);
		submit(PERIOD, vec![(user.clone(), HOUR_COST, hash(1))]);

		set_current_period(PERIOD + RETENTION + 1);
		Marketplace::on_idle(System::block_number(), Weight::MAX);
		assert_eq!(record(PERIOD, &user), None);

		// The record is gone, and the lag is what stops the replay.
		assert_noop!(
			submit_as(
				RuntimeOrigin::signed(biller()),
				PERIOD,
				vec![(user.clone(), HOUR_COST, hash(1))]
			),
			Error::<Runtime>::ComputeBillingPeriodTooOld
		);
		assert_eq!(credits(&user), 10_000 - HOUR_COST);
	});
}

// ── Origins ──────────────────────────────────────────────────────────────

#[test]
fn only_an_authority_can_submit_or_settle() {
	new_test_ext().execute_with(|| {
		let stranger = account(99);
		let victim = account(10);
		deposit_credits(&victim, 10_000);
		ComputeArrears::<Runtime>::insert(&victim, 100);

		assert_noop!(
			submit_as(
				RuntimeOrigin::signed(stranger.clone()),
				PERIOD,
				vec![(victim.clone(), HOUR_COST, hash(1))]
			),
			Error::<Runtime>::NotComputeBillingAuthority
		);
		// Other privileged marketplace keys are not billing keys.
		assert_ok!(Marketplace::sudo_set_whitelist_canceller(
			RuntimeOrigin::root(),
			stranger.clone()
		));
		assert_noop!(
			submit_as(
				RuntimeOrigin::signed(stranger.clone()),
				PERIOD,
				vec![(victim.clone(), HOUR_COST, hash(1))]
			),
			Error::<Runtime>::NotComputeBillingAuthority
		);
		assert_noop!(
			settle_as(RuntimeOrigin::signed(stranger.clone()), vec![victim.clone()]),
			Error::<Runtime>::NotComputeBillingAuthority
		);
		assert_noop!(submit_as(RuntimeOrigin::root(), PERIOD, vec![]), DispatchError::BadOrigin);

		// An emptied list switches billing off, for the old key too.
		assert_ok!(Marketplace::set_compute_billing_authorities(RuntimeOrigin::root(), vec![]));
		assert_noop!(
			submit_as(RuntimeOrigin::signed(biller()), PERIOD, vec![]),
			Error::<Runtime>::NotComputeBillingAuthority
		);
	});
}

#[test]
fn only_root_sets_authorities_and_caps() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Marketplace::set_compute_billing_authorities(
				RuntimeOrigin::signed(biller()),
				vec![account(99)]
			),
			DispatchError::BadOrigin
		);
		assert_noop!(
			Marketplace::set_compute_charge_caps(RuntimeOrigin::signed(biller()), 1, 1),
			DispatchError::BadOrigin
		);
		assert_noop!(
			Marketplace::set_compute_billing_authorities(
				RuntimeOrigin::root(),
				(0..9u8).map(account).collect()
			),
			Error::<Runtime>::TooManyComputeBillingAuthorities
		);

		assert_ok!(Marketplace::set_compute_billing_authorities(
			RuntimeOrigin::root(),
			vec![account(3), account(3), biller()]
		));
		assert_eq!(
			pallet_marketplace::ComputeBillingAuthorities::<Runtime>::get(),
			vec![biller(), account(3)]
		);
		assert_eq!(pallet_marketplace::MaxComputeChargePerCall::<Runtime>::get(), PER_CALL_CAP);
	});
}

/// The state the `submit_compute_usage` benchmark prices a row at — a long
/// batch history ending in a batch that matured while frozen — actually takes
/// the charged path. The benchmark itself only runs under
/// `runtime-benchmarks`; this keeps its premise checked in the ordinary suite.
#[test]
fn the_benchmarked_worst_case_row_is_charged_through_the_unfreeze_branch() {
	use pallet_marketplace::{Batch, Batches, NextBatchId, UnbackedBatchAlpha, UserBatches};

	new_test_ext().execute_with(|| {
		let user = account(10);
		let referrer = account(20);
		let funding: u128 = 1_000_000_000;
		let push = |batch: Batch<AccountId, u64>| {
			let id = NextBatchId::<Runtime>::get();
			Batches::<Runtime>::insert(id, batch);
			UserBatches::<Runtime>::append(&user, id);
			NextBatchId::<Runtime>::put(id + 1);
			id
		};
		for _ in 1..16 {
			push(Batch {
				owner: user.clone(),
				credit_amount: funding,
				alpha_amount: funding,
				remaining_credits: 0,
				remaining_alpha: 0,
				pending_alpha: 0,
				is_frozen: false,
				release_time: 0,
			});
		}
		let funded = push(Batch {
			owner: user.clone(),
			credit_amount: funding,
			alpha_amount: funding,
			remaining_credits: funding,
			remaining_alpha: funding,
			pending_alpha: funding / 2,
			is_frozen: true,
			release_time: 0,
		});
		UnbackedBatchAlpha::<Runtime>::insert(funded, funding / 4);
		pallet_marketplace::TotalUndistributedBacking::<Runtime>::put(funding * 2);
		pallet_credits::AlphaBalances::<Runtime>::insert(&user, funding * 2);
		assert_ok!(Credits::do_mint(user.clone(), funding, None));
		ComputeArrears::<Runtime>::insert(&user, 500);
		pallet_credits::ReferralCodes::<Runtime>::insert(b"code".to_vec(), referrer.clone());
		pallet_credits::ReferredUsers::<Runtime>::insert(&user, b"code".to_vec());

		submit(PERIOD, vec![(user.clone(), 1_000, hash(1))]);

		assert_eq!(record(PERIOD, &user).unwrap().outcome, ComputeChargeOutcome::Charged);
		assert_eq!(ComputeArrears::<Runtime>::get(&user), 0);
		assert_eq!(credits(&user), funding - 1_500);
		assert!(!Batches::<Runtime>::get(funded).unwrap().is_frozen);
		assert!(Marketplace::accrued_referral_commission(&referrer) > 0);
	});
}

// ── Weight ───────────────────────────────────────────────────────────────

#[test]
fn the_declared_weight_scales_with_rows_and_is_feeless() {
	new_test_ext().execute_with(|| {
		let rows = |n: u8| -> Vec<(AccountId, u128, H256)> {
			(0..n).map(|i| (account(i), HOUR_COST, hash(i))).collect()
		};
		let info = |n: u8| {
			RuntimeCall::Marketplace(pallet_marketplace::Call::submit_compute_usage {
				period: PERIOD,
				rows: rows(n),
			})
			.get_dispatch_info()
		};

		let (zero, one, many) = (info(0).weight, info(1).weight, info(250).weight);
		let per_row = one - zero;
		assert!(per_row.ref_time() > 0, "a row must cost something");
		assert_eq!(many, zero + per_row * 250);
		assert_eq!(info(250).pays_fee, Pays::No);

		// Settling is priced the same way.
		let settle = |n: u8| {
			RuntimeCall::Marketplace(pallet_marketplace::Call::settle_compute_arrears {
				accounts: (0..n).map(account).collect(),
			})
			.get_dispatch_info()
		};
		let settle_row = settle(1).weight - settle(0).weight;
		assert!(settle_row.ref_time() > 0);
		assert_eq!(settle(200).weight, settle(0).weight + settle_row * 200);

		// A full call still fits in one normal-class extrinsic.
		let max_extrinsic = <Runtime as frame_system::Config>::BlockWeights::get()
			.get(DispatchClass::Normal)
			.max_extrinsic
			.expect("normal extrinsics are bounded");
		assert!(many.all_lte(max_extrinsic), "250 rows take {many:?} of {max_extrinsic:?}");
	});
}

#[test]
fn a_refused_origin_does_not_hold_the_full_block_weight() {
	new_test_ext().execute_with(|| {
		let rows: Vec<(AccountId, u128, H256)> =
			(0..250u8).map(|i| (account(i), HOUR_COST, hash(i))).collect();
		let call = RuntimeCall::Marketplace(pallet_marketplace::Call::submit_compute_usage {
			period: PERIOD,
			rows,
		});
		let declared = call.get_dispatch_info().weight;

		let err =
			sp_runtime::traits::Dispatchable::dispatch(call, RuntimeOrigin::signed(account(99)))
				.unwrap_err();

		let actual = err.post_info.actual_weight.expect("an early exit reports its weight");
		assert!(actual.ref_time() < declared.ref_time() / 10);
	});
}

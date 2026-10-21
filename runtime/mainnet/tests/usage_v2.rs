//! Itemised compute billing: `submit_usage_v2`, its fee kinds and their caps.
//!
//! v2 is `submit_compute_usage` with each row's total split into
//! `(UsageKind, amount)` lines. What it owes on top of v1 is that a malformed
//! breakdown costs only its own row, that it shares v1's idempotency record so
//! no `(period, account)` is ever debited twice across the two calls, and that
//! a fee kind — a monthly plan fee billed through the hourly path — is held to
//! its own caps and a budget on the chain clock rather than to the hourly cap.

use frame_support::{
	assert_noop, assert_ok,
	dispatch::{DispatchClass, GetDispatchInfo, Pays},
	traits::Currency,
	BoundedVec,
};
use hippius_mainnet_runtime::{
	AccountId, Balances, Credits, Hippocampus, Marketplace, Runtime, RuntimeCall, RuntimeEvent,
	RuntimeOrigin, System,
};
use pallet_marketplace::{
	AccountChargeCapOverride, ChargeCapAdmins, ComputeArrears, ComputeChargeOutcome,
	ComputeUsageCharged, ComputeUsageRecord, ComputeUsageRefusal, Error, Event as MarketplaceEvent,
	FeeCap, FeeCapConfig, FeeSpend, UsageBreakdown, UsageKind, UsageRowV2,
	COMPUTE_BILLING_PERIOD_MS, MAX_FEE_KINDS, MAX_USAGE_KINDS_PER_ROW,
};
use sp_core::{crypto::Ss58Codec, H256};
use sp_runtime::{AccountId32, BuildStorage, DispatchError};

/// 2026-01-01T00:00:00Z, a whole hour.
const JAN1_2026_MS: u64 = 1_767_225_600_000;

/// The current (open) period while the clock sits at `JAN1_2026_MS`.
const NOW: u64 = JAN1_2026_MS / COMPUTE_BILLING_PERIOD_MS;

/// The most recent closed period.
const PERIOD: u64 = NOW - 1;

/// `MaxComputeBillingLag` in the runtime.
const LAG: u64 = 72;

const PER_ACCOUNT_CAP: u128 = 1_000_000;
const PER_CALL_CAP: u128 = 10_000_000;

/// Deliberately odd, so a charge taken twice cannot land on an expected
/// balance by accident.
const HOUR_COST: u128 = 1_237;

/// A monthly plan fee: far above the hourly cap, which is the point of
/// charging it as a fee kind.
const PLAN_FEE: u128 = 5_000_000;

const BANK_FUND: u128 = 1_000_000;

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

fn depositor() -> AccountId {
	account(1)
}

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

fn breakdown(lines: &[(UsageKind, u128)]) -> UsageBreakdown {
	lines.to_vec().try_into().expect("at most 16 lines")
}

/// A row whose total is the sum of its lines.
fn row(who: &AccountId, lines: &[(UsageKind, u128)], h: H256) -> UsageRowV2<AccountId> {
	let total = lines.iter().map(|(_, a)| a).sum();
	UsageRowV2 { who: who.clone(), total, usage_hash: h, breakdown: breakdown(lines) }
}

fn rows(rows: Vec<UsageRowV2<AccountId>>) -> BoundedVec<UsageRowV2<AccountId>, ConstRows> {
	rows.try_into().expect("within MaxComputeUsageRowsPerCall")
}

type ConstRows = <Runtime as pallet_marketplace::Config>::MaxComputeUsageRowsPerCall;

fn submit(period: u64, r: Vec<UsageRowV2<AccountId>>) {
	assert_ok!(Marketplace::submit_usage_v2(RuntimeOrigin::signed(biller()), period, rows(r)));
}

fn submit_as(
	origin: RuntimeOrigin,
	period: u64,
	r: Vec<UsageRowV2<AccountId>>,
) -> Result<(), DispatchError> {
	Marketplace::submit_usage_v2(origin, period, rows(r))
		.map(|_| ())
		.map_err(|e| e.error)
}

fn submit_v1(period: u64, r: Vec<(AccountId, u128, H256)>) {
	assert_ok!(Marketplace::submit_compute_usage(RuntimeOrigin::signed(biller()), period, r));
}

fn record(period: u64, who: &AccountId) -> Option<ComputeUsageRecord> {
	ComputeUsageCharged::<Runtime>::get(period, who)
}

fn set_fee_cap(kind: UsageKind, cap: Option<FeeCap>) {
	assert_ok!(Marketplace::set_fee_kind_caps(RuntimeOrigin::root(), kind, cap));
}

/// S3Plan caps generous enough to stay out of the way of the test at hand.
fn plan_fee_cap() -> FeeCap {
	FeeCap {
		per_debit: PLAN_FEE,
		per_call: PLAN_FEE * 100,
		budget: PLAN_FEE * 100,
		window_periods: 720,
	}
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

fn refusals() -> Vec<(AccountId, ComputeUsageRefusal)> {
	marketplace_events()
		.into_iter()
		.filter_map(|e| match e {
			MarketplaceEvent::ComputeUsageRefused { who, reason, .. } => Some((who, reason)),
			_ => None,
		})
		.collect()
}

/// `(charged, arrears, duplicates, conflicts, refused, total_accepted)` of
/// the most recent call.
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

fn set_current_period(period: u64) {
	pallet_timestamp::Now::<Runtime>::put(period * COMPUTE_BILLING_PERIOD_MS);
}

// ── Charging ─────────────────────────────────────────────────────────────

#[test]
fn a_row_is_debited_recorded_and_its_breakdown_reported() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		deposit_credits(&user, 10_000);
		let lines =
			[(UsageKind::Compute, 1_000), (UsageKind::PublicIp, 200), (UsageKind::Backup, 37)];

		submit(PERIOD, vec![row(&user, &lines, hash(1))]);

		assert_eq!(credits(&user), 10_000 - HOUR_COST);
		assert_eq!(
			record(PERIOD, &user),
			Some(ComputeUsageRecord {
				amount: HOUR_COST,
				usage_hash: hash(1),
				outcome: ComputeChargeOutcome::Charged,
			})
		);
		let events = marketplace_events();
		assert!(events.contains(&MarketplaceEvent::UsageCharged {
			who: user.clone(),
			period: PERIOD,
			amount: HOUR_COST,
			arrears_collected: 0,
			usage_hash: hash(1),
			breakdown: breakdown(&lines),
		}));
		// The v1 event is not emitted alongside.
		assert!(!events.iter().any(|e| matches!(e, MarketplaceEvent::ComputeUsageCharged { .. })));
		assert_eq!(last_summary(), (1, 0, 0, 0, 0, HOUR_COST));
	});
}

#[test]
fn a_malformed_breakdown_refuses_only_its_own_row() {
	new_test_ext().execute_with(|| {
		let (good, mismatch, duplicate, unsorted, empty, overflow) =
			(account(10), account(11), account(12), account(13), account(14), account(15));
		for who in [&good, &mismatch, &duplicate, &unsorted, &empty, &overflow] {
			deposit_credits(who, 10_000);
		}
		let bad = |who: &AccountId, total: u128, lines: &[(UsageKind, u128)]| UsageRowV2 {
			who: who.clone(),
			total,
			usage_hash: hash(9),
			breakdown: breakdown(lines),
		};

		submit(
			PERIOD,
			vec![
				bad(&mismatch, 101, &[(UsageKind::Compute, 100)]),
				bad(&duplicate, 200, &[(UsageKind::Compute, 100), (UsageKind::Compute, 100)]),
				bad(&unsorted, 200, &[(UsageKind::PublicIp, 100), (UsageKind::Compute, 100)]),
				bad(&empty, 0, &[]),
				bad(&overflow, 1, &[(UsageKind::Compute, u128::MAX), (UsageKind::PublicIp, 2)]),
				row(&good, &[(UsageKind::Compute, HOUR_COST)], hash(1)),
			],
		);

		assert_eq!(credits(&good), 10_000 - HOUR_COST);
		for who in [&mismatch, &duplicate, &unsorted, &empty, &overflow] {
			assert_eq!(credits(who), 10_000);
			assert_eq!(record(PERIOD, who), None, "a refused row is left resubmittable");
		}
		assert_eq!(
			refusals(),
			[&mismatch, &duplicate, &unsorted, &empty, &overflow]
				.into_iter()
				.map(|who| (who.clone(), ComputeUsageRefusal::BreakdownInvalid))
				.collect::<Vec<_>>()
		);
		assert_eq!(last_summary(), (1, 0, 0, 0, 5, HOUR_COST));
	});
}

#[test]
fn v1_then_v2_on_the_same_hour_never_debits_twice() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		deposit_credits(&user, 10_000);
		submit_v1(PERIOD, vec![(user.clone(), HOUR_COST, hash(1))]);

		// The same total and hash is the same row.
		submit(PERIOD, vec![row(&user, &[(UsageKind::Compute, HOUR_COST)], hash(1))]);
		assert_eq!(last_summary(), (0, 0, 1, 0, 0, 0));

		// Anything else for the hour is a conflict.
		submit(PERIOD, vec![row(&user, &[(UsageKind::Compute, HOUR_COST)], hash(2))]);
		assert_eq!(last_summary(), (0, 0, 0, 1, 0, 0));

		assert_eq!(credits(&user), 10_000 - HOUR_COST);
		assert_eq!(record(PERIOD, &user).unwrap().usage_hash, hash(1));
	});
}

#[test]
fn v2_then_v1_on_the_same_hour_never_debits_twice() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		deposit_credits(&user, 10_000);
		let lines = [(UsageKind::Compute, 1_000), (UsageKind::PublicIp, 237)];
		submit(PERIOD, vec![row(&user, &lines, hash(1))]);

		submit_v1(PERIOD, vec![(user.clone(), HOUR_COST, hash(1))]);
		assert_eq!(last_summary(), (0, 0, 1, 0, 0, 0));
		submit_v1(PERIOD, vec![(user.clone(), HOUR_COST + 1, hash(1))]);
		assert_eq!(last_summary(), (0, 0, 0, 1, 0, 0));

		assert_eq!(credits(&user), 10_000 - HOUR_COST);
	});
}

#[test]
fn arrears_carry_the_breakdown_consume_budget_and_settle_later() {
	new_test_ext().execute_with(|| {
		set_fee_cap(UsageKind::S3Plan, Some(plan_fee_cap()));
		let user = account(10);
		deposit_credits(&user, 100);
		let lines = [(UsageKind::Compute, HOUR_COST), (UsageKind::S3Plan, PLAN_FEE)];
		let total = HOUR_COST + PLAN_FEE;

		submit(PERIOD, vec![row(&user, &lines, hash(1))]);

		assert_eq!(credits(&user), 100);
		assert_eq!(ComputeArrears::<Runtime>::get(&user), total);
		assert_eq!(record(PERIOD, &user).unwrap().outcome, ComputeChargeOutcome::Arrears);
		assert!(marketplace_events().contains(&MarketplaceEvent::UsageChargeFailed {
			who: user.clone(),
			period: PERIOD,
			amount: total,
			required: total,
			available: 100,
			usage_hash: hash(1),
			breakdown: breakdown(&lines),
		}));
		// The debt is on the books, so the budget is spent.
		assert_eq!(FeeSpend::<Runtime>::get(UsageKind::S3Plan, &user), Some((NOW, PLAN_FEE)));
		assert_eq!(last_summary(), (0, 1, 0, 0, 0, total));

		deposit_credits(&user, total);
		assert_ok!(Marketplace::settle_compute_arrears(
			RuntimeOrigin::signed(biller()),
			vec![user.clone()],
		));
		assert_eq!(ComputeArrears::<Runtime>::get(&user), 0);
		assert_eq!(credits(&user), 100);
		assert!(marketplace_events()
			.contains(&MarketplaceEvent::ComputeArrearsSettled { who: user, amount: total }));
	});
}

// ── Caps ─────────────────────────────────────────────────────────────────

#[test]
fn a_fee_kind_without_caps_is_refused() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		deposit_credits(&user, 10_000);

		// Even a fee far below the hourly cap: absent caps means forbidden.
		submit(PERIOD, vec![row(&user, &[(UsageKind::S3Plan, 10)], hash(1))]);
		assert_eq!(refusals(), vec![(user.clone(), ComputeUsageRefusal::FeeCapExceeded)]);
		assert_eq!(record(PERIOD, &user), None);

		// A zero line of it is no charge, so nothing to forbid.
		submit(
			PERIOD,
			vec![row(&user, &[(UsageKind::Compute, HOUR_COST), (UsageKind::S3Plan, 0)], hash(1))],
		);
		assert_eq!(credits(&user), 10_000 - HOUR_COST);
	});
}

#[test]
fn removing_caps_forbids_the_kind_again_and_ordinary_kinds_can_be_made_fees() {
	new_test_ext().execute_with(|| {
		let (a, b) = (account(10), account(11));
		deposit_credits(&a, 100_000_000);
		deposit_credits(&b, 100_000_000);
		set_fee_cap(UsageKind::S3Plan, Some(plan_fee_cap()));
		set_fee_cap(UsageKind::S3Plan, None);
		assert_eq!(FeeCapConfig::<Runtime>::get(UsageKind::S3Plan), None);

		submit(PERIOD, vec![row(&a, &[(UsageKind::S3Plan, 10)], hash(1))]);
		assert_eq!(refusals(), vec![(a.clone(), ComputeUsageRefusal::FeeCapExceeded)]);

		// Backup is ordinary usage, held to the hourly cap...
		submit(PERIOD, vec![row(&b, &[(UsageKind::Backup, PER_ACCOUNT_CAP + 1)], hash(1))]);
		assert_eq!(refusals()[1], (b.clone(), ComputeUsageRefusal::AccountCapExceeded));

		// ...until root gives it caps of its own.
		set_fee_cap(UsageKind::Backup, Some(plan_fee_cap()));
		submit(PERIOD, vec![row(&b, &[(UsageKind::Backup, PER_ACCOUNT_CAP + 1)], hash(1))]);
		assert_eq!(credits(&b), 100_000_000 - PER_ACCOUNT_CAP - 1);
	});
}

#[test]
fn the_hourly_cap_applies_to_the_row_less_its_fees() {
	new_test_ext().execute_with(|| {
		set_fee_cap(UsageKind::S3Plan, Some(plan_fee_cap()));
		let (fits, over) = (account(10), account(11));
		deposit_credits(&fits, 100_000_000);
		deposit_credits(&over, 100_000_000);

		submit(
			PERIOD,
			vec![
				// Non-fee part exactly at the cap; total far above it.
				row(
					&fits,
					&[(UsageKind::Compute, PER_ACCOUNT_CAP), (UsageKind::S3Plan, PLAN_FEE)],
					hash(1),
				),
				row(
					&over,
					&[
						(UsageKind::Compute, PER_ACCOUNT_CAP),
						(UsageKind::Cdn, 1),
						(UsageKind::S3Plan, 1),
					],
					hash(2),
				),
			],
		);

		assert_eq!(credits(&fits), 100_000_000 - PER_ACCOUNT_CAP - PLAN_FEE);
		assert_eq!(refusals(), vec![(over.clone(), ComputeUsageRefusal::AccountCapExceeded)]);
		// A refused row spends no budget.
		assert_eq!(FeeSpend::<Runtime>::get(UsageKind::S3Plan, &over), None);
		assert_eq!(FeeSpend::<Runtime>::get(UsageKind::S3Plan, &fits), Some((NOW, PLAN_FEE)));
	});
}

#[test]
fn a_fee_above_per_debit_is_refused() {
	new_test_ext().execute_with(|| {
		set_fee_cap(UsageKind::S3Plan, Some(plan_fee_cap()));
		let user = account(10);
		deposit_credits(&user, 100_000_000);

		submit(PERIOD, vec![row(&user, &[(UsageKind::S3Plan, PLAN_FEE + 1)], hash(1))]);

		assert_eq!(refusals(), vec![(user.clone(), ComputeUsageRefusal::FeeCapExceeded)]);
		assert_eq!(credits(&user), 100_000_000);
		assert_eq!(FeeSpend::<Runtime>::get(UsageKind::S3Plan, &user), None);
	});
}

/// The budget window runs on the chain clock. Were it keyed on the submitted
/// period, the 72 h lag would let a stolen key walk rows forward from the
/// oldest billable hour into three fresh 24 h windows; on the chain clock
/// every row, backdated or not, lands in the window that is open now.
#[test]
fn the_fee_budget_window_runs_on_the_chain_clock() {
	new_test_ext().execute_with(|| {
		let window = 24;
		set_fee_cap(
			UsageKind::S3Plan,
			Some(FeeCap {
				per_debit: PLAN_FEE,
				per_call: PLAN_FEE * 10,
				budget: PLAN_FEE * 2,
				window_periods: window,
			}),
		);
		let user = account(10);
		deposit_credits(&user, 100_000_000);
		let fee = |h: u8| row(&user, &[(UsageKind::S3Plan, PLAN_FEE)], hash(h));

		// The oldest billable hour opens the window at the chain's now...
		submit(NOW - LAG, vec![fee(1)]);
		assert_eq!(FeeSpend::<Runtime>::get(UsageKind::S3Plan, &user), Some((NOW, PLAN_FEE)));
		// ...so an hour a whole window later is still inside it,
		submit(NOW - LAG + window, vec![fee(2)]);
		assert_eq!(FeeSpend::<Runtime>::get(UsageKind::S3Plan, &user), Some((NOW, PLAN_FEE * 2)));
		// and so is one two windows later: no fresh budget to be found.
		submit(NOW - LAG + 2 * window, vec![fee(3)]);
		assert_eq!(refusals(), vec![(user.clone(), ComputeUsageRefusal::FeeBudgetExceeded)]);
		assert_eq!(record(NOW - LAG + 2 * window, &user), None);

		// The last period of the window is still the window.
		set_current_period(NOW + window - 1);
		submit(NOW + window - 2, vec![fee(4)]);
		assert_eq!(refusals().len(), 2);

		// Once the window has run, the budget is fresh from now.
		set_current_period(NOW + window);
		submit(NOW + window - 1, vec![fee(5)]);
		assert_eq!(
			FeeSpend::<Runtime>::get(UsageKind::S3Plan, &user),
			Some((NOW + window, PLAN_FEE))
		);
		assert_eq!(credits(&user), 100_000_000 - PLAN_FEE * 3);
	});
}

#[test]
fn per_call_caps_bound_fees_per_kind_and_the_rest_separately() {
	new_test_ext().execute_with(|| {
		set_fee_cap(
			UsageKind::S3Plan,
			Some(FeeCap {
				per_debit: PLAN_FEE,
				per_call: PLAN_FEE,
				budget: PLAN_FEE * 10,
				window_periods: 720,
			}),
		);
		assert_ok!(Marketplace::set_compute_charge_caps(
			RuntimeOrigin::root(),
			PER_ACCOUNT_CAP,
			HOUR_COST * 2,
		));
		let users: Vec<AccountId> = (10..14).map(account).collect();
		for who in &users {
			deposit_credits(who, 100_000_000);
		}

		submit(
			PERIOD,
			vec![
				row(
					&users[0],
					&[(UsageKind::Compute, HOUR_COST), (UsageKind::S3Plan, PLAN_FEE)],
					hash(1),
				),
				// The S3Plan per-call cap is spent.
				row(&users[1], &[(UsageKind::S3Plan, 1)], hash(2)),
				// The fee above does not count against the non-fee call cap...
				row(&users[2], &[(UsageKind::Compute, HOUR_COST)], hash(3)),
				// ...which is now spent.
				row(&users[3], &[(UsageKind::Compute, 1)], hash(4)),
			],
		);

		assert_eq!(
			refusals(),
			vec![
				(users[1].clone(), ComputeUsageRefusal::FeeCapExceeded),
				(users[3].clone(), ComputeUsageRefusal::CallCapExceeded),
			]
		);
		assert_eq!(last_summary(), (2, 0, 0, 0, 2, HOUR_COST * 2 + PLAN_FEE));

		// Per call: the next call accepts the refused rows.
		submit(
			PERIOD,
			vec![
				row(&users[1], &[(UsageKind::S3Plan, 1)], hash(2)),
				row(&users[3], &[(UsageKind::Compute, 1)], hash(4)),
			],
		);
		assert_eq!(last_summary(), (2, 0, 0, 0, 0, 2));
	});
}

#[test]
fn a_duplicate_is_honoured_after_the_fee_caps_are_removed() {
	new_test_ext().execute_with(|| {
		set_fee_cap(UsageKind::S3Plan, Some(plan_fee_cap()));
		let user = account(10);
		deposit_credits(&user, 100_000_000);
		let r = row(&user, &[(UsageKind::S3Plan, PLAN_FEE)], hash(1));
		submit(PERIOD, vec![r.clone()]);

		set_fee_cap(UsageKind::S3Plan, None);
		submit(PERIOD, vec![r]);

		assert_eq!(last_summary(), (0, 0, 1, 0, 0, 0));
		assert_eq!(credits(&user), 100_000_000 - PLAN_FEE);
	});
}

// ── Per-account cap override ─────────────────────────────────────────────

fn set_account_cap(who: &AccountId, cap: Option<u128>) {
	assert_ok!(Marketplace::set_account_charge_cap(RuntimeOrigin::root(), who.clone(), cap));
}

#[test]
fn an_override_above_the_default_admits_rows_between_the_two() {
	new_test_ext().execute_with(|| {
		let (enterprise, ordinary) = (account(10), account(11));
		deposit_credits(&enterprise, 100_000_000);
		deposit_credits(&ordinary, 100_000_000);
		set_account_cap(&enterprise, Some(PER_ACCOUNT_CAP * 2));
		assert!(marketplace_events().contains(&MarketplaceEvent::AccountChargeCapUpdated {
			who: enterprise.clone(),
			cap: Some(PER_ACCOUNT_CAP * 2),
			by: None,
		}));

		let lines = [(UsageKind::Compute, PER_ACCOUNT_CAP + 1)];
		submit(PERIOD, vec![row(&enterprise, &lines, hash(1)), row(&ordinary, &lines, hash(2))]);

		assert_eq!(credits(&enterprise), 100_000_000 - PER_ACCOUNT_CAP - 1);
		assert_eq!(refusals(), vec![(ordinary, ComputeUsageRefusal::AccountCapExceeded)]);

		// The override is a cap too.
		submit(
			PERIOD - 1,
			vec![row(&enterprise, &[(UsageKind::Compute, PER_ACCOUNT_CAP * 2 + 1)], hash(3))],
		);
		assert_eq!(refusals()[1], (enterprise, ComputeUsageRefusal::AccountCapExceeded));
	});
}

#[test]
fn an_override_below_the_default_refuses_and_removing_it_restores_the_default() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		deposit_credits(&user, 100_000_000);
		set_account_cap(&user, Some(HOUR_COST - 1));

		submit(PERIOD, vec![row(&user, &[(UsageKind::Compute, HOUR_COST)], hash(1))]);
		assert_eq!(refusals(), vec![(user.clone(), ComputeUsageRefusal::AccountCapExceeded)]);
		assert_eq!(record(PERIOD, &user), None);

		set_account_cap(&user, None);
		assert_eq!(AccountChargeCapOverride::<Runtime>::get(&user), None);
		assert!(marketplace_events().contains(&MarketplaceEvent::AccountChargeCapUpdated {
			who: user.clone(),
			cap: None,
			by: None,
		}));

		// Back on the default: the refused row now passes, and the default
		// still binds above it.
		submit(PERIOD, vec![row(&user, &[(UsageKind::Compute, HOUR_COST)], hash(1))]);
		assert_eq!(credits(&user), 100_000_000 - HOUR_COST);
		submit(PERIOD - 1, vec![row(&user, &[(UsageKind::Compute, PER_ACCOUNT_CAP + 1)], hash(2))]);
		assert_eq!(refusals()[1], (user, ComputeUsageRefusal::AccountCapExceeded));
	});
}

/// The per-call cap bounds ordinary accounts together; an overridden account
/// is bounded only by its own cap and takes none of their budget.
#[test]
fn an_overridden_account_is_left_out_of_the_per_call_cap() {
	new_test_ext().execute_with(|| {
		assert_ok!(Marketplace::set_compute_charge_caps(
			RuntimeOrigin::root(),
			PER_ACCOUNT_CAP,
			HOUR_COST,
		));
		let (enterprise, a, b) = (account(10), account(11), account(12));
		for who in [&enterprise, &a, &b] {
			deposit_credits(who, 100_000_000);
		}
		set_account_cap(&enterprise, Some(PER_ACCOUNT_CAP * 5));

		submit(
			PERIOD,
			vec![
				// Far above the per-call cap on its own.
				row(&enterprise, &[(UsageKind::Compute, PER_ACCOUNT_CAP * 5)], hash(1)),
				// The whole per-call budget is still there for the others...
				row(&a, &[(UsageKind::Compute, HOUR_COST)], hash(2)),
				// ...and they share it.
				row(&b, &[(UsageKind::Compute, 1)], hash(3)),
			],
		);

		assert_eq!(credits(&enterprise), 100_000_000 - PER_ACCOUNT_CAP * 5);
		assert_eq!(credits(&a), 100_000_000 - HOUR_COST);
		assert_eq!(refusals(), vec![(b, ComputeUsageRefusal::CallCapExceeded)]);
	});
}

#[test]
fn an_override_does_not_lift_fee_caps() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		deposit_credits(&user, 100_000_000);
		set_account_cap(&user, Some(u128::MAX));

		submit(PERIOD, vec![row(&user, &[(UsageKind::S3Plan, PLAN_FEE)], hash(1))]);

		assert_eq!(refusals(), vec![(user, ComputeUsageRefusal::FeeCapExceeded)]);
	});
}

/// With no admins configured, only root sets an account cap.
#[test]
fn without_admins_only_root_sets_an_account_cap() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Marketplace::set_account_charge_cap(
				RuntimeOrigin::signed(biller()),
				account(10),
				Some(1),
			),
			Error::<Runtime>::NotChargeCapAdmin
		);
		assert_noop!(
			Marketplace::set_account_charge_cap(RuntimeOrigin::none(), account(10), Some(1)),
			DispatchError::BadOrigin
		);
		assert_eq!(AccountChargeCapOverride::<Runtime>::get(account(10)), None);
	});
}

// ── Charge cap admins ────────────────────────────────────────────────────

fn cap_admin() -> AccountId {
	account(3)
}

fn set_cap_as(
	origin: RuntimeOrigin,
	who: &AccountId,
	cap: Option<u128>,
) -> Result<(), DispatchError> {
	Marketplace::set_account_charge_cap(origin, who.clone(), cap)
}

/// Admins in place, limited to twice the default.
fn with_cap_admin() {
	assert_ok!(Marketplace::set_charge_cap_admins(RuntimeOrigin::root(), vec![cap_admin()]));
	assert_ok!(Marketplace::set_max_account_charge_cap_override(
		RuntimeOrigin::root(),
		PER_ACCOUNT_CAP * 2,
	));
}

#[test]
fn an_admin_sets_overrides_up_to_the_limit_and_root_beyond_it() {
	new_test_ext().execute_with(|| {
		with_cap_admin();
		let user = account(10);
		let admin = RuntimeOrigin::signed(cap_admin());

		assert_ok!(set_cap_as(admin.clone(), &user, Some(PER_ACCOUNT_CAP * 2)));
		assert_eq!(AccountChargeCapOverride::<Runtime>::get(&user), Some(PER_ACCOUNT_CAP * 2));
		assert!(marketplace_events().contains(&MarketplaceEvent::AccountChargeCapUpdated {
			who: user.clone(),
			cap: Some(PER_ACCOUNT_CAP * 2),
			by: Some(cap_admin()),
		}));

		assert_noop!(
			Marketplace::set_account_charge_cap(admin, user.clone(), Some(PER_ACCOUNT_CAP * 2 + 1)),
			Error::<Runtime>::CapOverrideAboveLimit
		);

		assert_ok!(set_cap_as(RuntimeOrigin::root(), &user, Some(PER_ACCOUNT_CAP * 10)));
		assert_eq!(AccountChargeCapOverride::<Runtime>::get(&user), Some(PER_ACCOUNT_CAP * 10));
		assert!(marketplace_events().contains(&MarketplaceEvent::AccountChargeCapUpdated {
			who: user,
			cap: Some(PER_ACCOUNT_CAP * 10),
			by: None,
		}));
	});
}

/// Until root sets a limit, an admin can only take caps down: to zero, or
/// back to the default by removing an override.
#[test]
fn with_no_limit_set_an_admin_can_only_remove_or_zero_an_override() {
	new_test_ext().execute_with(|| {
		assert_ok!(Marketplace::set_charge_cap_admins(RuntimeOrigin::root(), vec![cap_admin()]));
		let user = account(10);
		let admin = RuntimeOrigin::signed(cap_admin());
		assert_ok!(set_cap_as(RuntimeOrigin::root(), &user, Some(PER_ACCOUNT_CAP * 10)));

		assert_noop!(
			Marketplace::set_account_charge_cap(admin.clone(), user.clone(), Some(1)),
			Error::<Runtime>::CapOverrideAboveLimit
		);
		assert_ok!(set_cap_as(admin.clone(), &user, Some(0)));
		assert_eq!(AccountChargeCapOverride::<Runtime>::get(&user), Some(0));
		assert_ok!(set_cap_as(admin, &user, None));
		assert_eq!(AccountChargeCapOverride::<Runtime>::get(&user), None);
		assert!(marketplace_events().contains(&MarketplaceEvent::AccountChargeCapUpdated {
			who: user,
			cap: None,
			by: Some(cap_admin()),
		}));
	});
}

#[test]
fn an_admin_may_lower_an_account_below_the_default() {
	new_test_ext().execute_with(|| {
		with_cap_admin();
		let user = account(10);
		deposit_credits(&user, 100_000_000);
		assert_ok!(set_cap_as(RuntimeOrigin::signed(cap_admin()), &user, Some(HOUR_COST - 1)));

		submit(PERIOD, vec![row(&user, &[(UsageKind::Compute, HOUR_COST)], hash(1))]);
		assert_eq!(refusals(), vec![(user, ComputeUsageRefusal::AccountCapExceeded)]);
	});
}

#[test]
fn only_root_and_admins_set_account_caps() {
	new_test_ext().execute_with(|| {
		with_cap_admin();
		for outsider in [account(99), biller()] {
			assert_noop!(
				Marketplace::set_account_charge_cap(
					RuntimeOrigin::signed(outsider),
					account(10),
					Some(1),
				),
				Error::<Runtime>::NotChargeCapAdmin
			);
		}
		// Removing an admin revokes it.
		assert_ok!(Marketplace::set_charge_cap_admins(RuntimeOrigin::root(), vec![]));
		assert_noop!(
			Marketplace::set_account_charge_cap(
				RuntimeOrigin::signed(cap_admin()),
				account(10),
				None,
			),
			Error::<Runtime>::NotChargeCapAdmin
		);
	});
}

#[test]
fn admins_and_their_limit_are_set_by_root_alone() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Marketplace::set_charge_cap_admins(
				RuntimeOrigin::signed(cap_admin()),
				vec![cap_admin()]
			),
			DispatchError::BadOrigin
		);
		with_cap_admin();
		assert_noop!(
			Marketplace::set_max_account_charge_cap_override(RuntimeOrigin::signed(cap_admin()), 1),
			DispatchError::BadOrigin
		);
		assert!(marketplace_events()
			.contains(&MarketplaceEvent::ChargeCapAdminsUpdated { admins: vec![cap_admin()] }));
		assert!(marketplace_events().contains(
			&MarketplaceEvent::MaxAccountChargeCapOverrideUpdated { max: PER_ACCOUNT_CAP * 2 }
		));
		// An admin cannot touch the other caps either.
		assert_noop!(
			Marketplace::set_compute_charge_caps(RuntimeOrigin::signed(cap_admin()), 1, 1),
			DispatchError::BadOrigin
		);
		assert_noop!(
			Marketplace::set_fee_kind_caps(
				RuntimeOrigin::signed(cap_admin()),
				UsageKind::S3Plan,
				None
			),
			DispatchError::BadOrigin
		);

		let nine: Vec<AccountId> = (100..109).map(account).collect();
		assert_noop!(
			Marketplace::set_charge_cap_admins(RuntimeOrigin::root(), nine),
			Error::<Runtime>::TooManyChargeCapAdmins
		);
	});
}

/// A billing key that could raise its own victims' caps would turn the cap
/// back into whatever the key wants, so the two roles never share an account.
#[test]
fn a_billing_authority_is_never_a_charge_cap_admin() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Marketplace::set_charge_cap_admins(RuntimeOrigin::root(), vec![cap_admin(), biller()]),
			Error::<Runtime>::AdminIsBillingAuthority
		);
		assert_ok!(Marketplace::set_charge_cap_admins(RuntimeOrigin::root(), vec![cap_admin()]));
		assert_noop!(
			Marketplace::set_compute_billing_authorities(
				RuntimeOrigin::root(),
				vec![biller(), cap_admin()],
			),
			Error::<Runtime>::AdminIsBillingAuthority
		);
		assert_eq!(ChargeCapAdmins::<Runtime>::get().into_inner(), vec![cap_admin()]);
	});
}

// ── Root, origin and period ──────────────────────────────────────────────

#[test]
fn fee_caps_are_set_by_root_within_bounds() {
	new_test_ext().execute_with(|| {
		let cap = plan_fee_cap();
		assert_noop!(
			Marketplace::set_fee_kind_caps(
				RuntimeOrigin::signed(biller()),
				UsageKind::S3Plan,
				Some(cap)
			),
			DispatchError::BadOrigin
		);
		assert_noop!(
			Marketplace::set_fee_kind_caps(
				RuntimeOrigin::root(),
				UsageKind::S3Plan,
				Some(FeeCap { window_periods: 0, ..cap }),
			),
			Error::<Runtime>::InvalidFeeCap
		);

		set_fee_cap(UsageKind::S3Plan, Some(cap));
		assert!(marketplace_events().contains(&MarketplaceEvent::FeeKindCapsUpdated {
			kind: UsageKind::S3Plan,
			cap: Some(cap),
		}));

		let kinds = [UsageKind::Compute, UsageKind::PublicIp, UsageKind::Backup];
		assert_eq!(kinds.len() as u32 + 1, MAX_FEE_KINDS);
		for kind in kinds {
			set_fee_cap(kind, Some(cap));
		}
		assert_noop!(
			Marketplace::set_fee_kind_caps(RuntimeOrigin::root(), UsageKind::Cdn, Some(cap)),
			Error::<Runtime>::TooManyFeeKinds
		);
		// Updating a configured kind is not adding one.
		set_fee_cap(UsageKind::S3Plan, Some(FeeCap { budget: 1, ..cap }));
		set_fee_cap(UsageKind::Compute, None);
		set_fee_cap(UsageKind::Cdn, Some(cap));
	});
}

#[test]
fn only_an_authority_submits_and_only_billable_periods() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		deposit_credits(&user, 10_000);
		let r = || vec![row(&user, &[(UsageKind::Compute, HOUR_COST)], hash(1))];

		assert_eq!(
			submit_as(RuntimeOrigin::signed(account(99)), PERIOD, r()),
			Err(Error::<Runtime>::NotComputeBillingAuthority.into())
		);
		assert_eq!(submit_as(RuntimeOrigin::root(), PERIOD, r()), Err(DispatchError::BadOrigin));
		assert_eq!(
			submit_as(RuntimeOrigin::signed(biller()), NOW, r()),
			Err(Error::<Runtime>::ComputeBillingPeriodNotClosed.into())
		);
		assert_eq!(
			submit_as(RuntimeOrigin::signed(biller()), NOW - LAG - 1, r()),
			Err(Error::<Runtime>::ComputeBillingPeriodTooOld.into())
		);
		assert_eq!(credits(&user), 10_000);

		assert_ok!(submit_as(RuntimeOrigin::signed(biller()), NOW - LAG, r()));
		assert_eq!(credits(&user), 10_000 - HOUR_COST);
	});
}

// ── Weight ───────────────────────────────────────────────────────────────

#[test]
fn a_full_call_of_widest_rows_fits_one_extrinsic_and_is_feeless() {
	new_test_ext().execute_with(|| {
		let wide: UsageBreakdown = (0..MAX_USAGE_KINDS_PER_ROW)
			.map(|_| (UsageKind::Compute, 1))
			.collect::<Vec<_>>()
			.try_into()
			.unwrap();
		let info = |n: u8, lines: &UsageBreakdown| {
			let r: Vec<UsageRowV2<AccountId>> = (0..n)
				.map(|i| UsageRowV2 {
					who: account(i),
					total: 16,
					usage_hash: hash(i),
					breakdown: lines.clone(),
				})
				.collect();
			RuntimeCall::Marketplace(pallet_marketplace::Call::submit_usage_v2 {
				period: PERIOD,
				rows: rows(r),
			})
			.get_dispatch_info()
		};

		let full = info(250, &wide);
		assert_eq!(full.pays_fee, Pays::No);
		assert_eq!(full.class, DispatchClass::Normal);

		// Lines cost something on top of the v1 row.
		let v1 = RuntimeCall::Marketplace(pallet_marketplace::Call::submit_compute_usage {
			period: PERIOD,
			rows: (0..250u8).map(|i| (account(i), 16, hash(i))).collect(),
		})
		.get_dispatch_info()
		.weight;
		let narrow = info(250, &breakdown(&[(UsageKind::Compute, 16)])).weight;
		assert!(narrow.ref_time() > v1.ref_time());
		assert!(full.weight.ref_time() > narrow.ref_time());

		let max_extrinsic = <Runtime as frame_system::Config>::BlockWeights::get()
			.get(DispatchClass::Normal)
			.max_extrinsic
			.expect("normal extrinsics are bounded");
		assert!(
			full.weight.all_lte(max_extrinsic),
			"250 rows of 16 lines take {:?} of {max_extrinsic:?}",
			full.weight
		);
	});
}

/// The widest valid call that can exist today — every kind on every row, the
/// most fee kinds root may configure — executes and records every row.
#[test]
fn a_full_call_of_every_kind_with_the_most_fee_kinds_executes() {
	new_test_ext().execute_with(|| {
		let all = [
			UsageKind::Compute,
			UsageKind::PublicIp,
			UsageKind::Backup,
			UsageKind::ManagedFees,
			UsageKind::Cdn,
			UsageKind::Bandwidth,
			UsageKind::S3Storage,
			UsageKind::S3Overage,
			UsageKind::S3Plan,
		];
		let fee_kinds = &all[all.len() - MAX_FEE_KINDS as usize..];
		for kind in fee_kinds {
			set_fee_cap(
				*kind,
				Some(FeeCap { per_debit: 1, per_call: 250, budget: 1, window_periods: 1 }),
			);
		}
		let lines: Vec<(UsageKind, u128)> = all.iter().map(|kind| (*kind, 1)).collect();
		let r: Vec<UsageRowV2<AccountId>> =
			(0..250u8).map(|i| row(&account(i), &lines, hash(i))).collect();

		submit(PERIOD, r);

		// Nobody is funded, so every row lands in arrears — recorded all the
		// same, budgets spent.
		assert_eq!(last_summary(), (0, 250, 0, 0, 0, 250 * 9));
		for i in 0..250u8 {
			for kind in fee_kinds {
				assert_eq!(FeeSpend::<Runtime>::get(kind, account(i)), Some((NOW, 1)));
			}
		}
	});
}

#[test]
fn a_refused_origin_does_not_hold_the_full_block_weight() {
	new_test_ext().execute_with(|| {
		let r: Vec<UsageRowV2<AccountId>> = (0..250u8)
			.map(|i| row(&account(i), &[(UsageKind::Compute, HOUR_COST)], hash(i)))
			.collect();
		let call = RuntimeCall::Marketplace(pallet_marketplace::Call::submit_usage_v2 {
			period: PERIOD,
			rows: rows(r),
		});
		let declared = call.get_dispatch_info().weight;

		let err =
			sp_runtime::traits::Dispatchable::dispatch(call, RuntimeOrigin::signed(account(99)))
				.unwrap_err();

		let actual = err.post_info.actual_weight.expect("an early exit reports its weight");
		assert!(actual.ref_time() < declared.ref_time() / 10);
	});
}

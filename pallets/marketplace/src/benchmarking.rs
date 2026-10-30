//! Benchmarks for the date-to-date billing hook and the hourly compute billing
//! calls.
//!
//! `charge_account_due` is the unit the renewal drain meters itself in and the
//! unit `MaxSubscriptionChargesPerRun` counts, so it decides whether
//! `on_initialize` stays inside its budget. The other hook entries in
//! `WeightInfo` are single storage operations whose conservative estimates are
//! already within a rounding error of any measurement.
//!
//! `submit_compute_usage` and `settle_compute_arrears` are feeless, so their
//! declared weight is the only thing that stops a full call from being
//! under-counted against the block. They are measured per row, each row posed
//! at its most expensive path.
//!
//! `charge_account_due` is measured with `#[block]` rather than `#[extrinsic_call]` because
//! `charge_account_due` is not an extrinsic and should not become one to be
//! measurable — a benchmark-only call in the dispatch enum is a permanent piece
//! of surface added for a temporary purpose.
//!
//! The setup writes storage directly rather than going through `purchase_plan`.
//! Two reasons: the purchase path would put its own cost inside the measured
//! block, and seeding lets the account be posed in its *worst* case — the
//! maximum subscriptions it may hold, all due on the same day, split across
//! both the storage and compute charging sides. The meter takes this figure as
//! the cost of the next account before knowing anything about it, so it has to
//! be the worst case rather than the typical one.
//!
//! Run:
//!
//! ```text
//! cargo build --release --features runtime-benchmarks
//! ./target/release/hippius benchmark pallet \
//!     --chain benchmark \
//!     --pallet pallet_marketplace \
//!     --extrinsic 'charge_account_due,hourly_charge,submit_compute_usage,settle_compute_arrears,buy_credits,submit_usage_v2,set_fee_kind_caps' \
//!     --steps 50 --repeat 20
//! ```

#![cfg(feature = "runtime-benchmarks")]

use super::*;
use crate::pallet::Pallet as Marketplace;
use frame_benchmarking::v2::*;
use frame_support::pallet_prelude::{BoundedVec, Get, PhantomData};
use frame_support::traits::Currency;
use frame_system::pallet_prelude::BlockNumberFor;
use frame_system::RawOrigin;
use pallet_credits::Pallet as CreditsPallet;
use sp_runtime::traits::{Hash, SaturatedConversion, Saturating};
use sp_std::vec;
use sp_std::vec::Vec;

const SEED: u32 = 0;

/// Credits generous enough that no charge in the measured block fails for want
/// of funds — a failed charge takes the deactivation path, which is a
/// *different* and cheaper shape than the renewal we are pricing.
const FUNDING: u128 = 1_000_000_000_000;

/// Plan price small relative to `FUNDING`, so the worst case stays the
/// full-charge path however many subscriptions the account holds.
const PLAN_PRICE: u128 = 1_000;

/// `max_catchup_months` in `charge_account_due`. Kept here as a named constant
/// because the benchmark's whole shape depends on it: the clock is placed far
/// enough past the due date that the catch-up loop runs its full count, and if
/// the cap ever changes this figure has to move with it or the measurement
/// silently starts pricing fewer cycles than the drain can actually run.
const MAX_CATCHUP_MONTHS: u32 = 3;

/// Unix day the benchmark's clock is placed on.
///
/// Subscriptions are seeded due on day 0, whose day-of-month is the 1st, so the
/// catch-up cycles land on days 31, 59 and 90. Day 120 is past all three, which
/// makes every iteration of the catch-up loop do real charging work and lets
/// `MAX_CATCHUP_MONTHS` — not the calendar — be what stops it.
const CLOCK_UNIX_DAY: u64 = 120;

fn plan_of<T: Config>(index: u32, is_storage_plan: bool) -> Plan<T::Hash> {
	let name: Vec<u8> = vec![b'p', index as u8];
	let id = T::Hashing::hash_of(&name);
	let plan = Plan {
		id,
		plan_name: name,
		plan_description: vec![b'{', b'}'],
		plan_technical_description: vec![b'{', b'}'],
		is_suspended: false,
		price: PLAN_PRICE,
		is_storage_plan,
		is_s3_plan: false,
		storage_limit: if is_storage_plan { Some(1_000_000) } else { None },
	};
	Plans::<T>::insert(id, plan.clone());
	plan
}

/// Fund `who` through `UserBatches` in the worst shape a charge can walk, and
/// set up a funded, whitelisted referrer behind them.
///
/// Fund through a *batch*, not `do_mint` alone. `consume_credits` spends from
/// `UserBatches` and requires the whole amount to come from there — minting
/// `FreeCredits` with no batch behind it leaves the charge failing with
/// `InsufficientFreeCredits`, which sends the charge down its deactivation /
/// skip path. That path is cheaper than a renewal, so a benchmark seeded that
/// way would quietly price the wrong thing and under-fund the meter. The
/// assertion at the end of each benchmark is what catches it.
///
/// Worst case for the debit's batch walk: a full `MaxBatchesPerBuyer` list of
/// already-spent batches ahead of one funding batch. A charge skips every
/// spent batch (0 credits), pays from the funder at the end, and rewrites the
/// list dropping all the spent ids — the maximum-length walk and prune a
/// single charge can drive.
///
/// The funder is not merely funded: it matured while frozen with alpha still
/// pending and partly unbacked, exactly the shape `compute_debtor` poses for
/// the compute-row benchmarks. The two billing charges walk the same
/// `consume_credits` debit, so each must also price the unfreeze branch — the
/// one that distributes the pending alpha and releases the proportional
/// remainder, touching `AlphaBalances`, the `UnbackedBatchAlpha` marker (via
/// `take_backed_portion`) and `TotalUndistributedBacking`. A funder with no
/// alpha, unfrozen and fully backed, skips all of that and under-prices the
/// charge.
///
/// The referrer is set up so the charge also accrues (and, on the monthly
/// path, pays through the bank) referral commission — otherwise the measured
/// weight misses the work a referred account drives. The bank is funded,
/// whitelisted and switched on so those payments actually land.
fn fund_worst_case<T: Config>(who: &T::AccountId) {
	let cap = T::MaxBatchesPerBuyer::get().max(1);
	let zero = BlockNumberFor::<T>::from(0u32);
	for _ in 0..cap.saturating_sub(1) {
		push_batch::<T>(
			who,
			Batch {
				owner: who.clone(),
				credit_amount: FUNDING,
				alpha_amount: FUNDING,
				remaining_credits: 0,
				remaining_alpha: 0,
				pending_alpha: 0,
				is_frozen: false,
				release_time: zero,
			},
		);
	}
	let funded = push_batch::<T>(
		who,
		Batch {
			owner: who.clone(),
			credit_amount: FUNDING,
			alpha_amount: FUNDING,
			remaining_credits: FUNDING,
			remaining_alpha: FUNDING,
			pending_alpha: FUNDING / 2,
			is_frozen: true,
			release_time: zero,
		},
	);
	UnbackedBatchAlpha::<T>::insert(funded, FUNDING / 4);
	TotalUndistributedBacking::<T>::mutate(|t| *t = t.saturating_add(FUNDING.saturating_mul(2)));
	pallet_credits::AlphaBalances::<T>::mutate(who, |a| {
		*a = a.saturating_add(FUNDING.saturating_mul(2))
	});
	let _ = CreditsPallet::<T>::do_mint(who.clone(), FUNDING, None);

	let referrer: T::AccountId = account("referrer", 0, SEED);
	let code: Vec<u8> = b"BENCHREF".to_vec();
	pallet_credits::ReferralCodes::<T>::insert(code.clone(), referrer.clone());
	pallet_credits::ReferredUsers::<T>::insert(who, code);
	ReferralCommissionRateBps::<T>::put(500);
	let bank = pallet_hippocampus::Pallet::<T>::account_id();
	let _ = <T as Config>::Currency::deposit_creating(&bank, FUNDING.saturated_into());
	let _ = <T as Config>::Currency::deposit_creating(&referrer, FUNDING.saturated_into());
	pallet_hippocampus::WhitelistedRequesters::<T>::insert(Marketplace::<T>::account_id(), ());
	pallet_hippocampus::DistributionEnabled::<T>::put(true);
}

/// An account holding `count` active subscriptions, every one of them due, and
/// funded well enough to pay for all of them.
///
/// The first is a Drive plan and the rest are compute, so the charge exercises
/// both sides of the storage/compute split rather than one branch twice.
fn due_account<T: Config>(count: u32) -> T::AccountId {
	let who: T::AccountId = account("subscriber", 0, SEED);
	fund_worst_case::<T>(&who);

	let subs: Vec<UserPlanSubscription<T>> = (0..count)
		.map(|i| UserPlanSubscription {
			id: i,
			owner: who.clone(),
			package: plan_of::<T>(i, i == 0),
			cdn_location_id: None,
			active: true,
			last_charged_at: BlockNumberFor::<T>::from(0u32),
			selected_image_name: None,
			// Due on day 0 while the benchmark puts the clock on
			// `CLOCK_UNIX_DAY`, so each of these is not merely due but months
			// in arrears — which is what makes the catch-up loop run its full
			// `MAX_CATCHUP_MONTHS` inside the measured block.
			next_charge_unix_day: Some(0),
			paid_per_month: PLAN_PRICE,
			_phantom: PhantomData,
		})
		.collect();

	UserAllSubscriptionPlans::<T>::insert(&who, subs);
	who
}

/// Hour the compute billing benchmarks bill. The clock is placed one hour
/// after it, so the period is closed and within `MaxComputeBillingLag`.
const COMPUTE_PERIOD: u64 = 500_000;

/// Period amount each benchmarked row carries, and the arrears each account
/// already owes. Both far below `FUNDING`, so every debit succeeds.
const COMPUTE_AMOUNT: u128 = 1_000;
const COMPUTE_ARREARS: u128 = 500;

/// Amount of each line of a benchmarked `submit_usage_v2` row.
const USAGE_LINE: u128 = 100;

/// Every `UsageKind`, in index order.
const ALL_USAGE_KINDS: [UsageKind; 12] = [
	UsageKind::Compute,
	UsageKind::PublicIp,
	UsageKind::Backup,
	UsageKind::ManagedFees,
	UsageKind::Cdn,
	UsageKind::Bandwidth,
	UsageKind::S3Storage,
	UsageKind::S3Overage,
	UsageKind::S3Plan,
	UsageKind::DriveStorage,
	UsageKind::DriveOverage,
	UsageKind::DrivePlan,
];

/// Point the chain clock inside the period after `COMPUTE_PERIOD`, so that
/// period is the most recent closed one.
fn close_compute_period<T: Config>() {
	let now_ms: T::Moment = COMPUTE_PERIOD
		.saturating_add(1)
		.saturating_mul(COMPUTE_BILLING_PERIOD_MS)
		.saturated_into();
	pallet_timestamp::Now::<T>::put(now_ms);
}

/// Deposit batches each benchmarked account holds.
///
/// `consume_credits` walks an account's batches oldest first, exhausted ones
/// included, and nothing prunes them — so the per-row cost grows with the
/// account's deposit history. At the time of writing the busiest mainnet
/// account holds 12 batches (p99 8, of 779 accounts); this is set above that,
/// and has to be raised, or the walk bounded, before accounts outgrow it.
pub const COMPUTE_BENCH_BATCHES: u32 = 16;

/// Registers the caller as the last of a full authority list, so the
/// membership check scans all of it, and lifts the caps so no benchmarked row
/// is refused.
fn compute_billing_authority<T: Config>() -> T::AccountId {
	let caller: T::AccountId = whitelisted_caller();
	let mut authorities: Vec<T::AccountId> = (1..MAX_COMPUTE_BILLING_AUTHORITIES)
		.map(|i| account("authority", i, SEED))
		.collect();
	authorities.push(caller.clone());
	ComputeBillingAuthorities::<T>::put(authorities);
	MaxComputeChargePerAccountPerPeriod::<T>::put(u128::MAX);
	MaxComputeChargePerCall::<T>::put(u128::MAX);
	caller
}

fn push_batch<T: Config>(who: &T::AccountId, batch: Batch<T::AccountId, BlockNumberFor<T>>) -> u64 {
	let batch_id = NextBatchId::<T>::get();
	Batches::<T>::insert(batch_id, batch);
	UserBatches::<T>::append(who, batch_id);
	NextBatchId::<T>::put(batch_id.saturating_add(1));
	batch_id
}

/// Account `index`, posed at the most expensive path a compute row can take:
///
/// - `COMPUTE_BENCH_BATCHES - 1` spent batches ahead of the funded one, each
///   read by the walk and then dropped from `UserBatches` in one rewrite. A
///   list in this shape only survives from before debits pruned their own,
///   until the idle cleanup reaches it, but it is still the longest walk;
/// - a funded batch that matured while frozen with alpha still pending and
///   partly unbacked, so the debit takes the unfreeze branch and touches the
///   alpha balance, the unbacked marker and the backing tally twice each;
/// - arrears the charge folds in and clears;
/// - a referrer, so the charge also accrues a commission.
///
/// The cheaper paths — a duplicate, a conflict, a refusal, or a failed debit
/// that only grows the arrears — touch a subset of what this one does.
fn compute_debtor<T: Config>(index: u32) -> T::AccountId {
	let who: T::AccountId = account("compute", index, SEED);
	let zero = BlockNumberFor::<T>::from(0u32);

	for _ in 1..COMPUTE_BENCH_BATCHES {
		push_batch::<T>(
			&who,
			Batch {
				owner: who.clone(),
				credit_amount: FUNDING,
				alpha_amount: FUNDING,
				remaining_credits: 0,
				remaining_alpha: 0,
				pending_alpha: 0,
				is_frozen: false,
				release_time: zero,
			},
		);
	}
	let funded = push_batch::<T>(
		&who,
		Batch {
			owner: who.clone(),
			credit_amount: FUNDING,
			alpha_amount: FUNDING,
			remaining_credits: FUNDING,
			remaining_alpha: FUNDING,
			pending_alpha: FUNDING / 2,
			is_frozen: true,
			release_time: zero,
		},
	);
	UnbackedBatchAlpha::<T>::insert(funded, FUNDING / 4);
	TotalUndistributedBacking::<T>::mutate(|t| *t = t.saturating_add(FUNDING.saturating_mul(2)));
	pallet_credits::AlphaBalances::<T>::mutate(&who, |a| {
		*a = a.saturating_add(FUNDING.saturating_mul(2))
	});
	let _ = CreditsPallet::<T>::do_mint(who.clone(), FUNDING, None);

	ComputeArrears::<T>::insert(&who, COMPUTE_ARREARS);

	let referrer: T::AccountId = account("referrer", index, SEED);
	let code: Vec<u8> = [b"ref".as_slice(), &index.to_le_bytes()].concat();
	pallet_credits::ReferralCodes::<T>::insert(&code, referrer);
	pallet_credits::ReferredUsers::<T>::insert(&who, code);

	who
}

#[benchmarks]
mod benchmarks {
	use super::*;

	/// One account's renewal, posed at the worst case the drain can meet:
	/// `MaxActiveSubscriptions` subscriptions, all due, all payable, and far
	/// enough in arrears to run the catch-up loop to its cap.
	///
	/// Payable is the worst case and not a convenience. A charge that succeeds
	/// takes credits, records a transaction, advances the due date and re-files
	/// the index entry; one that fails deactivates and refunds instead, which
	/// touches less. Pricing the cheaper path would under-fund the meter.
	///
	/// The arrears matter for the same reason and were missing at first. With
	/// the clock left at genesis the first cycle pushes the due date a month
	/// out, the catch-up loop's `today >= due` test fails immediately, and the
	/// measurement covers exactly one cycle — while the drain charges that flat
	/// figure for an account the loop may run `max_catchup_months` times. The
	/// gap only opens after downtime, which is precisely when the drain is
	/// busiest and the meter matters most, so the benchmark has to pose it.
	#[benchmark]
	fn charge_account_due() {
		let subs = T::MaxActiveSubscriptions::get().max(1);
		let who = due_account::<T>(subs);
		let block = BlockNumberFor::<T>::from(1u32);

		// Past every catch-up anniversary, so the loop is bounded by its own
		// cap rather than by running out of due cycles.
		let now_ms: T::Moment = (CLOCK_UNIX_DAY.saturating_mul(86_400_000)).saturated_into();
		pallet_timestamp::Now::<T>::put(now_ms);

		let before = CreditsPallet::<T>::get_free_credits(&who);

		#[block]
		{
			Marketplace::<T>::charge_account_due(&who, block);
		}

		// Every subscription was charged for every catch-up cycle. A weaker
		// "some credits moved" assertion is what let the single-cycle version
		// look correct: it passes just as happily when the loop breaks after
		// one pass, which is the whole failure being guarded against here.
		let spent = before.saturating_sub(CreditsPallet::<T>::get_free_credits(&who));
		let expected = PLAN_PRICE
			.saturating_mul(subs as u128)
			.saturating_mul(MAX_CATCHUP_MONTHS as u128);
		assert_eq!(spent, expected);
	}

	/// The per-user charge the hourly storage sweep applies once a user is past
	/// the nothing-to-bill gate: the per-GiB debit (`consume_credits`), the
	/// referral accrual, the transaction record and the last-charged marker.
	///
	/// Measured with `#[block]`, and for the same reason as `charge_account_due`:
	/// the charge is not an extrinsic, and `handle_hourly_storage_charging`
	/// reserves this unit per billable user before touching them, so it has to
	/// be the worst case. That worst case is the debit's batch walk — a full
	/// `MaxBatchesPerBuyer` spent-batch list ahead of one funding batch the
	/// charge drains and prunes — together with a referred user, so the
	/// commission accrual writes. The price and credit reads the sweep does
	/// before this point are counted by `hourly_probe` and the `past_gate`
	/// reserve in the loop, not here.
	///
	/// The sequence mirrors the sweep's charge branch exactly, calling the same
	/// pallet methods in the same order so the measurement tracks any change to
	/// what a charge does.
	#[benchmark]
	fn hourly_charge() {
		let who: T::AccountId = account("hourly", 0, SEED);
		fund_worst_case::<T>(&who);

		// Drain the funding batch exactly, so the debit walks and prunes the
		// whole spent list and the funder with it — the longest walk a single
		// charge can drive. A smaller amount would leave the funder in the list
		// and price a shorter prune.
		let charge_amount = FUNDING;
		let marker = BlockNumberFor::<T>::from(1_000u32);
		let referrer: T::AccountId = account("referrer", 0, SEED);

		#[block]
		{
			let _ = Marketplace::<T>::consume_credits(who.clone(), charge_amount);
			Marketplace::<T>::accrue_hourly_referral_commission(&who, charge_amount);
			let _ = Marketplace::<T>::record_credits_transaction(
				&who,
				NativeTransactionType::Subscription,
				charge_amount,
			);
			let _ = Marketplace::<T>::advance_storage_last_charged_at(&who, marker);
		}

		// The debit drained the funder and rewrote the list dropping every
		// spent id. A charge that fell through to the skip path (wrong funding
		// shape) would leave the credits untouched, and this catches it — the
		// same guard `charge_account_due` relies on.
		assert_eq!(CreditsPallet::<T>::get_free_credits(&who), 0);
		assert!(AccruedReferralCommission::<T>::get(&referrer) > 0);
		assert_eq!(StorageLastChargedAt::<T>::get(&who), marker);
	}

	/// `n` rows, each charged in full together with arrears it already owed,
	/// with a referral commission accrued on the lot.
	#[benchmark]
	fn submit_compute_usage(n: Linear<1, { T::MaxComputeUsageRowsPerCall::get() }>) {
		let caller = compute_billing_authority::<T>();
		close_compute_period::<T>();
		ReferralCommissionRateBps::<T>::put(500);
		let rows: Vec<(T::AccountId, u128, sp_core::H256)> = (0..n)
			.map(|i| (compute_debtor::<T>(i), COMPUTE_AMOUNT, sp_core::H256::repeat_byte(i as u8)))
			.collect();
		let debtors: Vec<T::AccountId> = rows.iter().map(|(who, _, _)| who.clone()).collect();

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), COMPUTE_PERIOD, rows);

		// Every row took the charged path, arrears included, and the debit
		// went through the unfreeze branch. A row that fell into arrears
		// instead would be cheaper, and a benchmark that let that happen would
		// price the wrong thing without failing.
		for who in debtors {
			let funded = *UserBatches::<T>::get(&who).unwrap_or_default().last().unwrap();
			assert!(!Batches::<T>::get(funded).unwrap().is_frozen);
			assert_eq!(ComputeArrears::<T>::get(&who), 0);
			assert_eq!(
				ComputeUsageCharged::<T>::get(COMPUTE_PERIOD, &who).map(|r| r.outcome),
				Some(ComputeChargeOutcome::Charged)
			);
			assert_eq!(
				CreditsPallet::<T>::get_free_credits(&who),
				FUNDING - COMPUTE_AMOUNT - COMPUTE_ARREARS
			);
		}
	}

	/// `n` accounts, each paying off its arrears in full.
	#[benchmark]
	fn settle_compute_arrears(n: Linear<1, { T::MaxComputeUsageRowsPerCall::get() }>) {
		let caller = compute_billing_authority::<T>();
		ReferralCommissionRateBps::<T>::put(500);
		let accounts: Vec<T::AccountId> = (0..n).map(compute_debtor::<T>).collect();
		let debtors = accounts.clone();

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), accounts);

		for who in debtors {
			assert_eq!(ComputeArrears::<T>::get(&who), 0);
			assert_eq!(CreditsPallet::<T>::get_free_credits(&who), FUNDING - COMPUTE_ARREARS);
		}
	}

	/// `n` rows of `k` breakdown lines each, every row charged in full with
	/// arrears like `submit_compute_usage`'s, and as many lines as possible
	/// fee kinds with a budget to check and spend to write.
	///
	/// `k` stops at the nine kinds that exist: lines must be distinct kinds.
	/// The weight function extrapolates it to `MAX_USAGE_KINDS_PER_ROW`.
	#[benchmark]
	fn submit_usage_v2(n: Linear<1, { T::MaxComputeUsageRowsPerCall::get() }>, k: Linear<1, 9>) {
		let caller = compute_billing_authority::<T>();
		close_compute_period::<T>();
		ReferralCommissionRateBps::<T>::put(500);
		let generous = FeeCap {
			per_debit: u128::MAX,
			per_call: u128::MAX,
			budget: u128::MAX,
			window_periods: 1_000,
		};
		for kind in &ALL_USAGE_KINDS[ALL_USAGE_KINDS.len() - MAX_FEE_KINDS as usize..] {
			FeeCapConfig::<T>::insert(kind, generous);
		}
		// The last `k` kinds, so the fee kinds are always among them.
		let kinds = &ALL_USAGE_KINDS[ALL_USAGE_KINDS.len() - k as usize..];
		let breakdown: UsageBreakdown = kinds
			.iter()
			.map(|kind| (*kind, USAGE_LINE))
			.collect::<Vec<_>>()
			.try_into()
			.unwrap();
		let total = USAGE_LINE.saturating_mul(k as u128);
		let rows: Vec<UsageRowV2<T::AccountId>> = (0..n)
			.map(|i| UsageRowV2 {
				who: compute_debtor::<T>(i),
				total,
				usage_hash: sp_core::H256::repeat_byte(i as u8),
				breakdown: breakdown.clone(),
			})
			.collect();
		let debtors: Vec<T::AccountId> = rows.iter().map(|row| row.who.clone()).collect();
		let rows: BoundedVec<_, T::MaxComputeUsageRowsPerCall> = rows.try_into().unwrap();

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), COMPUTE_PERIOD, rows);

		for who in debtors {
			assert_eq!(
				ComputeUsageCharged::<T>::get(COMPUTE_PERIOD, &who).map(|r| r.outcome),
				Some(ComputeChargeOutcome::Charged)
			);
			assert_eq!(
				CreditsPallet::<T>::get_free_credits(&who),
				FUNDING - total - COMPUTE_ARREARS
			);
		}
	}

	/// Adding a kind to a `FeeCapConfig` one short of `MAX_FEE_KINDS`, so the
	/// count runs over every existing entry.
	#[benchmark]
	fn set_fee_kind_caps() {
		let cap = FeeCap { per_debit: 1, per_call: 1, budget: 1, window_periods: 1 };
		for kind in &ALL_USAGE_KINDS[..MAX_FEE_KINDS as usize - 1] {
			FeeCapConfig::<T>::insert(kind, cap);
		}
		let kind = ALL_USAGE_KINDS[ALL_USAGE_KINDS.len() - 1];

		#[extrinsic_call]
		_(RawOrigin::Root, kind, Some(cap));

		assert_eq!(FeeCapConfig::<T>::get(kind), Some(cap));
	}

	/// A purchase with a referral code by an account with no client IP, so
	/// the mint checks and records the referral and probes the IP pool.
	#[benchmark]
	fn buy_credits() {
		let buyer: T::AccountId = whitelisted_caller();
		let alpha: u128 = 1_000_000_000_000_000_000;
		let funded: pallet_hippocampus::BalanceOf<T> = alpha.saturating_mul(10).saturated_into();
		let _ = <T as pallet_hippocampus::Config>::Currency::make_free_balance_be(&buyer, funded);
		BuyCreditsEnabled::<T>::put(true);
		pallet_credits::AlphaPrice::<T>::put(2_000_000_000_000_000_000u128);
		pallet_credits::AlphaPriceUpdatedAt::<T>::put(frame_system::Pallet::<T>::block_number());
		let referrer: T::AccountId = account("referrer", 0, SEED);
		let code: Vec<u8> = b"HIPPIUSBENCH".to_vec();
		pallet_credits::ReferralCodes::<T>::insert(&code, referrer);

		#[extrinsic_call]
		_(RawOrigin::Signed(buyer.clone()), alpha, 0, Some(code));

		assert_eq!(CreditsPallet::<T>::get_free_credits(&buyer), 2_000_000_000_000_000_000);
	}
}

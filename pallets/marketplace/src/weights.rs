//! Weights for the date-to-date billing hook.
//!
//! Scope is deliberately narrow: these are the units the renewal drain and the
//! due-index backfill *meter themselves in*, not a weight table for the
//! pallet's extrinsics. Those still carry hand-written literals and `Pays::No`;
//! pricing them is a separate job with a product decision attached (the
//! gateway relays on users' behalf, so users hold no tokens).
//!
//! What makes these load-bearing rather than decorative: `on_initialize` is not
//! rejected for being overweight. The block is simply produced heavier, so the
//! only thing that can hold the hook to a budget is the hook itself. It meters
//! against these numbers and stops when the next unit would not fit — which
//! means a wrong number here costs throughput (accounts wait for the next
//! tick), never a block that overruns.
//!
//! `charge_account_due` and `hourly_charge` are measured — the two charges the
//! billing hooks reserve per account, one on the monthly drain and one on the
//! hourly sweep. Re-measure them whenever a charge path gains or loses storage
//! access:
//!
//! ```text
//! cargo build --release --bin hippius --features hippius/runtime-benchmarks
//! ./target/release/hippius benchmark pallet \
//!     --chain benchmark \
//!     --pallet pallet_marketplace \
//!     --extrinsic 'charge_account_due,hourly_charge' \
//!     --steps 50 --repeat 20
//! ```
//!
//! The remaining single-storage-operation units (`hourly_probe`,
//! `backfill_account`, `drain_overhead`, `day_probe`) stay as estimates.
//!
//! The two feeless compute billing calls, `submit_compute_usage` and
//! `settle_compute_arrears`, are the exception to "not a weight table for the
//! extrinsics": a feeless call's declared weight is the only thing that keeps a
//! full one from being under-counted against the block. They are priced per
//! row off `compute_row`, which is still an estimate — see there.

#![allow(unused_parens)]

use frame_support::traits::Get;
use frame_support::weights::{constants::RocksDbWeight, Weight};
use sp_std::marker::PhantomData;

/// Weight functions for the parts of `pallet-marketplace` that run inside
/// `on_initialize` and therefore have to bound themselves.
pub trait WeightInfo {
	/// Charging one account's due subscriptions, including re-reading the
	/// account, taking credits, advancing the due dates and re-filing the
	/// index.
	///
	/// This is the unit both `MaxSubscriptionChargesPerRun` and the drain's
	/// weight meter count in, because it is the unit the work actually comes
	/// in: an account is charged whole or not at all, so that its subscriptions
	/// stay in deterministic id order.
	fn charge_account_due() -> Weight;

	/// Filing one account into the due-day index during the one-time backfill.
	fn backfill_account() -> Weight;

	/// Fixed cost of a drain tick regardless of how many accounts it reaches:
	/// reading the cursor and the backfill flag, and writing the cursor back.
	fn drain_overhead() -> Weight;

	/// Probing one day whose prefix is already empty, while the cursor walks
	/// forward to today.
	fn day_probe() -> Weight;

	/// Visiting one user in the hourly sweep and finding they owe nothing —
	/// covered by a plan, storing nothing, or not yet an hour since their last
	/// charge. Most users take this path on most ticks, so it is the cost that
	/// dominates the sweep.
	fn hourly_probe() -> Weight;

	/// The *additional* cost when that user does owe something: the per-GiB
	/// charge, the transaction record, the referral accrual and the
	/// last-charged marker.
	fn hourly_charge() -> Weight;

	/// Looking at one deposit batch in the alpha-release sweep and finding
	/// nothing to do — not frozen, nothing pending, or not yet matured. Almost
	/// every batch takes this path, and the map is never pruned on spend, so
	/// this is the cost that grows with the chain's whole deposit history.
	fn alpha_release_probe() -> Weight;

	/// The *additional* cost of actually releasing one matured batch. When the
	/// release leaves the batch spent, the owner's batch list (`n` ids, bounded
	/// by `MaxBatchesPerBuyer`) is rewritten whole, so `n` sizes that proof.
	fn alpha_release(n: u32) -> Weight;

	/// `submit_compute_usage` carrying `n` rows, every one of them posed at
	/// the most expensive path a row can take.
	fn submit_compute_usage(n: u32) -> Weight;

	/// `settle_compute_arrears` over `n` accounts, every one of them owing
	/// arrears it can now pay.
	fn settle_compute_arrears(n: u32) -> Weight;

	/// `buy_credits` with a referral code, by an account that has no client
	/// IP yet, so the mint also probes the IP pool.
	fn buy_credits() -> Weight;

	/// `submit_usage_v2` carrying `n` rows of up to `k` breakdown lines each,
	/// every row at the most expensive path and every line a fee kind as far
	/// as `MAX_FEE_KINDS` allows.
	fn submit_usage_v2(n: u32, k: u32) -> Weight;

	/// `set_fee_kind_caps` adding a kind to a full-but-one `FeeCapConfig`.
	fn set_fee_kind_caps() -> Weight;
}

/// Weights for `pallet-marketplace` using the Substrate node's RocksDB weights.
///
/// `charge_account_due` and `hourly_charge` are **measured**; the remaining
/// single-storage-operation units stay conservative estimates, where any
/// measurement would sit inside the rounding.
pub struct SubstrateWeight<T>(PhantomData<T>);

impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
	/// Measured (worst case: full MaxBatchesPerBuyer spent-batch list + referred
	/// account paying through the bank, with the funder matured while frozen,
	/// alpha pending and partly unbacked, so the debit takes the unfreeze branch
	/// — `AlphaBalances`, `UnbackedBatchAlpha`, `TotalUndistributedBacking`).
	/// 2026-10-27, steps 50 / repeat 20, on a dedicated Hippius benchmark VM
	/// (AMD EPYC 7543 32-Core Processor).
	fn charge_account_due() -> Weight {
		Weight::from_parts(790_121_000, 4436)
			.saturating_add(RocksDbWeight::get().reads(37))
			.saturating_add(RocksDbWeight::get().writes(25))
	}

	/// Read the account's subscriptions once, then one index write per
	/// subscription it holds. Bounded by `MaxActiveSubscriptions`, which is 5.
	fn backfill_account() -> Weight {
		Weight::from_parts(0, 0)
			.saturating_add(RocksDbWeight::get().reads(1))
			.saturating_add(RocksDbWeight::get().writes(5))
	}

	/// `DueDayCursor` and `BackfillDone` in, `DueDayCursor` out.
	fn drain_overhead() -> Weight {
		Weight::from_parts(0, 0)
			.saturating_add(RocksDbWeight::get().reads(2))
			.saturating_add(RocksDbWeight::get().writes(1))
	}

	/// One prefix probe that comes back empty.
	fn day_probe() -> Weight {
		Weight::from_parts(0, 0).saturating_add(RocksDbWeight::get().reads(1))
	}

	/// Estimated: the 5 reads a visited user cost in the sweep's own prior
	/// accounting — subscriptions, the last-charged marker, the S3 size, plus
	/// slack — and one write.
	///
	/// The write is the marker settling on a user with nothing billable, which
	/// every covered user does at most once an hour. It is not part of
	/// `hourly_charge`: that one prices a user who is actually billed, and this
	/// path bills nobody.
	fn hourly_probe() -> Weight {
		Weight::from_parts(0, 0)
			.saturating_add(RocksDbWeight::get().reads(5))
			.saturating_add(RocksDbWeight::get().writes(1))
	}

	/// The *additional* cost when a user does owe something, on top of the
	/// probe: the per-GiB debit (`consume_credits`), the referral accrual, the
	/// transaction record and the last-charged marker.
	///
	/// Measured by `hourly_charge` in `benchmarking.rs`, posed at the debit's
	/// worst case — a full `MaxBatchesPerBuyer` spent-batch list the charge
	/// walks and prunes, behind one funding batch it drains, with a referred
	/// user so the commission accrual writes, and the funder maturing while
	/// frozen with alpha pending and partly unbacked so the drain also takes the
	/// unfreeze branch. 2026-10-27, steps 50 / repeat 20, on the same VM as
	/// `charge_account_due` (AMD EPYC 7543). The 26 reads are dominated by the
	/// batch walk (`Batches` r:16 at the benchmark cap); the price and credit
	/// reads the sweep does before this point are counted by `hourly_probe` and
	/// the loop's `past_gate` reserve, not here.
	fn hourly_charge() -> Weight {
		Weight::from_parts(138_483_000, 3279)
			.saturating_add(RocksDbWeight::get().reads(26))
			.saturating_add(RocksDbWeight::get().writes(10))
	}

	/// Estimated: one map read, which is what the skip path costs.
	fn alpha_release_probe() -> Weight {
		Weight::from_parts(0, 0).saturating_add(RocksDbWeight::get().reads(1))
	}

	/// Estimated: the alpha balance, the backing tally and the batch row,
	/// plus — when the release leaves the batch spent — the owner's batch list,
	/// read and rewritten (the extra reads/writes are slack). The list is
	/// rewritten whole (`retain`), so its full bounded length (`n` ids, 16 bytes
	/// of proof each) is charged on top of that single access, as in
	/// `chargeback`.
	fn alpha_release(n: u32) -> Weight {
		Weight::from_parts(0, 0)
			.saturating_add(RocksDbWeight::get().reads(4))
			.saturating_add(RocksDbWeight::get().writes(6))
			.saturating_add(Weight::from_parts(0, 16u64.saturating_mul(n as u64)))
	}

	/// Estimated, pending a benchmark run — see `compute_row`.
	///
	/// Fixed part: the authority list, both caps and the clock in; the
	/// high-water mark, the prune cursor and the summary event out. Per row,
	/// `compute_row` plus the `AccountChargeCapOverride` read.
	fn submit_compute_usage(n: u32) -> Weight {
		Weight::from_parts(50_000_000, 2_000)
			.saturating_add(RocksDbWeight::get().reads(4))
			.saturating_add(RocksDbWeight::get().writes(3))
			.saturating_add(submit_row().saturating_mul(n.into()))
	}

	/// Estimated, pending a benchmark run — see `compute_row`. The fixed part
	/// is the authority list; a settled row does everything a charged row
	/// does except touch `ComputeUsageCharged`, so it is priced the same.
	fn settle_compute_arrears(n: u32) -> Weight {
		Weight::from_parts(50_000_000, 2_000)
			.saturating_add(RocksDbWeight::get().reads(1))
			.saturating_add(compute_row().saturating_mul(n.into()))
	}

	/// See [`buy_credits`].
	fn buy_credits() -> Weight {
		buy_credits()
	}

	/// See [`submit_usage_v2`].
	fn submit_usage_v2(n: u32, k: u32) -> Weight {
		submit_usage_v2::<Self>(n, k)
	}

	/// See [`set_fee_kind_caps`].
	fn set_fee_kind_caps() -> Weight {
		set_fee_kind_caps()
	}
}

/// For tests and mocks: same shape, same relative costs, no database pricing.
///
/// Not zero. A zero `charge_account_due` would make the meter's affordability
/// division degenerate and silently let every test drain without bound, which
/// is precisely the behaviour the meter exists to prevent — a test suite that
/// cannot reach the limit cannot catch a regression in it.
impl WeightInfo for () {
	fn charge_account_due() -> Weight {
		Weight::from_parts(156_200_000, 1309)
			.saturating_add(RocksDbWeight::get().reads(7))
			.saturating_add(RocksDbWeight::get().writes(11))
	}

	fn backfill_account() -> Weight {
		Weight::from_parts(0, 0)
			.saturating_add(RocksDbWeight::get().reads(1))
			.saturating_add(RocksDbWeight::get().writes(5))
	}

	fn drain_overhead() -> Weight {
		Weight::from_parts(0, 0)
			.saturating_add(RocksDbWeight::get().reads(2))
			.saturating_add(RocksDbWeight::get().writes(1))
	}

	fn day_probe() -> Weight {
		Weight::from_parts(0, 0).saturating_add(RocksDbWeight::get().reads(1))
	}

	fn hourly_probe() -> Weight {
		Weight::from_parts(0, 0)
			.saturating_add(RocksDbWeight::get().reads(5))
			.saturating_add(RocksDbWeight::get().writes(1))
	}

	fn hourly_charge() -> Weight {
		Weight::from_parts(0, 0)
			.saturating_add(RocksDbWeight::get().reads(13))
			.saturating_add(RocksDbWeight::get().writes(11))
	}

	fn alpha_release_probe() -> Weight {
		Weight::from_parts(0, 0).saturating_add(RocksDbWeight::get().reads(1))
	}

	fn alpha_release(n: u32) -> Weight {
		Weight::from_parts(0, 0)
			.saturating_add(RocksDbWeight::get().reads(4))
			.saturating_add(RocksDbWeight::get().writes(6))
			.saturating_add(Weight::from_parts(0, 16u64.saturating_mul(n as u64)))
	}

	/// Estimated, pending a benchmark run — see `compute_row`.
	///
	/// Fixed part: the authority list, both caps and the clock in; the
	/// high-water mark, the prune cursor and the summary event out. Per row,
	/// `compute_row` plus the `AccountChargeCapOverride` read.
	fn submit_compute_usage(n: u32) -> Weight {
		Weight::from_parts(50_000_000, 2_000)
			.saturating_add(RocksDbWeight::get().reads(4))
			.saturating_add(RocksDbWeight::get().writes(3))
			.saturating_add(submit_row().saturating_mul(n.into()))
	}

	/// Estimated, pending a benchmark run — see `compute_row`. The fixed part
	/// is the authority list; a settled row does everything a charged row
	/// does except touch `ComputeUsageCharged`, so it is priced the same.
	fn settle_compute_arrears(n: u32) -> Weight {
		Weight::from_parts(50_000_000, 2_000)
			.saturating_add(RocksDbWeight::get().reads(1))
			.saturating_add(compute_row().saturating_mul(n.into()))
	}

	/// See [`buy_credits`].
	fn buy_credits() -> Weight {
		buy_credits()
	}

	/// See [`submit_usage_v2`].
	fn submit_usage_v2(n: u32, k: u32) -> Weight {
		submit_usage_v2::<Self>(n, k)
	}

	/// See [`set_fee_kind_caps`].
	fn set_fee_kind_caps() -> Weight {
		set_fee_kind_caps()
	}
}

/// One compute billing row at its most expensive path: charged in full with
/// arrears folded in, a referral accrued, and the debit walking
/// `COMPUTE_BENCH_BATCHES` (16) deposit batches, the last of them unfreezing.
///
/// **Estimated from counted storage access, not measured.** The benchmark
/// that poses this case exists (`submit_compute_usage` /
/// `settle_compute_arrears` in `benchmarking.rs`); replace this figure with
/// its output on the next benchmark run. Counted as distinct keys, the way
/// the benchmark reports them:
///
/// - reads (28): the idempotency record, the arrears, `UserBatches`, 16
///   `Batches`, `FreeCredits`, `AlphaBalances`, `UnbackedBatchAlpha`, the
///   backing tally, the commission rate, `ReferredUsers`, `ReferralCodes`,
///   `AccruedReferralCommission`, plus one of slack;
/// - writes (24): the record, the arrears, 16 `Batches`, `FreeCredits`,
///   `AlphaBalances`, `UnbackedBatchAlpha`, the backing tally, the accrued
///   commission, plus one of slack.
///
/// The 200µs of execution is roughly six times what one subscription charge
/// costs inside the measured `charge_account_due` (526.9µs for 15), for a
/// debit that walks 16 batches instead of one. Proof size allows ~2.6 KB per
/// distinct key read — FRAME's allowance for an entry of an unbounded map,
/// trie path included — over the 25 the path actually reads. At these
/// figures a full 250-row call is ~0.8s, inside the normal-class share of a
/// 2s block.
fn compute_row() -> Weight {
	Weight::from_parts(200_000_000, 66_000)
		.saturating_add(RocksDbWeight::get().reads(28))
		.saturating_add(RocksDbWeight::get().writes(24))
}

/// One `submit_compute_usage` / `submit_usage_v2` row: `compute_row` plus
/// the `AccountChargeCapOverride` read that picks its cap, with the same
/// ~2.6 KB proof allowance per read as `compute_row`. Settling does not read
/// it.
fn submit_row() -> Weight {
	compute_row()
		.saturating_add(Weight::from_parts(0, 2_600))
		.saturating_add(RocksDbWeight::get().reads(1))
}

/// `buy_credits` at its most expensive path: a referral code to check and
/// record, and a buyer with no client IP, so the mint probes the IP pool.
///
/// **Estimated from counted storage access, not measured**; the benchmark
/// that poses this case is `buy_credits` in `benchmarking.rs`. Counted as
/// distinct keys:
///
/// - reads (21): `BuyCreditsEnabled`, `UserRequestsCount`, `AlphaPrice`,
///   `AlphaPriceUpdatedAt`, `NextBatchId`, `UserBatches`, `AlphaBalances`,
///   `FreeCredits`, `TotalCreditsPurchased`, `ReferralCodes`, the four IP pool
///   entries (`RoleToIp`, `AvailableClientIps`, `AssignedClientIps`,
///   `IpToRole`), both `System.Account` rows, `TotalDeposited`,
///   `TotalUndistributedBacking`, `LastPurchaseBatch`, plus two of slack;
/// - writes (19): the same minus the price, the flag and `ReferralCodes`,
///   plus `Batches` and `ReferredUsers`, plus slack.
///
/// The top-up path, taken once the buyer is at `MaxBatchesPerBuyer`, touches
/// a subset of these: it rewrites one batch instead of opening one.
///
/// The 150µs of execution covers a balances transfer and the batch append,
/// with margin. Proof size is FRAME's per-entry allowance for the unbounded
/// maps it reads.
///
/// The IP pool vectors are priced *empty*, which is what mainnet holds
/// (`PalletIp.AvailableClientIps` was cleared and never refilled). They are
/// unbounded, so if the pool is ever repopulated this figure no longer covers
/// the mint's IP assignment and must be re-measured with a seeded pool.
fn buy_credits() -> Weight {
	Weight::from_parts(150_000_000, 60_000)
		.saturating_add(RocksDbWeight::get().reads(21))
		.saturating_add(RocksDbWeight::get().writes(19))
}

/// `submit_usage_v2`: a `submit_compute_usage` row's cost for each row, plus
/// each breakdown line's `FeeCapConfig` read and validation, plus a
/// `FeeSpend` read and write for as many lines as can be fee kinds.
///
/// **Estimated, not measured**, like `compute_row` and for the same reason:
/// the runtime-benchmarks build does not compile on the pinned toolchain. The
/// benchmark that poses this case is `submit_usage_v2` in `benchmarking.rs`.
///
/// - fixed: the chain clock read a second time, for the budget windows;
/// - per line (`n·k`): one `FeeCapConfig` read (a bounded map, ~100 bytes of
///   proof) and 1µs for the order and sum checks and the event bytes;
/// - per fee line (`n·min(k, MAX_FEE_KINDS)`): one `FeeSpend` read and write,
///   2µs, and FRAME's unbounded-map proof allowance.
///
/// A full 250-row, 16-line call adds ~0.23s to `submit_compute_usage`'s
/// ~0.83s, inside a normal-class extrinsic.
fn submit_usage_v2<W: WeightInfo>(n: u32, k: u32) -> Weight {
	let lines = u64::from(n).saturating_mul(u64::from(k));
	let fee_lines = u64::from(n).saturating_mul(u64::from(k.min(crate::MAX_FEE_KINDS)));
	let line = Weight::from_parts(1_000_000, 100).saturating_add(RocksDbWeight::get().reads(1));
	let fee_line = Weight::from_parts(2_000_000, 2_600)
		.saturating_add(RocksDbWeight::get().reads(1))
		.saturating_add(RocksDbWeight::get().writes(1));
	W::submit_compute_usage(n)
		.saturating_add(RocksDbWeight::get().reads(1))
		.saturating_add(line.saturating_mul(lines))
		.saturating_add(fee_line.saturating_mul(fee_lines))
}

/// `set_fee_kind_caps`: the existing entry probed, the `MAX_FEE_KINDS` keys
/// counted, one entry written. Estimated; root-only and rare.
fn set_fee_kind_caps() -> Weight {
	Weight::from_parts(20_000_000, 1_000)
		.saturating_add(RocksDbWeight::get().reads(u64::from(crate::MAX_FEE_KINDS) + 2))
		.saturating_add(RocksDbWeight::get().writes(1))
}

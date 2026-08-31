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
//! `charge_account_due` is measured. Re-measure it whenever the charge path
//! gains or loses storage access:
//!
//! ```text
//! cargo build --release --bin hippius --features hippius/runtime-benchmarks
//! ./target/release/hippius benchmark pallet \
//!     --chain benchmark \
//!     --pallet pallet_marketplace \
//!     --extrinsic charge_account_due \
//!     --steps 50 --repeat 20
//! ```
//!
//! The remaining three are single storage operations and stay as estimates.

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

	/// The *additional* cost of actually releasing one matured batch.
	fn alpha_release() -> Weight;
}

/// Weights for `pallet-marketplace` using the Substrate node's RocksDB weights.
///
/// `charge_account_due` is **measured**; the other three remain conservative
/// estimates of single storage operations, where any measurement would sit
/// inside the rounding.
pub struct SubstrateWeight<T>(PhantomData<T>);

impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
	/// Measured: 526.9µs, 7 reads, 21 writes, 1341 bytes of proof.
	///
	/// `hippius benchmark pallet --chain dev --pallet pallet_marketplace
	/// --extrinsic charge_account_due`, against an account holding
	/// `MaxActiveSubscriptions` subscriptions, all due, all payable, and far
	/// enough in arrears that the catch-up loop runs its full
	/// `max_catchup_months`.
	///
	/// The arrears are why this figure nearly doubled. The first measurement
	/// posed the account merely *due*, which charges one cycle and pushes the
	/// due date a month out, so the catch-up loop broke after a single pass:
	/// 156.2µs / 7r / 11w, about 1.43ms all in. But the drain charges this flat
	/// figure for an account whose loop may run three times, and the arrears
	/// case is not exotic — it is exactly what a chain meets on the tick after
	/// downtime, when the drain is busiest. Posed that way the same account
	/// costs 2.80ms, with `PointTransactions` alone taking 15 writes
	/// (`MaxActiveSubscriptions` × `max_catchup_months`) instead of 5.
	///
	/// The old number was therefore about half of the real worst case, in the
	/// direction that lets a block run heavy. The cost of fixing it is
	/// throughput: the meter now charges 2.80ms for *every* account, including
	/// the steady-state ones that only ever cost ~1.43ms, so a tick admits
	/// roughly 35 accounts rather than 69. That is the price of a bound that
	/// holds in the case it exists for.
	fn charge_account_due() -> Weight {
		Weight::from_parts(526_900_000, 1341)
			.saturating_add(RocksDbWeight::get().reads(7))
			.saturating_add(RocksDbWeight::get().writes(21))
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
	/// slack.
	fn hourly_probe() -> Weight {
		Weight::from_parts(0, 0).saturating_add(RocksDbWeight::get().reads(5))
	}

	/// Estimated: the 13 reads and 11 writes a *charged* user cost in that same
	/// accounting, on top of the probe.
	fn hourly_charge() -> Weight {
		Weight::from_parts(0, 0)
			.saturating_add(RocksDbWeight::get().reads(13))
			.saturating_add(RocksDbWeight::get().writes(11))
	}

	/// Estimated: one map read, which is what the skip path costs.
	fn alpha_release_probe() -> Weight {
		Weight::from_parts(0, 0).saturating_add(RocksDbWeight::get().reads(1))
	}

	/// Estimated: the alpha balance, the backing tally and the batch row.
	fn alpha_release() -> Weight {
		Weight::from_parts(0, 0)
			.saturating_add(RocksDbWeight::get().reads(2))
			.saturating_add(RocksDbWeight::get().writes(4))
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
		Weight::from_parts(0, 0).saturating_add(RocksDbWeight::get().reads(5))
	}

	fn hourly_charge() -> Weight {
		Weight::from_parts(0, 0)
			.saturating_add(RocksDbWeight::get().reads(13))
			.saturating_add(RocksDbWeight::get().writes(11))
	}

	fn alpha_release_probe() -> Weight {
		Weight::from_parts(0, 0).saturating_add(RocksDbWeight::get().reads(1))
	}

	fn alpha_release() -> Weight {
		Weight::from_parts(0, 0)
			.saturating_add(RocksDbWeight::get().reads(2))
			.saturating_add(RocksDbWeight::get().writes(4))
	}
}

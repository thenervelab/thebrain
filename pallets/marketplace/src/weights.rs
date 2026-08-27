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
}

/// Weights for `pallet-marketplace` using the Substrate node's RocksDB weights.
///
/// `charge_account_due` is **measured**; the other three remain conservative
/// estimates of single storage operations, where any measurement would sit
/// inside the rounding.
pub struct SubstrateWeight<T>(PhantomData<T>);

impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
	/// Measured: 156.2µs, 7 reads, 11 writes, 1309 bytes of proof.
	///
	/// `hippius benchmark pallet --chain benchmark --pallet pallet_marketplace
	/// --extrinsic charge_account_due`, against an account holding
	/// `MaxActiveSubscriptions` subscriptions all due and all payable.
	///
	/// Worth noting against the estimate it replaces (16 reads / 10 writes,
	/// costed at ~1.4ms): the totals nearly agree, at ~1.43ms, but the shape
	/// does not. The real charge does fewer reads and *more* writes than the
	/// code claimed, and carries 156µs of compute the read/write count could
	/// not see at all. The estimate was low — the direction that lets a block
	/// run heavy — which is the argument for measuring rather than reasoning.
	fn charge_account_due() -> Weight {
		Weight::from_parts(156_200_000, 1309)
			.saturating_add(RocksDbWeight::get().reads(7))
			.saturating_add(RocksDbWeight::get().writes(11))
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
}

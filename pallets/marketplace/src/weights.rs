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
//! The values below are the pallet's own conservative estimates, carried over
//! from the comments they replace, and are marked as such. Replace them with a
//! generated table:
//!
//! ```text
//! cargo build --release --features runtime-benchmarks
//! ./target/release/hippius benchmark pallet \
//!     --chain benchmark \
//!     --pallet pallet_marketplace \
//!     --extrinsic '*' \
//!     --steps 50 --repeat 20 \
//!     --output pallets/marketplace/src/weights.rs
//! ```
//!
//! Until then `MaxSubscriptionChargesPerRun` is an argument and not evidence —
//! but the meter means the argument only has to be conservative, not exact.

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
/// **Estimated, not measured.** Each figure is the read/write count the code
/// itself was already claiming, priced at `RocksDbWeight` (25µs a read, 100µs
/// a write). They are intentionally generous: the meter treats them as the cost
/// of the *next* unit, so over-estimating means stopping early and finishing on
/// the following tick, while under-estimating is what lets a block run heavy.
pub struct SubstrateWeight<T>(PhantomData<T>);

impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
	/// 16 reads, 10 writes — the per-charged-account figure the sweep's own
	/// accounting used before it metered itself, which the design note costed
	/// at ~1.4ms.
	fn charge_account_due() -> Weight {
		Weight::from_parts(0, 0)
			.saturating_add(RocksDbWeight::get().reads(16))
			.saturating_add(RocksDbWeight::get().writes(10))
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
		Weight::from_parts(0, 0)
			.saturating_add(RocksDbWeight::get().reads(16))
			.saturating_add(RocksDbWeight::get().writes(10))
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

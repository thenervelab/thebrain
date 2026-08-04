//! Property tests for every money-math invariant the crate documents.
//!
//! Lives in `tests/` (not `src/`) so proptest stays out of the `no_std`
//! build entirely — dev-dependencies never reach the runtime Wasm.

use payment_math::{
	available, payable, pro_rata, prorate_first_month, split, tokens_for, BasisPoints, Blocks,
	ByteBlocks, Bytes, Credits, Tokens, Usd, UsdPerGibBlock, BPS_DENOM, E18, GIB,
};
use primitive_types::U256;
use proptest::prelude::*;

proptest! {
	#[test]
	fn accrual_zero_annihilates_and_saturates(x in any::<u128>()) {
		prop_assert_eq!((Bytes::new(0) * Blocks::new(x)).get(), 0);
		prop_assert_eq!((Bytes::new(x) * Blocks::new(0)).get(), 0);
		// Multiplying any non-zero byte count by MAX blocks must pin to the
		// saturation ceiling, never wrap back into a plausible small value.
		let ceiling = if x == 0 { 0 } else { u128::MAX };
		prop_assert_eq!((Bytes::new(x) * Blocks::new(u128::MAX)).get(), ceiling);
	}

	#[test]
	fn accrual_monotone_in_both_arguments(
		a in any::<u128>(), b in any::<u128>(), blocks in any::<u128>(),
	) {
		let (lo, hi) = (a.min(b), a.max(b));
		prop_assert!(
			(Bytes::new(lo) * Blocks::new(blocks)).get()
				<= (Bytes::new(hi) * Blocks::new(blocks)).get()
		);
		prop_assert!(
			(Bytes::new(blocks) * Blocks::new(lo)).get()
				<= (Bytes::new(blocks) * Blocks::new(hi)).get()
		);
	}

	/// The floor-division bound: pricing under-pays by strictly less than
	/// one planck, and never over-pays. (Skips saturated regions — the
	/// bound only makes sense for exact quotients.)
	#[test]
	fn pricing_rounding_error_is_below_one_planck(
		bb in 0u128..=(100 * 1024 * GIB * 14_400), // up to ~100 TiB × 1 day
		price in 0u128..=(4 * E18),
		tp in 1u128..=(4 * E18),
	) {
		let num = U256::from(bb) * U256::from(price) * U256::from(E18);
		let den = U256::from(GIB) * U256::from(tp);
		prop_assume!(num / den <= U256::from(u128::MAX));
		let tokens =
			tokens_for(ByteBlocks::new(bb), UsdPerGibBlock::new(price), Usd::new(tp)).get();
		prop_assert!(U256::from(tokens) * den <= num);
		prop_assert!(num < (U256::from(tokens) + U256::from(1u8)) * den);
	}

	/// Bank-never-overpays: for any partition of total_due, the pro-rata
	/// shares sum to at most the pool, and each share stays within its due —
	/// the bound that makes the caller's `arrears = due − share` recipe an
	/// exact reconstruction (given share ≤ due, `share + (due − share) == due`
	/// is arithmetic identity, so only the bound needs asserting).
	///
	/// A raw `any::<u128>()` pool alone would dwarf `total` (dues cap at
	/// 2^100) and near-never exercise the shortfall branch — verified by a
	/// round-up mutation that survived it. So each case also checks a pool
	/// scaled to 0–200% of `total`, covering both regimes every run.
	#[test]
	fn pro_rata_shares_never_exceed_pool_and_conserve_due(
		dues in prop::collection::vec(0u128..=(1u128 << 100), 1..20),
		raw_pool in any::<u128>(),
		pool_pct in 0u128..=200,
	) {
		let total: u128 = dues.iter().fold(0u128, |a, v| a.saturating_add(*v));
		prop_assume!(total > 0 && total < u128::MAX); // saturated totals are not a partition
		let total_t = Tokens::new(total);
		// total < 2^105 and pct ≤ 200 < 2^8, so the product fits u128.
		for pool in [raw_pool, total * pool_pct / 100] {
			let pool_t = Tokens::new(pool);
			let mut sum = 0u128;
			for d in &dues {
				let share = pro_rata(Tokens::new(*d), pool_t, total_t);
				// The load-bearing per-claimant bound: share ≤ due is what
				// keeps the caller's `due − share` arrears subtraction exact.
				prop_assert!(share.get() <= *d);
				sum += share.get(); // each share ≤ its due, Σ dues = total < MAX
			}
			prop_assert!(sum <= total);
			// Holds in both regimes: shortfall (Σ floors ≤ pool) and surplus
			// (Σ shares = total ≤ pool) — the bank can never overpay its pool.
			prop_assert!(sum <= pool);
			if pool >= total {
				prop_assert_eq!(sum, total); // full pool ⇒ everyone paid in full
			}
		}
	}

	#[test]
	fn split_conserves_exactly_even_with_clamped_ratio(
		amount in any::<u128>(),
		bps in 0u128..=(2 * BPS_DENOM), // beyond 100% exercises the clamp
	) {
		let (part, rest) = split(Tokens::new(amount), BasisPoints::new(bps));
		prop_assert!(part.get() <= amount);
		prop_assert_eq!(part.get().checked_add(rest.get()), Some(amount));
		// Floor characterization: part is exactly ⌊amount × ratio / 10_000⌋.
		// Conservation alone cannot see the rounding direction (rest absorbs
		// it either way), so pin the documented round-down explicitly.
		let eff = bps.min(BPS_DENOM);
		prop_assert!(
			U256::from(part.get()) * U256::from(BPS_DENOM) <= U256::from(amount) * U256::from(eff)
		);
		prop_assert!(
			U256::from(amount) * U256::from(eff)
				< (U256::from(part.get()) + U256::from(1u8)) * U256::from(BPS_DENOM)
		);
	}

	#[test]
	fn prorate_bounded_by_full_month_and_rounds_up(
		price in 0u128..=(u128::MAX / 31),
		drm in 0u32..=31,
		dim in 1u32..=31,
	) {
		// post-clamp semantics: bounded for ALL drm, ceil bound vs clamped drm
		let out = prorate_first_month(Credits::new(price), drm, dim).get();
		prop_assert!(out <= price);
		let eff = drm.min(dim);
		let (p, d, m) = (U256::from(price), U256::from(eff), U256::from(dim));
		prop_assert!(U256::from(out) * m >= p * d);
		prop_assert!(U256::from(out) * m < p * d + m);
	}

	/// The part-never-exceeds-whole clamp must hold over the FULL u128 price
	/// range — including where the internal `price × days` and `+ (dim − 1)`
	/// steps saturate, a region the ceil-characterization test above excludes
	/// by capping price at `u128::MAX / 31`.
	#[test]
	fn prorate_never_exceeds_full_price_even_when_saturating(
		price in any::<u128>(),
		drm in any::<u32>(),
		dim in 1u32..=u32::MAX,
	) {
		prop_assert!(prorate_first_month(Credits::new(price), drm, dim).get() <= price);
	}

	#[test]
	fn payable_and_available_are_caps(
		bal in any::<u128>(), ed in any::<u128>(), req in any::<u128>(),
	) {
		let avail = available(Tokens::new(bal), Tokens::new(ed));
		prop_assert!(avail.get() <= bal);
		let paid = payable(Tokens::new(req), avail);
		prop_assert!(paid.get() <= req);
		prop_assert!(paid.get() <= avail.get());
	}
}

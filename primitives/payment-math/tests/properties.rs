//! Property tests for every money-math invariant the crate documents.
//!
//! Lives in `tests/` (not `src/`) so proptest stays out of the `no_std`
//! build entirely — dev-dependencies never reach the runtime Wasm.

use payment_math::{
	available, payable, pro_rata, pro_rata_wide, prorate_first_month, split, sum_dues, tokens_for,
	Amount, BasisPoints, Blocks, ByteBlocks, Bytes, Credits, Tokens, Usd, UsdPerGibBlock,
	BPS_DENOM, E18, GIB,
};
use primitive_types::{U256, U512};
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

	/// Exact product when it fits; saturate to MAX when checked_mul fails.
	#[test]
	fn accrual_matches_checked_mul_or_saturates(a in any::<u128>(), b in any::<u128>()) {
		let got = (Bytes::new(a) * Blocks::new(b)).get();
		match a.checked_mul(b) {
			Some(p) => prop_assert_eq!(got, p),
			None => prop_assert_eq!(got, u128::MAX),
		}
	}

	#[test]
	fn byte_blocks_saturating_add_matches_u128(a in any::<u128>(), b in any::<u128>()) {
		prop_assert_eq!(
			ByteBlocks::new(a).saturating_add(ByteBlocks::new(b)).get(),
			a.saturating_add(b)
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

	/// Exact match against a `U512` oracle over the full `u128` domain —
	/// including inputs whose numerator exceeds `U256::MAX` (where a
	/// saturating-`U256` implementation under-pays) and inputs whose quotient
	/// saturates at `u128::MAX`.
	#[test]
	fn pricing_matches_u512_oracle_over_full_domain(
		bb in any::<u128>(),
		price in any::<u128>(),
		tp in 1u128..=u128::MAX,
	) {
		let num: U512 = U256::from(bb).full_mul(U256::from(price)) * U512::from(E18);
		let den = U512::from(GIB) * U512::from(tp);
		let exact = num / den;
		let tokens =
			tokens_for(ByteBlocks::new(bb), UsdPerGibBlock::new(price), Usd::new(tp)).get();
		if exact > U512::from(u128::MAX) {
			prop_assert_eq!(tokens, u128::MAX);
		} else {
			prop_assert_eq!(U512::from(tokens), exact);
			// Floor characterization when the quotient fits u128.
			prop_assert!(U512::from(tokens) * den <= num);
			prop_assert!(num < (U512::from(tokens) + U512::from(1u8)) * den);
		}
	}

	#[test]
	fn pricing_monotone_in_bb_and_price_antitone_in_token_price(
		a in any::<u128>(),
		b in any::<u128>(),
		fixed in any::<u128>(),
		tp_lo in 1u128..=u128::MAX,
		tp_hi in 1u128..=u128::MAX,
	) {
		let (lo, hi) = (a.min(b), a.max(b));
		let (tp_a, tp_b) = (tp_lo.min(tp_hi), tp_lo.max(tp_hi));
		let t = |bb: u128, price: u128, tp: u128| {
			tokens_for(ByteBlocks::new(bb), UsdPerGibBlock::new(price), Usd::new(tp)).get()
		};
		prop_assert!(t(lo, fixed, tp_a) <= t(hi, fixed, tp_a));
		prop_assert!(t(fixed, lo, tp_a) <= t(fixed, hi, tp_a));
		// Higher token USD price ⇒ fewer or equal tokens for the same accrual.
		prop_assert!(t(fixed, fixed, tp_a) >= t(fixed, fixed, tp_b));
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
				let due = Tokens::new(*d);
				let share = pro_rata(due, pool_t, total_t);
				// The load-bearing per-claimant bound: share ≤ due is what
				// keeps the caller's `due − share` arrears subtraction exact.
				prop_assert!(share.get() <= *d);
				prop_assert_eq!(share.saturating_add(due.saturating_sub(share)), due);
				sum += share.get(); // each share ≤ its due, Σ dues = total < MAX
			}
			prop_assert!(sum <= total);
			// Holds in both regimes: shortfall (Σ floors ≤ pool) and surplus
			// (Σ shares = total ≤ pool) — the bank can never overpay its pool.
			prop_assert!(sum <= pool);
			if pool >= total {
				prop_assert_eq!(sum, total); // full pool ⇒ everyone paid in full
			} else {
				// Floor-division dust left in the bank: strictly less than one
				// planck per claimant, so it cannot accumulate into a silent
				// extra full share under the caller contract.
				prop_assert!(pool - sum < dues.len() as u128);
			}
		}
	}

	/// Sole claimant identity over the full domain: when due == total_due,
	/// pro_rata is exactly min(due, pool).
	#[test]
	fn pro_rata_single_claimant_is_min_due_pool(
		due in 1u128..=u128::MAX,
		pool in any::<u128>(),
	) {
		let d = Tokens::new(due);
		let share = pro_rata(d, Tokens::new(pool), d).get();
		prop_assert_eq!(share, due.min(pool));
	}

	/// Floor formula for a single share, full u128 dues (not capped at 2^100).
	/// Generate `due` as a fraction of `total` so the caller contract always
	/// holds without mass `prop_assume` rejects.
	#[test]
	fn pro_rata_floor_formula_full_domain(
		total in 1u128..=u128::MAX,
		due_pct in 0u128..=100,
		pool in any::<u128>(),
	) {
		// due = floor(total * pct / 100) ≤ total always; product fits because
		// pct ≤ 100.
		let due = total.saturating_mul(due_pct) / 100;
		// Also exercise due == total explicitly via pct=100, and a raw due
		// taken as min(pool, total) for variety near the top of the domain.
		for due in [due, total, pool.min(total)] {
			let share = pro_rata(Tokens::new(due), Tokens::new(pool), Tokens::new(total)).get();
			if pool >= total {
				prop_assert_eq!(share, due);
			} else {
				let exact = (U256::from(due) * U256::from(pool) / U256::from(total)).as_u128();
				prop_assert_eq!(share, exact);
				prop_assert!(share <= due);
				prop_assert!(share <= pool);
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

	/// Same conservation + floor bound for Credits (marketplace path).
	#[test]
	fn split_credits_conserves_and_floors(
		amount in any::<u128>(),
		bps in 0u128..=(2 * BPS_DENOM),
	) {
		let (part, rest) = split(Credits::new(amount), BasisPoints::new(bps));
		prop_assert_eq!(part.get().checked_add(rest.get()), Some(amount));
		let eff = bps.min(BPS_DENOM);
		let exact = (U256::from(amount) * U256::from(eff) / U256::from(BPS_DENOM)).as_u128();
		prop_assert_eq!(part.get(), exact);
	}

	#[test]
	fn basis_points_never_exceeds_denom(raw in any::<u128>()) {
		prop_assert!(BasisPoints::new(raw).get() <= BPS_DENOM);
	}

	#[test]
	fn prorate_bounded_by_full_month_and_rounds_up(
		price in any::<u128>(),
		drm in 0u32..=31,
		dim in 1u32..=31,
	) {
		// post-clamp semantics: bounded for ALL drm, exact ceil vs clamped drm
		let out = prorate_first_month(Credits::new(price), drm, dim).get();
		prop_assert!(out <= price);
		let eff = drm.min(dim);
		let (p, d, m) = (U256::from(price), U256::from(eff), U256::from(dim));
		prop_assert!(U256::from(out) * m >= p * d);
		prop_assert!(U256::from(out) * m < p * d + m);
	}

	/// The part-never-exceeds-whole clamp must hold over the FULL u128 price
	/// range and arbitrary calendar inputs (including drm > dim).
	#[test]
	fn prorate_never_exceeds_full_price(
		price in any::<u128>(),
		drm in any::<u32>(),
		dim in 1u32..=u32::MAX,
	) {
		prop_assert!(prorate_first_month(Credits::new(price), drm, dim).get() <= price);
	}

	/// Exact U256 ceil over the full domain — including prices where
	/// `price × days` or `+ (dim − 1)` would overflow `u128`. Covers Tokens
	/// as well as Credits.
	#[test]
	fn prorate_matches_exact_u256_ceil_oracle(
		price in any::<u128>(),
		drm in any::<u32>(),
		dim in 1u32..=u32::MAX,
	) {
		let out_c = prorate_first_month(Credits::new(price), drm, dim).get();
		let out_t = prorate_first_month(Tokens::new(price), drm, dim).get();
		prop_assert_eq!(out_c, out_t);
		let days = drm.min(dim);
		let dim_u = U256::from(dim);
		let expected = (U256::from(price) * U256::from(days) + (dim_u - U256::from(1u8))) / dim_u;
		prop_assert_eq!(U256::from(out_c), expected);
		prop_assert!(out_c <= price);
	}

	/// dim==0 always charges the full price (calendar not initialised).
	#[test]
	fn prorate_zero_dim_always_full_price(price in any::<u128>(), drm in any::<u32>()) {
		prop_assert_eq!(prorate_first_month(Credits::new(price), drm, 0).get(), price);
	}

	#[test]
	fn payable_and_available_are_caps(
		bal in any::<u128>(), ed in any::<u128>(), req in any::<u128>(),
	) {
		let avail = available(Tokens::new(bal), Tokens::new(ed));
		prop_assert!(avail.get() <= bal);
		// Exact semantics, not just inequalities.
		prop_assert_eq!(avail.get(), bal.saturating_sub(ed));
		let paid = payable(Tokens::new(req), avail);
		prop_assert!(paid.get() <= req);
		prop_assert!(paid.get() <= avail.get());
		prop_assert_eq!(paid.get(), req.min(avail.get()));
	}

	/// Pipeline: priced due → pro-rata shortfall → bank payable never exceeds
	/// either the share or free-above-ED.
	#[test]
	fn composition_priced_share_respects_bank_caps(
		bb in 0u128..=(10 * GIB * 14_400),
		price in 0u128..=E18,
		tp in 1u128..=E18,
		pool_pct in 0u128..=100,
		ed in 0u128..=1_000,
	) {
		let due = tokens_for(ByteBlocks::new(bb), UsdPerGibBlock::new(price), Usd::new(tp)).get();
		// Two identical claimants → total = 2*due (saturate-safe: due ≤ tokens for small bb).
		let total = due.saturating_mul(2);
		prop_assume!(total > 0);
		let pool = total.saturating_mul(pool_pct) / 100;
		let share = pro_rata(Tokens::new(due), Tokens::new(pool), Tokens::new(total)).get();
		prop_assert!(share <= due);
		prop_assert!(share <= pool);
		// Bank balance = pool + ed when that addition does not saturate.
		prop_assume!(pool.checked_add(ed).is_some());
		let bal = pool + ed;
		let avail = available(Tokens::new(bal), Tokens::new(ed)).get();
		prop_assert_eq!(avail, pool);
		let paid = payable(Tokens::new(share), Tokens::new(avail)).get();
		prop_assert_eq!(paid, share.min(avail));
		prop_assert_eq!(share + due.saturating_sub(share), due);
	}

	/// Wide multi-claimant shortfall: dues may individually be near `u128::MAX`
	/// so their true sum exceeds `u128::MAX`. Shares still never exceed the
	/// pool, each due, or the floor formula against the U256 total.
	#[test]
	fn pro_rata_wide_multi_claimant_respects_pool(
		n in 2usize..8,
		pool in any::<u128>(),
		// High dues so sum regularly overflows u128.
		seed_dues in prop::collection::vec(any::<u128>(), 2..8),
	) {
		let dues: Vec<u128> = seed_dues.into_iter().take(n).collect();
		prop_assume!(!dues.is_empty());
		let total = sum_dues(dues.iter().copied().map(Tokens::new));
		prop_assume!(!total.is_zero());
		let pool_t = Tokens::new(pool);
		let mut sum = U256::zero();
		for &d in &dues {
			let share = pro_rata_wide(Tokens::new(d), pool_t, total);
			prop_assert!(share.get() <= d);
			prop_assert!(U256::from(share.get()) <= U256::from(pool));
			let exact = U256::from(d) * U256::from(pool) / total;
			// Full-cover branch only when pool ≥ total (impossible once total > MAX).
			if U256::from(pool) >= total {
				prop_assert_eq!(share.get(), d);
			} else {
				prop_assert_eq!(U256::from(share.get()), exact.min(U256::from(d)));
			}
			sum = sum.saturating_add(U256::from(share.get()));
		}
		prop_assert!(sum <= U256::from(pool));
		prop_assert!(sum <= total);
	}

	/// Narrow `pro_rata` is exactly `pro_rata_wide` with a `u128` total.
	#[test]
	fn pro_rata_matches_wide_for_u128_totals(
		due in any::<u128>(),
		pool in any::<u128>(),
		total in 1u128..=u128::MAX,
	) {
		// Keep the caller contract so both paths stay in documented territory.
		let due = due.min(total);
		let narrow = pro_rata(Tokens::new(due), Tokens::new(pool), Tokens::new(total)).get();
		let wide =
			pro_rata_wide(Tokens::new(due), Tokens::new(pool), U256::from(total)).get();
		prop_assert_eq!(narrow, wide);
	}

	/// Monotonicity: more due (≤ total) never yields a smaller share; larger
	/// pool never yields a smaller share; larger total never yields a larger share.
	#[test]
	fn pro_rata_monotone_in_due_pool_antitone_in_total(
		a in any::<u128>(),
		b in any::<u128>(),
		pool_a in any::<u128>(),
		pool_b in any::<u128>(),
		total in 1u128..=u128::MAX,
	) {
		let (due_lo, due_hi) = (a.min(b).min(total), a.max(b).min(total));
		let (pool_lo, pool_hi) = (pool_a.min(pool_b), pool_a.max(pool_b));
		let t = |due: u128, pool: u128, total: u128| {
			pro_rata(Tokens::new(due), Tokens::new(pool), Tokens::new(total)).get()
		};
		prop_assert!(t(due_lo, pool_lo, total) <= t(due_hi, pool_lo, total));
		prop_assert!(t(due_lo, pool_lo, total) <= t(due_lo, pool_hi, total));
		// Larger total (still ≥ due) ⇒ smaller or equal share in the shortfall path.
		let total_hi = total;
		let total_lo = due_hi.max(1); // ≥ due so contract holds
		prop_assume!(total_lo <= total_hi);
		prop_assert!(t(due_hi, pool_lo, total_hi) <= t(due_hi, pool_lo, total_lo));
	}
}

//! Historical audit of the hand-rolled money math that briefly replaced
//! `payment_math` in `pallets/marketplace/src/lib.rs` (commits
//! 8c8c056..92d96b2, removed again by 5342f1b, which re-routed the pallet
//! through the crate).
//!
//! Each `hand_rolled` function is a verbatim transcription of a saturating
//! `u128` expression from that reverted window; each `via_crate` function is
//! the `payment_math` call that replaced it. None of the `hand_rolled` forms
//! exist in the pallet anymore — the tests pin the exact input at which each
//! pair stops agreeing, so if anyone reintroduces the saturating shape the
//! size of the behaviour change is a number rather than an opinion.
//!
//! This file only reads `payment_math`'s public API — it does not touch the
//! pallet or the crate's own sources.

use payment_math::{prorate_first_month, split, BasisPoints, Credits};

const M: u128 = u128::MAX;

/// The saturating expressions from the reverted window (no longer in the
/// pallet).
mod hand_rolled {
	/// Referral discount as `saturating_mul(5) / 100`.
	pub fn referral_discount(face_credits: u128) -> u128 {
		face_credits.saturating_mul(5) / 100u128
	}

	/// Referral commission — same expression as the discount path.
	pub fn referral_commission(total_charged: u128) -> u128 {
		total_charged.saturating_mul(5) / 100
	}

	/// Discounted monthly price as `saturating_mul(9_500) / 10_000`.
	pub fn paid_per_month(price: u128) -> u128 {
		price.saturating_mul(9_500u128) / 10_000u128
	}

	/// `prorated_monthly_price` with the calendar reads lifted into
	/// parameters. Note the absent `min` clamp.
	pub fn prorated_monthly_price(monthly_price: u128, drm: u128, dim: u128) -> u128 {
		if dim == 0 {
			return monthly_price;
		}
		let numerator = monthly_price.saturating_mul(drm);
		numerator.saturating_add(dim.saturating_sub(1)) / dim
	}
}

/// The `payment_math` calls the pallet uses today.
mod via_crate {
	use super::*;

	pub fn five_percent(amount: u128) -> u128 {
		split(Credits::new(amount), BasisPoints::new(500)).0.get()
	}

	pub fn ninety_five_percent(amount: u128) -> u128 {
		split(Credits::new(amount), BasisPoints::new(9_500)).0.get()
	}

	pub fn prorated_monthly_price(monthly_price: u128, drm: u32, dim: u32) -> u128 {
		prorate_first_month(Credits::new(monthly_price), drm, dim).get()
	}
}

/// The implementation that `payment_math` itself replaced, kept as a yardstick:
/// on overflow it discarded the whole amount.
fn old_old_five_percent(amount: u128) -> u128 {
	amount.checked_mul(5).unwrap_or_default() / 100
}

/// Exact `floor(amount × ratio / 10_000)` in 256-bit arithmetic — the ground
/// truth both implementations are measured against.
fn exact_bps(amount: u128, ratio: u128) -> u128 {
	use primitive_types::U256;
	(U256::from(amount) * U256::from(ratio) / U256::from(10_000u128)).as_u128()
}

// ── substitution 1 & 2: the 5% paths (discount, commission) ──────────────────

/// `u128::MAX` is divisible by 5, so `saturating_mul(5)` is exact for every
/// input up to and including `MAX / 5` — the saturation cliff starts one above.
#[test]
fn five_percent_saturation_threshold_is_max_over_five() {
	assert_eq!(M % 5, 0, "u128::MAX is divisible by 5");
	let last_safe = M / 5;
	assert_eq!(last_safe, 68_056_473_384_187_692_692_674_921_486_353_642_291);
	assert_eq!(last_safe.checked_mul(5), Some(M));
	assert_eq!((last_safe + 1).checked_mul(5), None);

	// At the last exact input the two agree.
	assert_eq!(hand_rolled::referral_discount(last_safe), via_crate::five_percent(last_safe));
}

/// Above the cliff the numerator pins at `u128::MAX`, so the result freezes at
/// `MAX / 100` no matter how large the input grows. The first *observable*
/// divergence lags the cliff by 9, because `MAX/100` still floors to the right
/// answer for the first few saturating inputs.
#[test]
fn five_percent_first_diverging_input() {
	let saturated_output = M / 100;
	// Smallest x whose true 5% exceeds the frozen output.
	let first_bad = 20 * (saturated_output + 1);
	assert_eq!(first_bad, 68_056_473_384_187_692_692_674_921_486_353_642_300);
	assert_eq!(first_bad, M / 5 + 9);

	// Everything at or below is still exact...
	assert_eq!(
		hand_rolled::referral_discount(first_bad - 1),
		via_crate::five_percent(first_bad - 1)
	);
	// ...and here they part company.
	let rolled = hand_rolled::referral_discount(first_bad);
	let exact = via_crate::five_percent(first_bad);
	assert_eq!(exact, exact_bps(first_bad, 500));
	assert_eq!(exact, 3_402_823_669_209_384_634_633_746_074_317_682_115);
	assert_eq!(rolled, 3_402_823_669_209_384_634_633_746_074_317_682_114);
	assert_eq!(exact - rolled, 1, "understated by 1 at the divergence boundary");
}

/// At the top of the domain the hand-rolled form understates by 5×: it returns
/// 1% of the amount where 5% is owed. It is still strictly better than the
/// implementation `payment_math` replaced, which returned zero.
#[test]
fn five_percent_worst_case_understates_five_fold() {
	let rolled = hand_rolled::referral_discount(M);
	let exact = via_crate::five_percent(M);
	assert_eq!(exact, exact_bps(M, 500));
	assert_eq!(exact, 17_014_118_346_046_923_173_168_730_371_588_410_572);
	assert_eq!(rolled, 3_402_823_669_209_384_634_633_746_074_317_682_114);
	assert_eq!(exact / rolled, 5);
	assert!(rolled < exact, "the saturating form always understates, never overstates");

	// Ranking against both predecessors: exact > saturating > zero.
	assert_eq!(old_old_five_percent(M), 0);
	assert!(rolled > old_old_five_percent(M));
}

/// The commission path is the same expression as the discount path, so it
/// inherits the same cliff — worth pinning so a future edit to one is caught.
#[test]
fn commission_and_discount_share_one_expression() {
	for x in [0u128, 1, 99, 100, M / 5, M / 5 + 9, M] {
		assert_eq!(hand_rolled::referral_commission(x), hand_rolled::referral_discount(x));
	}
}

/// Below the cliff `floor(5x/100)` and `split(x, 500 bps)` are the same
/// function, sampled densely and at every power of two that stays safe.
#[test]
fn five_percent_agrees_everywhere_below_the_cliff() {
	for x in 0u128..10_000 {
		assert_eq!(hand_rolled::referral_discount(x), via_crate::five_percent(x), "x={x}");
	}
	for bit in 0..125 {
		let x = 1u128 << bit;
		assert_eq!(hand_rolled::referral_discount(x), via_crate::five_percent(x), "2^{bit}");
	}
	let last_safe = M / 5;
	for delta in 0..1_000u128 {
		let x = last_safe - delta;
		assert_eq!(hand_rolled::referral_discount(x), via_crate::five_percent(x), "x={x}");
	}
}

// ── substitution 3: the 95% path (paid_per_month) ────────────────────────────

/// `9_500` is a much smaller headroom than `5`, so this path saturates about
/// 1_900× earlier — at roughly 3.58e34 rather than 6.8e37.
#[test]
fn ninety_five_percent_saturation_threshold() {
	let last_safe = M / 9_500;
	assert_eq!(last_safe, 35_819_196_517_993_522_469_828_906_045_449_285);
	assert!(last_safe.checked_mul(9_500).is_some());
	assert_eq!((last_safe + 1).checked_mul(9_500), None);
	assert_eq!(hand_rolled::paid_per_month(last_safe), via_crate::ninety_five_percent(last_safe));
}

#[test]
fn ninety_five_percent_first_diverging_input() {
	let saturated_output = M / 10_000;
	// Smallest x with floor(19x/20) > frozen output.
	let mut first_bad = M / 9_500 + 1;
	while hand_rolled::paid_per_month(first_bad) == via_crate::ninety_five_percent(first_bad) {
		first_bad += 1;
	}
	assert_eq!(first_bad, 35_819_196_517_993_522_469_828_906_045_449_287);
	assert_eq!(first_bad, M / 9_500 + 2, "diverges two above the saturation cliff");

	let rolled = hand_rolled::paid_per_month(first_bad);
	let exact = via_crate::ninety_five_percent(first_bad);
	assert_eq!(exact, exact_bps(first_bad, 9_500));
	assert_eq!(rolled, saturated_output);
	assert_eq!(exact - rolled, 1);
}

/// Worst case: 9_500× understatement — the renewal price collapses from 95% of
/// the plan price to 0.01% of `u128::MAX`.
#[test]
fn ninety_five_percent_worst_case_understates_by_the_ratio() {
	let rolled = hand_rolled::paid_per_month(M);
	let exact = via_crate::ninety_five_percent(M);
	assert_eq!(exact, exact_bps(M, 9_500));
	assert_eq!(exact, 323_268_248_574_891_540_290_205_877_060_179_800_882);
	assert_eq!(rolled, 34_028_236_692_093_846_346_337_460_743_176_821);
	assert_eq!(exact / rolled, 9_500);
	assert!(rolled < exact);
}

#[test]
fn ninety_five_percent_agrees_everywhere_below_the_cliff() {
	for x in 0u128..10_000 {
		assert_eq!(hand_rolled::paid_per_month(x), via_crate::ninety_five_percent(x), "x={x}");
	}
	for bit in 0..113 {
		let x = 1u128 << bit;
		assert_eq!(hand_rolled::paid_per_month(x), via_crate::ninety_five_percent(x), "2^{bit}");
	}
	let last_safe = M / 9_500;
	for delta in 0..1_000u128 {
		let x = last_safe - delta;
		assert_eq!(hand_rolled::paid_per_month(x), via_crate::ninety_five_percent(x), "x={x}");
	}
}

// ── substitution 4: the hand-rolled proration ────────────────────────────────

/// Mirror of `pallets/calendar/src/calendar.rs` — the only producer of the
/// `drm`/`dim` pair the pallet feeds to `prorated_monthly_price`.
mod calendar_mirror {
	pub fn month_length(year: i32, month: u8) -> u8 {
		match month {
			1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
			4 | 6 | 9 | 11 => 30,
			2 => {
				if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
					29
				} else {
					28
				}
			},
			_ => unreachable!("month out of range"),
		}
	}

	/// `days_remaining_in_month`: `month_length − day + 1`, inclusive of today.
	pub fn days_remaining(year: i32, month: u8, day: u8) -> u8 {
		month_length(year, month).saturating_sub(day).saturating_add(1)
	}
}

/// The missing clamp is unreachable through `pallet_calendar`: `day` is always
/// a valid day of its month, so `drm = len − day + 1` lies in `1..=len = dim`.
/// Checked over every (year, month, day) across a leap cycle.
#[test]
fn calendar_can_never_produce_days_remaining_above_days_in_month() {
	for year in 1970..2400 {
		for month in 1u8..=12 {
			let dim = calendar_mirror::month_length(year, month);
			for day in 1..=dim {
				let drm = calendar_mirror::days_remaining(year, month, day);
				assert!(drm >= 1 && drm <= dim, "{year}-{month}-{day}: drm={drm} dim={dim}");
			}
		}
	}
}

/// Over the whole reachable calendar domain the forms agree for every price
/// whose `price × dim` product fits in `u128` (the hand-rolled path's safe
/// region). At `u128::MAX` they diverge — payment_math is exact, the
/// saturating form under-charges — pinned separately below.
#[test]
fn prorate_agrees_over_every_reachable_calendar_input() {
	// Prices whose ×31 product still fits; MAX is checked for divergence after.
	let prices = [0u128, 1, 7, 100, 999, 1_000_000, 10u128.pow(18), 10u128.pow(30), M / 31];
	for year in [2024i32, 2025, 2026, 2100, 2400] {
		for month in 1u8..=12 {
			let dim = calendar_mirror::month_length(year, month);
			for day in 1..=dim {
				let drm = calendar_mirror::days_remaining(year, month, day);
				for &price in &prices {
					let rolled = hand_rolled::prorated_monthly_price(price, drm.into(), dim.into());
					let exact = via_crate::prorated_monthly_price(price, drm.into(), dim.into());
					assert_eq!(rolled, exact, "{year}-{month}-{day} price={price}");
				}
				// Extreme: hand-rolled under-charges, payment_math keeps full-month identity.
				let rolled_m = hand_rolled::prorated_monthly_price(M, drm.into(), dim.into());
				let exact_m = via_crate::prorated_monthly_price(M, drm.into(), dim.into());
				assert!(rolled_m <= exact_m, "{year}-{month}-{day}");
				if drm == dim {
					assert_eq!(exact_m, M, "full month at MAX must charge MAX");
					assert_eq!(rolled_m, M / u128::from(dim));
				}
			}
		}
	}
}

/// The clamp only matters for inputs the calendar cannot emit. If some future
/// caller passes `drm > dim`, the unclamped form bills more than a full month —
/// this pins how much, so the defensive value of the clamp is on record.
#[test]
fn prorate_diverges_only_when_days_remaining_exceeds_the_month() {
	// 60 days of a 30-day month: two months' rent for one month's service.
	assert_eq!(via_crate::prorated_monthly_price(100, 60, 30), 100);
	assert_eq!(hand_rolled::prorated_monthly_price(100, 60, 30), 200);

	// The clamp is what bounds the charge at one monthly price.
	assert_eq!(via_crate::prorated_monthly_price(100, u32::MAX, 31), 100);
	assert_eq!(
		hand_rolled::prorated_monthly_price(100, u128::from(u32::MAX), 31),
		13_854_733_210, // ≈ 138 million months of rent for one month
	);

	// Equal is the boundary: at drm == dim both give exactly one month.
	for dim in [28u32, 29, 30, 31] {
		assert_eq!(hand_rolled::prorated_monthly_price(1_000, dim.into(), dim.into()), 1_000);
		assert_eq!(via_crate::prorated_monthly_price(1_000, dim, dim), 1_000);
	}
}

/// `dim == 0` (timestamp outside the representable range) agrees; extreme
/// prices expose the hand-rolled form's saturating under-charge that
/// `payment_math` no longer shares (exact U256 ceil).
#[test]
fn prorate_zero_month_agrees_saturation_diverges() {
	// Calendar failure charges a full month in both.
	assert_eq!(hand_rolled::prorated_monthly_price(100, 0, 0), 100);
	assert_eq!(via_crate::prorated_monthly_price(100, 0, 0), 100);
	// `days_in_month` and `days_remaining_in_month` fail together on the same
	// timestamp, so `dim == 0 && drm != 0` is not reachable — pinned anyway.
	assert_eq!(hand_rolled::prorated_monthly_price(100, 99, 0), 100);
	assert_eq!(via_crate::prorated_monthly_price(100, 99, 0), 100);

	// `dim.saturating_sub(1)` vs `dim - 1`: identical, dim >= 1 in this branch.
	assert_eq!(hand_rolled::prorated_monthly_price(100, 10, 30), 34);
	assert_eq!(via_crate::prorated_monthly_price(100, 10, 30), 34);

	// Hand-rolled saturating mul under-charges a full month at MAX by 31×;
	// payment_math keeps the full-month identity.
	assert_eq!(hand_rolled::prorated_monthly_price(M, 31, 31), M / 31);
	assert_eq!(via_crate::prorated_monthly_price(M, 31, 31), M);
	let rolled_partial = hand_rolled::prorated_monthly_price(M, 30, 31);
	let exact_partial = via_crate::prorated_monthly_price(M, 30, 31);
	assert!(rolled_partial < exact_partial);
	assert!(exact_partial < M); // partial month is strictly less than full price
}

// ── property sweep over the whole domain ─────────────────────────────────────

proptest::proptest! {
	/// The bps substitutions agree with `split` exactly when the product fits,
	/// and understate (never overstate) when it does not.
	#[test]
	fn bps_substitutions_are_exact_below_saturation_and_low_above(x: u128) {
		let rolled_five = hand_rolled::referral_discount(x);
		let exact_five = via_crate::five_percent(x);
		if x <= M / 5 {
			proptest::prop_assert_eq!(rolled_five, exact_five);
		} else {
			proptest::prop_assert!(rolled_five <= exact_five);
		}

		let rolled_ninety_five = hand_rolled::paid_per_month(x);
		let exact_ninety_five = via_crate::ninety_five_percent(x);
		if x <= M / 9_500 {
			proptest::prop_assert_eq!(rolled_ninety_five, exact_ninety_five);
		} else {
			proptest::prop_assert!(rolled_ninety_five <= exact_ninety_five);
		}
	}

	/// Proration agrees whenever the hand-rolled ceil numerator fits in
	/// `u128`. Above that cliff the saturating form understates and
	/// payment_math stays on the exact ceil (never higher than the price).
	#[test]
	fn prorate_agrees_below_product_cliff_and_exact_is_ge_above(
		price: u128,
		dim in 1u32..=31,
		offset in 0u32..=31,
	) {
		let drm = offset.min(dim);
		let rolled = hand_rolled::prorated_monthly_price(price, drm.into(), dim.into());
		let exact = via_crate::prorated_monthly_price(price, drm, dim);
		let days = u128::from(drm);
		let dim_u = u128::from(dim);
		// Hand-rolled path is exact iff both saturating steps are non-saturating.
		let hand_rolled_exact = price
			.checked_mul(days)
			.and_then(|n| n.checked_add(dim_u - 1))
			.is_some();
		if hand_rolled_exact {
			proptest::prop_assert_eq!(rolled, exact);
		} else {
			proptest::prop_assert!(rolled <= exact);
			proptest::prop_assert!(exact <= price);
		}
	}
}

//! Typed money math for the Hippius runtime.
//!
//! Every quantity in the deposit → bank → distribution pipeline gets its own
//! unit type, so mixing units (bytes vs blocks vs prices vs planck) is a
//! compile error rather than a silent bug. Storage keeps raw `u128`; pallets
//! convert at the read/write boundary with `Unit::new(..)` / `.get()`.
//!
//! Every operation is total: it saturates or rounds (direction documented)
//! rather than panicking, and states the conservation invariant its tests
//! enforce.
#![cfg_attr(not(feature = "std"), no_std)]

use primitive_types::{U256, U512};

/// One GiB — the denominator unit of [`UsdPerGibBlock`].
pub const GIB: u128 = 1 << 30;
/// Fixed-point scale shared by USD prices and token planck (18 decimals).
pub const E18: u128 = 1_000_000_000_000_000_000;
/// Denominator of [`BasisPoints`]: 10_000 bps = 100%.
pub const BPS_DENOM: u128 = 10_000;

macro_rules! unit {
	($(#[$doc:meta])* $name:ident) => {
		$(#[$doc])*
		#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
		pub struct $name(u128);

		impl $name {
			/// Wrap a raw `u128` read from storage or config.
			pub const fn new(raw: u128) -> Self {
				Self(raw)
			}

			/// Unwrap to the raw `u128` for storage, events, or transfers.
			pub const fn get(self) -> u128 {
				self.0
			}
		}
	};
}

unit!(/// Shard bytes held by a miner at a point in time.
	Bytes);
unit!(/// Elapsed chain blocks.
	Blocks);
unit!(/// Bytes integrated over blocks — the unit storage is priced in.
	ByteBlocks);
unit!(/// Storage price in USD per GiB per block, fixed-point [`E18`].
	UsdPerGibBlock);
unit!(/// A USD value, fixed-point [`E18`].
	Usd);
unit!(/// Native token amount in planck (18 decimals).
	Tokens);
unit!(/// Marketplace credit amount.
	Credits);

impl core::ops::Mul<Blocks> for Bytes {
	type Output = ByteBlocks;

	/// Byte-blocks accumulated by holding `self` bytes for `rhs` blocks.
	/// Saturates at `u128::MAX` instead of wrapping.
	fn mul(self, rhs: Blocks) -> ByteBlocks {
		ByteBlocks(self.0.saturating_mul(rhs.0))
	}
}

impl ByteBlocks {
	pub const fn saturating_add(self, other: Self) -> Self {
		Self(self.0.saturating_add(other.0))
	}

	pub const fn is_zero(self) -> bool {
		self.0 == 0
	}
}

/// A split ratio in basis points (1/10_000).
///
/// The constructor clamps to 100% so a split part can never exceed the
/// whole — misuse (e.g. `BasisPoints::new(70_000)` meaning 70%) caps the
/// damage at "give everything to the part".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct BasisPoints(u128);

impl BasisPoints {
	pub const fn new(raw: u128) -> Self {
		Self(if raw > BPS_DENOM { BPS_DENOM } else { raw })
	}

	pub const fn get(self) -> u128 {
		self.0
	}
}

/// Currency-like quantities that can be split, prorated, or paid out.
///
/// Implemented only for [`Tokens`] and [`Credits`]; dimensioned units
/// (bytes, blocks, prices) deliberately cannot be split by a ratio.
pub trait Amount: Copy + Ord {
	fn raw(self) -> u128;
	fn from_raw(raw: u128) -> Self;

	fn saturating_add(self, other: Self) -> Self {
		Self::from_raw(self.raw().saturating_add(other.raw()))
	}

	fn saturating_sub(self, other: Self) -> Self {
		Self::from_raw(self.raw().saturating_sub(other.raw()))
	}

	fn is_zero(self) -> bool {
		self.raw() == 0
	}
}

impl Amount for Tokens {
	fn raw(self) -> u128 {
		self.0
	}
	fn from_raw(raw: u128) -> Self {
		Self(raw)
	}
}

impl Amount for Credits {
	fn raw(self) -> u128 {
		self.0
	}
	fn from_raw(raw: u128) -> Self {
		Self(raw)
	}
}

/// Tokens owed for `byte_blocks` at `price` USD per GiB-block, with the
/// token at `token_price` USD (both fixed-point [`E18`]).
///
/// `tokens = byte_blocks × price × 10^18 / (2^30 × token_price)`
///
/// Exact floor division over the full `u128` input domain: the numerator
/// `byte_blocks × price × E18` is up to ~316 bits, so it is formed in
/// [`U512`] via [`U256::full_mul`] rather than saturating in `U256` (which
/// would under-pay once the product exceeded `2^256 − 1`). Rounds down
/// (error < 1 planck), saturates only the *output* at `u128::MAX`. A zero
/// token price yields zero rather than dividing by zero.
pub fn tokens_for(byte_blocks: ByteBlocks, price: UsdPerGibBlock, token_price: Usd) -> Tokens {
	if token_price.0 == 0 {
		return Tokens(0);
	}
	// max(bb)×max(price) fits in U512 via full_mul; ×E18 stays ≤ ~316 bits.
	let num: U512 = U256::from(byte_blocks.0).full_mul(U256::from(price.0)) * U512::from(E18);
	// GIB×token_price ≤ 2^30 × (2^128−1) < 2^158, always fits U256.
	let den = U256::from(GIB) * U256::from(token_price.0);
	// Cap the floor quotient at u128::MAX (min, not a >/>= branch — both
	// comparisons are observationally identical at equality).
	Tokens((num / U512::from(den)).min(U512::from(u128::MAX)).as_u128())
}

/// Pro-rata share of `pool` owed to one claimant with `due` out of
/// `total_due`.
///
/// When the pool covers everything the share is exactly `due`; otherwise
/// `due × pool / total_due`, rounded down — so the sum of all shares never
/// exceeds `pool` and each share never exceeds `due`. The caller keeps
/// `due − share` as arrears, making `share + arrears == due` exact: a
/// shortfall defers value, never destroys it.
///
/// Caller contract: `total_due` must be the sum of all claimants' dues
/// (so `due ≤ total_due`); the conservation claims above assume it. A
/// violation is a caller accounting bug — asserted in debug builds, not
/// masked by capping the result in release.
///
/// Prefer [`pro_rata_wide`] when the true sum of dues may exceed `u128::MAX`:
/// folding with `u128::saturating_add` and then calling this function makes
/// `pool >= total_due` spuriously true and pays every claimant their full
/// `due` (first BTreeMap key wins the bank).
pub fn pro_rata(due: Tokens, pool: Tokens, total_due: Tokens) -> Tokens {
	pro_rata_wide(due, pool, U256::from(total_due.0))
}

/// Like [`pro_rata`], but `total_due` is a wide sum so multi-claimant totals
/// that overflow `u128` still scale fairly. A `u128` pool can never fully cover
/// a total above `u128::MAX`, so that path always uses floor division rather
/// than returning each claimant's full `due`.
pub fn pro_rata_wide(due: Tokens, pool: Tokens, total_due: U256) -> Tokens {
	if total_due.is_zero() {
		return Tokens(0);
	}
	debug_assert!(
		U256::from(due.0) <= total_due,
		"pro_rata_wide caller contract violated: due > total_due",
	);
	if U256::from(pool.0) >= total_due {
		return due;
	}
	// pool < total ⇒ due × pool / total < due ≤ u128::MAX (when due ≤ total).
	let q = U256::from(due.0) * U256::from(pool.0) / total_due;
	Tokens(q.min(U256::from(due.0)).as_u128())
}

/// Wide sum of token dues for settlement totals. Saturates only at `U256::MAX`,
/// so ordinary multi-family extremes that overflow `u128` still produce a
/// faithful denominator for [`pro_rata_wide`].
pub fn sum_dues<I>(dues: I) -> U256
where
	I: IntoIterator<Item = Tokens>,
{
	let mut total = U256::zero();
	for d in dues {
		total = total.saturating_add(U256::from(d.get()));
	}
	total
}

/// Compress a wide due total into the `u128` event/storage domain (caps at
/// `u128::MAX` when the true sum overflowed).
pub fn total_due_for_event(total: U256) -> u128 {
	total.min(U256::from(u128::MAX)).as_u128()
}

/// Split `amount` into `(part, rest)`: `part = amount × ratio / 10_000`
/// rounded down, `rest` the exact remainder.
///
/// Conservation is exact — `part + rest == amount` always, because `rest`
/// comes from subtraction, never a second rounding division: a split can
/// neither mint nor burn value.
pub fn split<A: Amount>(amount: A, part_ratio: BasisPoints) -> (A, A) {
	// ratio ≤ 10_000 (clamped in the constructor) ⇒ part ≤ amount: the
	// U256 quotient fits u128 and the subtraction cannot underflow.
	let part =
		(U256::from(amount.raw()) * U256::from(part_ratio.0) / U256::from(BPS_DENOM)).as_u128();
	(A::from_raw(part), A::from_raw(amount.raw() - part))
}

/// First-month charge for a subscription starting mid-month:
/// `monthly_price × days_remaining / days_in_month`, rounded **up** so a
/// tiny non-zero price never prorates to zero.
///
/// `days_remaining` is clamped to `days_in_month`, so the charge can never
/// exceed one monthly price — a part can never exceed the whole. The ceil is
/// exact over the full `u128` domain (`U256` numerator); saturating `u128`
/// math would under-charge near `u128::MAX` (e.g. a full month at `MAX`
/// wrongly returned `MAX / days_in_month`).
///
/// A zero `days_in_month` (calendar not initialised) charges the full month.
pub fn prorate_first_month<A: Amount>(
	monthly_price: A,
	days_remaining: u32,
	days_in_month: u32,
) -> A {
	if days_in_month == 0 {
		return monthly_price;
	}
	let days = days_remaining.min(days_in_month);
	// days ≤ dim ⇒ ceil(price×days/dim) ≤ price ≤ u128::MAX, so the U256
	// quotient always fits the amount's raw type without a second clamp.
	let dim = U256::from(days_in_month);
	let num = U256::from(monthly_price.raw()) * U256::from(days) + (dim - U256::from(1u8));
	A::from_raw((num / dim).as_u128())
}

/// Balance a payout source can spend without reaping itself: everything
/// above the existential deposit.
pub fn available(balance: Tokens, existential_deposit: Tokens) -> Tokens {
	balance.saturating_sub(existential_deposit)
}

/// Amount actually payable for a request: never more than requested, never
/// more than available.
pub fn payable(requested: Tokens, available: Tokens) -> Tokens {
	requested.min(available)
}

#[cfg(test)]
mod tests {
	use super::*;

	// ── units / BasisPoints ──────────────────────────────────────────────

	#[test]
	fn units_round_trip_raw_values() {
		assert_eq!(Bytes::new(7).get(), 7);
		assert_eq!(Tokens::new(u128::MAX).get(), u128::MAX);
		assert_eq!(Usd::new(0).get(), 0);
		assert_eq!(UsdPerGibBlock::new(E18).get(), E18);
		assert_eq!(Blocks::new(1).get(), 1);
		assert_eq!(ByteBlocks::new(GIB).get(), GIB);
	}

	#[test]
	fn unit_default_is_zero() {
		assert_eq!(Bytes::default().get(), 0);
		assert_eq!(Tokens::default().get(), 0);
		assert_eq!(Credits::default().get(), 0);
		assert_eq!(BasisPoints::default().get(), 0);
	}

	#[test]
	fn basis_points_clamp_at_one_hundred_percent() {
		assert_eq!(BasisPoints::new(0).get(), 0);
		assert_eq!(BasisPoints::new(BPS_DENOM).get(), BPS_DENOM); // exact 100%
		assert_eq!(BasisPoints::new(7_000).get(), 7_000);
		assert_eq!(BasisPoints::new(10_001).get(), BPS_DENOM);
		assert_eq!(BasisPoints::new(u128::MAX).get(), BPS_DENOM);
	}

	// ── accrual ──────────────────────────────────────────────────────────

	const GIB_B: Bytes = Bytes::new(GIB);

	#[test]
	fn accrue_is_bytes_times_blocks() {
		assert_eq!((Bytes::new(0) * Blocks::new(100)).get(), 0);
		assert_eq!((GIB_B * Blocks::new(0)).get(), 0);
		assert_eq!((GIB_B * Blocks::new(100)).get(), GIB * 100);
		assert_eq!((Bytes::new(u128::MAX) * Blocks::new(2)).get(), u128::MAX);
	}

	#[test]
	fn accrue_exact_when_product_fits_u128() {
		// When checked_mul succeeds the typed product must match exactly —
		// not a nearby saturated value.
		assert_eq!((Bytes::new(1) * Blocks::new(1)).get(), 1);
		assert_eq!((Bytes::new(GIB) * Blocks::new(1)).get(), GIB);
		assert_eq!((Bytes::new(1_000) * Blocks::new(1_000)).get(), 1_000_000);
		let a = 1u128 << 40;
		let b = 1u128 << 40;
		assert_eq!(a.checked_mul(b), Some(1u128 << 80));
		assert_eq!((Bytes::new(a) * Blocks::new(b)).get(), 1u128 << 80);
	}

	#[test]
	fn accrue_saturates_at_first_overflow_boundary() {
		// 2^64 × 2^64 = 2^128, which does not fit u128 → saturate.
		assert_eq!((Bytes::new(1u128 << 64) * Blocks::new(1u128 << 64)).get(), u128::MAX);
		// Just under: 2^63 × 2^64 = 2^127 fits.
		assert_eq!((Bytes::new(1u128 << 63) * Blocks::new(1u128 << 64)).get(), 1u128 << 127);
	}

	#[test]
	fn byte_blocks_accumulate_saturating() {
		let a = ByteBlocks::new(u128::MAX - 1);
		assert_eq!(a.saturating_add(ByteBlocks::new(5)).get(), u128::MAX);
		assert_eq!(a.saturating_add(ByteBlocks::new(0)).get(), u128::MAX - 1);
		assert_eq!(a.saturating_add(ByteBlocks::new(1)).get(), u128::MAX);
		assert!(ByteBlocks::new(0).is_zero());
		assert!(!ByteBlocks::new(1).is_zero());
	}

	// ── tokens_for ───────────────────────────────────────────────────────

	const USD: u128 = E18;

	fn t(bb: u128, price: u128, tp: u128) -> u128 {
		tokens_for(ByteBlocks::new(bb), UsdPerGibBlock::new(price), Usd::new(tp)).get()
	}

	#[test]
	fn one_gib_block_at_one_usd_each_is_one_token() {
		assert_eq!(t(GIB, USD, USD), USD);
	}

	#[test]
	fn scales_linearly_with_bytes_price_and_token_price() {
		assert_eq!(t(100 * GIB, 2 * USD, 4 * USD), 50 * USD);
		assert_eq!(t(GIB / 2, USD, USD), USD / 2);
	}

	#[test]
	fn realistic_magnitudes_do_not_overflow() {
		let byte_blocks = 100 * 1024 * GIB * 14_400;
		assert_eq!(t(byte_blocks, USD / 100_000_000, USD / 20), 294_912 * USD / 1_000);
	}

	#[test]
	fn zero_token_price_pays_zero() {
		assert_eq!(t(GIB, USD, 0), 0);
		// All-zero inputs take the early exit, never divide by zero.
		assert_eq!(t(0, 0, 0), 0);
	}

	#[test]
	fn zero_price_or_zero_byte_blocks_pays_zero() {
		assert_eq!(t(GIB, 0, USD), 0);
		assert_eq!(t(0, USD, USD), 0);
	}

	/// Floor to zero: value strictly less than one planck must not round up.
	/// With bb=1, tp=$1 (E18): tokens = price / GIB, so price < GIB ⇒ 0.
	#[test]
	fn sub_planck_accrual_floors_to_zero() {
		assert_eq!(t(1, GIB - 1, USD), 0);
		assert_eq!(t(1, 1, USD), 0);
		// One byte for one block at any price below 1 USD/GiB-block / GIB scale.
		assert_eq!(t(1, 0, USD), 0);
	}

	/// Just crosses one planck: price == GIB, bb=1, tp=$1 ⇒ exactly 1 planck.
	#[test]
	fn one_planck_threshold() {
		assert_eq!(t(1, GIB, USD), 1);
		assert_eq!(t(2, GIB, USD), 2);
		// One GiB-block at 1 planck of USD/GiB-block, token at $1 → 1 planck.
		assert_eq!(t(GIB, 1, USD), 1);
	}

	/// GIB−1 bytes (just under one GiB) floors correctly.
	#[test]
	fn just_under_one_gib_floors() {
		// (GIB−1) × USD × E18 / (GIB × USD) = (GIB−1) × E18 / GIB = E18 − floor(E18/GIB)*?
		// = floor((GIB−1)/GIB × E18) = E18 − 1? No: floor((2^30−1)/2^30 × 10^18).
		let got = t(GIB - 1, USD, USD);
		let expected = ((U512::from(GIB - 1) * U512::from(USD) * U512::from(E18))
			/ (U512::from(GIB) * U512::from(USD)))
		.as_u128();
		assert_eq!(got, expected);
		assert!(got < USD); // strictly less than a full token
	}

	#[test]
	fn extreme_inputs_saturate_at_u128_max() {
		assert_eq!(t(u128::MAX, u128::MAX, 1), u128::MAX);
	}

	/// Quotient lands *exactly* on u128::MAX (not greater) — both `>` and
	/// `>=` branches must return MAX, but this pins the equality case.
	#[test]
	fn tokens_for_output_exactly_u128_max() {
		// GIB × MAX × E18 / (GIB × E18) = MAX.
		assert_eq!(t(GIB, u128::MAX, USD), u128::MAX);
	}

	/// Regression: `bb × price × E18` exceeds `U256::MAX` (~2^256) while the
	/// true quotient still fits in `u128`. Forming the numerator with
	/// saturating `U256` muls would under-pay; `full_mul` → `U512` must not.
	#[test]
	fn tokens_for_exact_when_numerator_exceeds_u256() {
		// 2^100 × 2^100 × 10^18 ≈ 2^259.8 > 2^256; den = 2^30 × 2^50 × 10^18.
		// tokens = 2^200 × E18 / (2^80 × E18) = 2^120.
		let bb = 1u128 << 100;
		let price = 1u128 << 100;
		let tp = (1u128 << 50).saturating_mul(E18);
		assert_eq!(t(bb, price, tp), 1u128 << 120);
	}

	#[test]
	fn tokens_for_minimum_nonzero_token_price() {
		// tp = 1 planck of USD — maximises tokens for a fixed accrual.
		assert_eq!(t(GIB, USD, 1), USD.saturating_mul(USD)); // may saturate
													   // Small accrual still well-defined.
		assert_eq!(t(1, 1, 1), E18 / GIB); // floor(10^18 / 2^30)
	}

	// ── pro_rata ─────────────────────────────────────────────────────────

	#[test]
	fn pro_rata_full_pool_pays_due_exactly() {
		let d = Tokens::new(30);
		assert_eq!(pro_rata(d, Tokens::new(100), Tokens::new(100)), d);
		assert_eq!(pro_rata(d, Tokens::new(500), Tokens::new(100)), d); // surplus pool
	}

	#[test]
	fn pro_rata_pool_exactly_equals_total_due() {
		// Boundary of the full-pool branch: pool == total_due, not greater.
		let due = Tokens::new(40);
		assert_eq!(pro_rata(due, Tokens::new(100), Tokens::new(100)), due);
	}

	#[test]
	fn pro_rata_one_planck_shortfall() {
		// First shortfall step: pool = total − 1 must leave the full-pool path.
		// due=10, total=30, pool=29 → floor(10×29/30) = floor(9.666…) = 9.
		assert_eq!(pro_rata(Tokens::new(10), Tokens::new(29), Tokens::new(30)).get(), 9);
		// Arrears reconstruct.
		let due = Tokens::new(10);
		let share = pro_rata(due, Tokens::new(29), Tokens::new(30));
		assert_eq!(share.get() + due.saturating_sub(share).get(), 10);
	}

	#[test]
	fn pro_rata_shortfall_rounds_down_and_never_overpays() {
		// pool 10 over total 30: floor(10*10/30)=3
		assert_eq!(pro_rata(Tokens::new(10), Tokens::new(10), Tokens::new(30)).get(), 3);
		assert_eq!(pro_rata(Tokens::new(0), Tokens::new(10), Tokens::new(30)).get(), 0);
	}

	/// Sole claimant: share = min(due, pool) when due == total_due.
	#[test]
	fn pro_rata_single_claimant_is_min_of_due_and_pool() {
		let due = Tokens::new(100);
		assert_eq!(pro_rata(due, Tokens::new(100), due).get(), 100); // exact cover
		assert_eq!(pro_rata(due, Tokens::new(250), due).get(), 100); // surplus
		assert_eq!(pro_rata(due, Tokens::new(40), due).get(), 40); // shortfall → whole pool
		assert_eq!(pro_rata(due, Tokens::new(0), due).get(), 0);
		assert_eq!(pro_rata(due, Tokens::new(99), due).get(), 99); // one-planck short
	}

	#[test]
	fn pro_rata_share_plus_arrears_reconstructs_due() {
		// Caller's settlement recipe: arrears = due − share must be exact.
		let due = Tokens::new(10);
		let share = pro_rata(due, Tokens::new(10), Tokens::new(30));
		assert_eq!(share.saturating_add(due.saturating_sub(share)), due);
	}

	#[test]
	fn pro_rata_zero_total_due_pays_zero() {
		assert_eq!(pro_rata(Tokens::new(5), Tokens::new(10), Tokens::new(0)).get(), 0);
	}

	#[test]
	fn pro_rata_zero_pool_pays_zero() {
		assert_eq!(pro_rata(Tokens::new(10), Tokens::new(0), Tokens::new(30)).get(), 0);
	}

	#[test]
	fn pro_rata_near_u128_max_dues() {
		// Large-but-valid partition: two claimants near the top of u128.
		let half = u128::MAX / 2;
		let a = Tokens::new(half);
		let b = Tokens::new(half);
		let total = Tokens::new(half.saturating_mul(2)); // 2*(MAX/2) fits
												   // Full pool.
		assert_eq!(pro_rata(a, total, total), a);
		assert_eq!(pro_rata(b, total, total), b);
		// Shortfall: each gets floor(half * pool / total).
		let pool = Tokens::new(half);
		let share_a = pro_rata(a, pool, total);
		let share_b = pro_rata(b, pool, total);
		assert!(share_a.get() <= half);
		assert!(share_b.get() <= half);
		assert!(share_a.get().saturating_add(share_b.get()) <= half);
		// Sole max claimant.
		let max = Tokens::new(u128::MAX);
		assert_eq!(pro_rata(max, Tokens::new(u128::MAX), max).get(), u128::MAX);
		assert_eq!(pro_rata(max, Tokens::new(1), max).get(), 1);
	}

	#[test]
	fn pro_rata_all_claimants_equal_split_floor() {
		// Three equal dues, pool not divisible by 3 → each floors, dust < 3.
		let due = Tokens::new(10);
		let total = Tokens::new(30);
		let pool = Tokens::new(10);
		let s1 = pro_rata(due, pool, total).get();
		let s2 = pro_rata(due, pool, total).get();
		let s3 = pro_rata(due, pool, total).get();
		assert_eq!(s1, 3); // floor(10*10/30)
		assert_eq!(s1 + s2 + s3, 9);
		let dust = pool.get() - (s1 + s2 + s3);
		assert!(dust < 3, "dust {dust} must stay below the claimant count");
	}

	/// Regression: two `u128::MAX` dues must not be collapsed to total=MAX and
	/// then take the full-cover branch (which would grant each claimant MAX).
	#[test]
	fn pro_rata_wide_two_max_dues_scales_against_true_sum() {
		let max = Tokens::new(u128::MAX);
		let total = sum_dues([max, max]);
		assert!(total > U256::from(u128::MAX));
		let pool = Tokens::new(1_000);
		let a = pro_rata_wide(max, pool, total).get();
		let b = pro_rata_wide(max, pool, total).get();
		// Equal dues → equal floor shares; sum ≤ pool.
		assert_eq!(a, b);
		assert_eq!(a, 500);
		assert!(a.saturating_add(b) <= pool.get());
		// Narrow pro_rata with a saturated total would wrongly return MAX each.
		assert_ne!(pro_rata(max, pool, max).get(), a);
	}

	/// Unequal near-max partition: dust stays in the bank, larger claim takes
	/// almost the whole pool, the 1-planck claim floors to zero.
	#[test]
	fn pro_rata_wide_unequal_max_and_one() {
		let max = Tokens::new(u128::MAX);
		let one = Tokens::new(1);
		let total = sum_dues([max, one]);
		assert_eq!(total, U256::from(u128::MAX) + U256::from(1u8));
		let pool = Tokens::new(u128::MAX);
		let s_max = pro_rata_wide(max, pool, total).get();
		let s_one = pro_rata_wide(one, pool, total).get();
		assert_eq!(s_one, 0); // floor(1 × MAX / (MAX+1)) = 0
		assert_eq!(s_max, u128::MAX - 1); // floor(MAX²/(MAX+1)) = MAX−1
		assert_eq!(s_max.saturating_add(s_one), u128::MAX - 1);
		assert!(s_max.saturating_add(s_one) < pool.get()); // 1 planck dust
	}

	/// Five equal MAX dues against a divisible pool — equal floors, no overpay.
	#[test]
	fn pro_rata_wide_five_max_dues_equal_split() {
		let max = Tokens::new(u128::MAX);
		let dues = [max; 5];
		let total = sum_dues(dues);
		assert_eq!(total, U256::from(u128::MAX) * U256::from(5u8));
		let pool = Tokens::new(1_000);
		let shares: [u128; 5] = core::array::from_fn(|_| pro_rata_wide(max, pool, total).get());
		assert!(shares.iter().all(|&s| s == 200));
		assert_eq!(shares.iter().sum::<u128>(), 1_000);
	}

	#[test]
	fn sum_dues_and_event_compress() {
		assert_eq!(sum_dues([]), U256::zero());
		assert_eq!(sum_dues([Tokens::new(1), Tokens::new(2)]), U256::from(3u128));
		assert_eq!(
			sum_dues([Tokens::new(u128::MAX), Tokens::new(u128::MAX)]),
			U256::from(u128::MAX) * U256::from(2u8)
		);
		assert_eq!(total_due_for_event(U256::from(7u128)), 7);
		assert_eq!(total_due_for_event(U256::from(u128::MAX) + U256::from(1u128)), u128::MAX);
	}

	// ── split ────────────────────────────────────────────────────────────

	#[test]
	fn split_seventy_thirty_matches_distribute_alpha() {
		let (rankings, marketplace) = split(Tokens::new(1_000), BasisPoints::new(7_000));
		assert_eq!((rankings.get(), marketplace.get()), (700, 300));
	}

	#[test]
	fn split_marketplace_production_ratios() {
		// 95% plan face / 5% discount paths used by pallet-marketplace.
		let (kept, discount) = split(Credits::new(10_000), BasisPoints::new(9_500));
		assert_eq!((kept.get(), discount.get()), (9_500, 500));
		let (discount_part, rest) = split(Credits::new(10_000), BasisPoints::new(500));
		assert_eq!((discount_part.get(), rest.get()), (500, 9_500));
		// Odd face value still conserves.
		let (k, d) = split(Credits::new(999), BasisPoints::new(9_500));
		assert_eq!(k.get() + d.get(), 999);
		assert_eq!(k.get(), 999 * 9_500 / 10_000); // floor
	}

	#[test]
	fn split_conserves_odd_amounts_exactly() {
		let (part, rest) = split(Credits::new(101), BasisPoints::new(500)); // 5%
		assert_eq!(part.get(), 5); // floor(101 × 500 / 10_000)
		assert_eq!(part.get() + rest.get(), 101);
	}

	#[test]
	fn split_extremes() {
		let (p, r) = split(Tokens::new(u128::MAX), BasisPoints::new(BPS_DENOM));
		assert_eq!((p.get(), r.get()), (u128::MAX, 0));
		let (p, r) = split(Tokens::new(u128::MAX), BasisPoints::new(0));
		assert_eq!((p.get(), r.get()), (0, u128::MAX));
		// Zero amount.
		let (p, r) = split(Tokens::new(0), BasisPoints::new(7_000));
		assert_eq!((p.get(), r.get()), (0, 0));
		// One planck amount.
		let (p, r) = split(Tokens::new(1), BasisPoints::new(7_000));
		assert_eq!(p.get() + r.get(), 1);
		assert_eq!(p.get(), 0); // floor(1 × 7000 / 10000) = 0
	}

	#[test]
	fn split_works_for_both_amount_types() {
		let (tp, tr) = split(Tokens::new(1_000), BasisPoints::new(2_500));
		let (cp, cr) = split(Credits::new(1_000), BasisPoints::new(2_500));
		assert_eq!((tp.get(), tr.get()), (250, 750));
		assert_eq!((cp.get(), cr.get()), (250, 750));
	}

	#[test]
	fn split_one_basis_point() {
		// Smallest non-zero ratio on a large amount.
		let (p, r) = split(Tokens::new(10_000), BasisPoints::new(1));
		assert_eq!((p.get(), r.get()), (1, 9_999));
		// On amount < 10_000 the part floors to zero for 1 bps.
		let (p, r) = split(Tokens::new(9_999), BasisPoints::new(1));
		assert_eq!((p.get(), r.get()), (0, 9_999));
	}

	// ── prorate_first_month ──────────────────────────────────────────────

	#[test]
	fn prorate_mid_month_rounds_up() {
		// 10 days of 30 at price 100 → ceil(1000/30) = 34
		assert_eq!(prorate_first_month(Credits::new(100), 10, 30).get(), 34);
		// tiny non-zero price never prorates to zero
		assert_eq!(prorate_first_month(Credits::new(1), 1, 31).get(), 1);
	}

	#[test]
	fn prorate_exact_division_does_not_round_up() {
		// 15/30 of 100 = 50 exactly — ceil must not bump a clean quotient.
		assert_eq!(prorate_first_month(Credits::new(100), 15, 30).get(), 50);
		assert_eq!(prorate_first_month(Credits::new(310), 10, 31).get(), 100);
		// 1/1 of anything is itself.
		assert_eq!(prorate_first_month(Credits::new(77), 1, 1).get(), 77);
	}

	#[test]
	fn prorate_full_month_is_full_price() {
		assert_eq!(prorate_first_month(Credits::new(100), 30, 30).get(), 100);
		// Calendar month lengths used in production.
		for dim in [28u32, 29, 30, 31] {
			assert_eq!(prorate_first_month(Credits::new(1_000), dim, dim).get(), 1_000);
		}
	}

	#[test]
	fn prorate_zero_days_remaining_charges_zero() {
		// Calendar says nothing left in the month → no first-month charge.
		assert_eq!(prorate_first_month(Credits::new(100), 0, 30).get(), 0);
	}

	#[test]
	fn prorate_uninitialised_calendar_charges_full_month() {
		assert_eq!(prorate_first_month(Credits::new(100), 0, 0).get(), 100);
		// dim==0 always returns full price regardless of days_remaining.
		assert_eq!(prorate_first_month(Credits::new(100), 99, 0).get(), 100);
	}

	#[test]
	fn prorate_clamps_excess_days_to_one_full_month() {
		assert_eq!(prorate_first_month(Credits::new(100), 60, 30).get(), 100);
		assert_eq!(prorate_first_month(Credits::new(100), u32::MAX, 31).get(), 100);
	}

	#[test]
	fn prorate_zero_price_is_zero() {
		assert_eq!(prorate_first_month(Credits::new(0), 10, 30).get(), 0);
		assert_eq!(prorate_first_month(Tokens::new(0), 31, 31).get(), 0);
	}

	#[test]
	fn prorate_works_for_tokens_as_well_as_credits() {
		// Same Amount path; both types must agree on the number.
		assert_eq!(prorate_first_month(Tokens::new(100), 10, 30).get(), 34);
		assert_eq!(prorate_first_month(Credits::new(100), 10, 30).get(), 34);
	}

	#[test]
	fn prorate_last_day_of_month_is_one_day() {
		// Inclusive "days remaining = 1" on the last day.
		assert_eq!(prorate_first_month(Credits::new(310), 1, 31).get(), 10);
	}

	/// Extreme prices must still ceil exactly in wide arithmetic. The old
	/// saturating-`u128` form charged `MAX/31` for a full 31-day month at
	/// `MAX` — a 31× under-charge that U256 ceil eliminates.
	#[test]
	fn prorate_exact_ceil_near_u128_max() {
		let price = u128::MAX;
		// Full month: identity, not MAX/31.
		assert_eq!(prorate_first_month(Credits::new(price), 31, 31).get(), price);
		// One day of 28: only the + (dim−1) step would overflow u128; exact ceil.
		let one_of_28 = ((U256::from(price) + U256::from(27u8)) / U256::from(28u8)).as_u128();
		assert_eq!(prorate_first_month(Credits::new(price), 1, 28).get(), one_of_28);
		// Partial month at MAX: ceil(MAX×30/31) ≤ MAX.
		let thirty_of_31 = ((U256::from(price) * U256::from(30u8) + U256::from(30u8))
			/ U256::from(31u8))
		.as_u128();
		let out = prorate_first_month(Credits::new(price), 30, 31).get();
		assert_eq!(out, thirty_of_31);
		assert!(out <= price);
		// First price where price×31 overflows u128 still full-months to itself.
		let first_overflowing = u128::MAX / 31 + 1;
		assert!(first_overflowing.checked_mul(31).is_none());
		assert_eq!(
			prorate_first_month(Credits::new(first_overflowing), 31, 31).get(),
			first_overflowing
		);
	}

	// ── available / payable ──────────────────────────────────────────────

	#[test]
	fn available_is_balance_above_existential_deposit() {
		assert_eq!(available(Tokens::new(100), Tokens::new(1)).get(), 99);
		assert_eq!(available(Tokens::new(1), Tokens::new(1)).get(), 0);
		assert_eq!(available(Tokens::new(0), Tokens::new(1)).get(), 0); // no underflow
	}

	#[test]
	fn available_edge_extremes() {
		assert_eq!(available(Tokens::new(u128::MAX), Tokens::new(0)).get(), u128::MAX);
		assert_eq!(available(Tokens::new(u128::MAX), Tokens::new(u128::MAX)).get(), 0);
		assert_eq!(available(Tokens::new(u128::MAX), Tokens::new(1)).get(), u128::MAX - 1);
		assert_eq!(available(Tokens::new(0), Tokens::new(0)).get(), 0);
		assert_eq!(available(Tokens::new(0), Tokens::new(u128::MAX)).get(), 0);
	}

	#[test]
	fn payable_caps_at_both_request_and_availability() {
		assert_eq!(payable(Tokens::new(50), Tokens::new(99)).get(), 50);
		assert_eq!(payable(Tokens::new(150), Tokens::new(99)).get(), 99);
	}

	#[test]
	fn payable_equality_and_zero_edges() {
		assert_eq!(payable(Tokens::new(77), Tokens::new(77)).get(), 77); // req == avail
		assert_eq!(payable(Tokens::new(0), Tokens::new(99)).get(), 0);
		assert_eq!(payable(Tokens::new(99), Tokens::new(0)).get(), 0);
		assert_eq!(payable(Tokens::new(0), Tokens::new(0)).get(), 0);
		assert_eq!(payable(Tokens::new(u128::MAX), Tokens::new(u128::MAX)).get(), u128::MAX);
		assert_eq!(payable(Tokens::new(u128::MAX), Tokens::new(1)).get(), 1);
		assert_eq!(payable(Tokens::new(1), Tokens::new(u128::MAX)).get(), 1);
	}

	// ── Amount trait / is_zero ───────────────────────────────────────────

	#[test]
	fn is_zero_discriminates_zero_from_nonzero() {
		assert!(ByteBlocks::new(0).is_zero());
		assert!(!ByteBlocks::new(1).is_zero());
		assert!(Tokens::new(0).is_zero());
		assert!(!Tokens::new(1).is_zero());
		assert!(Credits::new(0).is_zero());
		assert!(!Credits::new(u128::MAX).is_zero());
	}

	#[test]
	fn amounts_saturate_instead_of_wrapping() {
		assert_eq!(Tokens::new(u128::MAX).saturating_add(Tokens::new(1)), Tokens::new(u128::MAX));
		assert_eq!(Tokens::new(u128::MAX).saturating_add(Tokens::new(0)), Tokens::new(u128::MAX));
		assert_eq!(Credits::new(0).saturating_sub(Credits::new(1)), Credits::new(0));
		assert_eq!(
			Credits::new(u128::MAX).saturating_sub(Credits::new(u128::MAX)),
			Credits::new(0)
		);
		// Symmetric coverage: Credits add-sat and Tokens sub-sat.
		assert_eq!(
			Credits::new(u128::MAX).saturating_add(Credits::new(1)),
			Credits::new(u128::MAX)
		);
		assert_eq!(Tokens::new(0).saturating_sub(Tokens::new(1)), Tokens::new(0));
	}

	#[test]
	fn amount_ord_and_raw_round_trip() {
		assert!(Tokens::new(1) < Tokens::new(2));
		assert!(Credits::new(0) <= Credits::new(0));
		assert_eq!(Tokens::from_raw(42).raw(), 42);
		assert_eq!(Credits::from_raw(u128::MAX).raw(), u128::MAX);
	}

	// ── composition: pricing → pro-rata → bank caps ──────────────────────

	/// Pure-math stand-in for the arion settlement recipe on one family.
	#[test]
	fn composition_pricing_into_pro_rata_into_payable() {
		let bb = 100 * GIB * 1_000; // 100 GiB × 1000 blocks
		let price = 10_000_000_000u128; // $1e-8 / GiB / block
		let tp = USD / 20; // $0.05
		let due = Tokens::new(t(bb, price, tp));
		assert!(!due.is_zero());

		// Two equal families, bank holds half + ED floor.
		let total = due.saturating_add(due);
		let pool = Tokens::new(total.get() / 2);
		let share = pro_rata(due, pool, total);
		assert!(share.get() <= due.get());
		assert!(share.get() <= pool.get());

		// Bank keeps ED=1; request the pro-rata share.
		let bal = pool.saturating_add(Tokens::new(1));
		let avail = available(bal, Tokens::new(1));
		assert_eq!(avail, pool);
		let paid = payable(share, avail);
		assert_eq!(paid, share);
		// Arrears exact.
		assert_eq!(share.saturating_add(due.saturating_sub(share)), due);
	}

	#[cfg(debug_assertions)]
	#[test]
	#[should_panic(expected = "pro_rata_wide caller contract")]
	fn pro_rata_debug_asserts_due_within_total() {
		let _ = pro_rata(Tokens::new(31), Tokens::new(10), Tokens::new(30));
	}
}

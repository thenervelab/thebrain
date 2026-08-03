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

use primitive_types::U256;

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
/// Deterministic integer math via `U256`; rounds down (error < 1 planck),
/// saturates at `u128::MAX`. A zero token price yields zero rather than
/// dividing by zero.
pub fn tokens_for(byte_blocks: ByteBlocks, price: UsdPerGibBlock, token_price: Usd) -> Tokens {
	if token_price.0 == 0 {
		return Tokens(0);
	}
	let num = U256::from(byte_blocks.0)
		.saturating_mul(U256::from(price.0))
		.saturating_mul(U256::from(E18));
	let den = U256::from(GIB).saturating_mul(U256::from(token_price.0));
	let out = num / den;
	if out > U256::from(u128::MAX) {
		Tokens(u128::MAX)
	} else {
		Tokens(out.as_u128())
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn units_round_trip_raw_values() {
		assert_eq!(Bytes::new(7).get(), 7);
		assert_eq!(Tokens::new(u128::MAX).get(), u128::MAX);
	}

	#[test]
	fn basis_points_clamp_at_one_hundred_percent() {
		assert_eq!(BasisPoints::new(7_000).get(), 7_000);
		assert_eq!(BasisPoints::new(10_001).get(), BPS_DENOM);
		assert_eq!(BasisPoints::new(u128::MAX).get(), BPS_DENOM);
	}

	const GIB_B: Bytes = Bytes::new(GIB);

	#[test]
	fn accrue_is_bytes_times_blocks() {
		assert_eq!((Bytes::new(0) * Blocks::new(100)).get(), 0);
		assert_eq!((GIB_B * Blocks::new(0)).get(), 0);
		assert_eq!((GIB_B * Blocks::new(100)).get(), GIB * 100);
		assert_eq!((Bytes::new(u128::MAX) * Blocks::new(2)).get(), u128::MAX);
	}

	#[test]
	fn byte_blocks_accumulate_saturating() {
		let a = ByteBlocks::new(u128::MAX - 1);
		assert_eq!(a.saturating_add(ByteBlocks::new(5)).get(), u128::MAX);
		assert!(ByteBlocks::new(0).is_zero());
	}

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
	}

	#[test]
	fn extreme_inputs_saturate_at_u128_max() {
		assert_eq!(t(u128::MAX, u128::MAX, 1), u128::MAX);
	}

	#[test]
	fn amounts_saturate_instead_of_wrapping() {
		assert_eq!(Tokens::new(u128::MAX).saturating_add(Tokens::new(1)), Tokens::new(u128::MAX));
		assert_eq!(Credits::new(0).saturating_sub(Credits::new(1)), Credits::new(0));
	}
}

# Payment Math Crate Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Load the `rust-style` skill BEFORE the first Rust edit, and the `property-based-testing` skill before Task 9.

**Goal:** Extract all money math on `feat/arion-miner-payments` into a typed, property-tested `no_std` crate `primitives/payment-math`, then wire arion, bank, and marketplace through it with no behavior change.

**Architecture:** Unit newtypes over `u128` (`Bytes`, `Blocks`, `ByteBlocks`, `UsdPerGibBlock`, `Usd`, `Tokens`, `Credits`, `BasisPoints`) with explicit `new`/`get` at the storage boundary; pure total functions (`tokens_for`, `pro_rata`, `split`, `prorate_first_month`, `available`, `payable`) carrying documented conservation invariants; proptest properties enforce each invariant. Design: `docs/plans/2026-08-03-payment-math-design.md`.

**Design addendum (discovered during planning):** marketplace splits operate on *credits*, not tokens, so the crate adds a `Credits` unit and a sealed-ish `Amount` trait (implemented only for `Tokens` and `Credits`) making `split`/`prorate_first_month` generic. Dimensioned units (bytes, blocks, prices) deliberately do not implement it.

**Tech Stack:** Rust (workspace toolchain), `primitive-types::U256` (workspace dep, same type as `sp_core::U256`), `proptest` (dev-dep, native tests only), `cargo-mutants` for mutation check.

---

## Task 1: Crate scaffold

**Files:**
- Create: `primitives/payment-math/Cargo.toml`
- Create: `primitives/payment-math/src/lib.rs`
- Modify: `Cargo.toml` (workspace members + `[workspace.dependencies]`)

**Step 1: Create `primitives/payment-math/Cargo.toml`**

```toml
[package]
name = "payment-math"
version = { workspace = true }
authors = { workspace = true }
edition = { workspace = true }
license = { workspace = true }
homepage = { workspace = true }
repository = { workspace = true }
description = "Typed, tested money math for the Hippius runtime"

[dependencies]
primitive-types = { workspace = true }

[features]
default = ["std"]
std = ["primitive-types/std"]
```

**Step 2: Create `primitives/payment-math/src/lib.rs`** (skeleton only)

```rust
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
```

**Step 3: Register in workspace `Cargo.toml`**

Add `"primitives/payment-math"` to `[workspace] members` (after `"primitives/ext"`), and under `[workspace.dependencies]` (near `hippius-primitives`, line ~329):

```toml
payment-math = { path = "primitives/payment-math", default-features = false }
```

**Step 4: Add proptest dev-dependency at current stable version**

Run: `cargo add proptest --dev -p payment-math` (never hardcode a remembered version).

**Step 5: Verify it builds, both std and no_std**

Run: `cargo check -p payment-math && cargo check -p payment-math --no-default-features`
Expected: both succeed with zero warnings.

**Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock primitives/payment-math
git commit -m "feat(payment-math): scaffold no_std money-math crate"
```

## Task 2: Unit types, `BasisPoints`, `Amount` trait

**Files:**
- Modify: `primitives/payment-math/src/lib.rs`

**Step 1: Write failing tests** (append `#[cfg(test)] mod tests`)

```rust
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

    #[test]
    fn amounts_saturate_instead_of_wrapping() {
        assert_eq!(Tokens::new(u128::MAX).saturating_add(Tokens::new(1)), Tokens::new(u128::MAX));
        assert_eq!(Credits::new(0).saturating_sub(Credits::new(1)), Credits::new(0));
    }
}
```

**Step 2: Run** `cargo test -p payment-math` — expected: FAIL (types not defined).

**Step 3: Implement** (before the tests module)

```rust
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
```

**Step 4: Run** `cargo test -p payment-math` — expected: PASS. Also `cargo clippy -p payment-math --all-targets -- -D warnings`.

**Step 5: Commit** — `git commit -m "feat(payment-math): unit newtypes, BasisPoints, Amount trait"`

## Task 3: Accrual (`Bytes × Blocks → ByteBlocks`)

**Step 1: Failing tests** (port of arion's `accrue_is_bytes_times_blocks`)

```rust
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
```

**Step 2: Run — FAIL. Step 3: Implement**

```rust
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
```

**Step 4: Run — PASS. Step 5: Commit** `feat(payment-math): typed byte-block accrual`

## Task 4: Pricing (`tokens_for`)

**Step 1: Failing tests** — port ALL anchors from `pallets/arion-pallet/src/lib.rs:3018-3069` verbatim numbers:

```rust
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
```

**Step 2: Run — FAIL. Step 3: Implement** (body identical to the arion original — this is a move, not a rewrite):

```rust
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
```

**Step 4: PASS. Step 5: Commit** `feat(payment-math): move token pricing math from pallet-arion`

## Task 5: Pro-rata

**Step 1: Failing tests**

```rust
#[test]
fn pro_rata_full_pool_pays_due_exactly() {
    let d = Tokens::new(30);
    assert_eq!(pro_rata(d, Tokens::new(100), Tokens::new(100)), d);
    assert_eq!(pro_rata(d, Tokens::new(500), Tokens::new(100)), d); // surplus pool
}

#[test]
fn pro_rata_shortfall_rounds_down_and_never_overpays() {
    // pool 10 over total 30: shares 3+3+... floor(10*10/30)=3
    assert_eq!(pro_rata(Tokens::new(10), Tokens::new(10), Tokens::new(30)).get(), 3);
    assert_eq!(pro_rata(Tokens::new(0), Tokens::new(10), Tokens::new(30)).get(), 0);
}

#[test]
fn pro_rata_zero_total_due_pays_zero() {
    assert_eq!(pro_rata(Tokens::new(5), Tokens::new(10), Tokens::new(0)).get(), 0);
}
```

**Step 3: Implement**

```rust
/// Pro-rata share of `pool` owed to one claimant with `due` out of
/// `total_due`.
///
/// When the pool covers everything the share is exactly `due`; otherwise
/// `due × pool / total_due`, rounded down — so the sum of all shares never
/// exceeds `pool` and each share never exceeds `due`. The caller keeps
/// `due − share` as arrears, making `share + arrears == due` exact: a
/// shortfall defers value, never destroys it.
pub fn pro_rata(due: Tokens, pool: Tokens, total_due: Tokens) -> Tokens {
    if total_due.0 == 0 {
        return Tokens(0);
    }
    if pool.0 >= total_due.0 {
        return due;
    }
    // pool < total_due ⇒ due × pool / total_due < due ≤ u128::MAX, so the
    // U256 quotient always fits back into u128.
    Tokens((U256::from(due.0) * U256::from(pool.0) / U256::from(total_due.0)).as_u128())
}
```

**Steps 2/4: red then green. Step 5: Commit** `feat(payment-math): pro-rata shortfall split with conservation`

## Task 6: Split

**Step 1: Failing tests**

```rust
#[test]
fn split_seventy_thirty_matches_distribute_alpha() {
    let (rankings, marketplace) = split(Tokens::new(1_000), BasisPoints::new(7_000));
    assert_eq!((rankings.get(), marketplace.get()), (700, 300));
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
}
```

**Step 3: Implement**

```rust
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
```

**Step 5: Commit** `feat(payment-math): exact-conservation basis-point split`

## Task 7: Proration

**Step 1: Failing tests** (behavior copied from `pallets/marketplace/src/lib.rs:1231-1247`)

```rust
#[test]
fn prorate_mid_month_rounds_up() {
    // 10 days of 30 at price 100 → ceil(1000/30) = 34
    assert_eq!(prorate_first_month(Credits::new(100), 10, 30).get(), 34);
    // tiny non-zero price never prorates to zero
    assert_eq!(prorate_first_month(Credits::new(1), 1, 31).get(), 1);
}

#[test]
fn prorate_full_month_is_full_price() {
    assert_eq!(prorate_first_month(Credits::new(100), 30, 30).get(), 100);
}

#[test]
fn prorate_uninitialised_calendar_charges_full_month() {
    assert_eq!(prorate_first_month(Credits::new(100), 0, 0).get(), 100);
}
```

**Step 3: Implement**

```rust
/// First-month charge for a subscription starting mid-month:
/// `monthly_price × days_remaining / days_in_month`, rounded **up** so a
/// tiny non-zero price never prorates to zero.
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
    let dim = u128::from(days_in_month);
    let num = monthly_price.raw().saturating_mul(u128::from(days_remaining));
    A::from_raw(num.saturating_add(dim - 1) / dim)
}
```

**Step 5: Commit** `feat(payment-math): round-up first-month proration`

## Task 8: Bank availability

**Step 1: Failing tests**

```rust
#[test]
fn available_is_balance_above_existential_deposit() {
    assert_eq!(available(Tokens::new(100), Tokens::new(1)).get(), 99);
    assert_eq!(available(Tokens::new(1), Tokens::new(1)).get(), 0);
    assert_eq!(available(Tokens::new(0), Tokens::new(1)).get(), 0); // no underflow
}

#[test]
fn payable_caps_at_both_request_and_availability() {
    assert_eq!(payable(Tokens::new(50), Tokens::new(99)).get(), 50);
    assert_eq!(payable(Tokens::new(150), Tokens::new(99)).get(), 99);
}
```

**Step 3: Implement**

```rust
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
```

**Step 5: Commit** `feat(payment-math): bank availability + payable caps`

## Task 9: Property tests

Load the `property-based-testing` skill first. Create `primitives/payment-math/tests/properties.rs` (integration test, std-only, so proptest never enters the no_std build):

```rust
use payment_math::{
    available, payable, pro_rata, prorate_first_month, split, tokens_for, Amount, BasisPoints,
    Blocks, ByteBlocks, Bytes, Credits, Tokens, BPS_DENOM, E18, GIB,
};
use primitive_types::U256;
use proptest::prelude::*;

proptest! {
    #[test]
    fn accrual_zero_annihilates_and_saturates(x in any::<u128>()) {
        prop_assert_eq!((Bytes::new(0) * Blocks::new(x)).get(), 0);
        prop_assert_eq!((Bytes::new(x) * Blocks::new(0)).get(), 0);
        prop_assert!((Bytes::new(x) * Blocks::new(u128::MAX)).get() >= x.min(u128::MAX));
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
        let tokens = tokens_for(
            ByteBlocks::new(bb), payment_math::UsdPerGibBlock::new(price), payment_math::Usd::new(tp),
        ).get();
        prop_assert!(U256::from(tokens) * den <= num);
        prop_assert!(num < (U256::from(tokens) + U256::from(1u8)) * den);
    }

    /// Bank-never-overpays: for any partition of total_due, the pro-rata
    /// shares sum to at most the pool, and each claimant's share + arrears
    /// reconstructs their due exactly.
    #[test]
    fn pro_rata_shares_never_exceed_pool_and_conserve_due(
        dues in prop::collection::vec(0u128..=(1u128 << 100), 1..20),
        pool in any::<u128>(),
    ) {
        let total: u128 = dues.iter().fold(0u128, |a, v| a.saturating_add(*v));
        prop_assume!(total > 0 && total < u128::MAX); // saturated totals are not a partition
        let pool_t = Tokens::new(pool);
        let total_t = Tokens::new(total);
        let mut sum = 0u128;
        for d in &dues {
            let share = pro_rata(Tokens::new(*d), pool_t, total_t);
            prop_assert!(share.get() <= *d);
            let arrears = Tokens::new(*d).saturating_sub(share);
            prop_assert_eq!(share.get() + arrears.get(), *d);
            sum += share.get(); // each share ≤ its due, Σ dues = total < MAX
        }
        prop_assert!(sum <= total);
        prop_assert!(sum <= pool || pool >= total);
        if pool >= total {
            prop_assert_eq!(sum, total); // full pool ⇒ everyone paid in full
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
    }

    #[test]
    fn prorate_bounded_by_full_month_and_rounds_up(
        price in 0u128..=(u128::MAX / 31),
        drm in 0u32..=31,
        dim in 1u32..=31,
    ) {
        prop_assume!(drm <= dim);
        let out = prorate_first_month(Credits::new(price), drm, dim).get();
        prop_assert!(out <= price);
        // ceil bound: out is the smallest integer with out × dim ≥ price × drm
        let (p, d, m) = (U256::from(price), U256::from(drm), U256::from(dim));
        prop_assert!(U256::from(out) * m >= p * d);
        prop_assert!(U256::from(out) * m < p * d + m);
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
```

Run: `cargo test -p payment-math` — expected: PASS. If a property fails, treat it as a real finding: minimize, understand, fix the *implementation or the property* deliberately (superpowers:systematic-debugging), never loosen a conservation bound to make it pass.

**Commit:** `test(payment-math): property tests for all money-math invariants`

## Task 10: Wire pallet-arion

**Files:**
- Modify: `pallets/arion-pallet/Cargo.toml` (add dep + std feature)
- Modify: `pallets/arion-pallet/src/lib.rs:27` (drop `U256` from `sp_core` import — it becomes unused), `:1456-1467` (`accrue_miner_bytes`), `:1473-1591` (`settle_miner_payments`), `:2985-3069` (delete moved functions + tests module)

**Step 1:** Cargo.toml: `payment-math = { workspace = true }` in `[dependencies]`; `"payment-math/std"` in the `std` feature list.

**Step 2:** Delete `accrue_byte_blocks`, `tokens_for_byte_blocks`, and `mod payment_math_tests` (lib.rs:2985-3069). Add `use payment_math::{Amount, Blocks, ByteBlocks, Bytes, Tokens, Usd, UsdPerGibBlock};`.

**Step 3:** Retype `accrue_miner_bytes` — conversions only at the storage boundary:

```rust
fn accrue_miner_bytes(uid: u32, now: BlockNumberFor<T>) {
    let prev_bytes =
        Bytes::new(MinerStatsByUid::<T>::get(uid).map(|s| s.shard_data_bytes).unwrap_or(0));
    MinerAccruals::<T>::mutate(uid, |acc| {
        let mut a = acc.take().unwrap_or(MinerAccrual { byte_blocks: 0, last_block: now });
        let elapsed = Blocks::new(now.saturating_sub(a.last_block).saturated_into::<u128>());
        a.byte_blocks = ByteBlocks::new(a.byte_blocks).saturating_add(prev_bytes * elapsed).get();
        a.last_block = now;
        *acc = Some(a);
    });
}
```

**Step 4:** Retype `settle_miner_payments`: `family_due` becomes `BTreeMap<T::AccountId, Tokens>`; `price`/`token_price` wrapped at read (`UsdPerGibBlock::new(..)`, `Usd::new(..)`, zero checks via `.get() == 0`); `tokens = payment_math::tokens_for(ByteBlocks::new(byte_blocks), price, token_price)`; `total_due` folded with `Amount::saturating_add`; the U256 pro-rata expression (lib.rs:1546-1551) becomes:

```rust
let pool = Tokens::new(T::PayoutSource::available().saturated_into::<u128>());
// …
let pay = payment_math::pro_rata(*due, pool, total_due);
```

(drop the old `.min(total_due)` on pool — `pro_rata` returns `due` whenever `pool ≥ total_due`, same result); `arrears = due.saturating_sub(paid)`; `.get()` only at `FamilyArrears` writes, `request_payment`, and event fields.

**Step 5:** `cargo test -p pallet-arion` (if the pallet has unit tests) and `cargo check -p pallet-arion` — zero warnings (the `U256`/`sp_core` import must not dangle).

**Step 6: Commit** `refactor(arion): route accrual and settlement through payment-math`

## Task 11: Wire pallet-marketplace

**Files:**
- Modify: `pallets/marketplace/Cargo.toml` (dep + std feature, as Task 10)
- Modify: `pallets/marketplace/src/lib.rs:1089` (referral discount), `:1231-1247` (`prorated_monthly_price`), `:1404`, `:1533` (95% referral charge), `:1758` (5% commission), `:2191-2203` (`distribute_alpha`)

Import: `use payment_math::{split, prorate_first_month, BasisPoints, Credits, Tokens, Amount};`

**Step 1:** `distribute_alpha` (lib.rs:2197-2203):

```rust
// 70% ranking / 30% marketplace, conserving alpha_to_release exactly.
// (The old checked_mul(70).unwrap_or_default() sent 100% to the
// marketplace on overflow; split computes in U256 and cannot overflow.)
let (rankings, marketplace) = split(Tokens::new(alpha_to_release), BasisPoints::new(7_000));
let rankings_amount = rankings.get();
let marketplace_amount = marketplace.get();
```

**Step 2:** Referral discount (lib.rs:1089): `let discount = split(Credits::new(face_credits), BasisPoints::new(500)).0.get();` — same floor value as `×5/100`, but exact where the old `saturating_mul(5)` saturated. Same replacement for the commission at lib.rs:1758.

**Step 3:** 95% charge (lib.rs:1404 and 1533): `split(Credits::new(plan.price), BasisPoints::new(9_500)).0.get()`.

**Step 4:** `prorated_monthly_price` (lib.rs:1231-1247) becomes a one-line delegate:

```rust
fn prorated_monthly_price(monthly_price: u128) -> u128 {
    prorate_first_month(
        Credits::new(monthly_price),
        u32::from(pallet_calendar::Pallet::<T>::days_remaining_in_current_month()),
        u32::from(pallet_calendar::Pallet::<T>::days_in_current_month()),
    )
    .get()
}
```

**Step 5:** `cargo check -p pallet-marketplace` — zero warnings.

**Step 6: Commit** `refactor(marketplace): route splits and proration through payment-math`

## Task 12: Wire pallet-bank

**Files:**
- Modify: `pallets/bank/Cargo.toml` (dep + std feature)
- Modify: `pallets/bank/src/lib.rs:173-175` (`available_for_payout`), `:222` (`request_payment` cap)

```rust
pub fn available_for_payout() -> BalanceOf<T> {
    payment_math::available(
        Tokens::new(T::Currency::free_balance(&Self::account_id()).saturated_into()),
        Tokens::new(T::Currency::minimum_balance().saturated_into()),
    )
    .get()
    .saturated_into()
}
```

and in `request_payment`: `let paid = payment_math::payable(Tokens::new(amount.saturated_into()), Tokens::new(Self::available_for_payout().saturated_into())).get().saturated_into();`

Run: `cargo test -p pallet-bank` — the existing mock-runtime tests must pass unchanged.

**Commit** `refactor(bank): route availability caps through payment-math`

## Task 13: Verification gate

1. `cargo test -p payment-math` — all unit + property tests pass.
2. `cargo test -p pallet-bank` — mock tests pass.
3. `cargo test -p hippius-mainnet-runtime --test miner_payments` — the end-to-end no-behavior-change safety net.
4. `cargo clippy -p payment-math -p pallet-arion -p pallet-bank -p pallet-marketplace --all-targets -- -D warnings` — zero warnings.
5. `cargo fmt -p payment-math -- --check` (pallets: match surrounding style by hand; do not repo-wide fmt).
6. `cargo check -p payment-math --no-default-features` — no_std still builds.
7. Mutation check: `cargo mutants -p payment-math` (install with `cargo install --locked cargo-mutants` if missing). Every surviving mutant in an arithmetic operator gets a test that kills it.
8. Use superpowers:verification-before-completion before claiming done; final commit of any fixes.

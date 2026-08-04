# Payment math crate — design

Date: 2026-08-03
Branch: feat/arion-miner-payments
Status: approved

## Problem

All money math on this branch — arion accrual/pricing/pro-rata, bank
availability, marketplace revenue splits and proration — operates on bare
`u128`. Bytes, blocks, byte-blocks, USD fixed-point (10^18), token planck,
and split ratios share one type, so a swapped argument (e.g. passing the
token price where the storage price belongs) compiles silently. The
arithmetic is scattered across three pallets as inline expressions, only
partially tested.

## Decision

A new `no_std` crate `primitives/payment-math`, depended on by
`pallet-arion`, `pallet-bank`, and `pallet-marketplace`. Storage, events,
and extrinsics keep `u128` (no SCALE/metadata change, no migration); all
arithmetic goes through unit newtypes, converting explicitly at the
boundary.

## Type vocabulary

Newtypes over `u128`, `Copy`, no blanket `From`/`Into` in arithmetic
position — construction and extraction are explicit (`Bytes::new(x)`,
`.get()`):

| Type | Meaning | Today's bare form |
|---|---|---|
| `Bytes` | stored shard bytes | `shard_data_bytes: u128` |
| `Blocks` | elapsed block count | `now - last_block` |
| `ByteBlocks` | bytes integrated over blocks | `byte_blocks: u128` |
| `UsdPerGibBlock` | storage price, fixed-point 10^18 | `MinerPriceUsdPerGbBlock` |
| `Usd` | token price, fixed-point 10^18 | `TokenPriceUsd` |
| `Tokens` | token planck (18 decimals) | `tokens`, `due`, `arrears`, `pool` |
| `BasisPoints` | split ratio, /10_000 | `70/100`, `5/100`, `9_500/10_000` literals |

Only meaningful combinations exist: `Bytes × Blocks → ByteBlocks` is the
sole constructor of `ByteBlocks`; `Tokens` supports only
`saturating_add/sub`, `min`, pro-rata, and split.

## Operations and invariants

All operations are total: no panics, saturation or documented rounding
instead.

- **Accrual** `Bytes × Blocks → ByteBlocks`, saturating.
  Invariants: zero-annihilation, monotone in both arguments, saturates.
- **Pricing** `tokens_for(ByteBlocks, UsdPerGibBlock, Usd) → Tokens` via
  `U256`: `byte_blocks × price × 10^18 / (2^30 × token_price)`.
  Invariants: linear up to rounding; rounds down, error < 1 planck; zero
  token price → zero; saturating.
- **Pro-rata** `pro_rata(due, pool, total_due) → Tokens`.
  Invariants: `pool ≥ total_due` → share = due; share ≤ due;
  Σ shares ≤ pool; `share + arrears == due` exactly (value only deferred,
  never destroyed).
- **Split** `split(amount, BasisPoints) → (part, rest)`.
  Invariants: `part + rest == amount` exactly (rest by subtraction, never
  a second division); `part ≤ amount` for bps ≤ 10_000.
- **Proration** `prorate_first_month(monthly_price, days_remaining,
  days_in_month)` keeping the existing round-up-numerator behavior.
  Invariants: ≤ monthly price when `days_remaining ≤ days_in_month`;
  ≥ round-down quotient; full month → full price.
- **Bank** `available(balance, existential_deposit)` and
  `payable(requested, available) = min`.
  Invariants: `payable ≤ requested`, `payable ≤ available`, no underflow.

## Test plan

In-crate, native (no mock runtime):

1. **proptest** — one property per invariant, over full `u128` range plus
   a realistic-magnitudes strategy (TiB bytes, e18 prices, day-scale
   blocks). Conservation properties are the priority: they catch the
   fund-loss class found in the PR #36 review.
2. **Example tests** — migrate the arion `payment_math_tests` anchors
   (1 GiB at $1 → 1 token; 294.912-token realistic case; saturation).
3. **Mutation check** — `cargo mutants -p payment-math` to confirm the
   suite kills arithmetic mutations.

## Wiring (no behavior change intended)

- **arion**: delete in-pallet `accrue_byte_blocks` /
  `tokens_for_byte_blocks`, import from the crate;
  `settle_miner_payments` converts at read/write boundaries; inline
  `due × pool / total_due` becomes `pro_rata`.
- **marketplace**: `distribute_alpha` 70/30 → `split(alpha, bps(7_000))`;
  5% discount and 95% referral literals likewise;
  `calculate_first_month_price` → `prorate_first_month`.
- **bank**: `available_for_payout` and the `min` in `request_payment`
  route through `available` / `payable`.

Existing runtime integration tests
(`runtime/mainnet/tests/miner_payments.rs`) keep every assertion unchanged
as the no-behavior-change safety net; their five calls to the deleted
`pallet_arion::tokens_for_byte_blocks` are mechanically renamed to a local
delegate that calls `payment_math::tokens_for`.

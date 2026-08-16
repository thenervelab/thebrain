# `pallet-compute-scoring`

The on-chain settlement layer for Hippius DePIN **compute** (confidential
SEV-SNP tenant VMs). It is the authoritative store the validator (`vali`)
writes to and the scheduler reads from. Spec of record: `ARCHITECTURE.md`
§13 / §22 / §23.

It owns five concerns:

| Concern | What it holds | Written by |
|---|---|---|
| **Reward weights** | `EpochWeights[epoch][node_id] → u128` — per-miner reward ∝ what each miner actually hosts | `vali_submit_epoch_close` (root) |
| **Miner status** | `MinerStatuses` — the §13 `Active / Quarantined / Decommissioned` state machine | `vali_submit_epoch_close` / `set_miner_status` |
| **Registration** | family/child anti-Sybil registration + native-deposit lockup/unbonding | `register_child` / `deregister_child` |
| **Stake** | `$`-denominated, hosting-proportional collateral (oracle + asymmetric EMA) | `set_alpha_per_usd` + `top_up_stake` / … |
| **Slashing + Marketplace** | kill-switch-gated slashing; miner-set, capped, **announced-ahead** prices | `slash_stake` / `announce_price_change` |

The stake / slashing / marketplace layers are **default-OFF** — see
[Operational toggles](#operational-toggles). Design notes:
`docs/design/marketplace-stake-slashing.md`.

---

## Installing the pallet in a runtime

> **The `construct_runtime!` name MUST be exactly `ComputeScoring`.** The
> off-chain reader (`binaries/ticket-validator`) derives the storage prefix
> from `twox_128("ComputeScoring")`; any other name makes every read return
> zero miners. This is the single most important install constraint.

### 1. Add the dependency

In the runtime crate's `Cargo.toml`:

```toml
[dependencies]
pallet-compute-scoring = { path = "../pallets/compute-scoring", default-features = false }
# (or, cross-repo:)
# pallet-compute-scoring = { git = "https://github.com/thenervelab/hippius-compute", default-features = false }
```

And propagate the features it exposes (`std`, `try-runtime`):

```toml
[features]
std = [
    # …
    "pallet-compute-scoring/std",
]
try-runtime = [
    # …
    "pallet-compute-scoring/try-runtime",
]
```

### 2. Implement `Config`

The pallet decouples the heavy `pallet-registration` / `pallet-proxy`
wiring behind four runtime-bound trait abstractions
(`FamilyRegistry`, `ProxyVerifier`, `NodeRegistrationProvider`,
`RankingsSink`) so test runtimes can bind `()` / stand-ins. A production
runtime binds them to the real pallets.

```rust
impl pallet_compute_scoring::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;

    // --- Origins ---
    // Privileged admin (toggles, oracle, price/stake/slash params).
    // Council / sudo in production:
    type ComputeScoringAdminOrigin = EnsureRoot<AccountId>;
    // The single authoritative validator submitter (epoch close + audit).
    type AuditAuthorityOrigin = EnsureRoot<AccountId>;

    // --- Currency (deposits + stake reserve/slash) ---
    type DepositCurrency = Balances;

    // --- Runtime-bound abstractions (bind to the real pallets) ---
    type FamilyRegistry = Registration;        // = pallet_registration::Pallet<Runtime>
    type ProxyVerifier  = Proxy;               // = pallet_proxy::Pallet<Runtime>
    type Registration   = Registration;        // NodeRegistrationProvider
    type RankingsSink    = ();                  // see "RankingsSink" below

    // --- Registration economics ---
    type MaxFamilies                  = ConstU32<4096>;
    type MaxChildrenTotal             = ConstU32<65536>;
    type MaxChildrenPerFamily         = ConstU32<256>;
    type BaseChildDeposit             = BaseChildDeposit;          // BalanceOf
    type GlobalDepositHalvingPeriodBlocks = ConstU32<{ 7 * DAYS }>;
    type UnregisterCooldownBlocks     = ConstU32<{ 1 * DAYS }>;
    type UnbondingPeriodBlocks        = ConstU32<{ 7 * DAYS }>;

    // --- Audit-VM signed aggregates (§I) ---
    type MaxAggregateBody    = ConstU32<8192>;
    type MaxValidatorIdLen   = ConstU32<64>;
    type MaxFamilyIdLen      = ConstU32<64>;
    type MaxAuditVmKeyIdLen  = ConstU32<64>;
    type ComputePalletInstance = ComputePalletInstance;  // [u8; 32] replay domain
    type ComputeChainGenesis   = ComputeChainGenesis;    // [u8; 32] replay domain
    type NowUnix             = Timestamp;                // impl UnixTime

    // --- Epoch close batching ---
    // Bounds the per-call batch (and thus the close weight). 64–128 is a
    // sane start; paginate if the network outgrows one batch per epoch.
    type MaxMinerStatusUpdatesPerCall = ConstU32<128>;

    // --- Live attestation (§322) ---
    type MaxLiveAttestationBody  = ConstU32<1024>;
    type MaxVmIdLen              = ConstU32<64>;
    type MaxKbsAttestationPubkeys = ConstU32<8>;

    type WeightInfo = pallet_compute_scoring::weights::SubstrateWeight<Runtime>;
}
```

A complete, compiling reference `Config` lives in
[`src/mock.rs`](src/mock.rs) — copy it as the starting template.

#### The four runtime-bound traits

- **`FamilyRegistry`** — anti-Sybil "is this a registered owner / a
  validator node?" Bind to `pallet_registration::Pallet<Runtime>` (the
  pallet ships the impl behind `T: pallet_registration::Config`).
- **`ProxyVerifier`** — "has `family` authorised `child` as a proxy?" Bind
  to `pallet_proxy::Pallet<Runtime>`.
- **`NodeRegistrationProvider`** (`Registration`) — node-registration
  lookups used by the audit/attestation gates.
- **`RankingsSink`** — called once per epoch close with the full weight
  snapshot. **Recommended production binding is `()` (no-op)**: off-chain
  validators read `Pallet::epoch_weights_for(epoch)` and dispatch the
  ranking update themselves. If you bind a real sink, it MUST be `O(n)`
  bounded and its cost reflected in the `WeightInfo` benchmark (it runs
  inside `vali_submit_epoch_close`, whose weight is parameterised by `n`).

### 3. Add to `construct_runtime!`

```rust
construct_runtime!(
    pub enum Runtime {
        System: frame_system,
        Balances: pallet_balances,
        Timestamp: pallet_timestamp,
        // …
        // NAME MUST BE EXACTLY `ComputeScoring` (off-chain reader prefix):
        ComputeScoring: pallet_compute_scoring,
    }
);
```

### 4. Genesis (optional)

```rust
GenesisConfig {
    compute_scoring: ComputeScoringConfig {
        base_child_deposit: Some(1_000_000_000_000u128),
        lockup_enabled: true,
    },
    // …
}
```

The stake / slashing / marketplace storage has safe `ValueQuery` defaults
(off / zero), so they need no genesis — they are enabled post-upgrade via
the admin extrinsics below.

### 5. Weights

The pallet ships [`src/weights.rs`](src/weights.rs) with the `WeightInfo`
trait and a `SubstrateWeight<T>` impl (conservative hand-set weights — the
extrinsics are O(1) except `vali_submit_epoch_close`, which is bounded by
`MaxMinerStatusUpdatesPerCall`). Use it in `Config`:

```rust
type WeightInfo = pallet_compute_scoring::weights::SubstrateWeight<Runtime>;
// or, for tests:        type WeightInfo = ();
```

> Dedicated FRAME benchmarks (`benchmarking.rs` + a `runtime-benchmarks`
> feature) are a follow-up. The bundled `SubstrateWeight` is adequate for a
> default-off deployment; generate hardware-specific weights before
> enabling the economic layers at scale.

---

## Operational toggles

All three economic layers ship **OFF** so a runtime upgrade is behaviour-
neutral; ops enables them via `ComputeScoringAdminOrigin` once ready:

| Layer | Enable | Configure |
|---|---|---|
| Registration lockup | `set_lockup_enabled(true)` | `set_base_child_deposit` |
| **Stake** | `set_stake_enabled(true)` | `set_alpha_per_usd` (oracle), `set_stake_floor`, `set_ema_permille` |
| **Slashing** | `set_slashing_enabled(true)` | `set_slash_beneficiary` |
| **Marketplace** | (per-miner `announce_price_change`) | `set_price_bounds`, `set_price_change_policy` |

While a layer is OFF it is inert (the oracle/EMA still track, staking still
works, but eligibility never blocks and no balance is ever burned).

---

## The off-chain half

This pallet is the on-chain settlement layer only. It does **not** compute
merit. The validator (`vali`, the Django app in `vali/`) is the producer:

- `vali/apps/scheduler/scoring.py` derives each miner's reward weight from
  the tenant VMs it actually hosts and submits it via
  `chain.submit_epoch_close` → `vali_submit_epoch_close`.
- The scheduler reads the authoritative state back via
  `binaries/ticket-validator read-miner-status` (the `twox_128("ComputeScoring")`
  consumer).

---

## Build & test

```bash
cargo build  -p pallet-compute-scoring
cargo test   -p pallet-compute-scoring          # mock + 77 unit/conformance tests
cargo clippy -p pallet-compute-scoring --all-targets
```

The tests include storage-layout conformance vectors that pin the byte
shapes the off-chain reader decodes — keep them green when touching
storage.

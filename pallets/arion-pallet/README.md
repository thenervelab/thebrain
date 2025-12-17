# Arion Pallet (FRAME) — `pallet-arion`

This is a **standalone scaffold** for a Substrate/FRAME pallet that stores:

- A deterministic **CRUSH input map** (what clients need to compute placement) **per epoch**
- Periodic **miner stats** (strikes / aggregated heartbeats / bandwidth buckets) every **N blocks**

It is **not wired into this repository’s Rust workspace** (to avoid pulling Substrate deps into the storage node workspace).  
You should copy this folder into your Substrate node repository (or add it as a git submodule) and include it in your runtime.

## What goes on-chain

### 1) CRUSH map (placement-critical)
Stored only when `epoch` changes.

- `epoch`
- `pg_count`
- `ec_k`, `ec_m`
- `miners[]` where each miner includes:
  - `uid`
  - `node_id` (32 bytes)
  - `weight`
  - `family_id` (**AccountId**, shown as SS58 in UIs)
  - `endpoint` fields (bounded bytes; you decide your canonical encoding)
  - optional `http_addr` (bounded bytes)

The pallet also stores a `map_root` (hash of canonical SCALE encoding of the miner list) for quick verification.

### 1b) Optional: on-chain child/node registration (anti-spoof / anti-sybil)

This pallet also includes **registration primitives** for miner “child” accounts under a “family” account:

- **Spoof resistance**: `node_id` is an ed25519 public key (32 bytes). Registration requires a valid signature from the
  corresponding private key.
- **Anti-Sybil adaptive pricing**:
  - **First child is free per family (one-time)**.
  - After that, the required deposit is **global** (network-wide) and **doubles** after each paid registration.
  - The global deposit **halves** after each `GlobalDepositHalvingPeriodBlocks` of inactivity (lazy, only computed on registration).
- **Anti-yoyo**:
  - `deregister_child` places the child into **unbonding** and applies a **cooldown**.
  - The deposit remains reserved until `claim_unbonded` after `UnbondingPeriodBlocks`.

#### Registration signature payload

The pallet verifies `node_sig` over the SCALE-encoded bytes:

- `("ARION_NODE_REG_V1", family, child, node_id, nonce)`

Where `nonce` is stored per `node_id` in `NodeIdNonce` (prevents replay across registration cycles).

#### Hooks to integrate with your chain

The pallet is standalone, so chain-specific rules are exposed as hooks in `Config`:

- `FamilyRegistry`: **required** — wire to your family/registration pallet to validate `family` exists / is allowed.
- `ProxyVerifier`: **required** — wire to your `pallet-proxy` rules (non-transfer proxy type for children).

**Security:** both hooks are **deny-by-default** (return `false` for `()`). If you leave them as `()` in your runtime, `register_child` will always fail with `FamilyNotRegistered` or `ProxyVerificationFailed`. This prevents accidental misconfiguration from allowing unauthorized registrations.

### 2) Miner stats (non-placement-critical)
Submitted periodically (e.g. every 300 blocks) as **aggregates** via `submit_miner_stats(bucket, updates, network_totals)`:
- Per-miner (by `uid`): `MinerStats { shard_count, shard_data_bytes, strikes, last_seen_bucket, bandwidth_bytes, integrity_fails }`
- Optional network totals: `NetworkTotals { total_shards, total_shard_data_bytes, total_bandwidth_bytes }`

This pallet’s `MinerStats` also supports validator-observed storage totals:
- `shard_count` (total stored shard/blob count)
- `shard_data_bytes` (total stored bytes)

And optional **network-wide totals** for the current stats bucket:
- `total_shards`
- `total_shard_data_bytes`
- `total_bandwidth_bytes`

### 3) Incentive weights (non-cheatable by miners; validator-reported)

This pallet stores **final incentive weights per family** (submit to Bittensor), computed deterministically from
validator-reported **per-node quality metrics**.

#### Why per-family weights?

- Placement uses **per-node** weights (CRUSH).
- Economic incentives and anti-sybil policy typically want **per-family** weights (one identity / failure domain).

#### Weight update flow

- The validator calls `submit_node_quality(bucket, updates)` where each update is:
  - `(child_account_id, NodeQuality { shard_data_bytes, bandwidth_bytes, uptime_permille, strikes, integrity_fails })`
  - the pallet computes `node_weight_u16` deterministically on-chain, then computes `family_weight_u16`
- The pallet derives the `family_id` from on-chain registration (`child -> family`), so updates **cannot spoof families**.

#### Node weight formula (high level)

Node weights are computed using a **concave** scoring to avoid runaway “rich get richer” behavior:
- `log2(1 + bandwidth_bytes)` (dominant term)
- `log2(1 + shard_data_bytes)` (secondary term)
Then:
- multiply by `uptime_permille / 1000`
- subtract penalties for `strikes` and `integrity_fails`

All parameters are constants in `Config`:
- `NodeBandwidthWeightPermille`, `NodeStorageWeightPermille`, `NodeScoreScale`
- `StrikePenalty`, `IntegrityFailPenalty`

#### Anti “rich get richer” / anti “family flooding”

Per-family weight is computed from node weights using:

- **Top-N** nodes per family (`FamilyTopN`)
- With **rank decay** (`FamilyRankDecayPermille`)
  - Example: `800` means the 2nd node contributes 0.8x, 3rd contributes 0.8², etc.

This encourages adding some capacity, but makes “add infinite children” ineffective.

#### Stability / anti-yoyo

After computing raw per-family weight, the pallet applies:

- **EMA smoothing** (`FamilyWeightEmaAlphaPermille`)
- **Delta clamp per bucket** (`MaxFamilyWeightDeltaPerBucket`)

This prevents sudden weight spikes from transient behavior and promotes stable service.

#### Newcomer entry

During a short grace window (`NewcomerGraceBuckets`), if a family’s computed raw weight is > 0, the pallet applies a
small floor (`NewcomerFloorWeight`). This gives new miners a path to “get scheduled”, without rewarding totally inactive
families (raw must be > 0).

## Integration steps ( Hippius chain )

1. Add this crate to your runtime workspace.
2. In `runtime/Cargo.toml`, add:
   - `pallet-arion = { path = "../arion-pallet", default-features = false }`
3. In runtime `construct_runtime!`, include `Arion: pallet_arion`.
4. Configure:
   - `ArionAdminOrigin` (set to `EnsureRoot` / sudo)
   - `MapAuthorityOrigin` (e.g. `EnsureRoot` or a dedicated council/validator origin)
   - `StatsAuthorityOrigin`
   - `WeightAuthorityOrigin` (validator set / governance)
   - **`FamilyRegistry`** — wire to `pallet_registration::Pallet::<Runtime>` (or your family pallet)
   - **`ProxyVerifier`** — wire to `pallet_proxy::Pallet::<Runtime>` (ensures family -> child proxy relationship)
   - **`DepositCurrency`** — wire to `pallet_balances::Pallet::<Runtime>` (for deposit reserve/unreserve)
   - registration-related constants:
     - `MaxFamilies` (e.g. 256)
     - `MaxChildrenTotal` (e.g. 2000)
     - `MaxChildrenPerFamily`
     - `BaseChildDeposit`
     - `GlobalDepositHalvingPeriodBlocks` (e.g. 14_400 @ 6s blocks ≈ 24h)
     - `UnregisterCooldownBlocks`
     - `UnbondingPeriodBlocks`
   - `MaxMiners`, `MaxEndpointLen`, `MaxHttpAddrLen`, `MaxStatsUpdates`
   - weight-related constants:
     - `MaxNodeWeight` / `MaxFamilyWeight` (caps)
     - `FamilyTopN`, `FamilyRankDecayPermille`
     - `FamilyWeightEmaAlphaPermille`, `MaxFamilyWeightDeltaPerBucket`
     - `NewcomerGraceBuckets`, `NewcomerFloorWeight`
     - node scoring constants:
       - `NodeBandwidthWeightPermille`, `NodeStorageWeightPermille`, `NodeScoreScale`
       - `StrikePenalty`, `IntegrityFailPenalty`

## Notes

- This pallet is intentionally conservative:
  - Rejects epoch regressions
  - Enforces unique miner UIDs
  - Encourages stable, bounded encodings for addresses/ids
  - Provides optional guard `EnforceRegisteredMinersInMap` to require miners in CRUSH maps are registered + active

## Admin (sudo) extrinsics

This pallet includes sudo/admin-controlled extrinsics (configure `AdminOrigin = EnsureRoot`):

- `set_lockup_enabled(enabled: bool)`
  - toggles whether `register_child` reserves deposits (lockup) and whether unbonding delays apply
- `set_base_child_deposit(deposit)`
  - sets the **base deposit floor** used by the global registration fee curve



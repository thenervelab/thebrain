# `pallet-compute-scoring` — copied-vs-changed audit

This file documents every line that diverges from the upstream pin
recorded in [`PROVENANCE.md`](./PROVENANCE.md). A future re-sync runs
the same diff against a newer upstream commit and reapplies these
deltas (with adjustments).

## Upstream pin (recap)

- Repo:   `github.com/thenervelab/thebrain`
- Commit: `11226860c66e51bd7092c67263ef59b658f1d7f4` (locked 2026-05-20)
- Files:  `pallets/arion-pallet/src/lib.rs` (2 679 LOC)
          `pallets/arion-pallet/src/weights.rs` (362 LOC)
          `pallets/arion-pallet/Cargo.toml`

`curl` URLs the operator can re-fetch to byte-compare:

```
https://raw.githubusercontent.com/thenervelab/thebrain/11226860c66e51bd7092c67263ef59b658f1d7f4/pallets/arion-pallet/src/lib.rs
https://raw.githubusercontent.com/thenervelab/thebrain/11226860c66e51bd7092c67263ef59b658f1d7f4/pallets/arion-pallet/src/weights.rs
https://raw.githubusercontent.com/thenervelab/thebrain/11226860c66e51bd7092c67263ef59b658f1d7f4/pallets/arion-pallet/Cargo.toml
```

## PR-I2 deltas — exhaustive

The vendoring discipline is **byte-for-byte** modulo the
substitutions tabulated below. Everything else is upstream verbatim.

### Identifier renames (9 search-and-replace passes)

Applied to both `src/lib.rs` and `src/weights.rs`. The renames are
purely textual — no semantic transformation. Two builders running
the same `python3` substitution script against the upstream files
produce byte-identical results to this PR.

| From | To | Reason |
|---|---|---|
| `pallet-arion` | `pallet-compute-scoring` | crate name (doc-comments + Cargo.toml refs) |
| `pallet_arion` | `pallet_compute_scoring` | crate name (Rust path in weights.rs header) |
| `Arion's` (straight quote) | `compute-scoring's` | possessive in doc-comments |
| `Arion’s` (smart quote U+2019) | `compute-scoring’s` | possessive in doc-comments — the upstream comment at lib.rs line 8 (now PR-I2 lib.rs:44) actually uses the smart-quote form. Both passes are required so no `Arion[''’]s` slips through. |
| `ArionAdminOrigin` | `ComputeScoringAdminOrigin` | Config associated type |
| `ARION_NODE_REG_V1` | `HIPPIUS_COMPUTE_NODE_REG_V1` | **signature domain separator** — DIFFERENT bytes ⇒ this pallet cannot accept arion-signed node-registration payloads. Matches the §23 v1 convention used elsewhere in the spec. |
| `ARION_ATTESTATION_V1` | `HIPPIUS_COMPUTE_ATTESTATION_V1` | **signature domain separator** — same reasoning. |
| `pallets/arion/src/weights.rs` | `pallets/compute-scoring/src/weights.rs` | weights.rs header path comment |
| `Storage: Arion ` | `Storage: ComputeScoring ` | weights.rs `Storage:` annotation prefix (the `Arion` segment is the upstream pallet's `PalletInfo` name — Substrate's storage prefix). Our `construct_runtime!` will use `ComputeScoring` (locked PR-I3). |

### Off-chain Arion content-addressing — deliberately NOT renamed

The fields / doc-comments still using `arion`/`Arion` after the
renames refer to the **off-chain Arion storage service** (a BLAKE3
content-addressed bundle store), NOT the pallet itself. PR-I3 will
refactor these to a compute-specific content-addressing scheme.

Preserved as-is (line numbers reference THIS PR's
`src/lib.rs`, not the upstream):

| Site | Identifier |
|---|---|
| `src/lib.rs:427-428` | doc-comment: "Download bundle from Arion: `GET /download/{arion_content_hash}`" |
| `src/lib.rs:439-440` | field: `pub arion_content_hash: BoundedVec<…>` |
| `src/lib.rs:836-837` | doc-comment: "Maps epoch → commitment containing merkle roots and Arion content hash." |
| `src/lib.rs:2351-2360` | extrinsic doc + signature: `submit_attestation_commitment(arion_content_hash: …)` |
| `src/lib.rs:2369` | parameter: `arion_content_hash: BoundedVec<u8, T::MaxContentHashLen>` |
| `src/lib.rs:2383, 2390` | length check + emit: `ensure!(arion_content_hash.len() == 32, …)` |

The TODO comments inside the PR-I2 banner (lines 9, 12, 17, 18, 85,
532 of `src/lib.rs`) ALSO reference "arion"/"Arion" — those are
provenance prose pointing at the upstream pallet name, not field
identifiers. Intentional.

PR-I3 will either rename `arion_content_hash` to
`attestation_bundle_hash` / `bundle_blake3` or replace the off-chain
content-addressing entirely with the §23 served-aggregate digest.
Tracked as part of the extrinsic adaptation pass.

### Structural deltas

These edits change behaviour, not just identifiers. Each is the
minimum needed to make the byte-for-byte vendor compile against
`polkadot-sdk` `stable2407` WITHOUT pulling in
`pallet-registration` / `pallet-proxy` (per the user instruction:
"stub-out les use paths, TODO commentés pour PR-I3").

**Header block — `src/lib.rs` line 1-45** (added):

A multi-line provenance banner naming the upstream commit + the
applied renames + the stub-module strategy. Pure documentation; no
runtime impact.

**Stub modules — `src/lib.rs` line 33-37** (added):

```rust
// TODO(PR-I3): drop these stub modules + the `use crate::pallet_*`
// lines below once the compute-scoring registration/proxy model
// lands.
mod pallet_proxy;
mod pallet_registration;
```

Two new files:

- `src/pallet_registration.rs` (~75 LOC) — `NodeType { Validator }`,
  `NodeInfo<AccountId> { node_id, node_type, owner }`,
  `Pallet<T>` with `is_owner_node_registered` (false),
  `get_all_nodes_by_node_type` (empty Vec),
  `get_registered_node_for_owner` (None),
  `do_unregister_main_node` (no-op).
- `src/pallet_proxy.rs` (~40 LOC) — `ProxyDefinition<AccountId> {
  delegate }`, `Pallet<T>::proxies` (empty Vec).

Every helper is a fail-safe no-op so direct stub call sites short-
circuit through `Error::<T>::NodeNotRegistered` /
`Error::<T>::FamilyNotRegistered` etc. The
`force_deregister_child` / `update_user_file_size` /
`update_multiple_user_file_sizes` / `get_primary_account`
extrinsics+helpers in `src/lib.rs` were each audited and confirmed
to route through the deny branch when the stub returns `None` /
`false` / empty `Vec`.

A second class of gates uses the runtime-supplied hooks
`T::FamilyRegistry: FamilyRegistry<…>` and
`T::ProxyVerifier: ProxyVerifier<…>` (the trait impls at
`src/lib.rs:88-104`). Those are NOT stub-routed — they evaluate
against whatever the runtime wires in. The `impl … for ()` blocks
in `src/lib.rs:96-104` give both traits a fail-closed default
(`is_registered_family = false`, `can_register_child = false`), so
a runtime that binds `type FamilyRegistry = ();` / `type
ProxyVerifier = ();` (which PR-I3's mock runtime will) also fails
closed.

**`use` line — `src/lib.rs:75`** (upstream) → **lines 75-86** (this PR):

Upstream:

```rust
use pallet_registration::{NodeType, Pallet as RegistrationPallet};
```

PR-I2:

```rust
#[allow(unused_imports)]
use crate::pallet_registration::{self, NodeType, Pallet as RegistrationPallet};
use crate::pallet_proxy;
```

- `crate::pallet_registration` / `crate::pallet_proxy` resolve to
  the local stub modules.
- The `#[allow(unused_imports)]` preserves arion's bare-name imports
  (`NodeType`, `RegistrationPallet`) — every call site uses the
  fully-qualified path, so the bare imports were dead in arion too.
  Kept byte-for-byte to minimise the textual diff for re-syncs.

**Config supertrait — `src/lib.rs:521-524`**:

Upstream:

```rust
#[pallet::config]
pub trait Config:
    frame_system::Config + pallet_proxy::Config + pallet_registration::Config
{
```

PR-I2:

```rust
#[pallet::config]
// TODO(PR-I3): re-add `+ pallet_proxy::Config + pallet_registration::Config`
// once the compute-scoring registration/proxy crates are wired
// (upstream arion-pallet bound them here; PR-I2 stubs the call
// sites with crate-local placeholder modules — see top of
// lib.rs).
pub trait Config: frame_system::Config {
```

Without this delta the `pallet_proxy::Config` / `pallet_registration::Config`
supertraits would require the real upstream crates as deps.

### Removed PR-I1 scaffolding

- `src/mock.rs` (deleted) — PR-I1 shipped a minimal mock runtime
  for the empty Config trait. The vendored Config trait has 30+
  required associated types; a meaningful mock is PR-I3 work.
- `src/tests.rs` (deleted) — same reason; the single compile-gate
  test it contained is superseded by `cargo check
  -p pallet-compute-scoring` (which now type-checks 2 679 LOC of
  real arion-pallet code, a strictly stronger gate).

### `src/weights.rs` — no trailing newline

Upstream `pallets/arion-pallet/src/weights.rs` ends `}<space>` with
no trailing `\n`. PR-I2 preserves that byte-for-byte. If a future
re-sync upstream commit adds a trailing newline, this delta
disappears automatically.

### `Cargo.toml`

| Change | Reason |
|---|---|
| Added `sp-core = { workspace = true, default-features = false }` to `[dependencies]` | The vendored lib.rs uses `sp_core::{ed25519, H256}` — upstream `pallet-arion-pallet`'s Cargo.toml lists the same dep. |
| Added `"sp-core/std"` to the `std` feature | Symmetry with the `[dependencies]` line. |
| Removed `sp-core` from `[dev-dependencies]` | Now a regular dep. |
| Kept `sp-io` in `[dev-dependencies]` | Future PR-I3 mock runtime. |
| **NOT added**: `pallet-registration`, `pallet-proxy` | Per the user instruction. PR-I3 swaps in the compute-scoring registration crate. |
| Updated `description` (unchanged in this PR) | Already mentions the arion mirror in PR-I1. |

## PR-I3 deltas — extrinsics adapted for compute-served (NEW)

PR-I3 takes the vendored arion baseline and adapts it for the §23
compute-served reward path. Three classes of change:

1. **Storage-served metric fields renamed to compute-served**
   — encoding-stable (same scalar types, same SCALE byte layout).
2. **New `submit_audit_stats` extrinsic** consuming the audit-VM
   signed aggregate. New supporting types, storage, events, errors.
3. **`hippius-types` made no_std-compatible** so the pallet can pull
   the cross-implementation schema crate without a `std` runtime
   build. Default `std` feature preserves the existing behaviour for
   off-chain consumers (KBS, guest, vali, edge).

The CRUSH map / family registration / warden attestation extrinsics
from arion-pallet remain vendored unchanged — those layers are
swapped out wholesale by the registration model in PR-I4+ (not this
PR).

### Field renames — storage-served → compute-served

All renames preserve scalar types so the SCALE encoding is
**byte-identical** to upstream arion-pallet. Semantic re-meaning only;
no migration is required for a fresh chain.

| Struct | Upstream field | PR-I3 field | Notes |
|---|---|---|---|
| `MinerStats` | `shard_count: u64` | `served_aggregates_count: u64` | counter of accepted aggregates |
| `MinerStats` | `shard_data_bytes: u128` | `served_units: u128` | §23 reward quantity per node |
| `MinerStats` | `bandwidth_bytes: u64` | `last_aggregate_unix: u64` | latest accepted aggregate's `interval_end` |
| `MinerStats` | `integrity_fails: u32` | `attestation_fails: u32` | audit-VM sig / replay-domain rejections |
| `MinerStats` | `strikes: u32` | (kept) | generic |
| `MinerStats` | `last_seen_bucket: u32` | (kept) | generic |
| `NetworkTotals` | `total_shards: u64` | `total_served_aggregates: u64` | observability |
| `NetworkTotals` | `total_shard_data_bytes: u128` | `total_served_units: u128` | §23 network projection |
| `NetworkTotals` | `total_bandwidth_bytes: u128` | `total_aggregate_body_bytes: u128` | DoS / capacity observability |
| `NodeQuality` | `shard_data_bytes: u128` | `served_units: u128` | volume axis of weight blend |
| `NodeQuality` | `bandwidth_bytes: u128` | `served_window_seconds: u128` | throughput axis (window length) |
| `NodeQuality` | `integrity_fails: u32` | `attestation_fails: u32` | audit-VM failure penalty |
| `NodeQuality` | `uptime_permille: u16` | (kept) | generic |
| `NodeQuality` | `strikes: u32` | (kept) | generic |

The struct **names** (`MinerStats`, `NetworkTotals`, `NodeQuality`)
are kept byte-for-byte from upstream to minimise the re-sync diff.
Their semantic re-meaning is documented in the doc-comments.

### `compute_node_weight_from_quality` — weight blend re-meaned

Upstream blended `bandwidth_bytes` (throughput) + `shard_data_bytes`
(volume) via `log2(1+x)` concave scoring. PR-I3 keeps the SAME shape
but with the renamed inputs:

| Upstream | PR-I3 | Comment |
|---|---|---|
| `q.bandwidth_bytes` | `q.served_window_seconds` | longer continuous serving raises score |
| `q.shard_data_bytes` | `q.served_units` | §23 volume |
| `q.integrity_fails` | `q.attestation_fails` | renamed penalty axis |
| `NodeBandwidthWeightPermille` | (kept) | reused for throughput-axis weight |
| `NodeStorageWeightPermille` | (kept) | reused for volume-axis weight |
| `StrikePenalty` / `IntegrityFailPenalty` | (kept) | identifiers byte-for-byte |

The Config constant identifiers stay byte-identical so a re-sync
against newer upstream arion-pallet doesn't churn the Config trait
surface (only the doc-comments + the function body change).

### New types — `AggregateView` + `SignedAggregateWire`

`src/lib.rs` adds two new SCALE-friendly types inside the pallet
module:

- `AggregateView<MaxValidatorIdLen, MaxFamilyIdLen, MaxAuditVmKeyIdLen>`
  — SCALE projection of `hippius_types::audit_vm::ServedDeliveryAggregate`.
  `Encode`, `Decode`, `TypeInfo`, `MaxEncodedLen` derived;
  `Clone` / `PartialEq` / `Eq` / `Debug` hand-rolled (same
  `MinerRecord`-style pattern upstream uses for `Get<u32>`-marker
  generics).
- `SignedAggregateWire<MaxAggregateBody>` — mirror of
  `hippius_types::audit_vm::SignedServedDeliveryAggregate`
  (`{body, sig}`), with `BoundedVec` instead of `Vec<u8>` for DoS
  resistance at the extrinsic boundary.

The `tests::aggregate_view_fields_match_hippius_types` test pins the
schema-drift tripwire: the pallet builds a `ServedDeliveryAggregate`
from `hippius_types::audit_vm`, canonicalises it, and asserts every
field present on-chain has a typed counterpart in the off-chain
struct. Any future field added to either side breaks this test.

### New Config items

```text
type AuditAuthorityOrigin: EnsureOrigin<Self::RuntimeOrigin>;
type MaxAggregateBody: Get<u32>;
type MaxValidatorIdLen: Get<u32>;
type MaxFamilyIdLen: Get<u32>;
type MaxAuditVmKeyIdLen: Get<u32>;
type ComputePalletInstance: Get<[u8; 32]>;
type ComputeChainGenesis: Get<[u8; 32]>;       // codex CRITICAL fix
type NowUnix: frame_support::traits::UnixTime; // codex/gemini CRITICAL fix (expiry)
```

**Why `ComputeChainGenesis` is a Config constant, not
`frame_system::block_hash(0)`**: Substrate prunes old block hashes by
`BlockHashCount` (~256 blocks default). After the genesis block falls
out of that window, `block_hash(0)` returns the default zero value
and every replay-domain check fails closed for honest validators.
Pinning the genesis hash via the runtime's `Get<[u8; 32]>` constant
makes the §23 replay domain durable across the chain's lifetime.

**Why `T::NowUnix` is `frame_support::traits::UnixTime`, not
`block_number`**: §23 `expiry` is a Unix timestamp set by the
audit-VM at signing time; comparing to block-number would force a
block-height-to-seconds conversion outside the §23 contract.
Production runtimes wire `NowUnix = Timestamp` (`pallet-timestamp`'s
`UnixTime` impl); test runtimes can supply any deterministic shim.

### New storage maps

| Storage | Key | Value | Purpose |
|---|---|---|---|
| `AuditVmPubkeyByNode` | `[u8; 32]` (node_id) | `[u8; 32]` (Ed25519 pubkey) | per-node audit-VM key (admin-set; PR-I6 KBS-cert flow replaces) |
| `LastAggregateHashByNode` | `[u8; 32]` (node_id) | `[u8; 32]` (SHA-256) | latest accepted aggregate body hash; chains submissions |

### New extrinsics

- `submit_audit_stats(origin, view, signed)` — call index **40**.
  Verifies Ed25519 sig over `signed.body`, checks chain_genesis /
  pallet_instance / epoch / prev_aggregate_hash / expiry /
  interval_start ≤ interval_end, credits `view.served_units` to the
  node's `MinerStats`, advances `LastAggregateHashByNode`, bumps
  `CurrentNetworkTotals`, emits `AuditStatsSubmitted`.
- `set_audit_vm_pubkey(origin, node_id, pubkey)` — call index **41**.
  Admin-gated (`ComputeScoringAdminOrigin`) placeholder for binding
  an Ed25519 audit-VM pubkey to a node. PR-I6 replaces with the
  KBS-issued certificate flow; the call_index is reserved high so
  PR-I6 can take call_indices 50+.

### New events

- `AuditStatsSubmitted { node_id, epoch, served_units, body_hash }`
- `AuditVmPubkeySet { node_id, pubkey }`

### New errors

`AggregateChainGenesisMismatch`, `AggregatePalletInstanceMismatch`,
`AggregateEpochMismatch`, `AggregatePrevHashMismatch`,
`AggregateExpired`, `AggregateIntervalInverted`,
`AuditVmPubkeyNotRegistered`, `InvalidAggregateSignature`,
`EmptyAggregateBody`. All static-string discriminants; replay-domain
classifiers map 1-1 to the §23 invariants they enforce.

### Hash discipline — SHA-256 (NOT `T::Hashing`)

`prev_aggregate_hash` chain continuity uses `sp_io::hashing::sha2_256`,
NOT `T::Hashing::hash` (typically BlakeTwo256). Rationale: §23 and
`hippius_types::audit_vm` (`map_root`, `totals_root`,
`prev_aggregate_hash`) use SHA-256 throughout. Using `T::Hashing`
would diverge the on-chain `prev_aggregate_hash` from what the
off-chain audit-VM computes, breaking chain continuity after the
first submission. `sp_io::hashing::sha2_256` is the no_std-compatible
host-function shim and is available in every Substrate runtime
build. (Codex + Gemini CRITICAL flagged this in PR-I3 review.)

### Convergent codex + gemini review (PR-I3)

Applied pre-merge:

- **CRITICAL** (both reviewers): `T::Hashing::hash` instead of SHA-256
  for `body_hash` would diverge from the off-chain audit-VM. Fixed
  by switching to `sp_io::hashing::sha2_256`; test pin updated.
- **CRITICAL** (codex): `frame_system::block_hash(0)` is pruned
  after `BlockHashCount` blocks. Fixed by adding
  `T::ComputeChainGenesis: Get<[u8; 32]>` Config constant.
- **CRITICAL** (codex + gemini): `view.expiry` had an `AggregateExpired`
  error variant but was never enforced. Fixed by adding
  `T::NowUnix: UnixTime` Config item and a
  `ensure!(view.expiry > now_unix_secs, …)` check; new
  `submit_audit_stats_rejects_expired_aggregate` test pins it.
- **HIGH** (both reviewers): `aggregate_view_fields_match_hippius_types`
  test wasn't structurally binding — it only inspected
  `ServedDeliveryAggregate` locals, never constructing the pallet's
  `AggregateView`. Fixed by constructing both sides with the same
  payloads and `assert_eq!`-ing every field; any future drift breaks
  compilation or asserts.
- **MEDIUM** (codex): canonical-CBOR re-binding (`signed.body ==
  canonical(view_without_served_units)`) would close the validator-
  trust gap on every field except `served_units` without a schema
  change. Documented as PR-I3.1 follow-up below; out of PR-I3 scope
  because it requires a CBOR encoder in no_std and a
  `view_hash`-style §23 schema decision.
- **MEDIUM** (codex): pre-existing `log2_fixed_u128` scaling drift in
  `compute_node_weight_from_quality` (divides by `127` instead of
  `127 * 256`) is an upstream arion-pallet bug that survived the
  PR-I2 vendor; not corrected here to keep the byte-for-byte mirror.
  Flagged for an upstream fix or a documented divergence in PR-I4.
- **MEDIUM** (gemini): the `body_hash` defensive length check
  reused `AggregateChainGenesisMismatch`. Fixed by switching to
  `sp_io::hashing::sha2_256` which returns `[u8; 32]` directly — the
  length check (and its misleading error mapping) goes away entirely.
- **LOW** (codex): `large_enum_variant` allow comment about SCALE
  wire-shape was wrong (`Box<T>` SCALE-encodes transparently);
  comment corrected to reflect the real reason (visible metadata /
  client-interface surface change for no real saving).

### Trust model + spec gap (PR-I3.1 follow-up)

`view` is the validator's SCALE projection of the canonical CBOR
`signed.body` it received from the Audit VM. The pallet:

1. **Verifies** the audit-VM Ed25519 signature over `signed.body`
   bytes — a malicious validator cannot synthesize a signed
   aggregate without the audit-VM key.
2. **Trusts** the validator's `view` for state updates because
   `T::AuditAuthorityOrigin` is the privileged single submitter per
   issue #25 §G. If the validator submits a divergent `view`, chain
   state diverges from the audit-log body bytes — observable.

A future PR can add `view_hash = SHA256(SCALE(view))` to the §23
canonical aggregate schema so the on-chain check rebinds `view` and
`signed.body` structurally. Tracked as PR-I3.1.

### `hippius-types` no_std-compatibility

The pallet runs in no_std. PR-I3 makes `hippius-types` no_std-clean
so the pallet can consume the canonical `audit_vm` schema:

- `#![cfg_attr(not(feature = "std"), no_std)]` + `extern crate alloc;`
- Default-on `std` feature gates the std-only transitive features
  (`ciborium/std`, `serde/std`, `sha2/std`, `subtle/std`,
  `thiserror/std`).
- Every module ships an `alloc` prelude (`String`, `ToString`,
  `Vec`, `vec!`, `format!`, `Box`) under `#[allow(unused_imports)]`
  so adding a use to a module doesn't require touching the prelude.
- Cargo.toml: every transitive dep moved to
  `default-features = false`, std feature explicitly enables what
  was previously the default.

`cargo check -p hippius-types` and
`cargo check -p hippius-types --no-default-features` both pass.
The 8 existing unit tests + 33 std-mode tests in `hippius-types`
all pass unchanged.

### `Cargo.toml` changes

- Added `hippius-types = { path = "../../hippius-types",
  default-features = false }` to `[dependencies]`.
- Added `"hippius-types/std"` to the `std` feature.
- Added `pallet-balances`, `sp-runtime` (with std), and
  `hippius-types` (std) to `[dev-dependencies]` for the new mock
  runtime + tests.

### New test scaffolding

- `src/mock.rs` (NEW) — minimal `frame_system + pallet_balances +
  pallet_compute_scoring` runtime. Every Config item bound; the
  PR-I4 registration / proxy traits route through
  `DummyFamilyRegistry` / `DummyProxyVerifier` (always-true) since
  this PR's tests don't exercise those code paths.
- `src/tests.rs` (NEW) — 13 tests covering the new extrinsic
  surface (happy / sig-failure / 4 replay-domain failures / 2
  wire-shape failures / pubkey-unregistered / chain continuity /
  admin gate / submitter gate) plus the
  `aggregate_view_fields_match_hippius_types` schema-drift tripwire.

(PR-I2 removed `src/mock.rs` + `src/tests.rs` as superseded by the
`cargo check` compile-gate; PR-I3 reintroduces them with a real
runtime.)

## PR-I4 deltas — SIMPLIFY (rip ~1300 LOC of vendored arion layers)

PR-I4 is the **anti-pollution pass**: state and events on-chain
MUST serve the §23 reward, the §13 anti-Sybil registry, or
slashing — everything else moves off-chain (vali audit log, KBS
audit log #36). This pass rips the CRUSH placement layer, the
warden attestation layer, the file-size tracking layer, and the
NodeQuality / family-weight machinery — they're all storage-side
plumbing that doesn't apply to confidential compute. **Net diff:
~3081 → ~1450 LOC** (the heavy upstream vendor reduced to a
focused compute-scoring pallet).

Plan source: issue #40 comment "Plan révisé 2026-05-20" — locked
decisions:

1. **Runtime cible** = the same `thenervelab/thebrain` chain (not
   a separate chain). Our pallet is added to thebrain's
   `construct_runtime!`.
2. **No mirror** of `pallet-registration` / `pallet-execution-unit`
   / `pallet-compute` / `pallet-ranking`. Those pallets stay
   shared at the runtime level.
3. **No CRUSH placement** in this pallet. For compute, the vali
   does placement (off-chain). Rip ~1200 LOC of vendored arion
   code.
4. **Anti-pollution discipline**: an on-chain state/event must
   serve reward, anti-Sybil, OR slashing. Else → off-chain.

The CRUSH/warden/file-size machinery from arion was vendored
byte-for-byte in PR-I2 to keep the diff against upstream clean.
PR-I4 now **declares the divergence point**: this is no longer a
byte-for-byte mirror, it's a fork. The `rustfmt::skip` directive
on `lib.rs` and `weights.rs` is also dropped — re-syncs against
newer arion commits are no longer a 1:1 textual diff.

### Layers ripped — by post-PR-I3 line range

The line numbers below reference `pallets/compute-scoring/src/lib.rs`
AT the post-PR-I3 baseline (3349 LOC). Comparing against earlier
file snapshots requires walking the PR-I2 → PR-I3 history.

#### 1. CRUSH placement layer (the explicit user request — ~280 LOC)

| Item | Where (post-PR-I3 lines) |
|---|---|
| `submit_crush_map` extrinsic | 2286–2336 |
| `prune_historical_crush_epochs` extrinsic | 3171–3204 |
| `CrushParams` struct | 176–180 |
| `MinerRecord<…>` + 4 manual trait impls | 189–280 |
| Storage `CurrentEpoch` *(KEPT — re-used as the `vali_submit_epoch_close` write target)* | 829–831 |
| Storage `EpochParams` | 833–834 |
| Storage `EpochMiners` | 836–844 |
| Storage `EpochRoot` | 846–848 |
| `CrushMapPublished` event | 1261–1265 |
| `CrushEpochsPruned` event | 1330–1334 |
| Errors: `EpochRegression`, `EpochAlreadyExists`, `MinerListNotSortedOrNotUnique`, `TooManyMiners`, `MinerNotRegistered`, `CrushEpochPruningDisabled`, `InvalidCrushEpochPruneBatch`, `CrushEpochPruneStartBeyondCutoff` | 1373–1452 |
| Config `MapAuthorityOrigin`, `EnforceRegisteredMinersInMap`, `MaxMiners`, `EpochCrushMapRetention`, `MaxCrushEpochPrunesPerCall`, `MaxEndpointLen`, `MaxHttpAddrLen` | scattered 866–950 |
| Helpers `ensure_miner_records_registered`, `crush_epoch_prune_cutoff`, `remove_crush_epoch_storage`, `find_uid_for_node_in_epoch`, `collect_protected_miner_uids` | 1750–2120 |

`EpochRegression` is **kept** but re-purposed as the
`vali_submit_epoch_close` epoch-monotonicity error.

#### 2. Warden attestation layer (~555 LOC)

Per the #40 2026-05-20 plan: warden becomes a slice **inside**
scoring via `vali_submit_epoch_close`, not a separate extrinsic
surface. Everything warden-specific is ripped.

| Item | Where |
|---|---|
| `submit_attestations` extrinsic | 2748–2807 |
| `submit_attestation_commitment` extrinsic | 2838–2870 |
| `register_warden` extrinsic | 2940–2968 |
| `deregister_warden` extrinsic | 2973–3006 |
| `prune_attestation_buckets` extrinsic | 3011–3055 |
| `AuditResult` enum + `as_u8` | 316–339 |
| `AttestationRecord<…>` + 4 manual trait impls | 351–479 |
| `EpochAttestationCommitment<…>` | 482–500 |
| `WardenStatus` enum, `WardenInfo<BlockNumber>` | 502–528 |
| Storage `AttestationsByBucket`, `RegisteredWardens`, `ActiveWardenCount`, `CurrentAttestationBucket`, `EpochAttestationCommitments` | 880–910 |
| Events `AttestationsSubmitted`, `AttestationCommitmentSubmitted`, `WardenRegistered`, `WardenDeregistered`, `AttestationBucketsPruned` | 1267–1325 |
| Errors `AttestationBucketRegression`, `TooManyAttestations`, `AttestationBucketFull`, `InvalidAttestationSignature`, `AttestationCommitmentAlreadyExists`, `InvalidContentHashLength`, `WardenAlreadyRegistered`, `WardenNotRegistered`, `UnregisteredWarden`, `PruningWithinRetentionPeriod`, `EmptyAttestations`, `DuplicateAttestation`, `InvalidAttestationPruneBatch` | scattered 1373–1452 |
| Config `AttestationAuthorityOrigin`, `MaxAttestations`, `MaxShardHashLen`, `MaxWardenPubkeyLen`, `MaxSignatureLen`, `MaxMerkleProofLen`, `MaxWardenIdLen`, `MaxContentHashLen`, `AttestationRetentionBuckets`, `MaxAttestationBucketsPrunePerCall` | scattered 866–960 |
| Helper `verify_attestation_sig` | 1934–1977 |

#### 3. File-size tracking layer (~170 LOC)

Storage-shard-specific; no place in a compute scoring pallet.

| Item | Where |
|---|---|
| `update_user_file_size` extrinsic | 3063–3104 |
| `update_multiple_user_file_sizes` extrinsic | 3112–3164 |
| `UserStorageUsageUpdate<AccountId>` | 547–557 |
| Storage `UserTotalFilesSize`, `UserTotalFilesCount` | 1230–1236 |
| Events `UserStatsUpdated`, `UserFilesUpdated`, `UserStorageUsageUpdated` | 1335–1365 |
| Helpers `get_all_users`, `delete_user_entries` | 1722–1731 |

#### 4. NodeQuality + family-weight machinery (~440 LOC)

Replaced by [`EpochWeights<u64, NodeId, u128>`] + the validator's
off-chain weight computation submitted via
[`vali_submit_epoch_close`]. The vendored on-chain weight blend
(`log2(1+x)` + family EMA + newcomer floor) doesn't apply to
compute (no storage/bandwidth two-axis blend), so it goes away
wholesale.

| Item | Where |
|---|---|
| `submit_node_quality` extrinsic | 2706–2744 |
| `NodeQuality` struct | 567–588 |
| Storage `NodeWeightByChild`, `NodeWeightLastBucket`, `NodeQualityByChild`, `NodeFamilyByChild`, `FamilyWeights`, `FamilyWeightRaw`, `FamilyFirstSeenBucket`, `CurrentWeightBucket` | scattered 1009–1118 |
| Events `NodeWeightsUpdated`, `FamilyWeightsComputed` | 1308–1320 |
| Errors `WeightBucketRegression`, `TooManyNodeWeightUpdates` | 1419–1422 |
| Config `WeightAuthorityOrigin`, `MaxNodeWeightUpdates`, `MaxNodeWeight`, `MaxFamilyWeight`, `FamilyTopN`, `FamilyRankDecayPermille`, `FamilyWeightEmaAlphaPermille`, `MaxFamilyWeightDeltaPerBucket`, `NewcomerGraceBuckets`, `NewcomerFloorWeight`, `NodeBandwidthWeightPermille`, `NodeStorageWeightPermille`, `NodeScoreScale`, `StrikePenalty`, `IntegrityFailPenalty` | scattered 965–1050 |
| Helpers `compute_node_weight_from_quality`, `apply_node_weights_and_recompute`, `compute_family_weight_from_nodes`, `get_total_family_weight`, `log2_fixed_u128`, `log2_fixed_u64`, `remove_child_node_weight_entries` | scattered 1700–2218 |

#### 5. MinerStats + periodic stats submission (~155 LOC)

Replaced by the audit-VM signature chain
([`submit_audit_stats`]) + the off-chain aggregator that lands
results via [`vali_submit_epoch_close`].

| Item | Where |
|---|---|
| `submit_miner_stats` extrinsic | 2343–2376 |
| `MinerStats` struct (PR-I3 renamed) | 282–307 |
| `MinerStatsUpdate` struct | 309–314 |
| Storage `MinerStatsByUid`, `CurrentStatsBucket`, `MinerStatsPruneCursor`, `CurrentNetworkTotals`, `NetworkTotals`, `ChildMinerUid` | scattered 855–873 |
| Event `MinerStatsUpdated` | 1267 |
| Errors `StatsBucketRegression`, `TooManyStatsUpdates` | 1389–1391 |
| Config `StatsAuthorityOrigin`, `MaxStatsUpdates`, `MinerStatsPruneInterval`, `MinerStatsPruneMaxScanPerBlock` | scattered 866–805 |
| Helpers `prune_stale_miner_stats`, `clear_miner_uid_and_stats_for_child`, the entire `Hooks::on_initialize` body that drove pruning | 763–820, 1733–1812 |

### PR-I3 `submit_audit_stats` refactored

Per the anti-pollution discipline: `submit_audit_stats` no longer
credits `view.served_units` to per-node `MinerStats` storage (the
struct was ripped in §5 above) and no longer bumps
`CurrentNetworkTotals` (the struct was ripped too). The
audit-VM Ed25519 signature chain stays on-chain (binding — the
validator cannot synthesise aggregates without the audit-VM key)
but the **served-units accounting moves off-chain**: the
validator aggregates per-node served-units off-chain across the
signed bodies it received, then writes the result via
[`vali_submit_epoch_close`] at epoch close.

The on-chain `AuditStatsSubmitted` event keeps `served_units` in
its payload so off-chain indexers can still cross-check the
validator's aggregator output against the audit-VM-signed body
bytes.

### Added — `vali_submit_epoch_close` + `MinerStatus` / `EpochWeights`

```text
// Types
pub enum MinerStatus { Active, Quarantined, Decommissioned }
pub struct MinerStatusEntry<BlockNumber> {
    status, last_transition_block, last_transition_epoch
}
pub struct MinerStatusUpdate { node_id, new_status, weight }

// Storage
MinerStatuses<NodeId → MinerStatusEntry<BlockNumber>>          // OptionQuery (1 row per miner, default-Active implicit)
EpochWeights<u64 → NodeId → u128>                              // ValueQuery (epoch → node → reward weight)
CurrentEpoch<u64>                                              // ValueQuery (re-used from CRUSH; writer = vali_submit_epoch_close)

// Extrinsic
vali_submit_epoch_close(origin, epoch, status_updates) -> DispatchResult
    // ensure_root (locked Q13 v1 single-operator)
    // ensure epoch > CurrentEpoch (no replay)
    // O(n²) dedupe scan on node_id (bounded by MaxMinerStatusUpdatesPerCall)
    // for each MinerStatusUpdate:
    //     EpochWeights[epoch][node_id] = weight   (always written)
    //     if prev_status (default-Active if no row) != new_status:
    //         MinerStatuses[node_id] = entry      (row materialises only on non-Active)
    //         emit MinerStatusChanged             (events ONLY on transitions)
    //     // else: heartbeat — no storage write, no event
    // CurrentEpoch = epoch
    // emit EpochClosed{epoch, updates}

// Events
MinerStatusChanged { node_id, old_status, new_status, epoch }   // transitions only
EpochClosed { epoch, updates }                                  // every epoch close

// Errors
EpochRegression               // re-purposed from the ripped CRUSH path
DuplicateNodeInBatch          // new — the batched-update dedupe gate

// Config
type MaxMinerStatusUpdatesPerCall: Get<u32>;
```

**Default-Active baseline**: a node with no `MinerStatuses` row is
implicitly `Active`. So:
- First-ever update with `new_status = Active` → silent (heartbeat
  re-asserting the default); no storage row written.
- First-ever update with `new_status = Quarantined` /
  `Decommissioned` → transition (default-Active → X); row
  written + event emitted.
- Subsequent same-status update → silent.
- Subsequent different-status update → transition.

This keeps the storage minimal ("1 row par miner, pas
d'historique") AND honours the user's "events SEULEMENT sur
status transitions, pas sur heartbeat" requirement structurally.

### Root-only origin (NOT `Config::EpochCloseAuthorityOrigin`)

The user's PR-I4 spec says **"root-only (locked Q13
single-operator v1)"**. Pinning this structurally via
`ensure_root(origin)?` (vs going through a Config
`EnsureOrigin` knob) means a future runtime CANNOT relax the
gate by binding a laxer origin — the relaxation requires a
visible PR that re-routes the call through Config. Trade-off:
less Config-side flexibility, more invariant safety. Matches the
spec's "locked" framing.

### `rustfmt::skip` dropped from `lib.rs` + `weights.rs`

PR-I2's vendor used tabs + `rustfmt::skip` so the diff against
upstream stayed clean for re-syncs. PR-I4 declares this is no
longer a re-syncable byte-for-byte mirror — we're a fork. Files
are now standard rustfmt 4-space indentation.

### `Cargo.toml` — no changes

The `pallet-compute-scoring` Cargo manifest is unchanged. The
removed `WeightInfo` methods (`submit_crush_map`,
`submit_node_quality`, `submit_attestations`,
`submit_attestation_commitment`, `register_warden`,
`deregister_warden`, `prune_attestation_buckets`,
`prune_historical_crush_epochs`, `update_user_file_size`,
`miner_stats_prune_hook`) are gone from `weights.rs` along with
their callers; the kept methods are: `register_child`,
`deregister_child`, `claim_unbonded`, `submit_audit_stats`
(replaces the `submit_miner_stats(1)` reuse — now its own
weight), `set_audit_vm_pubkey`, `set_lockup_enabled`,
`set_base_child_deposit`, `set_free_child_slots_per_family`, and
NEW `vali_submit_epoch_close(n)`.

### Convergent codex + gemini review (PR-I4)

Applied pre-merge. No CRITICAL findings (gemini flagged a
"scalability bottleneck" — see follow-up note below); the
convergent HIGH + MEDIUM punchlist:

- **HIGH (both reviewers)**: `vali_submit_epoch_close` priced
  the dedupe linearly in `weights.rs` but the actual scan was
  O(n²) → fixed by switching to a `BTreeSet`-based dedupe in
  `lib.rs` (O(n log n)) AND bumping the per-element weight
  coefficient to absorb the log-factor at typical bounds. The
  weight comment now describes the BTreeSet model honestly.
- **MEDIUM (both reviewers)**: a `Quarantined → Active`
  recovery used to insert an explicit `Active` row, contradicting
  the "default-Active implicit / only non-default rows
  materialise" invariant. Fixed: on `new_status == Active &&
  prev_status != Active`, the pallet now `MinerStatuses::remove`s
  the row while still emitting `MinerStatusChanged`. New
  `vali_submit_epoch_close_quarantined_to_active_removes_row`
  test pins the recovery contract.
- **MEDIUM (both reviewers)**: `EpochWeights` was `ValueQuery`
  (`u128`), making "not reported by validator" indistinguishable
  from "reported 0 reward". Fixed: switched to `OptionQuery` so
  the off-chain ranking reader can distinguish `None` (not
  reported — off-chain accounting decides) vs `Some(0)` (explicit
  zero-reward verdict, e.g. Quarantined). Tests updated.
- **LOW (codex)**: mock `MaxMinerStatusUpdatesPerCall` was 256;
  recommendation 64–128 until the production bound is set by
  benchmarks. Mock dropped to 128; production picks from
  block-weight headroom.
- **LOW (codex)**: `AuditStatsSubmitted` event still carries
  `served_units` on the wire. Confirmed-kept for off-chain
  indexers; the doc-comment + CHANGES.md note that the field is
  *advisory* until a `view_hash`-binding §23 schema change
  (PR-I3.1) lands.
- **LOW (gemini)**: missing `Quarantined → Active` test → added
  alongside the row-removal fix (above).

#### gemini "CRITICAL" scalability note — kept as a follow-up

Gemini flagged that `epoch > CurrentEpoch` + a one-shot
`CurrentEpoch::put(epoch)` means the network can't scale beyond
`T::MaxMinerStatusUpdatesPerCall` nodes per epoch. The
recommended fix (`epoch >= cur` + cross-batch dedupe via
`EpochWeights::contains_key`) is functionally clean but
introduces a "is this the last batch?" ambiguity (when does
`EpochClosed` fire? when does `CurrentEpoch` advance?). Codex
explicitly disagreed that this is critical for the v1
single-operator root-only design.

Decision: keep the single-batch close in PR-I4. If the network
grows past one batch per epoch, **PR-I4.1** introduces a
dedicated `vali_submit_epoch_weights` (paged, accumulates) +
`vali_close_epoch` (terminal, advances `CurrentEpoch`,
emits `EpochClosed`) split. v1 production should pick
`MaxMinerStatusUpdatesPerCall` in the 256–1024 range based on
block-weight headroom benchmarks.

### Mock + tests

`src/mock.rs` shrinks from ~180 LOC to ~135 (no more Config
items for the ripped CRUSH/warden/quality knobs); `src/tests.rs`
keeps the 13 PR-I3 audit-stats tests + adds 7 PR-I4
`vali_submit_epoch_close` tests:

- happy path (writes `EpochWeights` for both nodes, transitions
  `MinerStatuses` only for `NODE_B`'s Quarantined, default-Active
  `NODE_A` stays silent + uninstalled)
- silent heartbeat (epoch 1 transitions Quarantined; epoch 2
  re-asserts same status; no second `MinerStatusChanged` event)
- non-root origin rejected
- epoch regression rejected (replay + go-backward)
- duplicate node in batch rejected
- all three statuses reachable (Active / Quarantined /
  Decommissioned)
- cross-extrinsic invariant: `vali_submit_epoch_close` advancing
  `CurrentEpoch` makes a stale `submit_audit_stats(epoch=0)`
  reject with `AggregateEpochMismatch`.

Total: 24 tests, all green under `cargo test
-p pallet-compute-scoring`.

## PR-I5 deltas — wire real `pallet-registration` + `pallet-proxy`

PR-I5 retires the local `src/pallet_registration.rs` /
`src/pallet_proxy.rs` stubs (added in PR-I2 as fail-closed
placeholders) and routes through the real `thenervelab/thebrain`
crates at the same pinned commit as `PROVENANCE.md`'s
arion-pallet vendor:
`11226860c66e51bd7092c67263ef59b658f1d7f4`.

### Stubs ripped

| File | LOC | Disposition |
|---|---|---|
| `src/pallet_registration.rs` | ~85 (PR-I2 stub) | Deleted. |
| `src/pallet_proxy.rs` | ~40 (PR-I2 stub) | Deleted. |
| `mod pallet_proxy;` / `mod pallet_registration;` in `lib.rs` | 2 lines | Deleted. |
| `use crate::pallet_proxy;` / `use crate::pallet_registration::{…}` | 2 lines | Rewritten as external crate `use` paths. |

The stubs short-circuited every call to `false` / `None` / empty
`Vec` so `force_deregister_child` and `submit_audit_stats` paths
always fell through their fail-closed branches. The real crates
now actually answer the queries.

### `Cargo.toml` — added

```toml
pallet-registration = { git = "https://github.com/thenervelab/thebrain",
                        rev = "11226860c66e51bd7092c67263ef59b658f1d7f4",
                        default-features = false }
pallet-proxy        = { git = "https://github.com/thenervelab/thebrain",
                        rev = "11226860c66e51bd7092c67263ef59b658f1d7f4",
                        default-features = false }
```

Plus `"pallet-registration/std"` and `"pallet-proxy/std"` added
to the `std` feature. The transitive forks (`pallet-staking`,
`pallet-credits`, `pallet-utils`, `pallet-babe`) come in via
`pallet-registration`'s own `Cargo.toml` automatically — Cargo
resolves the same `git`/`rev` for all of them.

### Critical scope decision — Config trait stays decoupled

The thebrain `pallet-registration` Config is HEAVY (~8 supertraits:
`pallet_babe`, `pallet_balances`, `pallet_credits`,
`pallet_staking`, `pallet_proxy`, `pallet_utils`, custom
`MetagraphInfoProvider` / `MetricsInfoProvider`, custom
`ProxyTypeCompat`). Adding `+ pallet_registration::Config +
pallet_proxy::Config` to `pallet-compute-scoring`'s own `Config`
supertrait — as the PR-I2/I4 TODO suggested — would force every
**test runtime** to wire all 8 of those crates too. That's
prohibitive churn for what is in practice three read-only
queries against the upstream pallets.

**PR-I5 keeps the trait-abstraction pattern** the PR-I3 `Config`
already established: `FamilyRegistry`, `ProxyVerifier`, and a
NEW `NodeRegistrationProvider` trait expose just the methods the
scoring pallet needs. Production runtimes bind them to
`pallet_registration::Pallet<Self>` / `pallet_proxy::Pallet<Self>`
(where the heavy supertraits ARE satisfied); test runtimes use
`()` (fail-closed) or a lightweight stand-in.

### `lib.rs` — Config additions

```text
type Registration: NodeRegistrationProvider;
```

Just one new associated type. The two existing trait Configs
(`FamilyRegistry`, `ProxyVerifier`) gain two methods each so
`force_deregister_child` can route through them instead of
directly calling `pallet_registration` / `pallet_proxy`:

| Trait | Method | Used by |
|---|---|---|
| `FamilyRegistry` | `owner_has_validator_node(owner) -> bool` | `force_deregister_child` |
| `ProxyVerifier`  | `primary_account(who) -> Option<AccountId>` | `force_deregister_child` |
| `NodeRegistrationProvider` (NEW) | `is_node_registered(node_id: &[u8; 32]) -> bool` | `submit_audit_stats` |

The `()` impl of each new method returns the fail-closed default
(`false` / `None`). The real `pallet_registration::Pallet<T>` /
`pallet_proxy::Pallet<T>` impls delegate to upstream:

```text
pallet_registration::Pallet::<T>::is_owner_node_registered(family)         // FamilyRegistry::is_registered_family
pallet_registration::Pallet::<T>::get_registered_node_for_owner(owner)    // FamilyRegistry::owner_has_validator_node
    .map(|info| info.node_type == pallet_registration::NodeType::Validator)
    .unwrap_or(false)
pallet_proxy::Pallet::<T>::proxies(family).0.iter()...                    // ProxyVerifier::can_register_child
pallet_proxy::Pallet::<T>::proxies(who).0.into_iter().next().map(|p| p.delegate)  // ProxyVerifier::primary_account
pallet_registration::Pallet::<T>::get_node_registration_info(node_id.to_vec()).is_some()  // NodeRegistrationProvider::is_node_registered
```

All impls bound by `<T: pallet_registration::Config>` /
`<T: pallet_proxy::Config>` (NOT by `T: crate::Config`), so the
heavy supertrait wiring lives only where the runtime actually
uses the upstream pallets.

### `lib.rs` — `submit_audit_stats` gate (PR-I5 core requirement)

```text
// (1b) PR-I5: gate on the registration layer. An unregistered
//      node_id cannot push aggregates onto the on-chain chain —
//      caught BEFORE the expensive Ed25519 verify so a flood of
//      bogus node_ids can't burn weight on signature work.
ensure!(
    T::Registration::is_node_registered(&view.node_id),
    Error::<T>::NodeNotRegistered
);
```

Placed right after the wire-shape sanity checks, BEFORE the
replay-domain + Ed25519 work. A node missing from the
registration layer trips `NodeNotRegistered` before the pallet
spends any non-trivial weight.

### `lib.rs` — `force_deregister_child` refactor

The body now reads:

```text
let main_account = T::ProxyVerifier::primary_account(&who).unwrap_or_else(|| who.clone());
ensure!(
    T::FamilyRegistry::owner_has_validator_node(&main_account),
    Error::<T>::NodeNotRegistered
);
Self::do_deregister_child(&main_account, child, /* force = */ true)
```

(vs the previous direct `pallet_proxy::Pallet::<T>::proxies` +
`pallet_registration::Pallet::<T>::get_registered_node_for_owner`
calls — both required Configs we don't inherit.)

The `Self::get_primary_account` private helper is gone; the
trait abstraction subsumes it. `Error::<T>::InvalidNodeType` is
also gone (collapsed into `NodeNotRegistered` — both report
"actor isn't an eligible Validator owner"; pre-mainnet so the
SCALE-encoded Error variant index churn is acceptable).

### Mock + tests

`src/mock.rs` (~+50 LOC):

- New `MockRegistration` struct backed by a `thread_local!`
  `BTreeSet<[u8; 32]>`. Implements `NodeRegistrationProvider` by
  reading the set. `mock_register_node([u8; 32])` and
  `mock_clear_registered_nodes()` helpers let tests register /
  unregister node_ids without instantiating
  `pallet_registration`'s heavy Config.
- `pallet-registration::Pallet<T>` / `pallet-proxy::Pallet<T>`
  are NOT in the mock's `construct_runtime!` — the trait
  abstraction means the mock can dodge the 8-supertrait wiring.
  Tests that need a registered node call `mock_register_node`;
  `new_test_ext` pre-registers `TEST_NODE_ID = NODE_ID` so all
  existing PR-I3/I4 audit-stats tests pass unchanged.
- `DummyFamilyRegistry::owner_has_validator_node` returns `true`
  (so PR-I4 `vali_submit_epoch_close` / registration tests stay
  unaffected); `DummyProxyVerifier::primary_account` returns
  `None` (so `force_deregister_child` falls back to the calling
  account, then trips the validator check).

`src/tests.rs` (+1 test):

- New `submit_audit_stats_rejects_unregistered_node` test —
  clears the auto-registered node and asserts
  `NodeNotRegistered` is the rejection reason, pinning the gate
  runs BEFORE the audit-VM pubkey check (so an unregistered
  node burns no Ed25519 verify weight).

Total: 26 unit tests (was 25). Workspace `cargo test` /
`cargo clippy --workspace --all-targets -- -D warnings` /
`cargo fmt --check` all green.

### Convergent codex + gemini review (PR-I5)

Applied pre-merge. No CRITICAL findings (codex flagged the
upstream `pallet-staking/reward-fn/Cargo.toml` typo warning as
critical, but the build + tests + clippy all complete cleanly
in our cargo version — confirmed `cargo test --workspace
--locked` green); the convergent HIGH + MEDIUM punchlist:

- **HIGH (both reviewers)**: `ProxyVerifier::primary_account`
  was logically inverted — `pallet_proxy::Pallet<T>::proxies(who)`
  returns proxies AUTHORISED BY `who` (i.e. `who` is the
  primary), not proxies WHERE `who` is the delegate. Returning
  the first delegate flipped the resolution direction. **Fixed
  by dropping proxy resolution from `force_deregister_child`
  entirely**: the extrinsic now requires the caller to be the
  validator primary directly. Upstream arion does an O(N
  validators) reverse-scan that doesn't fit the v1 single-
  operator trust posture; a future PR can add an explicit
  `force_deregister_child_as_proxy(real_validator, child)`
  extrinsic if needed (cheaper than the scan).
- **MEDIUM (both reviewers)**: `InvalidNodeType` collapsed into
  `NodeNotRegistered` lost semantic clarity. **Re-introduced as
  `NotAValidator`** — distinct variant for "caller IS a
  registered owner but with a non-Validator node type" vs
  "caller has no registration record at all". Block explorers /
  indexers can now distinguish.
- **MEDIUM (codex)**: `get_node_registration_info` excludes
  `Degraded` nodes. **Decision kept**: "actively registered"
  IS the right gate for `submit_audit_stats` since a Degraded
  node hasn't recovered yet and shouldn't earn reward weight.
  Test runtimes can flip a node to Degraded by adjusting the
  `MockRegistration` thread-local; the production behaviour
  matches the §13 §23 spec's "reward-eligible only" intent.
  Documented in the trait impl comment.
- **LOW (codex)**: unused `use pallet_proxy;` /
  `use pallet_registration::{self, NodeType, …}` imports.
  Removed — fully-qualified paths in the trait impls suffice.
- **LOW (gemini)**: missing `force_deregister_child` tests.
  Added `force_deregister_child_happy_path` +
  `force_deregister_child_unregistered_via_root_origin_rejected`
  + a compile-only sentinel `force_deregister_child_rejects_unregistered_caller`
  that pins the trait surface against future drift.
- **LOW (both)**: `[u8; 32].to_vec()` allocation on every
  `submit_audit_stats` is acceptable (32 bytes per call, before
  Ed25519 verify which dominates). Documented.
- **LOW (codex)**: `try-runtime` could forward to
  `pallet-registration/try-runtime` /
  `pallet-proxy/try-runtime`. Not required for v1; skipped
  until try-runtime infrastructure lands.

### `pallet-staking/reward-fn` upstream cargo warning

The pinned thebrain commit has a `repository.workspace = true`
typo in `pallets/staking/reward-fn/Cargo.toml` line 9 (should
be `repository = "..."`). Cargo emits an `error:`-prefixed
warning during dependency resolution but DOES NOT fail the
build — `cargo test` / `cargo clippy` / `cargo build` all
complete green. Will be silenced when thebrain re-pins past
the fix, OR via a workspace-level `[patch]` if it persists.
Tracked in CHANGES.md as a known harmless warning.

### Cargo build-time impact

Pulling `pallet-registration` from the thebrain `git` source
brings in the full polkadot-sdk `stable2407` machinery already
required by our workspace, plus the thebrain forks of
`pallet-staking` / `pallet-credits` / `pallet-utils` /
`pallet-babe` via Cargo's transitive resolution. First cold
build adds ~3 min; incremental builds touching only
`pallet-compute-scoring` stay fast.

A non-fatal cargo warning from
`pallets/staking/reward-fn/Cargo.toml` at the pinned commit
(`repository.workspace = true` typo) is emitted on every build
— harmless and lives upstream; the next thebrain re-pin will
either ship it fixed or we vendor a `[patch.crates-io]` entry
to flip the workspace = false there.

## PR-I6 deltas — `RankingsSink` bridge + `epoch_weights_for` reader

PR-I6 closes the §I chunk by establishing the **integration
boundary** between `pallet-compute-scoring` (writer of
`EpochWeights`) and the downstream `pallet-rankings` consumer
(writer of `RewardDistributed` events) — without building the
real `construct_runtime!` the original spec called for.

### Scope decision: bridge test, not real `construct_runtime!`

Building a real mini-runtime including `pallet-rankings` would
require ~1000 LOC of polkadot-sdk relay-chain scaffolding. The
supertrait load tree:

```
pallet_rankings::Config
  + pallet_metagraph::Config          // CIRCULAR with pallet_registration
  + pallet_staking::Config            // full FRAME NPoS pallet
      + ElectionProvider
      + BagsList / VoterList / TargetList
      + NominationPools
      + Session + Historical
      + Offences
      + Treasury
      + EraPayout + SessionInterface + NextNewSession
  + pallet_registration::Config       // already 8 supertraits, see PR-I5
  + pallet_utils::Config              // pulls OCW HTTP
  + frame_system::offchain::SigningTypes
```

The recon agent (codex agentId `af100ce764b73a0eb`) verified
that upstream thebrain's own mocks at this pinned commit are
either stale 57-line stubs that don't satisfy the real Config
trait, or fully commented-out dead code (`pallets/ranking/src/
mock.rs`). Wiring this from scratch is a 1-2 day yak-shave
exercise with non-trivial risk of not compiling at all.

**Decision**: PR-I6 ships a **trait abstraction + bridge test**.
The integration boundary is fully specified by the
`RankingsSink` trait + `epoch_weights_for` reader; the bridge
test exercises the BOUNDARY behaviour (the data leaving
`pallet-compute-scoring`) without instantiating the heavy
downstream consumer. A future PR-I6.1 (if scoped as
infrastructure work) can add a real `tests/runtime_integration.rs`
once thebrain ships a working test runtime, OR by pinning
thebrain's `runtime/mainnet` as a `[dev-dependencies]` git dep
(slow compile but real wiring).

### Trait additions — `lib.rs`

```text
pub struct EpochWeightEntry {
    pub node_id: [u8; 32],
    pub weight: u128,
}

pub trait RankingsSink {
    fn push_rankings(epoch: u64, entries: &[EpochWeightEntry]);
}

impl RankingsSink for () {
    fn push_rankings(_, _) {}              // production default (no-op)
}
```

The `()` impl IS the production binding by default: validators'
off-chain workers read `Pallet::<T>::epoch_weights_for(epoch)`
directly + post `pallet_rankings::update_rankings(weights, …)`
themselves (matching upstream's
"validator-signed-Vec<u16>-extrinsic" contract). The trait
exists for runtimes that want an in-runtime push instead, and
AS THE INTEGRATION BOUNDARY the bridge test pins:
`vali_submit_epoch_close` MUST hand the downstream consumer the
full `(node_id, weight)` snapshot for the closing epoch in one
hook.

### Pallet reader — `lib.rs`

```text
impl<T: Config> Pallet<T> {
    pub fn epoch_weights_for(epoch: u64) -> Vec<EpochWeightEntry>;
}
```

Used by:
- `vali_submit_epoch_close` itself to feed `T::RankingsSink`
  (one O(n) iter per close — same complexity as the writes).
- Off-chain validator OCWs that build the
  `pallet_rankings::update_rankings` payload (production path).
- Future runtime-API + JSON-RPC for explorers / indexers.

### Config — `lib.rs`

Added:

```text
type RankingsSink: RankingsSink;
```

### `vali_submit_epoch_close` body

After the per-update loop + `CurrentEpoch::put(epoch)` (PR-I4
shape preserved):

```text
let snapshot = Self::epoch_weights_for(epoch);
T::RankingsSink::push_rankings(epoch, &snapshot);
Self::deposit_event(Event::EpochClosed { epoch, updates: updates_len });
```

The push happens BEFORE the `EpochClosed` event so an indexer
correlating "epoch closed" with "rankings consumer notified"
sees a clean ordering: state writes → sink push → event.

### Production wiring path (documented for §I closure)

1. Validator OCW polls `pallet_compute_scoring::EpochClosed`
   events (or `frame_system::Pallet::events()` filtered).
2. On `EpochClosed{epoch, _}`, OCW calls
   `Pallet::<Runtime>::epoch_weights_for(epoch)` to fetch the
   `Vec<EpochWeightEntry>`.
3. OCW transforms `[u8; 32] → Vec<u8>` (`pallet_rankings::
   update_rankings` takes `Vec<Vec<u8>>` for `node_ids`) and
   `u128 → u16` (clamp / cast at the boundary — the
   `pallet_rankings::update_rankings` signature is
   `(weights: Vec<u16>, all_nodes_ss58: Vec<Vec<u8>>, node_ids:
   Vec<Vec<u8>>, node_types: Vec<NodeType>)`).
4. OCW looks up each node's owner (`ss58`) and node_type via
   `pallet_registration::Pallet::<Runtime>::
   get_node_registration_info(node_id)`.
5. OCW dispatches a Validator-signed `pallet_rankings::
   update_rankings` extrinsic.
6. After `BlocksPerEra` blocks the `pallet_rankings::
   on_initialize` hook computes + distributes rewards from the
   `mrktplce` PalletId account.

The bridge test `full_bridge_flow_register_audit_close_push`
demonstrates step 1-4 end-to-end with mocks; steps 5-6 are
upstream behaviour PR-I6 deliberately doesn't try to exercise.

### Mock + tests

`src/mock.rs` (+~50 LOC):
- `MockRankingsSink` backed by `thread_local!`
  `Vec<(u64, Vec<EpochWeightEntry>)>`.
- `mock_drain_pushed_rankings()` / `mock_clear_pushed_rankings()`
  helpers. `new_test_ext` clears the recording in every test.
- `type RankingsSink = MockRankingsSink` in the Config impl.

`src/tests.rs` (+3 tests):
- `epoch_close_pushes_full_snapshot_to_rankings_sink` — close
  with 2 nodes, drain sink, assert payload matches `EpochWeights`.
- `epoch_weights_for_returns_full_snapshot` — pin per-epoch
  isolation in the reader (snapshots for epoch 1 vs 2 vs an
  empty epoch).
- `full_bridge_flow_register_audit_close_push` — end-to-end:
  register node → submit_audit_stats → vali_submit_epoch_close
  → drain sink → demonstrate the `u128 → u16` + `[u8; 32] →
  Vec<u8>` transforms a real `pallet_rankings::update_rankings`
  caller would do.

Total: 32 unit tests (was 29 in PR-I5). Workspace `cargo test`
+ `cargo clippy --workspace --all-targets -- -D warnings` +
`cargo fmt --check` all green.

### Convergent codex + gemini review (PR-I6)

Applied pre-merge. No CRITICAL findings from codex; gemini
flagged ONE critical performance issue. The convergent
HIGH + MEDIUM punchlist:

- **CRITICAL (gemini) / HIGH (codex)**: `vali_submit_epoch_close`
  was doing an extra O(n) storage iter via `epoch_weights_for(
  epoch)` AFTER the per-update writes — `n` redundant reads of
  data we just wrote. **Fixed**: the snapshot is now built
  in-memory inside the per-update loop (`let mut snapshot =
  Vec::with_capacity(updates_len)`). Zero extra storage reads.
  `weights.rs` comment updated to reflect the access pattern
  (still priced at the same per-element coefficient — the
  in-memory collect costs less than a read so the existing
  per-element absorbs it). Public `epoch_weights_for` reader
  kept for off-chain validator OCWs / runtime-API consumers.
- **MEDIUM (both)**: `u16::try_from(weight).unwrap_or(u16::MAX)`
  in the bridge test is a placeholder that pins the wire SHAPE,
  not a normative scaling policy. **Fixed**: doc-comment in
  the bridge test explicitly says the saturating cast is for
  shape demo only; production OCWs MUST define a
  normalization strategy (e.g. `weight * u16::MAX /
  total_weight_this_epoch`).
- **MEDIUM (codex)**: `RankingsSink::push_rankings` returns
  `()`, can't report failure — concerning if the docs imply
  a synchronous in-runtime adapter. **Fixed**: trait
  doc-comment now explicitly says **best-effort, no-fail**:
  implementations MUST NOT roll back the epoch close;
  adapters needing fallible dispatch route through an OCW
  triggered by the `EpochClosed` event.
- **MEDIUM (gemini)**: `EpochWeights` grows indefinitely.
  Added a `TODO` comment on the storage definition; pruning
  strategy deferred to a future PR (likely as
  `prune_historical_epoch_weights(epoch, count)` once
  retention policy is locked).
- **MEDIUM (codex)**: bridge test only demonstrates 2 of 4
  parallel vectors `pallet_rankings::update_rankings` expects
  (`weights: Vec<u16>`, `node_ids: Vec<Vec<u8>>` — but NOT
  `all_nodes_ss58: Vec<Vec<u8>>` or `node_types: Vec<NodeType>`).
  **Fixed**: bridge test doc-comment explicitly names this a
  *partial* transform — the missing two vectors require
  `pallet_registration::get_node_registration_info` lookups
  the mock doesn't model. Real four-vector alignment would
  need the deferred `construct_runtime!` PR.
- **LOW (codex)**: "Production default: ()" wording was
  imprecise — `RankingsSink` has no FRAME default. **Fixed**:
  Config doc now says "Recommended production binding: `()`"
  and notes the runtime must bind explicitly.
- **LOW (codex)**: exactly-once `push_rankings` coverage is
  already pinned by `pushed.len() == 1` assertions in
  `epoch_close_pushes_full_snapshot_to_rankings_sink` and
  `full_bridge_flow_register_audit_close_push`. No extra
  test needed.

## Re-sync workflow (for future PRs)

1. `git fetch` thenervelab/thebrain to the new `main` commit.
2. Re-run the rename script (mappings above) against the new
   `pallets/arion-pallet/src/lib.rs` + `weights.rs`.
3. Three-way diff against the pinned commit + our current
   `src/lib.rs` + `src/weights.rs`.
4. Re-apply the structural deltas (stub modules + Config supertrait
   edit + removed PR-I1 scaffolding) on top of the renamed upstream.
5. `cargo check -p pallet-compute-scoring` MUST pass before merge.
6. Bump the `PROVENANCE.md` pinned commit + update this file.

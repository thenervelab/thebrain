# `pallet-compute-scoring` provenance

## Upstream pin

- **Source repo:** [`thenervelab/thebrain`](https://github.com/thenervelab/thebrain)
- **Pinned commit (locked 2026-05-20):** `11226860c66e51bd7092c67263ef59b658f1d7f4` (current `main` HEAD, "Merge pull request #25 from thenervelab/dev")
- **Upstream pallet:** `pallets/arion-pallet/` (2 679 LOC in `lib.rs`, 362 LOC in `weights.rs`).

## Polkadot-SDK pin

- **Branch:** `stable2407` (same as upstream).
- **Declared in:** workspace root `Cargo.toml` `[workspace.dependencies]`. The pallet's own `Cargo.toml` references those workspace deps so the substrate version always matches what `pallet-arion` was built against.

## Vendoring plan (issue #40)

This skeleton lives at `pallets/compute-scoring/` in **this** repo
(`thenervelab/hippius-compute`) — NOT in `thebrain`. The pallet is a
**mirror** of arion-pallet's *shape*, not a runtime-shared crate.

| PR | Scope |
| --- | --- |
| PR-I1 | Skeleton: workspace member, polkadot-sdk deps, minimal `#[pallet]` module that compiles, mock runtime, compile-gate test. |
| **PR-I2** (this) | Vendor `arion-pallet/src/lib.rs` (2 679 LOC) + `weights.rs` (362 LOC) byte-for-byte at the pinned commit; rename `pallet-arion` → `pallet-compute-scoring` (9 textual passes); stub the `pallet-registration` / `pallet-proxy` use paths in crate-local modules. Per-line audit in `CHANGES.md`. |
| PR-I3 | Adapt storage/extrinsics: replace storage-served metrics with compute-served (`SignedServedDeliveryAggregate`); add `submit_audit_stats` extrinsic; integrate `hippius_types::audit_vm::verify_aggregate`. |
| PR-I4 | Paged epoch-close + deterministic `u128` `f` per §23. |
| PR-I5 | Anti-Sybil capped doubling deposit. |
| PR-I6 | Audit-VM cert issuance + rotation hooks. |
| PR-I7 | Host TPM/IMA corroborator wire. |

## Re-syncing with upstream

If `thenervelab/thebrain`'s arion-pallet evolves significantly,
re-vendoring follows a documented diff workflow:

1. Clone the new upstream commit.
2. Three-way diff against the pinned commit + our current code.
3. Manually merge upstream changes that don't conflict with our
   compute-specific adaptations.
4. Update the pin in this file + the polkadot-sdk branch if needed.

The deliberate choice (per locked decision Q4, 2026-05-20) is to
**mirror, not import** — we own this code, even when it stays close to
upstream. This avoids a hard runtime-time coupling between thebrain's
release cadence and our chain.

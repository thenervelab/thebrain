# check-referenced-names

Forward-looking §13 name-existence check for **unreleased** runtime PRs. See the
module doc in `src/main.rs` for the full reasoning; summary:

- The indexer (`hippius-indexer`) checks every chain name it references against the
  *compiled, pinned* runtime metadata. That check can't see a change in this repo
  before it merges and gets compiled into a new metadata artifact.
- This tool parses **this repo's own pallet source** with `syn` to answer the same
  question one PR earlier — does the indexer's referenced surface (events, calls,
  storage items) still resolve against what this branch's pallet code actually
  declares?
- **WARN-only.** It always exits `0`. Findings print as GitHub Actions `::warning::`
  annotations, visible on the PR, never blocking merge. This is the first CI check
  this repo has had; landing it as a blocking check on day one is how it gets deleted
  instead of fixed.
- **Scope:** this repo's own custom pallets (`pallets/*`) only. Upstream Substrate/
  FRAME pallets (`System`, `Balances`, `Timestamp`, ...) live in external crates this
  repo doesn't version — there's no local source to check them against, and the
  indexer's own metadata-based check already covers them at release time.

## Running locally

```sh
cargo run --manifest-path .github/scripts/check-referenced-names/Cargo.toml
```

Deliberately its own `[workspace]` (see `Cargo.toml`) — not a member of the root
workspace, so it can never affect the node's own build or dependency graph.

## Re-syncing `referenced_names.json`

`referenced_names.json` is a bundled snapshot of `hippius-indexer`'s
`indexer-rs/metadata/referenced_names.json` — the full list of chain names the
indexer references, with the file(s) that read each one. It's copied, not fetched
live, so this check has no cross-repo network dependency in CI.

Re-sync when the indexer's referenced surface changes meaningfully (a new
pipeline/consumer added, an existing one's filters changed):

```sh
cp /path/to/hippius-indexer/indexer-rs/metadata/referenced_names.json \
   .github/scripts/check-referenced-names/referenced_names.json
```

A stale copy only means this check might miss a newly-added reference or warn about
one that's since been removed on the indexer side — it degrades gracefully, it
doesn't produce wrong answers about the *pallet source in this repo*.

## Validation

Run once against `dev` HEAD at the time this tool was built: found 40 dead names (39
archive filters + `execution_unit.nodeMetrics`, the §19.6 wrong-pallet-name bug),
matching `hippius-indexer`'s own independently-derived, compiled-metadata-based
baseline (`metadata/name_check_baseline.json`) exactly — the one baseline entry this
tool doesn't also find (`balances.freeBalance`) is `Balances`, an upstream pallet
correctly out of this tool's scope, not a missed detection.

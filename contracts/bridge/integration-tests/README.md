# Bridge Contract Integration Tests

This workspace mirrors the OTC contract harness to exercise the bridge contract end-to-end against a running `contracts-node`.

## Prerequisites
- Build the bridge ink! artifact: `cargo contract build --release`.
- Start a local contracts-enabled node (default: `ws://127.0.0.1:9944`).
- Install dependencies: `npm install` (or `pnpm install`) inside this directory.
- Generate descriptors (only required after ABI/runtime changes):
  ```bash
  npm run generate-types
  npm run generate-contract
  ```

> **Note:** The `postinstall` hook patches `@polkadot-api/sdk-ink` to align with the node's contract APIs. Leave the `patches/` directory intact.

## Running the Suite
- Default run (verbosely executes all Vitest specs):
  ```bash
  npm test
  ```
- Headless CI run:
  ```bash
  npm run test:run
  ```
- UI explorer:
  ```bash
  npm run test:ui
  ```

The tests orchestrate subnet registration, validator setup, guardian workflows, deposits, releases, refunds, and TTL expiry. Expect multi-minute runtimes—expiry coverage waits slightly longer than the bridge signature TTL (100 blocks).

## Environment Variables
- `CONTRACTS_NODE_URL`: override the websocket endpoint (`ws://127.0.0.1:9944` by default).

## Repo Hygiene
- Test fixtures persist the deployed contract address in `.contract-address`—delete this file to force a redeploy after rebuilding the WASM.

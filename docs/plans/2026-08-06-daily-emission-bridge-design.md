# Daily emission bridging: Bittensor multisig → Hippius distribution

Date: 2026-08-06
Status: validated design, pre-implementation
Branch context: `feat/arion-miner-payments`

## Goal

Move the daily subnet-owner emission (netuid 75) from the Bittensor side to the
Hippius chain and distribute it to actors there, with an automated, quorum-secured
pipeline: no single key can mint, every mint is anchored to an observable Bittensor
event, and hAlpha stays 1:1 backed by escrowed Alpha.

## Decisions

| Question | Decision |
|---|---|
| Recipients on Hippius | Split between stakers (staking pot → `EraPayout`) and Arion miners (hippocampus bank → `settle_miner_payments`) |
| Bittensor-side backing | The multisig stakes the day's emission into the bridge escrow hotkey — same escrow that backs user deposits |
| Hippius attestation | Existing T-of-N guardian set attests the escrow event; the separate `RewardBridgeWhitelist` is removed |
| Split governance | `Perbill` stored in the bridge pallet, root-set; applied atomically at finalization |
| Multisig operation | Proposer script + independent approver daemons (fully automated, exact-match approval) |

## End-to-end flow

1. **Accrual (Bittensor).** Owner emission for netuid 75 accrues to the multisig
   coldkey as today.
2. **Lock (Bittensor, automated).** Once a day the proposer script computes the
   day's accrued emission from finalized state over a fixed day boundary
   (UTC day → finalized block range, defined once in shared config), and proposes
   a `pallet_multisig` call moving that stake to the bridge escrow hotkey. Each
   co-signer's approver daemon independently recomputes the expected amount and
   approves only on exact match. The finalized executed transfer is the day's
   **anchor event**.
3. **Attest (Hippius).** Each guardian observes the anchor event and calls
   `attest_emission(day_index, amount)`. The pallet recomputes the record ID as
   `blake2_256("EMISSION-V1" || day_index || amount)` — guardians who agree on
   the ID necessarily agree on the parameters (same anti-poisoning scheme as
   deposits). `SettledEmissionDays` makes each day settleable exactly once,
   forever — no replay after TTL cleanup, unlike deposits today.
4. **Distribute (Hippius, atomic).** On threshold: check per-day limit and global
   cap, then split by the root-set `Perbill`. Staker share mints into the staking
   pot (swept by the existing `EraPayout`); miner share mints to the bridge account
   and is pushed via `pallet_hippocampus::deposit_from(bridge, miner_share,
   DepositType::Emission)` so the bank's per-type accounting stays truthful.

## On-chain changes (pallet-alpha-bridge + runtime)

**Removed entirely** (replace, don't deprecate — this flow has not shipped past
this branch): `propose_staking_reward_transfer`, `attest_staking_reward_transfer`,
`admin_cancel_staking_reward_transfer`, `cleanup_staking_reward_transfer`,
`add/remove_reward_bridge_whitelist`, `set_max_mint_per_era`, and storage
`RewardBridgeWhitelist`, `StakingRewardTransfers`, `NextStakingRewardTransferNonce`,
`LastTransferEra`, `EraMintedAmount`, `MaxMintPerEra`. This also removes the
`1b60c8b` defect (guardian check dropped from the reward-transfer calls).

**New storage:**

- `EmissionRecords: map RecordId → EmissionRecord { day_index, amount, votes:
  BTreeSet<AccountId>, status, created_at_block }` — pending attestations,
  TTL-cleaned like deposits.
- `SettledEmissionDays: map u32 → ()` — permanent, never cleaned; idempotency
  backbone.
- `EmissionSplit: Perbill` — staker share; remainder to miners; root-set.
- `MaxDailyEmission: Balance` — hard per-day ceiling, root-set. Replaces the
  era-based limit (the ~6 h era was the wrong unit for a daily flow).

**New extrinsics:**

- `attest_emission(day_index, amount)` — guardian-only; first call creates the
  record (pattern of `attest_deposit`). Rejects settled days, double votes,
  paused state.
- `set_emission_split(Perbill)`, `set_max_daily_emission(Balance)` — root.
- `admin_cancel_emission(record_id)` — root; `cleanup_emission(record_id)` —
  guardian; mirroring the deposit admin/cleanup calls.

**Finalization order:** mark day settled → `amount ≤ MaxDailyEmission` →
global-cap check and `TotalMintedByBridge += amount` → `staker_share =
EmissionSplit * amount`, `miner_share = amount - staker_share` (conservation by
construction, no rounding loss) → mint staker share to `RewardDestination` →
mint miner share to bridge account → `deposit_from` into hippocampus.

**Runtime config:** new associated type (e.g. `MinerEmissionSink`) bound to
hippocampus `deposit_from`, keeping the bridge decoupled from hippocampus.

**Known gotchas to respect (team memory):**

- Escrow-transit payouts with `AllowDeath` can burn sub-ED residuals as dust and
  double-pull on failed transfers (PR #36 finding). The bridge→hippocampus hop
  must use keep-alive semantics and assert the full amount arrived.
- Hippocampus per-type accounting has known sharp edges around
  `TotalUndistributedBacking` (PR #36 findings) — verify `DepositType::Emission`
  deposits interact correctly with `ArionPayoutSource::available()`.

**Cleanup in scope:** fix or delete the stale
`test_propose_staking_reward_transfer_not_guardian_fails`; update
`contracts/bridge/spec.md` (already stale on domain separators, silent on
emission).

## Off-chain automation (`bridge-emission/`, Python)

Follows the `vali-weights-submitter/` ops precedent; talks to subtensor via
`substrate-interface`. Three programs, one shared config (day-boundary
definition, multisig members/threshold, escrow coldkey/hotkey, netuid).

**Proposer (daily cron, holds one multisig member key):**
computes `day_index`; reads emission accrued over the day's finalized block
range; builds `transfer_stake` of that exact amount to the escrow; submits
`as_multi`. Fully idempotent — on restart it checks whether the day's call
already exists or executed. Can propose past missed days; each settles
independently.

**Approver daemon (one per co-signer; separate keys, separate hosts):**
watches `Multisig::NewMultisig` on the multisig account; decodes the pending
call; recomputes the expected amount from its own node's finalized state (never
trusts the proposer); approves only on exact match of amount, destination,
netuid, and day-not-yet-done. Any mismatch → refuse and alert. Wrong or
malicious proposals never reach quorum.

**Guardian extension (existing guardian operators):**
watch for the finalized `MultisigExecuted` + stake-transfer event at the escrow;
derive `(day_index, amount)`; call `attest_emission` on Hippius — mechanical
attestation of an observed fact, exactly like deposits.

**Failure handling:** a missed day is safe — nothing mints without the anchor
event. Alerts fire on proposal-without-quorum after N hours and on attestation
lag.

## Security invariants

1. **Exactly-once per day** — a `day_index` in `SettledEmissionDays` can never
   mint again: not by re-attestation, not after cleanup, not via a second record
   with a different amount.
2. **Conservation** — `staker_share + miner_share == amount` exactly;
   `TotalMintedByBridge` increases by exactly `amount`.
3. **Backing** — every settled emission corresponds to a finalized escrow stake
   transfer of the same amount (enforced by approver daemons and guardian quorum;
   documented as the bridge solvency invariant).
4. **Bounded blast radius** — even with a fully colluding guardian set, mint per
   day ≤ `MaxDailyEmission`, lifetime ≤ `GlobalMintCap`.
5. **One authority set** — guardians gate emissions; the whitelist is gone.

## Testing

- **Pallet unit tests:** attest→finalize happy path; double-settle rejection;
  replay-after-cleanup rejection; split edge cases (0 %, 100 %, odd amounts);
  daily/global cap breaches; pause; non-guardian rejection; vote rollback when
  finalization fails.
- **Runtime integration test** (pattern of `runtime/mainnet/tests/
  referral_mechanics.rs`): emission → split → staking pot swept by `EraPayout`
  to validators, and hippocampus balance → `settle_miner_payments` pays a miner.
- **Property test (proptest):** any sequence of attest/cleanup/cancel calls
  never violates invariants 1–2.
- **Script tests:** approver daemon against recorded proposals — exact match
  approves; wrong amount / destination / netuid / duplicate day all refuse.
- **CI:** add pallet-alpha-bridge to the CI test matrix (its tests are currently
  not gated — known gap).

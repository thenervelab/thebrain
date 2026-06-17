# Alpha Bridge Specification

A minimal viable bridge for transferring Alpha (Bittensor subnet token) to hAlpha (Hippius).

## Overview

The bridge consists of two components:
- **Bittensor Contract** (`contracts/bridge/`) - ink! smart contract on Bittensor
- **Hippius Pallet** (`pallets/alpha-bridge/`) - Substrate pallet on Hippius

Users can:
1. **Deposit**: Lock Alpha on Bittensor → Receive hAlpha on Hippius
2. **Withdraw**: Burn hAlpha on Hippius → Receive Alpha on Bittensor

## Terminology

| Term | Description |
|------|-------------|
| **Alpha** | Subnet token on Bittensor (unit: alphaRao) |
| **hAlpha** | Bridged token on Hippius (unit: halphaRao) |
| **Guardian** | Authorized relayer that attests cross-chain events |
| **Threshold** | Minimum guardian votes required for an action |
| **Source chain** | Chain where user initiates the transfer |
| **Destination chain** | Chain where user receives tokens |

### Naming Convention

| Chain | User Creates | Guardian Creates |
|-------|--------------|------------------|
| **Bittensor** | `deposit_requests` (lock Alpha) | `withdrawals` (release Alpha) |
| **Hippius** | `withdrawal_requests` (burn hAlpha) | `deposits` (credit hAlpha) |

**Mental model:**
- Users create **requests** on the chain they're leaving
- Guardians create the matching **record** on the chain they're entering
- Threshold reached → destination chain executes
- If stuck → admin manually resolves

## Design Principles

1. **Stateless guardians** — Chain is the only source of truth; no Redis or external state
2. **First-attestation-creates-record** — No separate propose step; first guardian vote creates the record
3. **Symmetric recovery** — Both directions have admin recovery paths for stuck transactions
4. **Nonce-based unique IDs** — Monotonic nonces prevent hash collisions
5. **Hash-based ID verification** — Request IDs are deterministic blake2_256 hashes of a chain-specific domain plus the relevant account, amount, and nonce fields, so any parameter mismatch produces a different ID

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                              DEPOSIT FLOW                                    │
│                         (Alpha → hAlpha)                                     │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│   Bittensor (Source)              Guardians              Hippius (Dest)     │
│   ══════════════════              ═════════              ═══════════════    │
│                                                                             │
│   User calls deposit()                                                      │
│         │                                                                   │
│         ▼                                                                   │
│   ┌─────────────────┐                                                       │
│   │ deposit_request │──── DepositRequestCreated ────▶ Monitor event         │
│   │ status=Requested│                                      │                │
│   └─────────────────┘                                      │                │
│                                                            ▼                │
│                                             attest_deposit(..., nonce)      │
│                                                            │                │
│                                                            ▼                │
│                                                   ┌─────────────────┐       │
│                                                   │    deposit      │       │
│                                                   │ status=Pending  │       │
│                                                   └────────┬────────┘       │
│                                                            │                │
│                                                   When threshold reached:   │
│                                                            │                │
│                                                            ▼                │
│                                                   ┌─────────────────┐       │
│                                                   │    deposit      │       │
│                                                   │ status=Completed│       │
│                                                   │ (hAlpha minted) │       │
│                                                   └─────────────────┘       │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────────────┐
│                            WITHDRAWAL FLOW                                   │
│                         (hAlpha → Alpha)                                     │
├─────────────────────────────────────────────────────────────────────────────┤
│                                                                             │
│   Hippius (Source)               Guardians             Bittensor (Dest)     │
│   ════════════════               ═════════             ═════════════════    │
│                                                                             │
│   User calls withdraw()                                                     │
│   (hAlpha burned immediately)                                               │
│         │                                                                   │
│         ▼                                                                   │
│   ┌──────────────────┐                                                      │
│   │withdrawal_request│── WithdrawalRequestCreated ──▶ Monitor event         │
│   │ status=Requested │                                     │                │
│   └──────────────────┘                                     │                │
│                                                            ▼                │
│                                           attest_withdrawal(..., nonce)     │
│                                                            │                │
│                                                            ▼                │
│                                                   ┌─────────────────┐       │
│                                                   │   withdrawal    │       │
│                                                   │ status=Pending  │       │
│                                                   └────────┬────────┘       │
│                                                            │                │
│                                                   When threshold reached:   │
│                                                            │                │
│                                                            ▼                │
│                                                   ┌─────────────────┐       │
│                                                   │   withdrawal    │       │
│                                                   │ status=Completed│       │
│                                                   │ (Alpha released)│       │
│                                                   └─────────────────┘       │
│                                                                             │
└─────────────────────────────────────────────────────────────────────────────┘
```

## State Machines

### Bittensor Contract

#### DepositRequest (Source - user created)

```
┌───────────┐                      ┌────────┐
│ Requested │───── admin_fail ────▶│ Failed │
└───────────┘                      └────────┘
      │
      │ (Alpha stays locked as backing
      │  if Hippius deposit completed)
      ▼
    [END]
```

- `Requested`: Alpha locked, awaiting hAlpha credit on Hippius
- `Failed`: Admin marked after stuck; Alpha manually released via `admin_manual_release`

#### Withdrawal (Destination - guardian created)

```
┌─────────┐                        ┌───────────┐
│ Pending │───── threshold ───────▶│ Completed │
└─────────┘                        └───────────┘
      │
      │ admin_cancel
      ▼
┌───────────┐
│ Cancelled │
└───────────┘
```

- `Pending`: Collecting guardian votes for success
- `Completed`: Threshold reached, Alpha released to recipient
- `Cancelled`: Admin cancelled after stuck

### Hippius Pallet

#### Deposit (Destination - guardian created)

```
┌─────────┐                        ┌───────────┐
│ Pending │───── threshold ───────▶│ Completed │
└─────────┘                        └───────────┘
      │
      │ admin_cancel
      ▼
┌───────────┐
│ Cancelled │
└───────────┘
```

- `Pending`: Collecting guardian votes for success
- `Completed`: Threshold reached, hAlpha credited to recipient
- `Cancelled`: Admin cancelled after stuck

#### WithdrawalRequest (Source - user created)

```
┌───────────┐                      ┌────────┐
│ Requested │───── admin_fail ────▶│ Failed │
└───────────┘                      └────────┘
      │
      │ (hAlpha already burned; stays Requested
      │  while waiting for Bittensor release)
      ▼
    [END]
```

- `Requested`: hAlpha burned, awaiting Alpha release on Bittensor
- `Failed`: Admin marked after stuck; hAlpha manually minted back via `admin_fail_withdrawal_request`

## Extrinsics / Messages — Aligned

| Feature | Bittensor Contract | Hippius Pallet |
|---------|-------------------|----------------|
| **User: Deposit/Withdraw** | `deposit(amount, hotkey)` | `withdraw(amount)` |
| **Guardian: Attest** | `attest_withdrawal(id, recipient, amount, nonce)` | `attest_deposit(id, recipient, amount, nonce)` |
| **Admin: Guardian config** | `set_guardians_and_threshold(guardians, threshold)` | `set_guardians_and_threshold(guardians, threshold)` |
| **Admin: Pause** | `pause()` / `unpause()` | `pause()` / `unpause()` |
| **Admin: Fail request** | `admin_fail_deposit_request(id)` | `admin_fail_withdrawal_request(id)` |
| **Admin: Manual recovery** | `admin_manual_release(recipient, amount, id?)` | `admin_manual_mint(recipient, amount, id?)` |
| **Admin: Cancel dest record** | `admin_cancel_withdrawal(id, reason)` | `admin_cancel_deposit(id, reason)` |
| **Admin: Min amount** | `set_min_deposit_amount(amount)` | `set_min_withdrawal_amount(amount)` |
| **Admin: Cleanup TTL** | `set_cleanup_ttl(blocks)` | `set_cleanup_ttl(blocks)` |
| **Admin: Mint cap** | — | `set_global_mint_cap(cap)` |
| **Guardian: Cleanup source** | `cleanup_deposit_request(id)` | `cleanup_withdrawal_request(id)` |
| **Guardian: Cleanup dest** | `cleanup_withdrawal(id)` | `cleanup_deposit(id)` |
| **Contract-only** | `update_owner(new_owner)`, `set_contract_hotkey(hotkey)`, `set_code(hash)` | — |

## Storage — Aligned

| Feature | Bittensor Contract | Hippius Pallet |
|---------|-------------------|----------------|
| **Guardians** | `guardians: Vec<AccountId>` | `Guardians: Vec<AccountId>` |
| **Threshold** | `approve_threshold: u16` | `ApproveThreshold: u16` |
| **Pause** | `paused: bool` | `Paused: bool` |
| **Min amount** | `min_deposit_amount: Balance` | `MinWithdrawalAmount: u64` |
| **Cleanup TTL** | `cleanup_ttl_blocks: BlockNumber` | `CleanupTTLBlocks: BlockNumberFor<T>` |
| **Source records** | `deposit_requests: Mapping<Hash, DepositRequest>` | `WithdrawalRequests: StorageMap<Hash, WithdrawalRequest>` |
| **Dest records** | `withdrawals: Mapping<Hash, Withdrawal>` | `Deposits: StorageMap<Hash, Deposit>` |
| **Nonce** | `next_deposit_nonce: u64` | `NextWithdrawalRequestNonce: u64` |
| **Nonce lookup** | `nonce_to_deposit_request_id: Mapping<u64, Hash>` | — |
| **Mint cap** | — | `GlobalMintCap: u128` |
| **Total minted** | — | `TotalMintedByBridge: u128` |
| **Owner** | `owner: AccountId` | — (root) |
| **Chain/contract ID** | `chain_id: u16`, `contract_hotkey: AccountId` | — |

## Events — Aligned

| Feature | Contract Event | Pallet Event |
|---------|---------------|--------------|
| **Source request created** | `DepositRequestCreated { deposit_nonce, sender, amount, deposit_request_id }` | `WithdrawalRequestCreated { id, sender, recipient, amount }` |
| **Source request failed** | `DepositRequestFailed` | `WithdrawalRequestFailed` |
| **Dest attested** | `WithdrawalAttested` | `DepositAttested` |
| **Dest completed** | `WithdrawalCompleted` | `DepositCompleted` |
| **Dest cancelled** | `WithdrawalCancelled { withdrawal_id, reason }` | `DepositCancelled { id, reason }` |
| **Manual recovery** | `AdminManualRelease` | `AdminManualMint` |
| **Guardians updated** | `GuardiansUpdated` | `GuardiansUpdated` |
| **Pause** | `Paused` / `Unpaused` | `Paused` / `Unpaused` |
| **Min amount changed** | `MinDepositAmountUpdated` | `MinWithdrawalAmountUpdated` |
| **Cleanup TTL changed** | `CleanupTTLUpdated` | `CleanupTTLUpdated` |
| **Source cleaned up** | `DepositRequestCleanedUp` | `WithdrawalRequestCleanedUp` |
| **Dest cleaned up** | `WithdrawalCleanedUp` | `DepositCleanedUp` |
| **Mint cap changed** | — | `GlobalMintCapUpdated` |
| **Owner changed** | `OwnerUpdated` | — |
| **Hotkey changed** | `ContractHotkeyUpdated` | — |
| **Code upgraded** | `CodeUpgraded` | — |

## Errors — Aligned

| Feature | Contract Error | Pallet Error |
|---------|---------------|--------------|
| **Not authorized** | `Unauthorized` | (root check) |
| **Not guardian** | `NotGuardian` | `NotGuardian` |
| **Already voted** | `AlreadyVoted` | `AlreadyVoted` |
| **Insufficient funds** | `InsufficientStake` | `InsufficientBalance` |
| **Amount too small** | `AmountTooSmall` | `AmountTooSmall` |
| **Invalid thresholds** | `InvalidThresholds` | `ThresholdTooLow` / `ThresholdTooHigh` |
| **Too many guardians** | `TooManyGuardians` | `TooManyGuardians` |
| **Invalid request ID** | `InvalidRequestId` | `InvalidRequestId` |
| **Bridge paused** | `BridgePaused` | `BridgePaused` |
| **Record not found** | `DepositRequestNotFound` / `WithdrawalNotFound` | `DepositNotFound` / `WithdrawalRequestNotFound` |
| **Already finalized** | `DepositRequestAlreadyFinalized` / `WithdrawalAlreadyFinalized` | `DepositAlreadyCompleted` / `WithdrawalRequestAlreadyFinalized` |
| **Cap exceeded** | — | `CapExceeded` |
| **Overflow** | `Overflow` | `ArithmeticOverflow` |
| **Mint failed** | — | `MintFailed` |
| **Accounting underflow** | — | `AccountingUnderflow` |
| **Transfer failed** | `TransferFailed` / `TransferNotVerified` / `RuntimeCallFailed` | — |
| **Stake conviction-locked** | `StakeUnavailable` | — |
| **Stake availability query failed** | `StakeAvailabilityQueryFailed` | — |
| **TTL invalid** | `InvalidTTL` | `InvalidTTL` |
| **Record not finalized** | `RecordNotFinalized` | `RecordNotFinalized` |
| **TTL not expired** | `TTLNotExpired` | `TTLNotExpired` |

## ID Generation

Both chains use deterministic ID generation with monotonic nonces:

### Deposit Request ID (Bittensor)

```
ID = blake2_256(
    DOMAIN_DEPOSIT_REQUEST ||    // "DEPOSIT_REQUEST-V1"
    sender ||                    // encoded AccountId
    halpha_amount ||             // u128, little-endian, amount converted to halphaRao
    nonce                        // u64, little-endian
)
```

Guardians use the emitted `DepositRequestCreated.deposit_nonce` as `nonce` when calling
`attest_deposit`. The pallet recomputes the ID with `recipient = sender` and the hAlpha-side
amount (`alphaRao * 1_000_000_000`).

### Withdrawal Request ID (Hippius)

```
ID = blake2_256(
    DOMAIN_WITHDRAWAL_REQUEST || // "WITHDRAWAL_REQUEST-V1"
    recipient ||                 // encoded AccountId
    amount ||                    // u128, little-endian, alphaRao
    nonce                        // u64, little-endian
)
```

The pallet stores the generated nonce in `WithdrawalRequests[id].nonce`. Guardians include that
nonce when calling `attest_withdrawal`; the contract recomputes the ID with the Alpha-side amount
(`halphaRao / 1_000_000_000`).

## Security Considerations

### Poisoning Prevention (Hash-Based ID Verification)

Both guardian attestation paths recompute the expected request ID from the supplied parameters and
reject if it doesn't match:

- `attest_deposit` verifies `DOMAIN_DEPOSIT_REQUEST || recipient || amount || nonce`, where
  `amount` is in halphaRao.
- `attest_withdrawal` verifies `DOMAIN_WITHDRAWAL_REQUEST || recipient || amount || nonce`, where
  `amount` is in alphaRao.

```rust
// Hippius pallet - attest_deposit
let expected_id = H256::from(sp_core::hashing::blake2_256(&verify_data));
ensure!(expected_id == request_id, Error::<T>::InvalidRequestId);

// Bittensor contract - attest_withdrawal
let expected_id = Hash::from(ink::env::hash_bytes::<Blake2x256>(&data));
if expected_id != request_id { return Err(Error::InvalidRequestId); }
```

This makes poisoning impossible — a malicious guardian cannot create a record with wrong parameters because any change to recipient, amount, or nonce produces a different ID. Subsequent guardians attesting the same ID are guaranteed to agree on the parameters.

### Mint Cap Enforcement

The pallet tracks `TotalMintedByBridge` and enforces `GlobalMintCap`:

- `finalize_deposit`: Checks cap before minting
- `withdraw`: Decrements total (checked subtraction)
- `admin_fail_withdrawal_request`: Checks cap before restoring
- `admin_manual_mint`: Checks cap before minting

### Stake Verification

Before any Alpha leaves a coldkey through the bridge, the contract checks Bittensor's
rolled-forward stake availability via chain extension function `36` (`get_stake_availability`).
The transfer amount must be less than or equal to `available`; total stake that is currently
locked by conviction cannot be deposited, withdrawn, or manually released.

Stake availability is a coldkey/subnet-wide value, matching Bittensor's conviction-lock model.
It is not hotkey-scoped. Deposit still keeps the existing `(coldkey, hotkey, netuid)` stake check
before proxy transfer, and release paths still verify the contract has enough stake on
`contract_hotkey` before transfer. The availability check complements those hotkey-specific
checks by rejecting any amount that exceeds the coldkey's currently unlocked stake on the subnet.

Deployments must run against a Bittensor runtime that exposes chain extension function `36`.
If the extension is missing, fails, returns availability for the wrong subnet, or returns invalid
availability data such as `available > total`, the bridge fails closed with
`StakeAvailabilityQueryFailed`.

The contract also verifies deposit stake transfers by checking balances before and after:

```rust
let sender_decrease = sender_stake_before.saturating_sub(sender_stake_after);
let contract_increase = contract_stake_after.saturating_sub(contract_stake_before);

// Allow small tolerance for rounding (10 alphaRao)
let sender_ok = sender_decrease >= amount.saturating_sub(TRANSFER_TOLERANCE)
    && sender_decrease <= amount;
let contract_ok = contract_increase >= amount.saturating_sub(TRANSFER_TOLERANCE)
    && contract_increase <= amount;
```

### Insufficient Stake Check

Before releasing Alpha, the contract verifies it has sufficient stake on the contract hotkey and
that the contract coldkey has enough available, non-conviction-locked stake:

```rust
let contract_stake = self.get_stake_amount(contract_account, contract_hk, netuid)?;
if contract_stake < withdrawal.amount {
    return Err(Error::InsufficientContractStake);
}
self.ensure_alpha_available(contract_account, netuid, withdrawal.amount)?;
```

Admin manual release bypasses pause and deposit-record validation for emergency recovery, but it
does not bypass Bittensor conviction availability.

### Admin Pause Bypass

Admin recovery functions (`admin_fail_deposit_request`, `admin_manual_release`, `admin_cancel_withdrawal` on Bittensor; `admin_cancel_deposit`, `admin_fail_withdrawal_request`, `admin_manual_mint` on Hippius) intentionally do **not** check the pause state. This is by design — admin must be able to operate during emergencies when the bridge is paused.

## Admin Recovery Workflows

### Stuck Deposit (Alpha locked, hAlpha not credited)

1. Admin calls `admin_cancel_deposit(id, reason)` on Hippius → deposit status = `Cancelled`
2. Admin calls `admin_fail_deposit_request(id)` on Bittensor → status = `Failed`
3. Admin calls `admin_manual_release(sender, amount, deposit_request_id)` on Bittensor → Alpha returned

### Stuck Withdrawal (hAlpha burned, Alpha not released)

1. Admin calls `admin_cancel_withdrawal(id, reason)` on Bittensor → withdrawal status = `Cancelled`
2. Admin calls `admin_fail_withdrawal_request(id)` on Hippius → status = `Failed`, hAlpha minted back

## Transaction Counts

| Operation | User Txs | Guardian Txs | Total (3 guardians) |
|-----------|----------|--------------|---------------------|
| Deposit (Alpha → hAlpha) | 1 | 3 attest | 4 |
| Withdrawal (hAlpha → Alpha) | 1 | 3 attest | 4 |

## TTL Cleanup

### Overview

Records can be cleaned up after a configurable TTL to prevent unbounded storage growth. Guardians can call the cleanup functions to free storage.

### Cleanup Rules

| Record Type | Chain | TTL Measured From | Status Requirement |
|-------------|-------|-------------------|-------------------|
| `deposit_requests` | Bittensor | `created_at_block` | None (any status) |
| `withdrawal_requests` | Hippius | `created_at_block` | None (any status) |
| `withdrawals` | Bittensor | `finalized_at_block` | Must be Completed or Cancelled |
| `deposits` | Hippius | `finalized_at_block` | Must be Completed or Cancelled |

**Source records** (`deposit_requests`, `withdrawal_requests`) can be cleaned up after TTL regardless of status, since the source chain doesn't track whether the destination completed.

**Destination records** (`deposits`, `withdrawals`) can only be cleaned up after finalization and TTL, to ensure pending operations aren't deleted.

### Default TTL

`100,800 blocks` (~7 days at 6 second block times)

Configurable via `set_cleanup_ttl` (admin only). TTL must be greater than zero.

## Constants

| Constant | Contract | Pallet |
|----------|----------|--------|
| Deposit request ID domain | `"DEPOSIT_REQUEST-V1"` | Must match deposit verification |
| Withdrawal request ID domain | `"WITHDRAWAL_REQUEST-V1"` | Must match withdrawal creation |
| MAX_GUARDIANS | 10 | 10 |
| Transfer tolerance | 10 alphaRao | — |
| Default min amount | 1 Alpha (1e9 alphaRao) | 1 hAlpha (1e9 halphaRao) |
| Default cleanup TTL | 100,800 blocks | 100,800 blocks |
| Storage version | — | 0 |

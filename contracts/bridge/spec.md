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
5. **Poisoning prevention** — Subsequent attestations verify recipient/amount match existing record

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
│                                                   attest_deposit()          │
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
│                                                  attest_withdrawal()        │
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

## Storage

### Bittensor Contract

| Storage Item | Type | Description |
|--------------|------|-------------|
| `owner` | `AccountId` | Contract owner with admin privileges |
| `chain_id` | `u16` | Chain identifier (netuid) |
| `contract_hotkey` | `AccountId` | Hotkey for stake consolidation |
| `paused` | `bool` | Emergency pause switch |
| `guardians` | `Vec<AccountId>` | Authorized guardian accounts |
| `approve_threshold` | `u16` | Minimum votes for approval |
| `min_deposit_amount` | `Balance` | Minimum deposit (default: 1 Alpha) |
| `next_deposit_nonce` | `u64` | Monotonic nonce for ID generation |
| `deposit_requests` | `Mapping<Hash, DepositRequest>` | User deposit requests |
| `nonce_to_deposit_request_id` | `Mapping<u64, Hash>` | Nonce to ID lookup |
| `withdrawals` | `Mapping<Hash, Withdrawal>` | Guardian-created withdrawals |
| `cleanup_ttl_blocks` | `BlockNumber` | TTL for record cleanup (default: 100800) |

**Struct field notes:**
- `Withdrawal` struct includes `finalized_at_block: Option<BlockNumber>` to track when the withdrawal was completed or cancelled

### Hippius Pallet

| Storage Item | Type | Description |
|--------------|------|-------------|
| `Guardians` | `Vec<AccountId>` | Authorized guardian accounts |
| `ApproveThreshold` | `u16` | Minimum votes for approval |
| `GlobalMintCap` | `u128` | Maximum total hAlpha mintable |
| `TotalMintedByBridge` | `u128` | Running total of minted hAlpha |
| `Paused` | `bool` | Emergency pause switch |
| `Deposits` | `StorageMap<Hash, Deposit>` | Guardian-created deposits |
| `WithdrawalRequests` | `StorageMap<Hash, WithdrawalRequest>` | User withdrawal requests |
| `NextWithdrawalRequestNonce` | `u64` | Monotonic nonce for ID generation |
| `CleanupTTLBlocks` | `BlockNumberFor<T>` | TTL for record cleanup (default: 100800) |

**Struct field notes:**
- `Deposit` struct includes `finalized_at_block: Option<BlockNumberFor<T>>` to track when the deposit was completed or cancelled

## ID Generation

Both chains use deterministic ID generation with monotonic nonces:

### Deposit Request ID (Bittensor)

```
ID = blake2_256(
    DOMAIN_DEPOSIT_REQUEST ||    // "HIPPIUS_DEPOSIT_REQUEST-V2"
    chain_id ||                  // u16, little-endian
    contract_account ||          // 32 bytes
    nonce ||                     // u64, little-endian
    sender ||                    // 32 bytes
    recipient ||                 // 32 bytes
    amount                       // u64, little-endian
)
```

### Withdrawal Request ID (Hippius)

```
ID = blake2_256(
    DOMAIN_WITHDRAWAL_REQUEST || // "HIPPIUS-WITHDRAWAL-REQUEST-v1"
    sender ||                    // encoded AccountId
    recipient ||                 // encoded AccountId
    amount ||                    // u64, little-endian
    nonce                        // u64, little-endian
)
```

## Extrinsics / Messages

### Bittensor Contract

#### User Actions

| Function | Description |
|----------|-------------|
| `deposit(amount, recipient, hotkey, netuid)` | Lock Alpha, create deposit request |

#### Guardian Actions

| Function | Description |
|----------|-------------|
| `attest_withdrawal(request_id, recipient, amount)` | Vote to release Alpha |

#### Admin Actions

| Function | Description |
|----------|-------------|
| `admin_fail_deposit_request(request_id)` | Mark deposit as failed |
| `admin_manual_release(recipient, amount, deposit_request_id?)` | Release Alpha to user |
| `admin_cancel_withdrawal(request_id)` | Cancel stuck withdrawal |
| `set_guardians_and_threshold(guardians, threshold)` | Update guardian set |
| `pause()` / `unpause()` | Emergency controls |
| `update_owner(new_owner)` | Transfer ownership |
| `set_contract_hotkey(hotkey)` | Update hotkey |
| `set_code(code_hash)` | Upgrade contract |

### Hippius Pallet

#### User Actions

| Extrinsic | Description |
|-----------|-------------|
| `withdraw(amount, recipient)` | Burn hAlpha, create withdrawal request |

#### Guardian Actions

| Extrinsic | Description |
|-----------|-------------|
| `attest_deposit(request_id, recipient, amount)` | Vote to credit hAlpha |

#### Admin Actions (Root)

| Extrinsic | Description |
|-----------|-------------|
| `admin_cancel_deposit(request_id, reason)` | Cancel stuck deposit |
| `admin_fail_withdrawal_request(request_id)` | Fail withdrawal, mint hAlpha back |
| `admin_manual_mint(recipient, amount, deposit_id?)` | Emergency mint hAlpha |
| `add_guardian(guardian)` | Add guardian |
| `remove_guardian(guardian)` | Remove guardian |
| `set_approve_threshold(threshold)` | Update threshold |
| `set_global_mint_cap(cap)` | Update mint cap |
| `set_paused(paused)` | Emergency pause |

## Events

### Bittensor Contract

| Event | When Emitted |
|-------|--------------|
| `DepositRequestCreated` | User locks Alpha |
| `DepositRequestFailed` | Admin marks deposit as failed |
| `WithdrawalAttested` | Guardian votes on withdrawal |
| `WithdrawalCompleted` | Threshold reached, Alpha released |
| `WithdrawalCancelled` | Admin cancels withdrawal |
| `AdminManualRelease` | Admin releases Alpha manually (includes optional `deposit_request_id`) |
| `GuardiansUpdated` | Guardian set changed |
| `Paused` / `Unpaused` | Pause state changed |
| `OwnerUpdated` | Ownership transferred |
| `ContractHotkeyUpdated` | Hotkey changed |
| `CodeUpgraded` | Contract upgraded |

### Hippius Pallet

| Event | When Emitted |
|-------|--------------|
| `DepositAttested` | Guardian votes on deposit |
| `DepositCompleted` | Threshold reached, hAlpha credited |
| `DepositCancelled` | Admin cancels deposit |
| `WithdrawalRequestCreated` | User burns hAlpha |
| `WithdrawalRequestFailed` | Admin marks withdrawal as failed |
| `AdminManualMint` | Admin mints hAlpha manually (includes optional `deposit_id`) |
| `BridgePaused` | Pause state changed |
| `GlobalMintCapUpdated` | Cap changed |
| `ApproveThresholdUpdated` | Threshold changed |
| `GuardianAdded` / `GuardianRemoved` | Guardian set changed |

## Security Considerations

### Poisoning Prevention

When a record already exists, subsequent attestations verify that `recipient` and `amount` match:

```rust
// Bittensor contract - attest_withdrawal
if withdrawal.recipient != recipient {
    return Err(Error::InvalidWithdrawalDetails);
}
if withdrawal.amount != amount {
    return Err(Error::InvalidWithdrawalDetails);
}

// Hippius pallet - attest_deposit
ensure!(deposit.recipient == recipient, Error::<T>::InvalidDepositDetails);
ensure!(deposit.amount == amount, Error::<T>::InvalidDepositDetails);
```

This prevents a malicious guardian from creating a record with wrong parameters that other guardians then vote on.

### Mint Cap Enforcement

The pallet tracks `TotalMintedByBridge` and enforces `GlobalMintCap`:

- `finalize_deposit`: Checks cap before minting
- `withdraw`: Decrements total (checked subtraction)
- `admin_fail_withdrawal_request`: Checks cap before restoring
- `admin_manual_mint`: Checks cap before minting

### Stake Verification

The contract verifies stake transfers by checking balances before and after:

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

Before releasing Alpha, the contract verifies it has sufficient stake:

```rust
let contract_stake = self.get_stake_amount(contract_account, contract_hk, netuid)?;
if contract_stake < withdrawal.amount {
    return Err(Error::InsufficientContractStake);
}
```

### Admin Pause Bypass

Admin recovery functions (`admin_fail_deposit_request`, `admin_manual_release`, `admin_cancel_withdrawal` on Bittensor; `admin_cancel_deposit`, `admin_fail_withdrawal_request`, `admin_manual_mint` on Hippius) intentionally do **not** check the pause state. This is by design — admin must be able to operate during emergencies when the bridge is paused.

## Error Handling

### Bittensor Contract Errors

| Error | Description |
|-------|-------------|
| `Unauthorized` | Caller is not owner |
| `NotGuardian` | Caller is not a guardian |
| `AlreadyVoted` | Guardian already voted |
| `InsufficientStake` | User has insufficient stake |
| `TransferNotVerified` | Stake transfer verification failed |
| `InsufficientContractStake` | Contract lacks stake to release |
| `AmountTooSmall` | Below minimum deposit |
| `InvalidNetUid` | Network UID mismatch |
| `InvalidThresholds` | Invalid guardian configuration |
| `TooManyGuardians` | Exceeds MAX_GUARDIANS (10) |
| `InvalidWithdrawalDetails` | Recipient/amount mismatch (poisoning) |
| `BridgePaused` | Bridge is paused |
| `DepositRequestNotFound` | Deposit request doesn't exist |
| `WithdrawalNotFound` | Withdrawal doesn't exist |
| `DepositRequestAlreadyFinalized` | Already failed |
| `WithdrawalAlreadyFinalized` | Already completed/cancelled |
| `Overflow` | Arithmetic overflow |
| `RuntimeCallFailed` | External call failed |
| `StakeQueryFailed` | Stake query failed |
| `TransferFailed` | Stake transfer failed |
| `StakeConsolidationFailed` | Consolidation failed |
| `CodeUpgradeFailed` | Contract upgrade failed |
| `InvalidTTL` | TTL must be greater than zero |
| `RecordNotFinalized` | Destination record is still Pending |
| `TTLNotExpired` | TTL hasn't passed yet |

### Hippius Pallet Errors

| Error | Description |
|-------|-------------|
| `NotGuardian` | Caller is not a guardian |
| `AlreadyVoted` | Guardian already voted |
| `InsufficientBalance` | User has insufficient hAlpha |
| `CapExceeded` | Would exceed mint cap |
| `BridgePaused` | Bridge is paused |
| `DepositNotFound` | Deposit doesn't exist |
| `WithdrawalRequestNotFound` | Withdrawal request doesn't exist |
| `InvalidStatus` | Wrong status for operation |
| `ThresholdTooLow` | Threshold cannot be zero |
| `ThresholdTooHigh` | Threshold exceeds guardian count |
| `GuardianAlreadyExists` | Guardian already in set |
| `GuardianNotFound` | Guardian not in set |
| `NotAuthorized` | Not authorized for action |
| `AmountConversionFailed` | Balance conversion failed |
| `MintFailed` | Token minting failed |
| `ArithmeticOverflow` | Arithmetic overflow |
| `DepositAlreadyCompleted` | Deposit already finalized |
| `WithdrawalRequestAlreadyFinalized` | Already completed/failed |
| `InvalidDepositDetails` | Recipient/amount mismatch (poisoning) |
| `AmountTooSmall` | Amount is zero |
| `AccountingUnderflow` | Bug in mint tracking |
| `InvalidTTL` | TTL must be greater than zero |
| `RecordNotFinalized` | Destination record is still Pending |
| `TTLNotExpired` | TTL hasn't passed yet |

## Admin Recovery Workflows

### Stuck Deposit (Alpha locked, hAlpha not credited)

1. Admin calls `admin_cancel_deposit(id, reason)` on Hippius → deposit status = `Cancelled`
2. Admin calls `admin_fail_deposit_request(id)` on Bittensor → status = `Failed`
3. Admin calls `admin_manual_release(sender, amount, deposit_request_id)` on Bittensor → Alpha returned

### Stuck Withdrawal (hAlpha burned, Alpha not released)

1. Admin calls `admin_cancel_withdrawal(id)` on Bittensor → withdrawal status = `Cancelled`
2. Admin calls `admin_fail_withdrawal_request(id)` on Hippius → status = `Failed`, hAlpha minted back

## Transaction Counts

| Operation | User Txs | Guardian Txs | Total (3 guardians) |
|-----------|----------|--------------|---------------------|
| Deposit (Alpha → hAlpha) | 1 | 3 attest | 4 |
| Withdrawal (hAlpha → Alpha) | 1 | 3 attest | 4 |

## TTL Cleanup

### Overview

Records can be cleaned up after a configurable TTL to prevent unbounded storage growth. Anyone can call the cleanup functions to free storage.

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

### Cleanup Functions

#### Bittensor Contract

| Function | Description |
|----------|-------------|
| `cleanup_deposit_request(request_id)` | Anyone can call after TTL |
| `cleanup_withdrawal(withdrawal_id)` | Anyone can call after finalization + TTL |
| `set_cleanup_ttl(ttl_blocks)` | Admin sets TTL (in blocks) |
| `cleanup_ttl()` | Query current TTL |

#### Hippius Pallet

| Extrinsic | Description |
|-----------|-------------|
| `cleanup_deposit(deposit_id)` | Anyone can call after finalization + TTL |
| `cleanup_withdrawal_request(request_id)` | Anyone can call after TTL |
| `set_cleanup_ttl(ttl_blocks)` | Root sets TTL (in blocks) |

### Cleanup Events

#### Bittensor Contract

| Event | When Emitted |
|-------|--------------|
| `DepositRequestCleanedUp` | Deposit request deleted |
| `WithdrawalCleanedUp` | Withdrawal deleted |
| `CleanupTTLUpdated` | TTL changed by admin |

#### Hippius Pallet

| Event | When Emitted |
|-------|--------------|
| `DepositCleanedUp` | Deposit deleted |
| `WithdrawalRequestCleanedUp` | Withdrawal request deleted |
| `CleanupTTLUpdated` | TTL changed by root |

### Cleanup Errors

| Error | Description |
|-------|-------------|
| `InvalidTTL` | TTL must be greater than zero |
| `RecordNotFinalized` | Destination record is still Pending |
| `TTLNotExpired` | TTL hasn't passed yet |

---

## Admin-Adjustable Minimum Deposit

The minimum deposit amount can be adjusted by the contract owner to adapt to market conditions.

### Bittensor Contract

| Function | Description |
|----------|-------------|
| `set_min_deposit_amount(amount)` | Owner sets minimum (in alphaRao) |
| `min_deposit_amount()` | Query current minimum |

### Event

| Event | When Emitted |
|-------|--------------|
| `MinDepositAmountUpdated` | Minimum changed by owner |

### Default

`1,000,000,000 alphaRao` (1 Alpha)

---

## Constants

### Bittensor Contract

| Constant | Value | Description |
|----------|-------|-------------|
| `DOMAIN_DEPOSIT_REQUEST` | `"HIPPIUS_DEPOSIT_REQUEST-V2"` | ID domain separator |
| `MAX_GUARDIANS` | 10 | Maximum guardians |
| `TRANSFER_TOLERANCE` | 10 alphaRao | Rounding tolerance |
| `min_deposit_amount` | 1 Alpha (1e9 alphaRao) | Default minimum |

### Hippius Pallet

| Constant | Value | Description |
|----------|-------|-------------|
| `DOMAIN_WITHDRAWAL_REQUEST` | `"HIPPIUS-WITHDRAWAL-REQUEST-v1"` | ID domain separator |
| `MAX_GUARDIANS` | 10 | Maximum guardians |
| `STORAGE_VERSION` | 0 | Current storage version |

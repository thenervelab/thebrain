//! Event definitions for the minimal viable bridge contract

use crate::types::{Balance, BlockNumber, ChainId, DepositNonce, DepositRequestId, WithdrawalId};
use ink::prelude::vec::Vec;

// ============ Deposit Flow Events (Source Side) ============

/// Emitted when a user creates a deposit request by locking Alpha
///
/// **Guardian Action Required**: Monitor this event on Bittensor chain.
/// When seen, call attest_deposit on Hippius pallet with:
/// - request_id: deposit_request_id from this event
/// - recipient: recipient from this event
/// - amount: amount from this event (alphaRao converted to halphaRao)
#[ink::event]
pub struct DepositRequestCreated {
	#[ink(topic)]
	pub chain_id: ChainId,
	pub escrow_contract: ink::primitives::AccountId,
	pub deposit_nonce: DepositNonce,
	#[ink(topic)]
	pub sender: ink::primitives::AccountId,
	#[ink(topic)]
	pub recipient: ink::primitives::AccountId,
	pub amount: Balance,
	pub deposit_request_id: DepositRequestId,
}

/// Emitted when admin marks a deposit request as failed
#[ink::event]
pub struct DepositRequestFailed {
	#[ink(topic)]
	pub deposit_request_id: DepositRequestId,
}

// ============ Withdrawal Flow Events (Destination Side) ============

/// Emitted when a guardian attests a withdrawal
#[ink::event]
pub struct WithdrawalAttested {
	#[ink(topic)]
	pub withdrawal_id: WithdrawalId,
	pub guardian: ink::primitives::AccountId,
	/// Current vote count after this attestation
	pub vote_count: u16,
}

/// Emitted when a withdrawal is completed (Alpha released to recipient)
#[ink::event]
pub struct WithdrawalCompleted {
	#[ink(topic)]
	pub withdrawal_id: WithdrawalId,
	#[ink(topic)]
	pub recipient: ink::primitives::AccountId,
	pub amount: Balance,
}

/// Emitted when admin cancels a withdrawal
#[ink::event]
pub struct WithdrawalCancelled {
	#[ink(topic)]
	pub withdrawal_id: WithdrawalId,
}

// ============ Admin Events ============

/// Emitted when admin manually releases Alpha to a user
#[ink::event]
pub struct AdminManualRelease {
	#[ink(topic)]
	pub recipient: ink::primitives::AccountId,
	pub amount: Balance,
}

/// Emitted when guardian set and thresholds are updated
#[ink::event]
pub struct GuardiansUpdated {
	pub guardians: Vec<ink::primitives::AccountId>,
	pub approve_threshold: u16,
	pub updated_by: ink::primitives::AccountId,
}

/// Emitted when bridge is paused
#[ink::event]
pub struct Paused {
	pub paused_by: ink::primitives::AccountId,
}

/// Emitted when bridge is unpaused
#[ink::event]
pub struct Unpaused {
	pub unpaused_by: ink::primitives::AccountId,
}

/// Emitted when owner is updated
#[ink::event]
pub struct OwnerUpdated {
	pub old_owner: ink::primitives::AccountId,
	pub new_owner: ink::primitives::AccountId,
}

/// Emitted when contract hotkey is updated
#[ink::event]
pub struct ContractHotkeyUpdated {
	pub old_hotkey: ink::primitives::AccountId,
	pub new_hotkey: ink::primitives::AccountId,
	pub updated_by: ink::primitives::AccountId,
}

/// Emitted when contract code is upgraded
#[ink::event]
pub struct CodeUpgraded {
	pub code_hash: ink::primitives::Hash,
	pub upgraded_by: ink::primitives::AccountId,
}

// ============ Cleanup Events ============

/// Emitted when a deposit request is cleaned up after TTL
#[ink::event]
pub struct DepositRequestCleanedUp {
	#[ink(topic)]
	pub deposit_request_id: DepositRequestId,
}

/// Emitted when a withdrawal is cleaned up after TTL
#[ink::event]
pub struct WithdrawalCleanedUp {
	#[ink(topic)]
	pub withdrawal_id: WithdrawalId,
}

/// Emitted when the cleanup TTL is updated
#[ink::event]
pub struct CleanupTTLUpdated {
	pub old_ttl: BlockNumber,
	pub new_ttl: BlockNumber,
}

/// Emitted when the minimum deposit amount is updated
#[ink::event]
pub struct MinDepositAmountUpdated {
	pub old_amount: Balance,
	pub new_amount: Balance,
}

use crate::types::{Balance, BurnId, ChainId, CheckpointNonce, DepositId, DepositNonce};
use ink::prelude::vec::Vec;

/// Emitted when a deposit is successfully locked on Bittensor
///
/// **Guardian Action Required**: Monitor this event on Bittensor chain.
/// When seen, create a DepositProposal for the Hippius pallet with:
/// - id: deposit_id from this event
/// - recipient: sender from this event
/// - amount: amount from this event
#[ink::event]
pub struct DepositMade {
	#[ink(topic)]
	pub chain_id: ChainId,
	pub escrow_contract: ink::primitives::AccountId,
	pub deposit_nonce: DepositNonce,
	#[ink(topic)]
	pub sender: ink::primitives::AccountId,
	pub amount: Balance,
	#[ink(topic)]
	pub deposit_id: DepositId,
}

#[ink::event]
pub struct Released {
	#[ink(topic)]
	pub burn_id: BurnId,
	#[ink(topic)]
	pub recipient: ink::primitives::AccountId,
	pub amount: Balance,
}

#[ink::event]
pub struct Refunded {
	#[ink(topic)]
	pub deposit_id: DepositId,
	#[ink(topic)]
	pub recipient: ink::primitives::AccountId,
	pub amount: Balance,
}

#[ink::event]
pub struct BurnsProposed {
	#[ink(topic)]
	pub checkpoint_nonce: CheckpointNonce,
	pub proposer: ink::primitives::AccountId,
	pub burns_count: u32,
}

#[ink::event]
pub struct BurnDenied {
	#[ink(topic)]
	pub burn_id: BurnId,
}

#[ink::event]
pub struct BurnExpired {
	#[ink(topic)]
	pub burn_id: BurnId,
}

#[ink::event]
pub struct RefundsProposed {
	#[ink(topic)]
	pub checkpoint_nonce: CheckpointNonce,
	pub proposer: ink::primitives::AccountId,
	pub refunds_count: u32,
}

#[ink::event]
pub struct RefundDenied {
	#[ink(topic)]
	pub deposit_id: DepositId,
}

#[ink::event]
pub struct RefundExpired {
	#[ink(topic)]
	pub deposit_id: DepositId,
}

#[ink::event]
pub struct GuardiansUpdated {
	pub guardians: Vec<ink::primitives::AccountId>,
	pub approve_threshold: u16,
	pub deny_threshold: u16,
	pub updated_by: ink::primitives::AccountId,
}

#[ink::event]
pub struct Paused {
	pub paused_by: ink::primitives::AccountId,
}

#[ink::event]
pub struct Unpaused {
	pub unpaused_by: ink::primitives::AccountId,
}

#[ink::event]
pub struct OwnerUpdated {
	pub old_owner: ink::primitives::AccountId,
	pub new_owner: ink::primitives::AccountId,
}

#[ink::event]
pub struct ContractHotkeyUpdated {
	pub old_hotkey: ink::primitives::AccountId,
	pub new_hotkey: ink::primitives::AccountId,
	pub updated_by: ink::primitives::AccountId,
}

#[ink::event]
pub struct CodeUpgraded {
	pub code_hash: ink::primitives::Hash,
	pub upgraded_by: ink::primitives::AccountId,
}

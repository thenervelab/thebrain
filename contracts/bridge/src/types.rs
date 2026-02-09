//! Type definitions for the minimal viable bridge contract
//!
//! ## Naming Convention
//! - `deposit_requests` — Created by users on Bittensor (Alpha locked, waiting for hAlpha on Hippius)
//! - `withdrawals` — Created by guardians on Bittensor (Hippius withdrawal observed, release Alpha)

// Allow truncation warnings from scale_derive macro expansion on enum discriminants
#![allow(clippy::cast_possible_truncation)]

use ink::prelude::vec::Vec;
use ink::primitives::Hash;

pub type Balance = u64;
pub type ChainId = u16;
pub type DepositNonce = u64;
pub type BlockNumber = u32;

/// Unique identifier for deposit requests (created by users)
pub type DepositRequestId = Hash;

/// Unique identifier for withdrawals (created by guardians, matches Hippius withdrawal_request ID)
pub type WithdrawalId = Hash;

/// Domain separator for deposit request ID generation (prevents hash collision)
pub const DOMAIN_DEPOSIT_REQUEST: &[u8] = b"HIPPIUS_DEPOSIT_REQUEST-V2";

/// Maximum number of guardians allowed in the guardian set
pub const MAX_GUARDIANS: usize = 10;

/// Tolerance for stake transfer verification (10 alphaRao)
/// Accounts for rounding in Subtensor pallet's transfer_stake operation
pub const TRANSFER_TOLERANCE: u64 = 10;

/// Status of a deposit request (source side - user created)
///
/// Note: Bittensor (source) does not track whether Hippius (destination) credited.
/// Hippius is the source of truth for crediting. Deposits stay in Requested status
/// while Alpha remains locked as backing for minted hAlpha.
#[derive(Debug, Clone, PartialEq, Eq)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
#[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
pub enum DepositRequestStatus {
	/// Alpha locked, awaiting credit on Hippius (or already credited - Hippius is source of truth)
	Requested,
	/// Admin marked after stuck (Alpha manually released)
	Failed,
}

/// Status of a withdrawal (destination side - guardian created)
#[derive(Debug, Clone, PartialEq, Eq)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
#[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
pub enum WithdrawalStatus {
	/// Collecting votes for success
	Pending,
	/// Alpha released to recipient
	Completed,
	/// Admin cancelled after stuck
	Cancelled,
}

/// Reason for cancellation (for audit trail)
#[derive(Debug, Clone, PartialEq, Eq)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
#[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
pub enum CancelReason {
	AdminEmergency,
}

/// A deposit request created by a user locking Alpha
#[derive(Debug, Clone, PartialEq, Eq)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
#[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
pub struct DepositRequest {
	/// Original sender who locked the stake
	pub sender: ink::primitives::AccountId,
	/// Recipient on Hippius who will receive hAlpha
	pub recipient: ink::primitives::AccountId,
	/// Amount locked (in alphaRao)
	pub amount: Balance,
	/// Nonce used for ID generation
	pub nonce: DepositNonce,
	/// Hotkey used for the stake
	pub hotkey: ink::primitives::AccountId,
	/// Network UID where the stake is locked
	pub netuid: u16,
	/// Current status
	pub status: DepositRequestStatus,
	/// Block when request was created
	pub created_at_block: BlockNumber,
}

/// A withdrawal record created by guardians when they observe a withdrawal_request on Hippius
#[derive(Debug, Clone, PartialEq, Eq)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
#[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
pub struct Withdrawal {
	/// The withdrawal request ID from Hippius
	pub request_id: WithdrawalId,
	/// Recipient on Bittensor who will receive Alpha
	pub recipient: ink::primitives::AccountId,
	/// Amount to release (in alphaRao)
	pub amount: Balance,
	/// Guardian votes for success
	pub votes: Vec<ink::primitives::AccountId>,
	/// Current status
	pub status: WithdrawalStatus,
	/// Block when first guardian attested
	pub created_at_block: BlockNumber,
	/// Block when withdrawal was finalized (Completed or Cancelled)
	pub finalized_at_block: Option<BlockNumber>,
}
//! Error types for the minimal viable bridge contract

use ink::prelude::{format, string::String};

#[allow(clippy::cast_possible_truncation)]
#[derive(Debug, PartialEq, Eq, Clone)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
pub enum Error {
	// ============ Authorization Errors ============
	/// Caller is not the contract owner
	Unauthorized,
	/// Caller is not a guardian
	NotGuardian,
	/// Guardian has already voted on this withdrawal
	AlreadyVoted,

	// ============ Balance/Stake Errors ============
	/// User has insufficient stake to lock
	InsufficientStake,
	/// Stake transfer could not be verified
	TransferNotVerified,
	/// Contract has insufficient stake to release
	InsufficientContractStake,

	// ============ Validation Errors ============
	/// Deposit amount is below minimum
	AmountTooSmall,
	/// Guardian thresholds are invalid
	InvalidThresholds,
	/// Too many guardians provided
	TooManyGuardians,
	/// TTL must be greater than zero
	InvalidTTL,

	// ============ State Errors ============
	/// Bridge is paused
	BridgePaused,
	/// Deposit request not found
	DepositRequestNotFound,
	/// Withdrawal not found
	WithdrawalNotFound,
	/// Deposit request already finalized
	DepositRequestAlreadyFinalized,
	/// Withdrawal already finalized
	WithdrawalAlreadyFinalized,

	// ============ Arithmetic Errors ============
	/// Arithmetic overflow
	Overflow,

	// ============ External Call Errors ============
	/// Runtime call failed
	RuntimeCallFailed,
	/// Stake query failed
	StakeQueryFailed,
	/// Stake transfer failed
	TransferFailed,
	/// Stake consolidation failed
	StakeConsolidationFailed,
	/// Code upgrade failed
	CodeUpgradeFailed,

	// ============ Validation Errors (ID) ============
	/// Recomputed request ID does not match the provided one
	InvalidRequestId,

	// ============ Cleanup Errors ============
	/// Record is not finalized (must be Completed or Cancelled)
	RecordNotFinalized,
	/// TTL has not expired yet
	TTLNotExpired,
}

impl From<Error> for String {
	fn from(error: Error) -> Self {
		format!("{:?}", error)
	}
}

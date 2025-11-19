use ink::prelude::{format, string::String};

#[allow(clippy::cast_possible_truncation)]
#[derive(Debug, PartialEq, Eq, Clone)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
pub enum Error {
	// Authorization errors
	Unauthorized,
	NotGuardian,
	AlreadyAttestedBurn,
	AlreadyAttestedRefund,

	// Balance errors
	InsufficientStake,
	TransferNotVerified,

	// Validation errors
	AmountTooSmall,
	InvalidHotkey,
	InvalidNonce,
	InvalidNetUid,
	TooManyDeposits,
	InvalidCheckpointNonce,
	InvalidThresholds,
	TooManyGuardians,
	InvalidRefundRecipient,
	InvalidRefundAmount,

	// State errors
	BridgePaused,
	AlreadyProcessed,
	DepositAlreadyDenied,
	CheckpointNotFound,
	CheckpointExpired,
	CheckpointNotExpired,

	// Arithmetic errors
	Overflow,

	// External call errors
	RuntimeCallFailed,
	StakeQueryFailed,
	TransferFailed,
	StakeConsolidationFailed,
	CodeUpgradeFailed,
}

impl From<Error> for String {
	fn from(error: Error) -> Self {
		format!("{:?}", error)
	}
}

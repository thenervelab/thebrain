use ink::prelude::string::String;

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
}

impl From<Error> for String {
	fn from(error: Error) -> Self {
		format!("{:?}", error)
	}
}

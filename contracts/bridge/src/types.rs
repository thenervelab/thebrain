use ink::prelude::vec::Vec;
use ink::primitives::Hash;
use ink::storage::Mapping;

pub type Balance = u64;
pub type ChainId = u16;
pub type DepositNonce = u64;
pub type CheckpointNonce = u64;
pub type DepositId = Hash;
pub type BurnId = Hash;
pub type BlockNumber = u32;

/// Domain separator for deposit ID generation (prevents hash collision)
pub const DOMAIN_DEPOSIT: &[u8] = b"HIPPIUS_DEPOSIT-V1";
/// Maximum number of guardians allowed in the guardian set
pub const MAX_GUARDIANS: usize = 10; // TODO: adjust as needed
/// Tolerance for stake transfer verification (10 rao)
/// Accounts for rounding in Subtensor pallet's transfer_stake operation
pub const TRANSFER_TOLERANCE: u64 = 10;

pub type LockedMapping = Mapping<DepositId, Balance>;
pub type ProcessedDepositsMapping = Mapping<DepositId, bool>;
pub type DeniedDepositsMapping = Mapping<DepositId, bool>;
pub type PendingBurnsMapping = Mapping<BurnId, PendingBurn>;
pub type PendingRefundsMapping = Mapping<DepositId, PendingRefund>;
pub type UserDepositsMapping = Mapping<ink::primitives::AccountId, Vec<DepositId>>;
pub type DepositMetadataMapping = Mapping<DepositId, DepositMetadata>;

/// Metadata for a deposit, storing all context needed for releases and refunds
#[derive(Debug, Clone, PartialEq, Eq)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
#[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
pub struct DepositMetadata {
	/// Original sender who locked the stake on Bittensor
	pub sender: ink::primitives::AccountId,
	/// Hotkey used for the stake
	pub hotkey: ink::primitives::AccountId,
	/// Network UID where the stake is locked
	pub netuid: u16,
	/// Amount locked (in rao)
	pub amount: Balance,
}

/// Burn item from Hippius checkpoint (releases locked stake to recipient)
#[derive(Debug, Clone, PartialEq, Eq)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
#[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
pub struct CanonicalBurn {
	/// Burn ID from Hippius
	pub burn_id: BurnId,
	/// Recipient coldkey to receive the released stake
	pub recipient: ink::primitives::AccountId,
	/// Hotkey where the stake should be released
	pub hotkey: ink::primitives::AccountId,
	/// Network UID for the release
	pub netuid: u16,
	/// Amount to release (in rao)
	pub amount: Balance,
}

/// Refund item (returns locked stake when deposit is denied on Hippius)
#[derive(Debug, Clone, PartialEq, Eq)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
#[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
pub struct RefundItem {
	/// Deposit ID to refund
	pub deposit_id: DepositId,
	/// Recipient coldkey to receive the refund (should match original sender)
	pub recipient: ink::primitives::AccountId,
	/// Amount to refund (in rao)
	pub amount: Balance,
}

/// Pending burn approval record for individual burn
#[derive(Debug, Clone, PartialEq, Eq)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
#[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
pub struct PendingBurn {
	pub burn_id: BurnId,
	pub recipient: ink::primitives::AccountId,
	pub hotkey: ink::primitives::AccountId,
	pub netuid: u16,
	pub amount: Balance,
	pub approves: Vec<ink::primitives::AccountId>,
	pub denies: Vec<ink::primitives::AccountId>,
	pub proposed_at: BlockNumber,
}

/// Pending refund approval record for individual deposit refund
#[derive(Debug, Clone, PartialEq, Eq)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
#[cfg_attr(feature = "std", derive(ink::storage::traits::StorageLayout))]
pub struct PendingRefund {
	pub deposit_id: DepositId,
	pub recipient: ink::primitives::AccountId,
	pub amount: Balance,
	pub approves: Vec<ink::primitives::AccountId>,
	pub denies: Vec<ink::primitives::AccountId>,
	pub proposed_at: BlockNumber,
}

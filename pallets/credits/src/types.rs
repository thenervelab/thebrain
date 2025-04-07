use codec::{Decode, Encode};
use frame_support::pallet_prelude::*;
use scale_info::TypeInfo;
// use frame_system::{pallet_prelude::BlockNumberFor, offchain::SignedPayload};
use scale_info::prelude::vec::Vec;

// Define a struct for lock period
#[derive(Encode, Decode, Clone, PartialEq, Eq, Default, TypeInfo, MaxEncodedLen)]
pub struct LockPeriod<BlockNumber> {
	/// Start block of the lock period
	pub start_block: BlockNumber,
	/// End block of the lock period
	pub end_block: BlockNumber,
}

// Define a struct for locked credits
#[derive(Encode, Decode, Clone, PartialEq, Eq, Default, TypeInfo)]
pub struct LockedCredit<AccountId, BlockNumber> {
	pub owner: AccountId,
	pub amount_locked: u128,
	pub is_fulfilled: bool,
	pub tx_hash: Option<Vec<u8>>,
	pub created_at: BlockNumber,
	pub id: u64,
	pub is_migrated: bool,
}

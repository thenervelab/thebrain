use codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use frame_support::pallet_prelude::*;
use sp_std::prelude::*;
use serde::{Serialize, Deserialize};
use crate::Config;
use sp_std::{prelude::*, marker::PhantomData};
use frame_system::offchain::SignedPayload;

// Define maximum lengths for bounded vectors
pub const MAX_FILE_HASH_LENGTH: u32 = 350;
pub const MAX_FILE_NAME_LENGTH: u32 = 350;
pub const MAX_NODE_ID_LENGTH: u32 = 64;
pub const MAX_MINER_IDS: u32 = 5;
pub const MAX_BLACKLIST_ENTRIES: u32 = 350;
pub const MAX_UNPIN_REQUESTS: u32 = 350;
pub const MAX_REBALANCE_REQUESTS: u32 = 100000; 

/// Unique identifier for a node
pub type FileHash = BoundedVec<u8, ConstU32<MAX_FILE_HASH_LENGTH>>;
/// Unique identifier for a file name
pub type FileName = BoundedVec<u8, ConstU32<MAX_FILE_NAME_LENGTH>>;

#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo, MaxEncodedLen, Serialize, Deserialize)]
pub struct StorageUnpinRequest<AccountId> {
    pub owner: AccountId,
    pub file_hash: FileHash,
    pub selected_validator: AccountId,
}

// This will store info related storage request
#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo, MaxEncodedLen, Serialize, Deserialize)]
pub struct StorageRequest<AccountId, BlockNumberFor> {
    pub total_replicas: u32,
    pub owner: AccountId,
    pub file_hash: FileHash,
    pub file_name: FileName,
    pub last_charged_at: BlockNumberFor,
    pub created_at: BlockNumberFor,
    pub miner_ids: Option<BoundedVec<BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>, ConstU32<MAX_MINER_IDS>>>,
    pub selected_validator: AccountId,
    pub is_assigned: bool,
}

#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo, MaxEncodedLen, Serialize, Deserialize)]
pub enum MinerState {
    Free,
    Locked
}

impl Default for MinerState {
    fn default() -> Self {
        MinerState::Free
    }
}

#[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct MinerProfileItem {
    pub miner_node_id: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>,
    pub cid: FileHash,
    pub files_count: u32,
    pub files_size: u128,
}

#[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct FileInput {
    pub file_hash: Vec<u8>,
    pub file_name: Vec<u8>,
}


#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct StorageRequestUpdate<AccountId> {
	pub storage_request_owner: AccountId,
	pub storage_request_file_hash: FileHash,
	pub file_size: u128,
	pub user_profile_cid: FileHash,
}


#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct StorageUnpinUpdateRequest<AccountId> {
	pub miner_pin_requests: Vec<MinerProfileItem>,
	pub storage_request_owner: AccountId,
	pub storage_request_file_hash: FileHash,
	pub file_size: u128,
	pub user_profile_cid: FileHash,
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct UpdatedMinerProfileItem {
    pub miner_node_id: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>,
    pub cid: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>,
    pub added_files_count: u32,
    pub added_file_size : u128
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct UpdatedUserProfileItem<AccountId> {
    pub user: AccountId,
    pub cid: FileHash,
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug)]
pub struct ProcessMinersToRemoveRequestPayload<T: Config> {
    pub owner: T::AccountId,
    pub file_hash_to_remove: FileHash,
    pub miner_node_id: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>,
    pub new_cid_bounded: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>,
    pub public: T::Public,
    pub _marker: PhantomData<T>,
}

// Implement SignedPayload for ProcessMinersToRemoveRequestPayload
impl<T: Config> SignedPayload<T> for ProcessMinersToRemoveRequestPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

/// Represents a user's file with its pinning information
#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo)]
pub struct UserFile {
    pub file_hash: FileHash,
    pub file_name: FileName,
    pub miner_ids: Vec<Vec<u8>>,
    pub file_size: u32,
    pub created_at: u32,
}

/// Storage map to track miner lock information.
#[derive(Clone, Encode, Decode, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct MinersLockInfo<AccountId,BlockNumber> {
	pub miners_locked: bool,
	pub locker: AccountId, // Validator address
	pub locked_at: BlockNumber, // Block number when locked
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct RebalanceRequestItem {
    pub node_id: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>,
    pub miner_profile_id: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>,
}
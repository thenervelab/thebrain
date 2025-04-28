use codec::{Decode, Encode, MaxEncodedLen};
use scale_info::TypeInfo;
use frame_support::pallet_prelude::*;
use sp_std::prelude::*;
use serde::{Serialize, Deserialize};
use crate::Config;
use sp_std::{prelude::*, marker::PhantomData};
use frame_system::{pallet_prelude::BlockNumberFor, offchain::SignedPayload};

// Define maximum lengths for bounded vectors
pub const MAX_FILE_HASH_LENGTH: u32 = 350;
pub const MAX_FILE_NAME_LENGTH: u32 = 350;
pub const MAX_NODE_ID_LENGTH: u32 = 64;
pub const MAX_MINER_IDS: u32 = 5;
pub const MAX_BLACKLIST_ENTRIES: u32 = 350;
pub const MAX_UNPIN_REQUESTS: u32 = 350;

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


// This will store info relasinceted storage request
#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo, MaxEncodedLen, Serialize, Deserialize)]
pub struct MinerPinRequest<BlockNumber> {
    pub miner_node_id: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>,
    pub file_hash: FileHash,
    pub created_at: BlockNumber,
    pub file_size_in_bytes: u32,
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
}

#[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct FileInput {
    pub file_hash: Vec<u8>,
    pub file_name: Vec<u8>,
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug)]
pub struct UpdateIpfsRequestPayload<T: Config> {
    pub node_id: Vec<u8>,
    pub miner_pin_requests: Vec<MinerProfileItem>,
    pub storage_requests: StorageRequest<T::AccountId, BlockNumberFor<T>>,
    pub file_size: u128,
    pub public: T::Public,
    pub _marker: PhantomData<T>,
}

// Implement SignedPayload for UpdateRankingsPayload
impl<T: Config> SignedPayload<T> for UpdateIpfsRequestPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
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

/// Payload for updating UserProfile
#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug)]
pub struct UpdateUserProfilePayload<T: Config> {
    pub owner: T::AccountId,
    pub cid: FileHash,
    pub node_identity: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>,
    pub public: T::Public,
    pub _marker: PhantomData<T>,
}

// Implement SignedPayload for UpdateUserProfilePayload
impl<T: Config> SignedPayload<T> for UpdateUserProfilePayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}


/// Payload for updating UserProfile
#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug)]
pub struct MinerLockPayload<T: Config> {
    pub miner_node_ids: Vec<BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>>,
    pub block_number: BlockNumberFor<T>,
    pub node_identity: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>,
    pub public: T::Public,
    pub _marker: PhantomData<T>,
}

// Implement SignedPayload for MinerLockPayload
impl<T: Config> SignedPayload<T> for MinerLockPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

#[derive(Encode, Decode, TypeInfo)]
pub struct MarkStorageRequestAssignedPayload<T: Config> {
    pub owner: T::AccountId,
    pub file_hash: FileHash,
    pub public: T::Public,
    pub _marker: PhantomData<T>,
}

// Implement SignedPayload for MinerLockPayload
impl<T: Config> SignedPayload<T> for MarkStorageRequestAssignedPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug)]
pub struct UpdateMinerProfilePayload<T: Config> {
    pub miner_pin_requests: Vec<MinerProfileItem>,
    pub public: T::Public,
    pub _marker: PhantomData<T>,
}

// Implement SignedPayload for UpdateMinerProfilePayload
impl<T: Config> SignedPayload<T> for UpdateMinerProfilePayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

/// Storage map to track miner lock information.
#[derive(Clone, Encode, Decode, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct MinersLockInfo<AccountId,BlockNumber> {
	pub miners_locked: bool,
	pub locker: AccountId, // Validator address
	pub locked_at: BlockNumber, // Block number when locked
}
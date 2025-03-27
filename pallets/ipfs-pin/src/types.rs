use codec::{Decode, Encode};
use scale_info::TypeInfo;
use frame_support::pallet_prelude::*;
use sp_std::{prelude::*, marker::PhantomData};
use frame_system::{pallet_prelude::BlockNumberFor, offchain::SignedPayload};

use crate::Config;

/// Unique identifier for a node
pub type FileHash = Vec<u8>;
/// Unique identifier for a node
pub type FileName = Vec<u8>;

// This will store info related storage request
#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo)]
pub struct StorageRequest<AccountId, BlockNumberFor> {
	pub total_replicas: u32,
    pub fullfilled_replicas: u32,
    pub owner: AccountId,
    pub file_hash: FileHash,
    pub file_name: FileName,
    pub is_approved: bool,
    pub last_charged_at: BlockNumberFor,
    pub miner_ids: Option<Vec<Vec<u8>>>, // Optional if they have selected a miner
}

// This will store info related storage request
#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo)]
pub struct PinRequest<BlockNumber> {
    pub miner_node_id: Vec<u8>,
    pub file_hash: FileHash,
    pub is_pinned: bool,
    pub created_at: BlockNumber,
    pub file_size_in_bytes: u32,
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug)]
pub struct UpdateIpfsRequestPayload<T: Config> {
    pub node_id: Vec<u8>,
    pub pin_requests: Vec<PinRequest<BlockNumberFor<T>>>,
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
pub struct StoreIpfsPinPayload<T: Config> {
    pub file_hash: FileHash,
    pub node_id: Vec<u8>,
    pub file_size_in_bytes: u32,
    pub account_id: T::AccountId,
    pub public: T::Public,
    pub _marker: PhantomData<T>,
}

// Implement SignedPayload for UpdateRankingsPayload
impl<T: Config> SignedPayload<T> for StoreIpfsPinPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct RemoveIpfsUnpinPayload<T: Config> {
    pub file_hashes_updated_vec: Vec<FileHash>,
    pub node_id: Vec<u8>,
    pub public: T::Public,
    pub _marker: PhantomData<T>,
}

// Implement SignedPayload for UpdateRankingsPayload
impl<T: Config> SignedPayload<T> for RemoveIpfsUnpinPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct UpdateStorageUsagePayload<T: Config> {
    pub file_hash: FileHash,
    pub storage_request: Option<StorageRequest<T::AccountId, BlockNumberFor<T>>>,
    pub node_id: Vec<u8>,
    pub account_id: T::AccountId,
    pub public: T::Public,
    pub _marker: PhantomData<T>,
}

impl<T: Config> SignedPayload<T> for UpdateStorageUsagePayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

/// Represents a user's file with its pinning information
#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo)]
pub struct UserFile<BlockNumberFor> {
    pub file_hash: FileHash,
    pub file_name: FileName,
    pub miner_ids: Vec<Vec<u8>>,
    pub file_size: u32,
    pub date: BlockNumberFor,
}
use codec::{Decode, Encode};
use frame_support::pallet_prelude::*;
use sp_std::prelude::*;
use scale_info::TypeInfo;
use crate::Config;
use frame_system::offchain::SignedPayload;
use sp_std::marker::PhantomData;

/// Metadata for each snapshot
#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct SnapshotMetadata {
    pub cid: Vec<u8>,           // IPFS CID of the snapshot
    pub block_number: u32,     // Block number when the snapshot was added
    pub description: Option<Vec<u8>>, // Optional description of the snapshot
    pub request_id: u32,            // The compute request id
}

/// Metadata for each snapshot
#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct RestoreSnapshotMetadata {
    pub cid: Vec<u8>,           // IPFS CID of the snapshot
    pub block_number: u32,     // Block number when the snapshot was added
    pub description: Option<Vec<u8>>, // Optional description of the snapshot
    pub request_id: u32,            // The compute request id
    pub minner_request_id: u32,            // The compute request id
    pub is_fulfilled: bool,     // Whether the compute request is fulfilled
}

/// Metadata for each backup
#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct BackupMetadata<AccountId> {
    pub owner: AccountId,             // Owner of the backup
    pub snapshots: Vec<SnapshotMetadata>, // List of snapshot metadata
    pub last_snapshot: Option<Vec<u8>>, // CID of the latest snapshot
    pub created_at: u32,              // Block number of creation
    pub updated_at: u32,              // Block number of last update
    pub description: Option<Vec<u8>>, // Optional description
}

// Add a new payload type for the mark_compute_request_fulfilled transaction
#[derive(Encode, Decode, Clone, PartialEq, Eq, Debug, TypeInfo)]
pub struct DeleteUserStoragePayload<T: Config> {
    pub minner_account_id: Vec<u8>,
    pub account_id: T::AccountId,
    pub public: T::Public,
    pub _marker: PhantomData<T>,
}
        
impl<T: Config> SignedPayload<T> for DeleteUserStoragePayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}
    
/// Payload structure for add_snapshot unsigned transaction
#[derive(Encode, Decode, Clone)]
pub struct AddSnapshotPayload<T: Config> {
    pub node_id: Vec<u8>,
    pub snapshot_cid: Vec<u8>,
    pub description: Option<Vec<u8>>,
    pub request_id: u32,
    pub account_id: T::AccountId,
    pub public: T::Public,
    pub _marker: PhantomData<T>,
}

impl<T: Config> SignedPayload<T> for AddSnapshotPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}


// Add a new payload type for the mark_compute_request_fulfilled transaction
#[derive(Encode, Decode, Clone, PartialEq, Eq, Debug, TypeInfo)]
pub struct RestoreRequestFulfilledPayload<T: Config> {
	pub node_id: Vec<u8>,
	pub request_id: u32,
    pub public: T::Public,
	pub _marker: PhantomData<T>,
}

impl<T: Config> SignedPayload<T> for RestoreRequestFulfilledPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}


/// Payload for backup deletion request
#[derive(Encode, Decode, Clone, PartialEq, Eq, Debug, TypeInfo)]
pub struct BackupDeletionRequestPayload<T: Config> {
	pub node_id: Vec<u8>,
	pub public: T::Public,
	pub _marker: PhantomData<T>,
}

impl<T: Config> SignedPayload<T> for BackupDeletionRequestPayload<T> {
	fn public(&self) -> T::Public {
		self.public.clone()
	}
}

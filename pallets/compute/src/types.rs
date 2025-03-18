use codec::{Decode, Encode};
use frame_support::pallet_prelude::*;
use sp_std::prelude::*;
use scale_info::TypeInfo;
use crate::Config;
use frame_system::offchain::SignedPayload;
use sp_std::marker::PhantomData;
use frame_system::pallet_prelude::BlockNumberFor;

#[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct ImageMetadata {
    pub name: Vec<u8>,       // Name of the image
    pub image_url: Vec<u8>,  // IPFS hash of the image
}

#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo)]
pub struct ComputeRequest<AccountId, BlockNumberFor, Hash>
{
    pub request_id: u128,  // Unique identifier for the request
    pub plan_technical_description: Vec<u8>,
    pub plan_id: Hash,
    pub status: ComputeRequestStatus, // Optional: to track request status
    pub created_at: BlockNumberFor,
    pub owner: AccountId,
    pub selected_image: ImageMetadata,
    pub is_assigned: bool, // Optional: to track if the request is assigned
    pub cloud_init_cid: Option<Vec<u8>>, // Optional cloud-init configuration CID
    pub miner_id: Option<Vec<u8>>, // Optional if they have selected a miner
}

#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo)]
pub enum ComputeRequestStatus {
    Pending,
    Stopped,
    InProgress,
    Running,
    Failed,      // Task encountered an error
    Cancelled,   // Task was cancelled
}

// This will store info related storage request
#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo)]
pub struct MinerComputeRequest<BlockNumber,Hash, AccountId> {
    pub miner_node_id: Vec<u8>,
    pub miner_account_id: AccountId,
    pub job_id: Option<Vec<u8>>,
    pub hypervisor_ip: Option<Vec<u8>>,
    pub fail_reason: Option<Vec<u8>>,
    pub vnc_port: Option<u64>,
    pub ip_assigned: Option<Vec<u8>>,
    pub request_id: u128,
    pub plan_id: Hash,
    pub created_at: BlockNumber,
    pub fullfilled: bool,
}

// This will store info related storage request
#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo)]
pub struct MinerComputeDeletionRequest<BlockNumber,Hash, AccountId> {
    pub miner_node_id: Vec<u8>,
    pub job_id: Option<Vec<u8>>,
    pub request_id: u128,
    pub user_id: AccountId,
    pub plan_id: Hash,
    pub created_at: BlockNumber,
    pub fullfilled: bool,
}

// This will store info related storage request
#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo)]
pub struct MinerComputeStopRequest<BlockNumber,Hash, AccountId> {
    pub miner_node_id: Vec<u8>,
    pub job_id: Option<Vec<u8>>,
    pub request_id: u128,
    pub user_id: AccountId,
    pub plan_id: Hash,
    pub created_at: BlockNumber,
    pub fullfilled: bool,
}

// This will store info related storage request
#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo)]
pub struct MinerComputeBootRequest<BlockNumber,Hash, AccountId> {
    pub miner_node_id: Vec<u8>,
    pub job_id: Option<Vec<u8>>,
    pub request_id: u128,
    pub user_id: AccountId,
    pub plan_id: Hash,
    pub created_at: BlockNumber,
    pub fullfilled: bool,
}

// This will store info related storage request
#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo)]
pub struct MinerComputeRebootRequest<BlockNumber,Hash, AccountId> {
    pub miner_node_id: Vec<u8>,
    pub job_id: Option<Vec<u8>>,
    pub request_id: u128,
    pub user_id: AccountId,
    pub plan_id: Hash,
    pub created_at: BlockNumber,
    pub fullfilled: bool,
}

// This will store info related storage request
#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo)]
pub struct MinerComputeResizeRequest<BlockNumber,Hash, AccountId> {
    pub miner_node_id: Vec<u8>,
    pub job_id: Option<Vec<u8>>,
    pub request_id: u128,
    pub user_id: AccountId,
    pub plan_id: Hash,
    pub created_at: BlockNumber,
    pub fullfilled: bool,
    pub resize_gbs: u32,
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct ComputeRequestAssignmentPayload<T: Config>  {
    pub miner_node_id: Vec<u8>,
    pub plan_id: T::Hash,
    pub request_id: u128,
    pub owner: T::AccountId,
    pub public: T::Public,
    pub _marker: PhantomData<BlockNumberFor<T>>,
}

impl<T: Config> SignedPayload<T> for ComputeRequestAssignmentPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

// Add a new payload type for the mark_compute_request_fulfilled transaction
#[derive(Encode, Decode, Clone, PartialEq, Eq, Debug, TypeInfo)]
pub struct ComputeRequestFulfilledPayload<T: Config> {
	pub node_id: Vec<u8>,
	pub request_id: u128,
    pub public: T::Public,
	pub _marker: PhantomData<T>,
}

impl<T: Config> SignedPayload<T> for ComputeRequestFulfilledPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

// Add a new payload type for the mark_compute_request_fulfilled transaction
#[derive(Encode, Decode, Clone, PartialEq, Eq, Debug, TypeInfo)]
pub struct ComputeRequestFailurePayload<T: Config> {
	pub node_id: Vec<u8>,
	pub request_id: u128,
    pub fail_reason: Vec<u8>,
    pub public: T::Public,
	pub _marker: PhantomData<T>,
}

impl<T: Config> SignedPayload<T> for ComputeRequestFailurePayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

// Add a new payload type for the mark_compute_request_fulfilled transaction
#[derive(Encode, Decode, Clone, PartialEq, Eq, Debug, TypeInfo)]
pub struct ComputeRequestVncAssignementPayload<T: Config> {
	pub node_id: Vec<u8>,
	pub request_id: u128,
    pub vnc_port: u64,
    pub vm_name: Vec<u8>,
    pub public: T::Public,
	pub _marker: PhantomData<T>,
}

impl<T: Config> SignedPayload<T> for ComputeRequestVncAssignementPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}


// Add a new payload type for the mark_compute_request_fulfilled transaction
#[derive(Encode, Decode, Clone, PartialEq, Eq, Debug, TypeInfo)]
pub struct ComputeRequestNebluaIpAssignementPayload<T: Config> {
	pub node_id: Vec<u8>,
	pub request_id: u128,
    pub hypervisor_ip: Vec<u8>,
    pub public: T::Public,
	pub _marker: PhantomData<T>,
}

impl<T: Config> SignedPayload<T> for ComputeRequestNebluaIpAssignementPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

// Add a new payload type for the mark_compute_request_fulfilled transaction
#[derive(Encode, Decode, Clone, PartialEq, Eq, Debug, TypeInfo)]
pub struct ComputeRequestAssignementPayload<T: Config> {
	pub node_id: Vec<u8>,
	pub request_id: u128,
    pub job_id: Vec<u8>,
    pub ip: Vec<u8>,
    pub public: T::Public,
	pub _marker: PhantomData<T>,
}

impl<T: Config> SignedPayload<T> for ComputeRequestAssignementPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

// Add a new payload type for the mark_compute_request_fulfilled transaction
#[derive(Encode, Decode, Clone, PartialEq, Eq, Debug, TypeInfo)]
pub struct StopRequestFulfilledPayload<T: Config> {
	pub node_id: Vec<u8>,
	pub request_id: u128,
    pub public: T::Public,
	pub _marker: PhantomData<T>,
}

impl<T: Config> SignedPayload<T> for StopRequestFulfilledPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

// Add a new payload type for the mark_compute_request_fulfilled transaction
#[derive(Encode, Decode, Clone, PartialEq, Eq, Debug, TypeInfo)]
pub struct MinnerDeleteComputeRequestPayload<T: Config> {
	pub node_id: Vec<u8>,
	pub request_id: u128,
    pub vm_name: Vec<u8>,
    pub public: T::Public,
	pub _marker: PhantomData<T>,
}

impl<T: Config> SignedPayload<T> for MinnerDeleteComputeRequestPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

// Add a new payload type for the mark_compute_request_fulfilled transaction
#[derive(Encode, Decode, Clone, PartialEq, Eq, Debug, TypeInfo)]
pub struct MinnerBootComputeRequestPayload<T: Config> {
	pub node_id: Vec<u8>,
	pub request_id: u128,
    pub public: T::Public,
	pub _marker: PhantomData<T>,
}

impl<T: Config> SignedPayload<T> for MinnerBootComputeRequestPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

// Add a new payload type for the mark_compute_request_fulfilled transaction
#[derive(Encode, Decode, Clone, PartialEq, Eq, Debug, TypeInfo)]
pub struct MinnerRebootComputeRequestPayload<T: Config> {
	pub node_id: Vec<u8>,
	pub request_id: u128,
    pub public: T::Public,
	pub _marker: PhantomData<T>,
}

impl<T: Config> SignedPayload<T> for MinnerRebootComputeRequestPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

// Add a new payload type for the mark_compute_request_fulfilled transaction
#[derive(Encode, Decode, Clone, PartialEq, Eq, Debug, TypeInfo)]
pub struct MinnerResizeComputeRequestPayload<T: Config> {
	pub node_id: Vec<u8>,
	pub request_id: u128,
    pub public: T::Public,
	pub _marker: PhantomData<T>,
}

impl<T: Config> SignedPayload<T> for MinnerResizeComputeRequestPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

/// Struct to represent VM details for a user
#[derive(Encode, Decode, TypeInfo, RuntimeDebug)]
pub struct UserVmDetails<AccountId, BlockNumber, Hash> {
    pub request_id: u128,
    pub status: ComputeRequestStatus,
    pub plan_id: Hash,
    pub created_at: BlockNumber,
    pub miner_node_id: Option<Vec<u8>>,
    pub miner_account_id: Option<AccountId>,
    pub hypervisor_ip: Option<Vec<u8>>,
    pub vnc_port: Option<u64>,
    pub ip_assigned: Option<Vec<u8>>,
    pub error: Option<Vec<u8>>,
    pub is_fulfilled: bool,
}
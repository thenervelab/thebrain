use serde::{Serialize, Deserialize};
use scale_info::TypeInfo;
use parity_scale_codec::{Encode,Decode};
use sp_runtime::AccountId32;

#[derive(Clone ,Serialize, Deserialize, TypeInfo, Encode, Decode)]
pub enum NodeType {
    Validator,
    StorageMiner,
    StorageS3,
    ComputeMiner,
    GpuMiner
}

#[derive( Serialize, Clone,  Deserialize, TypeInfo, Encode, Decode)]
pub struct MinerRewardSummary {
    pub account: AccountId32,
    pub reward: u128,
}

#[derive( Serialize, Clone,  Deserialize, TypeInfo, Encode, Decode)]
pub struct UserFile {
    pub file_hash: Vec<u8>,
    pub file_name: String,
    pub miner_ids: Vec<Vec<u8>>,
    pub file_size: u32,  // Added file size field
}

/// Represents a bucket with its name and size
#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo)]
pub struct UserBucket {
	pub bucket_name: Vec<u8>,
	pub bucket_size: Vec<u128>,
}

/// Struct to represent VM details for a user
#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo, Serialize, Deserialize)]
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

#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo, Serialize, Deserialize)]
pub enum Status {
    Online,
    Degraded,
    Offline
}

#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo, Serialize, Deserialize)]
pub enum ComputeRequestStatus {
    Pending,
    Stopped,
    InProgress,
    Running,
    Failed,      // Task encountered an error
    Cancelled,   // Task was cancelled
}

#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo, Serialize, Deserialize)]
pub struct Batch<AccountId, BlockNumberFor> {
    pub owner: AccountId,        // User who deposited
    pub credit_amount: u128,     // Total credits in batch
    pub alpha_amount: u128,      // Total Alpha purchased
    pub remaining_credits: u128, // Remaining credits in the batch
    pub remaining_alpha: u128,   // Remaining Alpha in the batch
    pub pending_alpha: u128,     // Pending Alpha to be released
    pub is_frozen: bool,         // Freezes Alpha distribution, not credit use
    pub release_time: BlockNumberFor, // When Alpha can be distributed
}
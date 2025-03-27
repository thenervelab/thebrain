// pub use rpc_core_node_metrics::{types::NodeType};
#![cfg_attr(not(feature = "std"), no_std)]
use sp_api::decl_runtime_apis;

use scale_info::TypeInfo;
use parity_scale_codec::{Encode,Decode};
use sp_std::vec::Vec;
use scale_info::prelude::string::String;
use sp_runtime::AccountId32;
use serde::{Serialize, Deserialize};

#[derive( Clone, Serialize, Deserialize, TypeInfo, Encode, Decode)]
pub enum NodeType {
    Validator,
    StorageMiner,
    StorageS3,
    ComputeMiner,
    GpuMiner
}

decl_runtime_apis! {
    pub trait NodeMetricsRuntimeApi {
        fn get_active_nodes_metrics_by_type(node_type: NodeType) -> Vec<Option<NodeMetricsData>>;
        fn get_total_distributed_rewards_by_node_type(node_type: NodeType) -> u128;
        fn get_total_node_rewards(account: AccountId32) -> u128;
        fn get_miners_total_rewards(node_type: NodeType) -> Vec<MinerRewardSummary>;
        fn get_account_pending_rewards(account: AccountId32) -> Vec<MinerRewardSummary>;
        fn get_miners_pending_rewards(node_type: NodeType) -> Vec<MinerRewardSummary>;
        fn calculate_total_file_size(account: AccountId32) -> u128;
        fn get_user_files(account: AccountId32) -> Vec<UserFile>;
        fn get_user_buckets(account: AccountId32) -> Vec<UserBucket>;
        fn get_user_vms(account: AccountId32) -> Vec<UserVmDetails<AccountId32,  u32, [u8; 32]>>;
        fn get_node_metrics(node_id: Vec<u8>) -> Option<NodeMetricsData>;
        fn get_client_ip(client_id: AccountId32) -> Option<Vec<u8>>;
        fn get_hypervisor_ip( hypervisor_id: Vec<u8>) -> Option<Vec<u8>>;    
        fn get_vm_ip( vm_id: Vec<u8>) -> Option<Vec<u8>>;
        fn get_storage_miner_ip( miner_id: Vec<u8>) -> Option<Vec<u8>>;
        fn get_bucket_size( bucket_name: Vec<u8>) -> u128;
        fn get_total_bucket_size( account_id: AccountId32) -> u128;
        fn get_user_bandwidth( account_id: AccountId32) -> u128;
        fn get_miner_info(account_id: AccountId32) -> Option<(NodeType, Status)>;
        fn get_batches_for_user(account_id: AccountId32) -> Vec<Batch<AccountId32, u32>>;
        fn get_batch_by_id(batch_id: u64) -> Option<Batch<AccountId32, u32>>;
        fn get_free_credits_rpc(account: Option<AccountId32>) -> Vec<(AccountId32, u128)>;
        fn get_referred_users(account_id: AccountId32) -> Vec<AccountId32>;
        fn get_referral_rewards(account_id: AccountId32) -> u128;
        fn total_referral_codes() -> u32;
        fn total_referral_rewards() -> u128;
        fn get_referral_codes(account_id: AccountId32) -> Vec<Vec<u8>>;
        fn total_file_size_fulfilled(account_id: AccountId32) -> u128;
    }
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

#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo, Serialize, Deserialize)]
pub enum Status {
    Online,
    Degraded,
    Offline
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
pub enum ComputeRequestStatus {
    Pending,
    Stopped,
    InProgress,
    Running,
    Failed,      // Task encountered an error
    Cancelled,   // Task was cancelled
}

/// Represents a bucket with its name and size
#[derive( Serialize, Clone,  Deserialize, TypeInfo, Encode, Decode)]
pub struct UserBucket {
	pub bucket_name: Vec<u8>,
	pub bucket_size: Vec<u128>,
}

#[derive( Serialize, Clone,  Deserialize, TypeInfo, Encode, Decode)]
pub struct UserFile {
    pub file_hash: Vec<u8>,
    pub file_name: Vec<u8>,
    pub miner_ids: Vec<Vec<u8>>,
    pub file_size: u32,  // Added file size field
    pub date: u64, // Added date field
}

#[derive( Serialize,Clone,  Deserialize, TypeInfo, Encode, Decode)]
pub struct MinerRewardSummary {
    pub account: AccountId32,
    pub reward: u128,
}

#[derive( Serialize,Clone,  Deserialize, TypeInfo, Encode, Decode)]
pub struct NodeMetricsData {
    pub miner_id: String,
    pub bandwidth_bytes: u32, // will come  from node specs
    pub current_storage_bytes: u64, // will come from node specs
    pub total_storage_bytes: u64, // will come from node specs
    pub geolocation: String, // will come from node specs
    pub successful_pin_checks: u32, //
    pub total_pin_checks: u32, //
    pub storage_proof_time_ms: u32, // calculating via rpc method
    pub storage_growth_rate: u32,  // calculating by dividing the current_storage_bytes with uptime
    pub latency_ms: u32, // calculating via 'system_health' rpc method 
    pub total_latency_ms: u32, // new field to used calculate avg_response_time_ms
    pub total_times_latency_checked: u32, // new field to used calculate avg_response_time_ms
    pub avg_response_time_ms: u32,  // average_response_time = total_latency / n
    pub peer_count: u32, // getting this via system health rpc
    pub failed_challenges_count: u32, // number of times call failed
    pub successful_challenges: u32, // number of times call passes
    pub total_challenges: u32, // total number of times called
    pub uptime_minutes: u32, // can be done by timestamp
    pub total_minutes: u32, // can be done by timestamp
    pub consecutive_reliable_days: u32, // can be done by timestamp
    pub recent_downtime_hours: u32,  // can be done by timestamp
    pub is_sev_enabled: bool,
    pub zfs_info: Vec<String>,     
    pub ipfs_zfs_pool_size: u128,
    pub ipfs_zfs_pool_alloc: u128,
    pub ipfs_zfs_pool_free: u128,
    pub raid_info: Vec<String>,    
    pub vm_count: u32,
    pub primary_network_interface: Option<NetworkInterfaceInfo>, 
    pub disks: Vec<DiskInfo>,
    pub ipfs_repo_size: u64,
    pub ipfs_storage_max: u64,
    pub cpu_model: String,
    pub cpu_cores: u32,
    pub memory_bytes: u64,
    pub free_memory_bytes: u64,
    pub gpu_name: Option<String>,
    pub gpu_memory_bytes: Option<u32>,
    pub hypervisor_disk_type: Option<String>,
    pub vm_pool_disk_type: Option<String>,
}

#[derive( Clone,Serialize, Deserialize, TypeInfo, Encode, Decode)]
pub enum NetworkType {
    Private,
    Public,
}

#[derive( Clone,Serialize, Deserialize, TypeInfo, Encode, Decode)]
pub struct NetworkDetails {
    pub network_type: NetworkType,
    pub city: Option<String>,
    pub region: Option<String>,
    pub country: Option<String>,
    pub loc: Option<String>,
}

#[derive( Clone,Serialize, Deserialize, TypeInfo, Encode, Decode)]
pub struct NetworkInterfaceInfo {
    pub name: String,
    pub mac_address: Option<String>,
    pub uplink_mb: u64,    // Total transmitted data in MB
    pub downlink_mb: u64,  // Total received data in MB
    pub network_details: Option<NetworkDetails>,
}

#[derive( Clone,Serialize, Deserialize, TypeInfo, Encode, Decode)]
pub struct DiskInfo {
    pub name: String,
    pub disk_type: String,
    pub total_space_mb: u64,
    pub free_space_mb: u64,
}

// pub use rpc_core_node_metrics::{types::NodeType};
#![cfg_attr(not(feature = "std"), no_std)]
use sp_api::decl_runtime_apis;

use scale_info::TypeInfo;
use parity_scale_codec::{Encode,Decode};
use sp_std::vec::Vec;
use scale_info::prelude::string::String;
use sp_runtime::AccountId32;
use serde::{Serialize, Deserialize};

#[derive( Serialize, Deserialize, TypeInfo, Encode, Decode)]
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
    }
}


#[derive( Serialize,Clone,  Deserialize, TypeInfo, Encode, Decode)]
pub struct MinerRewardSummary {
    pub account: AccountId32,
    pub reward: u128,
}

// Define the NodeMetrics struct
#[derive( Clone,Serialize, Deserialize, TypeInfo, Encode, Decode)]
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

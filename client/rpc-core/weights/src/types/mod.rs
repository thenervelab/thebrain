use serde::{Deserialize, Serialize};
use scale_info::TypeInfo;
use codec::{Encode, Decode};
use sp_runtime::RuntimeDebug;

#[derive(Deserialize)]
pub struct SubmitWeightsParams {
	pub netuid: u16,
	pub dests: Vec<u16>,
	pub weights: Vec<u16>,
	pub version_key: u32,
	pub default_spec_version: u32,
	pub default_genesis_hash: String,
	pub finney_api_url: String,
}

#[derive(Deserialize, Clone, Encode, Decode, RuntimeDebug, TypeInfo, Default, PartialEq)]
pub struct SystemInfo {
	pub memory_mb: u64,
	pub free_memory_mb: u64,
	pub storage_total_mb: u64,
	pub storage_free_mb: u64,
	pub network_bandwidth_mb_s: u32,
	pub primary_network_interface: Option<NetworkInterfaceInfo>,
	pub disks: Vec<DiskInfo>,
	pub ipfs_repo_size: u64,
	pub ipfs_storage_max: u64,
	pub cpu_model: Vec<u8>,
	pub cpu_cores: u32,
	pub is_sev_enabled: bool,
	pub zfs_info: Vec<Vec<u8>>,
	pub ipfs_zfs_pool_size: u128,
	pub ipfs_zfs_pool_alloc: u128,
	pub ipfs_zfs_pool_free: u128,
	pub raid_info: Vec<Vec<u8>>,
	pub vm_count: u32,
	pub gpu_name: Option<Vec<u8>>,
	pub gpu_memory_mb: Option<u32>,
	pub hypervisor_disk_type: Option<Vec<u8>>,
	pub vm_pool_disk_type: Option<Vec<u8>>,
	pub disk_info: Vec<DiskDetails>,
}

#[derive(Deserialize, Clone, Encode, Decode, RuntimeDebug, TypeInfo, Default, PartialEq)]
pub struct DiskDetails {
	pub name: Vec<u8>,
	pub serial: Vec<u8>,
	pub model: Vec<u8>,
	pub size: Vec<u8>,
	pub is_rotational: bool,
	pub disk_type: Vec<u8>,
}

#[derive(Deserialize, PartialEq, Clone, Encode, Decode, RuntimeDebug, TypeInfo, Default)]
pub enum NetworkType {
	Private,
	#[default]
	Public,
}

#[derive(Deserialize, Clone, Encode, Decode, RuntimeDebug, TypeInfo, Default, PartialEq)]
pub struct NetworkDetails {
	pub network_type: NetworkType,
	pub city: Option<Vec<u8>>,
	pub region: Option<Vec<u8>>,
	pub country: Option<Vec<u8>>,
	pub loc: Option<Vec<u8>>,
}

#[derive(Deserialize, Clone, Encode, Decode, RuntimeDebug, TypeInfo, Default, PartialEq)]
pub struct NetworkInterfaceInfo {
	pub name: Vec<u8>,
	pub mac_address: Option<Vec<u8>>,
	pub uplink_mb: u64,   // Total transmitted data in MB
	pub downlink_mb: u64, // Total received data in MB
	pub network_details: Option<NetworkDetails>,
}

#[derive(Deserialize, Clone, Encode, Decode, RuntimeDebug, TypeInfo, Default, PartialEq)]
pub struct DiskInfo {
	pub name: Vec<u8>,
	pub disk_type: Vec<u8>,
	pub total_space_mb: u64,
	pub free_space_mb: u64,
}

// RPC parameters
#[derive(Deserialize, Debug)]
pub struct SubmitHardwareParams {
    pub node_id: Vec<u8>,
    pub system_info: SystemInfo,
    pub default_spec_version: u32,
    pub default_genesis_hash: String,
    pub local_rpc_url: String,
}

#[derive(Deserialize, Clone, Encode, Decode, RuntimeDebug, TypeInfo, Default, PartialEq)]
pub struct SubmitMetricsParams {
    pub node_id: Vec<u8>,
    pub storage_proof_time_ms: u32,
    pub latency_ms: u32,
    pub peer_count: u32,
    pub failed_challenges_count: u32,
    pub successful_challenges: u32,
    pub total_challenges: u32,
    pub uptime_minutes: u32,
    pub total_minutes: u32,
    pub consecutive_reliable_days: u32,
    pub recent_downtime_hours: u32,
    pub block_number: u32,
    pub default_spec_version: u32,
    pub default_genesis_hash: String,
    pub local_rpc_url: String,
}

#[derive(Deserialize, Clone, Encode, Decode, RuntimeDebug, TypeInfo, Default, PartialEq)]
pub struct SubmitRankingsParams {
    pub weights: Vec<u16>,
    pub all_nodes_ss58: Vec<Vec<u8>>,
    pub node_ids: Vec<Vec<u8>>,
    pub node_types: Vec<NodeType>,
    pub ranking_instance_id: u32,
    pub default_spec_version: u32,
    pub default_genesis_hash: String,
    pub local_rpc_url: String,
}

#[derive(Deserialize, Encode, Decode, Clone, Eq, PartialEq, Debug, TypeInfo)]
pub enum NodeType {
	Validator,
	StorageMiner,
	StorageS3,
	ComputeMiner,
	GpuMiner,
}

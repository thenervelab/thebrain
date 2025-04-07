use crate::Config;
use frame_support::pallet_prelude::RuntimeDebug;
use frame_system::{offchain::SignedPayload, pallet_prelude::BlockNumberFor};
use pallet_registration::NodeType;
use scale_codec::{Decode, Encode};
use scale_info::prelude::vec::Vec;
use scale_info::TypeInfo;
use sp_std::{marker::PhantomData, prelude::*};

#[derive(Clone, Encode, Decode, RuntimeDebug, TypeInfo, PartialEq, Default)]
pub struct BenchmarkMetrics {
	pub cpu_score: u32,
	pub memory_score: u32,
	pub storage_score: u32,
	pub disk_score: u32,
	pub network_score: u32,
}

#[derive(Clone, Encode, Decode, RuntimeDebug, TypeInfo, Default)]
pub struct BenchmarkResult<BlockNumber> {
	pub final_score: u32,
	pub timestamp: BlockNumber,
	pub trigger_type: TriggerType,
	pub metrics: BenchmarkMetrics,
}

#[derive(Clone, Encode, Decode, RuntimeDebug, TypeInfo, Default)]
pub enum TriggerType {
	Random,
	#[default]
	Manual,
}

#[derive(PartialEq, Clone, Encode, Decode, RuntimeDebug, TypeInfo, Default)]
pub enum NetworkType {
	Private,
	#[default]
	Public,
}

#[derive(Clone, Encode, Decode, RuntimeDebug, TypeInfo, Default, PartialEq)]
pub struct NetworkDetails {
	pub network_type: NetworkType,
	pub city: Option<Vec<u8>>,
	pub region: Option<Vec<u8>>,
	pub country: Option<Vec<u8>>,
	pub loc: Option<Vec<u8>>,
}

#[derive(Clone, Encode, Decode, RuntimeDebug, TypeInfo, Default, PartialEq)]
pub struct NetworkInterfaceInfo {
	pub name: Vec<u8>,
	pub mac_address: Option<Vec<u8>>,
	pub uplink_mb: u64,   // Total transmitted data in MB
	pub downlink_mb: u64, // Total received data in MB
	pub network_details: Option<NetworkDetails>,
}

#[derive(Clone, Encode, Decode, RuntimeDebug, TypeInfo, Default, PartialEq)]
pub struct DiskInfo {
	pub name: Vec<u8>,
	pub disk_type: Vec<u8>,
	pub total_space_mb: u64,
	pub free_space_mb: u64,
}

#[derive(Clone, Encode, Decode, RuntimeDebug, TypeInfo, Default)]
pub struct OfflineStatus<BlockNumber> {
	pub miner_node_id: Vec<u8>,    // the node id of miner
	pub reportor_node_id: Vec<u8>, // the node if of the reporting validator
	pub at_block: BlockNumber,     // the block at which the miner was offline
}

#[derive(Clone, Encode, Decode, RuntimeDebug, TypeInfo, Default, PartialEq)]
pub struct DiskDetails {
	pub name: Vec<u8>,
	pub serial: Vec<u8>,
	pub model: Vec<u8>,
	pub size: Vec<u8>,
	pub is_rotational: bool,
	pub disk_type: Vec<u8>,
}

#[derive(Clone, Encode, Decode, RuntimeDebug, TypeInfo, Default, PartialEq)]
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

// Define the NodeMetrics struct
#[derive(Clone, Encode, Decode, TypeInfo)]
pub struct NodeMetricsData {
	pub miner_id: Vec<u8>,
	pub bandwidth_mbps: u32,              // will come  from node specs
	pub current_storage_bytes: u64,       // will come from node specs
	pub total_storage_bytes: u64,         // will come from node specs
	pub geolocation: Vec<u8>,             // will come from node specs
	pub successful_pin_checks: u32,       //
	pub total_pin_checks: u32,            //
	pub storage_proof_time_ms: u32,       // calculating via rpc method
	pub storage_growth_rate: u32, // calculating by dividing the current_storage_bytes with uptime
	pub latency_ms: u32,          // calculating via 'system_health' rpc method
	pub total_latency_ms: u32,    // new field to used calculate avg_response_time_ms
	pub total_times_latency_checked: u32, // new field to used calculate avg_response_time_ms
	pub avg_response_time_ms: u32, // average_response_time = total_latency / n
	pub peer_count: u32,          // getting this via system health rpc
	pub failed_challenges_count: u32, // number of times call failed
	pub successful_challenges: u32, // number of times call passes
	pub total_challenges: u32,    // total number of times called
	pub uptime_minutes: u32,      // can be done by timestamp
	pub total_minutes: u32,       // can be done by timestamp
	pub consecutive_reliable_days: u32, // can be done by timestamp
	pub recent_downtime_hours: u32, // can be done by timestamp
	pub is_sev_enabled: bool,
	pub zfs_info: Vec<Vec<u8>>,
	pub ipfs_zfs_pool_size: u128,
	pub ipfs_zfs_pool_alloc: u128,
	pub ipfs_zfs_pool_free: u128,
	pub raid_info: Vec<Vec<u8>>,
	pub vm_count: u32,
	pub primary_network_interface: Option<NetworkInterfaceInfo>,
	pub disks: Vec<DiskInfo>,
	pub ipfs_repo_size: u64,
	pub ipfs_storage_max: u64,
	pub cpu_model: Vec<u8>,
	pub cpu_cores: u32,
	pub memory_mb: u64,
	pub free_memory_mb: u64,
	pub gpu_name: Option<Vec<u8>>,
	pub gpu_memory_mb: Option<u32>,
	pub hypervisor_disk_type: Option<Vec<u8>>,
	pub vm_pool_disk_type: Option<Vec<u8>>,
	pub disk_info: Vec<DiskDetails>,
}

impl Default for NodeMetricsData {
	fn default() -> Self {
		NodeMetricsData {
			miner_id: Vec::new(),
			// Network-wide configuration defaults
			successful_pin_checks: 0,
			total_pin_checks: 0,
			// successful_integrity_checks: 0,
			// total_integrity_checks: 0,
			avg_response_time_ms: 100, // Default target response time in ms
			bandwidth_mbps: 100,       // Default target bandwidth in Mbps
			storage_proof_time_ms: 0,
			uptime_minutes: 0,
			total_minutes: 0,
			successful_challenges: 0,
			total_challenges: 0,
			current_storage_bytes: 1024 * 1024 * 1024 * 100, // 100GB default storage
			total_storage_bytes: 1024 * 1024 * 1024 * 100,   // 100GB default storage
			storage_growth_rate: 0,
			peer_count: 10, // Default minimum peer count
			latency_ms: 0,
			total_latency_ms: 0, // new field to used calculate avg_response_time_ms
			total_times_latency_checked: 0,
			geolocation: Vec::new(),
			consecutive_reliable_days: 0,
			recent_downtime_hours: 0,
			failed_challenges_count: 0,
			is_sev_enabled: false,
			zfs_info: Vec::new(),
			ipfs_zfs_pool_size: 0,
			ipfs_zfs_pool_alloc: 0,
			ipfs_zfs_pool_free: 0,
			raid_info: Vec::new(),
			primary_network_interface: None,
			disks: Vec::new(),
			ipfs_repo_size: 0,
			ipfs_storage_max: 0,
			cpu_model: Vec::new(),
			cpu_cores: 0,
			memory_mb: 0,
			free_memory_mb: 0,
			vm_count: 0,
			gpu_name: None,
			gpu_memory_mb: None,
			hypervisor_disk_type: None,
			vm_pool_disk_type: None,
			disk_info: Vec::new(),
		}
	}
}

#[derive(Encode, Decode, Clone, PartialEq, RuntimeDebug, TypeInfo)]
pub struct SaveHardwareInfoPayload<T: Config> {
	pub node_id: Vec<u8>,
	pub system_info: SystemInfo, // Replace `HardwareInfo` with the actual type you use for hardware information
	pub public: T::Public,
	pub node_type: NodeType,
	pub _marker: PhantomData<T>,
}

// Implement SignedPayload for UpdateRankingsPayload
impl<T: Config> SignedPayload<T> for SaveHardwareInfoPayload<T> {
	fn public(&self) -> T::Public {
		self.public.clone()
	}
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct UpdatePinCheckMetricsPayload<T: Config> {
	pub node_id: Vec<u8>,
	pub total_pin_checks: u32,
	pub successful_pin_checks: u32,
	pub public: T::Public,
	pub _marker: PhantomData<T>,
}

impl<T: Config> SignedPayload<T> for UpdatePinCheckMetricsPayload<T> {
	fn public(&self) -> T::Public {
		self.public.clone()
	}
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct UpdateMetricsDataPayload<T: Config> {
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
	pub node_type: NodeType,
	pub block_number: u32,
	pub public: T::Public,
	pub _marker: PhantomData<T>,
}

impl<T: Config> SignedPayload<T> for UpdateMetricsDataPayload<T> {
	fn public(&self) -> T::Public {
		self.public.clone()
	}
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct UpdateBlockTimePayload<T: Config> {
	pub node_id: Vec<u8>,
	pub block_number: BlockNumberFor<T>,
	pub public: T::Public,
	pub _marker: PhantomData<T>,
}

impl<T: Config> SignedPayload<T> for UpdateBlockTimePayload<T> {
	fn public(&self) -> T::Public {
		self.public.clone()
	}
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct StoreOfflineStatusPayload<T: Config> {
	pub node_id: Vec<u8>,
	pub miner_id: Vec<u8>,
	pub block_number: BlockNumberFor<T>,
	pub public: T::Public,
	pub _marker: PhantomData<T>,
}

impl<T: Config> SignedPayload<T> for StoreOfflineStatusPayload<T> {
	fn public(&self) -> T::Public {
		self.public.clone()
	}
}

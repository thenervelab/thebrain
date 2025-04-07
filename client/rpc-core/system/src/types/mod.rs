use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct NetworkInterfaceInfo {
	pub name: String,
	pub mac_address: Option<String>,
	pub ipv4_address: Option<String>,
	pub ipv6_address: Option<String>,
	pub uplink_mb: u64,   // Total transmitted data in MB
	pub downlink_mb: u64, // Total received data in MB
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DiskInfo {
	pub name: String,
	pub disk_type: String,
	pub total_space_mb: u64,
	pub free_space_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DiskDetails {
	pub name: String,
	pub serial: String,
	pub model: String,
	pub size: String,
	pub is_rotational: bool,
	pub disk_type: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemInfo {
	pub cpu_model: String,
	pub cpu_cores: u32,
	pub memory_mb: u64,
	pub free_memory_mb: u64,
	pub storage_total_mb: u64,
	pub storage_free_mb: u64,
	pub network_bandwidth_mb_s: u32,
	pub primary_network_interface: Option<NetworkInterfaceInfo>,
	pub disks: Vec<DiskInfo>,
	pub is_sev_enabled: bool,
	pub zfs_info: Vec<String>,
	pub ipfs_zfs_pool_size: u128,
	pub ipfs_zfs_pool_alloc: u128,
	pub ipfs_zfs_pool_free: u128,
	pub raid_info: Vec<String>,
	pub vm_count: usize,
	pub gpu_name: Option<String>,
	pub gpu_memory_mb: Option<u32>,
	pub hypervisor_disk_type: Option<String>,
	pub vm_pool_disk_type: Option<String>,
	pub disk_info: Vec<DiskDetails>,
}

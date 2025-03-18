use jsonrpsee::core::RpcResult;
use sysinfo::{Disks, Networks, System};
use std::{fs, process::Command, process::Stdio, sync::Arc, collections::HashSet};
pub use rpc_core_system::{types::{SystemInfo, NetworkInterfaceInfo, DiskInfo, DiskDetails}, SystemInfoApiServer};
use sp_runtime::traits::Block as BlockT;
use sp_blockchain::HeaderBackend;
use fp_rpc::EthereumRuntimeRPCApi;
use sp_api::ProvideRuntimeApi;
use if_addrs::IfAddr;

/// Function to count running VMs
fn count_kvm_vms() -> Option<usize> {
    if let Ok(output) = Command::new("kubectl")
        .args(&["get", "vms", "--all-namespaces", "--no-headers"])
        .output() 
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            // Use wc -l equivalent by counting non-empty lines
            let vm_count = stdout.lines()
                .filter(|line| !line.trim().is_empty())
                .count();
            return Some(vm_count);
        }
    }
    None
}

/// Function to retrieve GPU information
fn get_gpu_info() -> Option<(String, u32)> {
    // First, try NVIDIA GPUs
    if let Ok(output) = Command::new("nvidia-smi")
        .arg("--query-gpu=name,memory.total")
        .arg("--format=csv,noheader,nounits")
        .stdout(Stdio::piped())
        .output() 
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let lines: Vec<&str> = stdout.trim().lines().collect();
            
            if !lines.is_empty() {
                let parts: Vec<&str> = lines[0].split(',').map(|s| s.trim()).collect();
                if parts.len() == 2 {
                    let gpu_name = parts[0].to_string();
                    if let Ok(memory_mb) = parts[1].parse::<u32>() {
                        return Some((gpu_name, memory_mb));
                    }
                }
            }
        }
    }

    // If NVIDIA fails, try fallback methods
    // Use lspci to detect GPU
    if let Ok(output) = Command::new("lspci")
        .arg("-vnn")
        .arg("|")
        .arg("grep")
        .arg("VGA")
        .stdout(Stdio::piped())
        .output()
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stdout.trim().is_empty() {
                // Basic GPU detection without memory
                return Some((stdout.trim().to_string(), 0));
            }
        }
    }

    None
}

/// Function to get hypervisor disk type
fn get_hypervisor_disk_type() -> Option<String> {
    // Try to detect disk type from /sys/block
    if let Ok(entries) = fs::read_dir("/sys/block") {
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            let device_name = path.file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("");

            // Check for NVMe devices
            if device_name.starts_with("nvme") {
                return Some("nvme".to_string());
            }

            // Check for SSD devices
            let rotational_path = path.join("queue/rotational");
            if let Ok(rotational_content) = fs::read_to_string(&rotational_path) {
                if let Ok(rotational) = rotational_content.trim().parse::<u8>() {
                    if rotational == 0 {
                        return Some("ssd".to_string());
                    }
                }
            }
        }
    }

    // Fallback to lsblk method
    if let Ok(output) = Command::new("lsblk")
        .arg("-d")
        .arg("-o")
        .arg("NAME,TYPE")
        .output() 
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.contains("nvme") {
            return Some("nvme".to_string());
        }
        if stdout.contains("ssd") {
            return Some("ssd".to_string());
        }
    }

    // Final fallback
    Some("hdd".to_string())
}

/// Function to get VM pool disk type 
fn get_vm_pool_disk_type() -> Option<String> {
    // Check if virsh has a default storage pool
    if let Ok(output) = Command::new("virsh")
        .arg("pool-list")
        .arg("--all")
        .output() 
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        
        // Look for active pools
        for line in stdout.lines() {
            if line.contains("active") {
                // Try to get pool path
                let pool_name = line.split_whitespace().next().unwrap_or("default");
                if let Ok(pool_path_output) = Command::new("virsh")
                    .arg("pool-dumpxml")
                    .arg(pool_name)
                    .output()
                {
                    let pool_path_xml = String::from_utf8_lossy(&pool_path_output.stdout);
                    
                    // Extract path from XML
                    if let Some(path_start) = pool_path_xml.find("<path>") {
                        let path_end = pool_path_xml[path_start..].find("</path>").unwrap_or(0);
                        let pool_path = &pool_path_xml[path_start + 6..path_start + path_end];
                        
                        // Check underlying device for the pool path
                        if let Ok(lsblk_output) = Command::new("lsblk")
                            .arg("-no")
                            .arg("NAME,TYPE,TRAN")
                            .output() 
                        {
                            let lsblk_stdout = String::from_utf8_lossy(&lsblk_output.stdout);

                            // Check disk type based on transport type and device characteristics
                            let lines: Vec<&str> = lsblk_stdout.lines().collect();
                            for line in lines {
                                let parts: Vec<&str> = line.split_whitespace().collect();
                                if parts.len() >= 3 {
                                    match parts[2] {
                                        "nvme" => return Some("nvme".to_string()),
                                        "sata" => return Some("ssd".to_string()),
                                        "sas" => return Some("hdd".to_string()),
                                        _ => {}
                                    }
                                }
                            }
                        }

                        // Fallback to filesystem type mapping
                        if let Ok(df_output) = Command::new("df")
                            .arg("-T")
                            .arg(pool_path)
                            .output()
                        {
                            let df_stdout = String::from_utf8_lossy(&df_output.stdout);
                            let lines: Vec<&str> = df_stdout.trim().lines().collect();
                            
                            if lines.len() > 1 {
                                let parts: Vec<&str> = lines[1].split_whitespace().collect();
                                if parts.len() > 1 {
                                    // Map filesystem types to disk types
                                    match parts[1] {
                                        "ext4" => return Some("ssd".to_string()),
                                        "xfs" => return Some("ssd".to_string()),
                                        "btrfs" => return Some("ssd".to_string()),
                                        "zfs" => return Some("zfs".to_string()),
                                        _ => return Some(parts[1].to_string()),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Check if libvirt default storage pool exists
    if let Ok(output) = Command::new("virsh")
        .arg("pool-info")
        .arg("default")
        .output() 
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.contains("Active") {
            // If default pool exists but no specific type detected
            return Some("default".to_string());
        }
    }

    None
}

fn get_disk_info() -> Result<Vec<DiskDetails>, std::io::Error> {
    use std::process::Command;
    use std::io::{Error, ErrorKind};
    use rpc_core_system::types::DiskDetails;

    // Run lsblk command with verbose output for debugging
    let output = Command::new("lsblk")
        .args(&["-o", "NAME,SERIAL,MODEL,UUID,PARTUUID,SIZE,ROTA,TYPE", "-P", "-n"])
        .output()?;

    // Convert output to string
    let output_str = String::from_utf8_lossy(&output.stdout);


    let disks: Vec<DiskDetails> = output_str
        .lines()
        .filter_map(|line| {
            // Log each line for debugging
            log::debug!("Processing line: {}", line);

            // Parse line using key-value pairs
            let mut name = "N/A".to_string();
            let mut serial = "N/A".to_string();
            let mut model = "N/A".to_string();
            let mut size = "N/A".to_string();
            let mut rota = "0".to_string();
            let mut disk_type = "N/A".to_string();

            // Split line into key-value pairs
            let pairs: Vec<&str> = line.split(' ').collect();
            for pair in pairs {
                let kv: Vec<&str> = pair.split('=').collect();
                if kv.len() == 2 {
                    let key = kv[0].trim();
                    let value = kv[1].trim_matches('"');
                    
                    match key {
                        "NAME" => name = value.to_string(),
                        "SERIAL" => serial = value.to_string(),
                        "MODEL" => model = value.to_string(),
                        "SIZE" => size = value.to_string(),
                        "ROTA" => rota = value.to_string(),
                        "TYPE" => disk_type = value.to_string(),
                        _ => {}
                    }
                }
            }

            // Check if this is a disk or partition
            if disk_type == "disk" || disk_type == "part" {
                let disk_details = DiskDetails {
                    name,
                    serial,
                    model,
                    size,
                    is_rotational: rota == "1",
                    disk_type,
                };

                Some(disk_details)
            } else {
                log::debug!("Skipping non-disk/non-partition: {}", line);
                None
            }
        })
        .collect();

    // If no disks or partitions found, return an error with context
    if disks.is_empty() {
        Err(Error::new(
            ErrorKind::NotFound, 
            "No disk or partition devices found. Check lsblk output and system configuration."
        ))
    } else {
        Ok(disks)
    }
}

/// Net API implementation.
pub struct SystemInfoImpl<B: BlockT, C> {
	_client: Arc<C>,
	_phantom_data: std::marker::PhantomData<B>,          
}

impl<B: BlockT, C> SystemInfoImpl<B, C> {
	pub fn new(_client: Arc<C>) -> Self {
		Self {
			_client,
			_phantom_data: Default::default(),
		}
	}
}

impl<B, C> SystemInfoApiServer for SystemInfoImpl<B, C>
where
	B: BlockT,
	C: ProvideRuntimeApi<B> + 'static,
	C::Api: EthereumRuntimeRPCApi<B>,
	C: HeaderBackend<B> + Send + Sync,
{

    fn get_system_info(&self) -> RpcResult<SystemInfo> {
        let mut sev_enabled = false;
        if let Ok(cpuinfo) = fs::read_to_string("/proc/cpuinfo") {
            if cpuinfo.contains("sev"){
                sev_enabled = true;
            }
        }

        let mut sys = System::new();
        let disks = Disks::new_with_refreshed_list();
        let networks = Networks::new_with_refreshed_list();
        sys.refresh_all();

        // CPU info
        let cpu_model = sys.cpus().first()
            .map(|cpu| cpu.brand().to_string())
            .unwrap_or_else(|| "Unknown".to_string());
        let cpu_cores = sys.cpus().len() as u32;

        // Memory info (in MB)
        let memory_mb = sys.total_memory() / 1024;
        let free_memory_mb = sys.available_memory() / 1024;

        // Create a HashSet to track seen disk names
        let mut seen_disks = HashSet::new();

        // Storage info
        let (mut storage_total_mb, mut storage_free_mb) = (0u64, 0u64);

        // Disk information with type
        let disks_info: Vec<DiskInfo> = disks.list().iter().filter_map(|disk| {
            let name = disk.name().to_string_lossy().to_string();
            
            // Check if the disk has already been seen
            if seen_disks.insert(name.clone()) { // Insert the name into the HashSet
                let disk_type = disk.file_system()
                    .to_string_lossy()
                    .to_string();  // Convert OsStr to String directly
                let total_space_mb = disk.total_space() / (1024 * 1024);
                let free_space_mb = disk.available_space() / (1024 * 1024);

                // Update total and free storage
                storage_total_mb += total_space_mb;
                storage_free_mb += free_space_mb;

                Some(DiskInfo {
                    name,
                    disk_type,
                    total_space_mb,
                    free_space_mb,
                }) // Return Some(DiskInfo) if it's a new disk
            } else {
                None // Return None for duplicates
            }
        }).collect();

        // Network bandwidth (MB/s)
        let network_bandwidth_mb_s: u32 = networks.list()
            .iter()
            .map(|(_, data)| {
                let received = data.total_received() / (1024 * 1024); // Convert to MB
                let transmitted = data.total_transmitted() / (1024 * 1024);
                received.saturating_add(transmitted) as u32
            })
            .sum();

        // Get a single primary network interface with IPs
        let mut primary_network_interface: Option<NetworkInterfaceInfo> = None;
        for (name, data) in networks.list().iter() {
            // Skip interfaces that start with "br-", "lo", or "docker"
            if name.starts_with("br-") || name.starts_with("lo") || name.starts_with("docker") {
                continue; // Skip to the next iteration if the name matches any of the prefixes
            }

            // Get MAC address (from the first available network interface with a MAC address)
            let mac_address = networks.list()
                .iter()
                .filter_map(|(_, network)| {
                    Some(network.mac_address())
                }) // if MAC is available
                .map(|mac| {
                    mac.to_string()
                })
                .next();

            let uplink_mb = data.total_transmitted() / (1024 * 1024); // Convert to MB
            let downlink_mb = data.total_received() / (1024 * 1024);  // Convert to MB

            // IP addresses using if-addrs
            let mut ipv4_address = None;
            let mut ipv6_address = None;
            if let Ok(if_addrs) = if_addrs::get_if_addrs() {
                for iface in if_addrs {
                    if iface.name == *name {
                        match iface.addr {
                            IfAddr::V4(v4) => {
                                ipv4_address = Some(v4.ip.to_string());
                            },
                            IfAddr::V6(v6) => {
                                ipv6_address = Some(v6.ip.to_string());
                            },
                        }
                    }
                }
            }

            // Define selection criteria
            if ipv4_address.is_some() || ipv6_address.is_some() {
                primary_network_interface = Some(NetworkInterfaceInfo {
                    name: name.clone(),
                    mac_address,
                    ipv4_address,
                    ipv6_address,
                    uplink_mb,
                    downlink_mb,
                });
                break; // Stop after finding the first matching interface
            }
        }

        // Get ZFS pool info
        let zfs_info = get_zfs_info();

        let (ipfs_zfs_pool_size, ipfs_zfs_pool_alloc, ipfs_zfs_pool_free) = match get_ipfs_pool_info() {
            Ok(values) => values,
            Err(_) => (0, 0, 0), // Return 0 for all values if there's an error
        };
     
        // Get RAID info (if applicable)
        let raid_info = get_raid_info();

        // Count VMs
        let vm_count = count_kvm_vms().unwrap_or(0);

        // Add GPU info retrieval
        let (gpu_name, gpu_memory_mb) = get_gpu_info()
            .map(|(name, memory)| (Some(name), Some(memory)))
            .unwrap_or((None, None));

        // Add hypervisor disk type retrieval
        let hypervisor_disk_type = get_hypervisor_disk_type();

        // Add VM pool disk type retrieval
        let vm_pool_disk_type = get_vm_pool_disk_type();

        // Retrieve disk information using lsblk command
        let disk_info = get_disk_info().unwrap_or_default();

        let info = SystemInfo {
            cpu_model,
            cpu_cores,
            memory_mb,
            free_memory_mb,
            storage_total_mb,
            storage_free_mb,
            network_bandwidth_mb_s,
            primary_network_interface,
            disks: disks_info,
            is_sev_enabled: sev_enabled,
            zfs_info,
            ipfs_zfs_pool_size,
            ipfs_zfs_pool_alloc,
            ipfs_zfs_pool_free,
            raid_info,
            vm_count,
            gpu_name,
            gpu_memory_mb,
            hypervisor_disk_type,
            vm_pool_disk_type,
            disk_info,
        };

        Ok(info)
    }
}

/// Function to get ZFS pool information.
fn get_zfs_info() -> Vec<String> {
    match Command::new("zpool")
          .arg("list")
          .output() {
              Ok(output) => {
                  String::from_utf8_lossy(&output.stdout)
                      .lines()
                      .map(|line| line.to_string())
                      .collect()
              },
              Err(e) => {
                  log::error!("Failed to execute zpool command: {}", e);
                  vec!["Error retrieving ZFS info".to_string()]
              }
          }
}

/// Function to get RAID information.
fn get_raid_info() -> Vec<String> {
    // First try lspci for hardware RAID controllers
    let lspci_raid = match Command::new("lspci")
          .arg("-v")
          .arg("-s")
          .arg("*:*:*")
          .output() {
              Ok(output) => {
                  let output_str = String::from_utf8_lossy(&output.stdout);
                  output_str
                      .lines()
                      .filter(|line| line.contains("RAID") || line.contains("RAID bus controller"))
                      .map(|line| line.to_string())
                      .collect::<Vec<String>>()
              },
              Err(e) => {
                  log::error!("Failed to read RAID info via lspci: {}", e);
                  Vec::new()
              }
          };

    // If no RAID controllers found via lspci, try mdadm for software RAID
    if lspci_raid.is_empty() {
        match Command::new("mdadm")
              .arg("--detail")
              .arg("--scan")
              .output() {
                  Ok(output) => {
                      let output_str = String::from_utf8_lossy(&output.stdout);
                      if output_str.trim().is_empty() {
                          vec!["No software RAID detected".to_string()]
                      } else {
                          output_str
                              .lines()
                              .map(|line| line.to_string())
                              .collect()
                      }
                  },
                  Err(e) => {
                      log::error!("Failed to read RAID info via mdadm: {}", e);
                      vec!["Error retrieving RAID info".to_string()]
                  }
              }
    } else {
        lspci_raid
    }
}

fn get_ipfs_pool_info() -> Result<(u128, u128, u128), String> {
    match Command::new("zpool")
        .arg("list")
        .arg("ipfs")
        .output() 
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            log::info!("IPFS pool info: {}", stdout);
            let lines: Vec<&str> = stdout.lines().collect();
            if lines.len() > 1 {
                let pool_info = lines[1].split_whitespace().collect::<Vec<&str>>();
                if pool_info.len() >= 4 { // Ensure enough fields exist
                    let size = parse_size(pool_info[1])?;
                    let alloc = parse_size(pool_info[2])?;
                    let free = parse_size(pool_info[3])?;

                    log::info!("Parsed IPFS ZFS pool size: {} bytes, allocated: {} bytes, free: {} bytes", size, alloc, free);

                    return Ok((size, alloc, free));
                } else {
                    log::error!("Unexpected pool_info format: {:?}", pool_info);
                }
            } else {
                log::error!("No pool information found in the output.");
            }
            Err("Failed to parse IPFS pool information".to_string())
        },
        Err(e) => {
            log::error!("Failed to execute zpool command for IPFS: {}", e);
            Err("Error retrieving IPFS pool info".to_string())
        }
    }
}


fn parse_size(size_str: &str) -> Result<u128, String> {
    let size_str = size_str.trim();
    
    // Extract number and unit separately
    let (number_part, unit) = size_str.split_at(size_str.chars().position(|c| c.is_alphabetic()).unwrap_or(size_str.len()));

    let mut size = number_part.trim().parse::<f64>().map_err(|_| format!("Failed to parse number: {}", number_part))?;

    match unit.trim().to_lowercase().as_str() {
        "t" | "tb" => size *= 1024.0 * 1024.0 * 1024.0 * 1024.0, // TiB to bytes
        "g" | "gb" => size *= 1024.0 * 1024.0 * 1024.0, // GiB to bytes
        "m" | "mb" => size *= 1024.0 * 1024.0,         // MiB to bytes
        "k" | "kb" => size *= 1024.0,                  // KiB to bytes
        "" => {} // No unit, assume bytes
        _ => return Err(format!("Unknown size unit: {}", unit)),
    }

    Ok(size as u128)
}

use sp_std::str::FromStr;
use crate::types::{SystemInfo, NetworkInterfaceInfo, DiskInfo, NetworkDetails, NetworkType, DiskDetails}; // Adjust paths if necessary
use sp_std::prelude::*;
use sp_runtime::offchain::http;
use sp_runtime::offchain::Duration;
use sp_runtime::format;
use serde_json;
use scale_info::prelude::string::String;

impl SystemInfo {
    /// Fetch IPFS repository statistics via an HTTP request.
    pub fn get_ipfs_repo_stat() -> Result<(u64, u64), http::Error> {
    
        /// dummy request body for getting last finalized block from rpc
        pub const DUMMY_REQUEST_BODY: &[u8; 78] =
        b"{\"id\": 10, \"jsonrpc\": \"2.0\", \"method\": \"chain_getFinalizedHead\", \"params\": []}";

        // Base URL of the IPFS node
        let url = format!("http://127.0.0.1:5001/api/v0/repo/stat");

        // Request timeout
        let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(2_000));

        let body = vec![DUMMY_REQUEST_BODY];

        // getting the stats of ipfs node
        let request = sp_runtime::offchain::http::Request::post(&url, body);

        let pending = request
            .deadline(deadline)
            .send()
            .map_err(|err| {
                log::warn!("Error making Request: {:?}", err);
                sp_runtime::offchain::http::Error::IoError
            })?;

        // Getting response
        let response = pending
            .try_wait(deadline)
            .map_err(|err| {
                log::warn!("Error getting Response: {:?}", err);
                sp_runtime::offchain::http::Error::DeadlineReached
            })??;

        // Check the response status code
        if response.code != 200 {
            log::warn!("Unexpected status code: {}", response.code);
            return Err(http::Error::Unknown);
        }

        // Read the response body
        let body = response.body().collect::<Vec<u8>>();	

        // Convert the body to a string
        let body_str = sp_std::str::from_utf8(&body).map_err(|_| {
            log::warn!("Response body is not valid UTF-8");
            http::Error::Unknown
        })?;

        // Helper function to extract numeric value for a given key
        fn extract_number(json_str: &str, key: &str) -> Option<u64> {
            json_str
                .split(key)
                .nth(1)?
                .trim_start()
                .trim_start_matches(':')
                .trim_start()
                .split(',')
                .next()?
                .trim()
                .parse()
                .ok()
        }

        // Extract RepoSize and StorageMax
        let repo_size = extract_number(body_str, "\"RepoSize\"")
            .ok_or_else(|| {
                log::warn!("Failed to parse RepoSize");
                http::Error::Unknown
            })?;

        let storage_max = extract_number(body_str, "\"StorageMax\"")
            .ok_or_else(|| {
                log::warn!("Failed to parse StorageMax");
                http::Error::Unknown
            })?;

        Ok((repo_size, storage_max))

    }

    // fetching ip details like Private/Pub , loc etc
    fn fetch_ip_details() -> Result<Option<NetworkDetails>, http::Error> {

        // let url = format!("https://api.ipify.org");
        
        // // Increase timeout to 10 seconds
        // let deadline = sp_io::offchain::timestamp()
        //     .add(Duration::from_millis(10_000));
            
        // let request = sp_runtime::offchain::http::Request::get(url.as_str());

        // let pending = request
        // .add_header("Content-Type", "application/json")
        // .deadline(deadline)
        // .send()
        // .map_err(|err| {
        //     log::warn!("Error making Request: {:?}", err);
        //     sp_runtime::offchain::http::Error::IoError
        // })?;
        
        // // Wait for response with better error handling
        // let response = match pending.try_wait(deadline) {
        //     Ok(Ok(r)) => r,
        //     Ok(Err(e)) => {
        //         log::error!("Request failed: {:?}", e);
        //         return Err(e);
        //     }
        //     Err(e) => {
        //         log::error!("Deadline reached: {:?}", e);
        //         return Err(http::Error::DeadlineReached);
        //     }
        // };
        
        // // Check the response status code
        // if response.code != 200 {
        //     log::warn!("Unexpected status code: {}", response.code);
        //     return Err(http::Error::Unknown);
        // }
        
        // // Read the response body
        // let body = response.body().collect::<Vec<u8>>();	

        // // Convert the body to a string
        // let body_str = sp_std::str::from_utf8(&body).map_err(|_| {
        //     log::warn!("Response body is not valid UTF-8");
        //     http::Error::Unknown
        // })?;
        
        let url = format!("https://ip-api.hippius.network/?format=json");
        
        // Increase timeout to 10 seconds
        let deadline = sp_io::offchain::timestamp()
            .add(Duration::from_millis(10_000));
            
        let request = sp_runtime::offchain::http::Request::get(url.as_str());

        let pending = request
        .add_header("Content-Type", "application/json")
        .deadline(deadline)
        .send()
        .map_err(|err| {
            log::warn!("Error making Request: {:?}", err);
            sp_runtime::offchain::http::Error::IoError
        })?;
        
        // Wait for response with better error handling
        let response = match pending.try_wait(deadline) {
            Ok(Ok(r)) => r,
            Ok(Err(e)) => {
                log::error!("Request failed: {:?}", e);
                return Err(e);
            }
            Err(e) => {
                log::error!("Deadline reached: {:?}", e);
                return Err(http::Error::DeadlineReached);
            }
        };
        
        // Check the response status code
        if response.code != 200 {
            log::warn!("Unexpected status code: {}", response.code);
            return Err(http::Error::Unknown);
        }
        
        // Read the response body
        let body = response.body().collect::<Vec<u8>>();	

        // Convert the body to a string
        let body_str = sp_std::str::from_utf8(&body).map_err(|_| {
            log::warn!("Response body is not valid UTF-8");
            http::Error::Unknown
        })?;

        // Improved helper function to extract value between quotes
        fn extract_value<'a>(json_str: &'a str, key: &str) -> Option<&'a str> {
            json_str
                .split(key)
                .nth(1)?
                .trim_start()
                .trim_start_matches(':')
                .trim_start()
                .trim_start_matches('"')
                .split('"')
                .next()
        }

        // Helper function to detect bogon field
        fn is_bogon(json_str: &str) -> bool {
            json_str.contains("\"bogon\": true")
        }

        // Check if it's a private IP (bogon)
        let is_private = is_bogon(body_str);
        let _ip = match extract_value(body_str, "\"ip\"") {
            Some(ip) => ip,
            None => {
                log::error!("Failed to extract IP from response");
                return Ok(None);
            }
        };

        if is_private {
            let details = NetworkDetails {
                network_type: NetworkType::Private,
                city: None,
                region: None,
                country: None,
                loc: None,
            };
            Ok(Some(details))
        } else {
            let details = NetworkDetails {
                network_type: NetworkType::Public,
                city: match extract_value(body_str, "\"city\"") {
                    Some(value) => Some(value.into()),
                    None => None
                },
                region: match extract_value(body_str, "\"region\"") {
                    Some(value) => Some(value.into()),
                    None => None
                },
                country: match extract_value(body_str, "\"country\"") {
                    Some(value) => Some(value.into()),
                    None => None
                },
                loc: match extract_value(body_str, "\"loc\"") {
                    Some(value) => Some(value.into()),
                    None => None
                },
            };
            Ok(Some(details))
        }
    }
}

impl FromStr for SystemInfo {
    type Err = &'static str;
    // parsing the request output
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // Parse cpu_model
        let cpu_model = if let Some(start) = s.find("\"cpu_model\":\"") {
            let after_key = &s[start + "\"cpu_model\":".len()..];
            if let Some(end) = after_key.find("\",") {
                after_key[..end].as_bytes().to_vec()
            } else {
                return Err("Missing closing quote for cpu_model value");
            }
        } else {
            return Err("cpu_model key not found");
        };
    
        // Parse memory_mb
        let memory_mb = if let Some(start) = s.find("\"memory_mb\":") {
            let after_key = &s[start + "\"memory_mb\":".len()..];
            if let Some(end) = after_key.find(",") {
                after_key[..end].trim().parse().map_err(|_| "Failed to parse memory_mb")?
            } else {
                return Err("Missing comma after memory_mb value");
            }
        } else {
            return Err("memory_mb key not found");
        };
    
        // Parse cpu_cores
        let cpu_cores = if let Some(start) = s.find("\"cpu_cores\":") {
            let after_key = &s[start + "\"cpu_cores\":".len()..];
            if let Some(end) = after_key.find(",") {
                after_key[..end].trim().parse().map_err(|_| "Failed to parse cpu_cores")?
            } else {
                return Err("Missing comma after cpu_cores value");
            }
        } else {
            return Err("cpu_cores key not found");
        };
    
        // Parse free_memory_mb
        let free_memory_mb = if let Some(start) = s.find("\"free_memory_mb\":") {
            let after_key = &s[start + "\"free_memory_mb\":".len()..];
            if let Some(end) = after_key.find(",") {
                after_key[..end].trim().parse().map_err(|_| "Failed to parse free_memory_mb")?
            } else {
                return Err("Missing comma after free_memory_mb value");
            }
        } else {
            return Err("free_memory_mb key not found");
        };
    
        // Parse storage_total_mb
        let storage_total_mb = if let Some(start) = s.find("\"storage_total_mb\":") {
            let after_key = &s[start + "\"storage_total_mb\":".len()..];
            if let Some(end) = after_key.find(",") {
                after_key[..end].trim().parse().map_err(|_| "Failed to parse storage_total_mb")?
            } else {
                return Err("Missing comma after storage_total_mb value");
            }
        } else {
            return Err("storage_total_mb key not found");
        };
    
        // Parse storage_free_mb
        let storage_free_mb = if let Some(start) = s.find("\"storage_free_mb\":") {
            let after_key = &s[start + "\"storage_free_mb\":".len()..];
            if let Some(end) = after_key.find(",") {
                after_key[..end].trim().parse::<u64>().map_err(|_| "Failed to parse storage_free_mb")?
            } else { 
                // If no comma found, try to parse the entire remaining substring
                after_key.trim().parse::<u64>().map_err(|_| "Failed to parse storage_free_mb")?
            }
        } else {
            // Fallback to free_memory_mb if storage_free_mb is not found
            free_memory_mb
        };
    
        // Parse network_bandwidth_mb_s
        let network_bandwidth_mb_s = if let Some(start) = s.find("\"network_bandwidth_mb_s\":") {
            let after_key = &s[start + "\"network_bandwidth_mb_s\":".len()..];
            if let Some(end) = after_key.find(",") {
                after_key[..end].trim().parse().map_err(|_| "Failed to parse network_bandwidth_mb_s")?
            } else {
                return Err("Missing comma after network_bandwidth_mb_s value");
            }
        } else {
            return Err("network_bandwidth_mb_s key not found");
        };
    
        let mut interface = NetworkInterfaceInfo {
            name: Vec::new(),
            mac_address: None,
            uplink_mb: 0,    
            downlink_mb: 0,  
            network_details: None,
        };        
    
        // Parse network interface name
        if let Some(start) = s.find("\"primary_network_interface\":{\"name\":\"") {
            let after_key = &s[start + "\"primary_network_interface\":{\"name\":\"".len()..];
            if let Some(end) = after_key.find("\",") {
                interface.name = after_key[..end].as_bytes().to_vec();
            } else {
                return Err("Missing closing quote for network interface name");
            }
        }
    
        // Parse mac_address
        if let Some(start) = s.find("\"mac_address\":\"") {
            let after_key = &s[start + "\"mac_address\":\"".len()..];
            if let Some(end) = after_key.find("\",") {
                interface.mac_address = Some(after_key[..end].as_bytes().to_vec());
            }
        }
    
        let network_details = match Self::fetch_ip_details() {
            Ok(Some(details)) => {
                Some(details)
            },
            _ => {
                log::warn!("Failed to fetch network details");
                None
            }
        };
        interface.network_details = network_details;
    
        let (repo_size, storage_max) = match Self::get_ipfs_repo_stat() {
            Ok((size, max)) => (size, max),
            Err(e) => {
                log::error!("Failed to get IPFS repo stats: {:?}", e);
                (0, 0) // or some other default values
            }
        };
    
        // Parse uplink_mb
        if let Some(start) = s.find("\"uplink_mb\":") {
            let after_key = &s[start + "\"uplink_mb\":".len()..];
            if let Some(end) = after_key.find(",") {
                interface.uplink_mb = after_key[..end].trim().parse().map_err(|_| "Failed to parse uplink_mb")?;
            }
        }
    
        // Parse downlink_mb - Fixed the bug here
        if let Some(start) = s.find("\"downlink_mb\":") {
            let after_key = &s[start + "\"downlink_mb\":".len()..];
            if let Some(end) = after_key.find("},") {
                interface.downlink_mb = after_key[..end].trim().parse().map_err(|_| "Failed to parse downlink_mb")?;
            }
        }
    
        // Parse disk_info
        let mut disk_info = Vec::new();
        if let Some(start) = s.find("\"disk_info\":[") {
            let after_start = &s[start + "\"disk_info\":[".len()..];
            if let Some(end) = after_start.find("]") {
                let disk_str = &after_start[..end];
                
                // Use a more robust parsing approach
                let mut current_disk = String::new();
                let mut in_object = false;
                let mut brace_count = 0;

                for ch in disk_str.chars() {
                    match ch {
                        '{' => {
                            in_object = true;
                            brace_count += 1;
                            current_disk.push(ch);
                        }
                        '}' => {
                            brace_count -= 1;
                            current_disk.push(ch);
                            
                            if brace_count == 0 && in_object {
                                // Complete object found
                                if let Ok(disk_details) = parse_single_disk_details(&current_disk) {
                                    disk_info.push(disk_details);
                                }
                                current_disk.clear();
                                in_object = false;
                            }
                        }
                        _ => {
                            if in_object {
                                current_disk.push(ch);
                            }
                        }
                    }
                }
            }
        }

        // Helper function to parse a single disk details object
        fn parse_single_disk_details(disk_str: &str) -> Result<DiskDetails, &'static str> {
            let mut name = b"N/A".to_vec();
            let mut serial = b"N/A".to_vec();
            let mut model = b"N/A".to_vec();
            let mut size = b"N/A".to_vec();
            let mut is_rotational = false;
            let mut disk_type = b"N/A".to_vec();

            // Use serde_json for robust parsing
            if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(disk_str) {
                if let Some(obj) = json_value.as_object() {
                    name = obj.get("name")
                        .and_then(|v| v.as_str())
                        .map(|s| s.as_bytes().to_vec())
                        .unwrap_or_else(|| b"N/A".to_vec());

                    serial = obj.get("serial")
                        .and_then(|v| v.as_str())
                        .map(|s| s.as_bytes().to_vec())
                        .unwrap_or_else(|| b"N/A".to_vec());

                    model = obj.get("model")
                        .and_then(|v| v.as_str())
                        .map(|s| s.as_bytes().to_vec())
                        .unwrap_or_else(|| b"N/A".to_vec());

                    size = obj.get("size")
                        .and_then(|v| v.as_str())
                        .map(|s| s.as_bytes().to_vec())
                        .unwrap_or_else(|| b"N/A".to_vec());

                    is_rotational = obj.get("is_rotational")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    disk_type = obj.get("disk_type")
                        .and_then(|v| v.as_str())
                        .map(|s| s.as_bytes().to_vec())
                        .unwrap_or_else(|| b"N/A".to_vec());
                }
            }

            Ok(DiskDetails {
                name,
                serial,
                model,
                size,
                is_rotational,
                disk_type,
            })
        }

        // Initialize disks vector
        let mut disks = Vec::new();
    
        // Extract disks
        if let Some(start) = s.find("\"disks\":") {
            if let Some(after_start) = s[start..].find("[") {
                if let Some(after_end) = s[start + after_start..].find("]") {
                    let disks_str = &s[start + after_start + 1..start + after_start + after_end];
    
                    for disk in disks_str.split("},{").filter(|d| !d.is_empty()) {
                        let cleaned_disk_str = disk.trim_start_matches('{').trim_end_matches('}');
    
                        let mut name = Vec::new();
                        let mut disk_type = Vec::new();
                        let mut total_space_mb = 0;
                        let mut free_space_mb = 0;
                        
                        if let Some(start) = cleaned_disk_str.find("\"name\":\"") {
                            let after_key = &cleaned_disk_str[start + "\"name\":\"".len()..];
                            if let Some(end) = after_key.find("\",") {
                                name = after_key[..end].as_bytes().to_vec();
                            } else if let Some(end) = after_key.find('"') { 
                                name = after_key[..end].as_bytes().to_vec();
                            }
                        }
    
                        if let Some(start) = cleaned_disk_str.find("\"disk_type\":\"") {
                            let after_key = &cleaned_disk_str[start + "\"disk_type\":\"".len()..];
                            if let Some(end) = after_key.find("\",") {
                                disk_type = after_key[..end].as_bytes().to_vec();
                            } else if let Some(end) = after_key.find('"') { 
                                disk_type = after_key[..end].as_bytes().to_vec();
                            }
                        }
    
                        if let Some(start) = cleaned_disk_str.find("\"total_space_mb\":") {
                            let after_key = &cleaned_disk_str[start + "\"total_space_mb\":".len()..];
                            if let Some(end) = after_key.find(",") {
                                total_space_mb = after_key[..end].trim().parse().map_err(|_| "Failed to parse total_space_mb")?;
                            } else if let Some(end) = after_key.find('"') { 
                                total_space_mb = after_key[..end].trim().parse().map_err(|_| "Failed to parse total_space_mb")?;
                            }
                        }
    
                        if let Some(start) = cleaned_disk_str.find("\"free_space_mb\":") {
                            let after_key = &cleaned_disk_str[start + "\"free_space_mb\":".len()..];
                            if let Some(end) = after_key.find(",") {
                                free_space_mb = after_key[..end].trim().parse().map_err(|_| "Failed to parse free_space_mb")?;
                            } else { 
                                free_space_mb = after_key.trim().parse().map_err(|_| "Failed to parse free_space_mb")?;
                            }
                        }
    
                        disks.push(DiskInfo {
                            name,
                            disk_type,
                            total_space_mb,
                            free_space_mb, 
                        });
                    }
                }
            }
        }
                  
        // Parse is_sev_enabled
        let is_sev_enabled = if let Some(start) = s.find("\"is_sev_enabled\":") {
            let after_key = &s[start + "\"is_sev_enabled\":".len()..];
            if let Some(end) = after_key.find(",") {
                after_key[..end].trim().parse().map_err(|_| "Failed to parse is_sev_enabled")?
            } else {
                return Err("Missing closing brace after is_sev_enabled value");
            }
        } else {
            return Err("is_sev_enabled key not found");
        };

        // Parse zfs_info
        let mut zfs_info = Vec::new();
        if let Some(start) = s.find("\"zfs_info\":[") {
            let after_start = &s[start + "\"zfs_info\":[".len()..];
            if let Some(end) = after_start.find("]") {
                let zfs_str = &after_start[..end];
                for zfs_item in zfs_str.split(',') {
                    let cleaned_zfs_item = zfs_item
                        .trim()
                        .trim_matches('"')
                        .as_bytes()
                        .to_vec();
                    zfs_info.push(cleaned_zfs_item);
                }
            }
        }

        // Parse IPFS pool information
        let mut ipfs_zfs_pool_size = 0u128;
        let mut ipfs_zfs_pool_alloc = 0u128;
        let mut ipfs_zfs_pool_free = 0u128;

        if let Some(start) = s.find("\"ipfs_zfs_pool_size\":") {
            let after_start = &s[start + "\"ipfs_zfs_pool_size\":".len()..];
            if let Some(end) = after_start.find(',') {
                let size_str = &after_start[..end].trim().trim_matches('"');
                ipfs_zfs_pool_size = size_str.parse::<u128>().unwrap_or(0);
            }
        }

        if let Some(start) = s.find("\"ipfs_zfs_pool_alloc\":") {
            let after_start = &s[start + "\"ipfs_zfs_pool_alloc\":".len()..];
            if let Some(end) = after_start.find(',') {
                let alloc_str = &after_start[..end].trim().trim_matches('"');
                ipfs_zfs_pool_alloc = alloc_str.parse::<u128>().unwrap_or(0);
            }
        }

        if let Some(start) = s.find("\"ipfs_zfs_pool_free\":") {
            let after_start = &s[start + "\"ipfs_zfs_pool_free\":".len()..];
            if let Some(end) = after_start.find('}') {
                let free_str = &after_start[..end].trim().trim_matches('"');
                ipfs_zfs_pool_free = free_str.parse::<u128>().unwrap_or(0);
            }
        }

        // Parse raid_info
        let mut raid_info = Vec::new();
        if let Some(start) = s.find("\"raid_info\":[") {
            let after_start = &s[start + "\"raid_info\":[".len()..];
            if let Some(end) = after_start.find("]") {
                let raid_str = &after_start[..end];
                for raid_item in raid_str.split(',') {
                    let cleaned_raid_item = raid_item
                        .trim()
                        .trim_matches('"')
                        .as_bytes()
                        .to_vec();
                    raid_info.push(cleaned_raid_item);
                }
            }
        }

        // Parse vm_count
        let vm_count = if let Some(start) = s.find("\"vm_count\":") {
            let after_key = &s[start + "\"vm_count\":".len()..];
            if let Some(end) = after_key.find(",") {
                after_key[..end].trim().parse().map_err(|_| "Failed to parse vm_count")?
            } else {
                return Err("Missing closing brace after vm_count value");
            }
        } else {
            0 // Default to 0 if not found
        };

        // Parse gpu_name
        let gpu_name = if let Some(start) = s.find("\"gpu_name\":") {
            let after_key = &s[start + "\"gpu_name\":".len()..];
            if let Some(end) = after_key.find(",") {
                let gpu_name_str = after_key[..end].trim().trim_matches('"');
                if !gpu_name_str.is_empty() && gpu_name_str != "null" {
                    Some(gpu_name_str.as_bytes().to_vec())
                } else {
                    None
                }
            } else {
                return Err("Missing closing brace after gpu_name value");
            }
        } else {
            None // Default to None if not found
        };

        // Parse gpu_memory_mb
        let gpu_memory_mb = if let Some(start) = s.find("\"gpu_memory_mb\":") {
            let after_key = &s[start + "\"gpu_memory_mb\":".len()..];
            if let Some(end) = after_key.find(",") {
                let gpu_memory_mb_str = after_key[..end].trim().trim_matches('"');
                if !gpu_memory_mb_str.is_empty() && gpu_memory_mb_str != "null" {
                    // Convert string to u32
                    gpu_memory_mb_str.parse::<u32>().ok()
                } else {
                    None
                }
            } else {
                return Err("Missing closing brace after gpu_memory_mb value");
            }
        } else {
            None // Default to None if not found
        };

        // Parse gpu_name
        let hypervisor_disk_type = if let Some(start) = s.find("\"hypervisor_disk_type\":") {
            let after_key = &s[start + "\"hypervisor_disk_type\":".len()..];
            if let Some(end) = after_key.find(",") {
                let hypervisor_disk_type_str = after_key[..end].trim().trim_matches('"');
                if !hypervisor_disk_type_str.is_empty() && hypervisor_disk_type_str != "null" {
                    Some(hypervisor_disk_type_str.as_bytes().to_vec())
                } else {
                    None
                }
            } else {
                return Err("Missing closing brace after gpu_memory_mb value");
            }
        } else {
            None // Default to None if not found
        };

        // Parse gpu_name
        let vm_pool_disk_type = if let Some(start) = s.find("\"vm_pool_disk_type\":") {
            let after_key = &s[start + "\"vm_pool_disk_type\":".len()..];
            if let Some(end) = after_key.find("}") {
                let vm_pool_disk_type_str = after_key[..end].trim().trim_matches('"');
                if !vm_pool_disk_type_str.is_empty() && vm_pool_disk_type_str != "null" {
                    Some(vm_pool_disk_type_str.as_bytes().to_vec())
                } else {
                    None
                }
            } else {
                return Err("Missing closing brace after gpu_memory_mb value");
            }
        } else {
            None // Default to None if not found
        };


        Ok(SystemInfo {
            cpu_model,
            cpu_cores,
            memory_mb,
            free_memory_mb,
            storage_total_mb,
            storage_free_mb,
            network_bandwidth_mb_s,
            primary_network_interface: Some(interface), 
            disks,
            ipfs_repo_size: repo_size,
            ipfs_storage_max: storage_max,  
            is_sev_enabled,
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
        })
    }
}
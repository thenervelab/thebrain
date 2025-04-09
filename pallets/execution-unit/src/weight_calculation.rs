pub use crate::types::NodeMetricsData;
use pallet_registration::NodeType;
use sp_std::{collections::btree_map::BTreeMap, vec::Vec};
// use pallet_compute::TechnicalDescription;
// use frame_system;

impl NodeMetricsData {
	// Configuration constants
	const MIN_PEER_COUNT: u32 = 10;
	const MAX_SCORE: u32 = 65535; // 16-bit maximum
	const MIN_STORAGE_GB: u32 = 2048; // Minimum 2TB storage
	const OPTIMAL_STORAGE_USAGE_MIN: u32 = 60; // 60% minimum usage
	const OPTIMAL_STORAGE_USAGE_MAX: u32 = 80; // 80% maximum usage
	const MAX_RESPONSE_TIME_MS: u32 = 1000;
	const MIN_UPTIME_PERCENTAGE: u32 = 95; // Minimum 95% uptime expected
	const MAX_CONSECUTIVE_FAILURES: u32 = 5;
	const GEOGRAPHIC_DIVERSITY_TARGET: u32 = 5; // Target number of distinct regions
	const INTERNAL_SCALING: u32 = 1_000_000;

	// New method to calculate compute RAM score
	fn calculate_compute_ram_score(metrics: &NodeMetricsData) -> u64 {
		if !metrics.is_sev_enabled {
			return 0;
		}
		// // RAM type scoring
		// let ram_type_score = match metrics.ram_type.as_str() {
		//     "DDR5" => 100,   // Highest score for latest technology
		//     "DDR4" => 75,    // Good, but older technology
		//     "DDR3" => 50,    // Older technology
		//     _ => 25,          // Unrecognized or very old RAM type
		// };

		// RAM quantity scoring
		// Minimum requirement is 128GB
		// Convert memory from MB to GB (divide by 1024)
		let ram_gb = metrics.free_memory_mb / 1024;
		let ram_quantity_score = if ram_gb >= 128 {
			// Bonus scoring for RAM beyond 128GB
			match ram_gb {
				x if x >= 1024 => 100, // 1TB or more of RAM
				x if x >= 512 => 90,   // 512GB to 1TB
				x if x >= 256 => 80,   // 256GB to 512GB
				x if x >= 192 => 70,   // 192GB to 256GB
				x if x >= 128 => 60,   // Minimum 128GB
				_ => 0,                // Below minimum, no score
			}
		} else {
			0 // Does not meet minimum RAM requirement
		};

		// Combine RAM type and quantity scores
		// // Weighted more towards quantity, but type still matters
		// let combined_ram_score = (
		//     (ram_type_score as u64 * 30) +  // 30% weight to RAM type
		//     (ram_quantity_score as u64 * 70)  // 70% weight to RAM quantity
		// ) / 100;

		let combined_ram_score = ram_quantity_score as u64 / 100;

		combined_ram_score
	}

	// // New method to calculate compute usage score
	// fn calculate_compute_usage_score<T: pallet_compute::Config + pallet_marketplace::Config>(metrics: &NodeMetricsData) -> u64 {
	//     // Use a more generic approach to retrieve compute requests
	//     let miner_id = metrics.miner_id.clone();
	//     let miner_compute_requests = pallet_compute::MinerComputeRequests::<T>::get(miner_id);

	//     let mut total_ram_usage = 0;
	//     let mut total_cpu_usage = 0;
	//     let mut total_storage_usage = 0;

	//     for request in miner_compute_requests {
	//         // Skip unfulfilled requests
	//         if !request.fullfilled {
	//             continue;
	//         }

	//         // Retrieve the plan to get technical description
	//         if let Some(plan) = pallet_marketplace::Plans::<T>::get(request.plan_id) {
	//             // Deserialize technical description
	//             if let Ok(tech_desc) = serde_json::from_slice::<TechnicalDescription>(&plan.plan_technical_description) {
	//                 // Accumulate resource usage
	//                 total_ram_usage += tech_desc.ram_gb as u64;
	//                 total_cpu_usage += tech_desc.cpu_cores as u64;
	//                 total_storage_usage += tech_desc.storage_gb as u64;
	//             }
	//         }
	//     }

	//     // Define weights for each resource type
	//     const RAM_WEIGHT: u64 = 50;   // 50% importance
	//     const CPU_WEIGHT: u64 = 30;   // 30% importance
	//     const STORAGE_WEIGHT: u64 = 20;  // 20% importance

	//     // Normalize and calculate weighted score out of 100
	//     // First, calculate individual resource scores
	//     let ram_score = total_ram_usage.saturating_mul(RAM_WEIGHT);
	//     let cpu_score = total_cpu_usage.saturating_mul(CPU_WEIGHT);
	//     let storage_score = total_storage_usage.saturating_mul(STORAGE_WEIGHT);

	//     // Sum up the weighted scores and divide by total weight to get a score out of 100
	//     let total_weighted_score = ram_score + cpu_score + storage_score;
	//     let normalized_score = total_weighted_score / (RAM_WEIGHT + CPU_WEIGHT + STORAGE_WEIGHT);

	//     normalized_score
	// }

	pub fn calculate_weight<T: pallet_marketplace::Config>(
		_node_type: NodeType,
		metrics: &NodeMetricsData,
		all_nodes_metrics: &[NodeMetricsData],
		geo_distribution: &BTreeMap<Vec<u8>, u32>,
	) -> u32 {
		// Early return for storage miners with less than 1 GB storage
		if _node_type == NodeType::StorageMiner
			&& metrics.ipfs_storage_max < (Self::MIN_STORAGE_GB as u64 * 1024 * 1024 * 1024)
		{
			return 0;
		}

		// Early return for storage miners with bandwidth less than 125 Mbps
		if _node_type == NodeType::StorageMiner && metrics.bandwidth_mbps < 125 {
			return 0;
		}

		// Calculate base scores with u64 casting for safety
		let availability_score =
			(Self::calculate_availability_score(metrics) as u64).saturating_div(100);
		let performance_score =
			(Self::calculate_performance_score(metrics) as u64).saturating_div(100);
		let reliability_score =
			(Self::calculate_reliability_score(metrics) as u64).saturating_div(100);

		// Conditionally include capacity score only for storage miners
		let capacity_score = match _node_type {
			NodeType::StorageMiner => {
				(Self::calculate_capacity_score::<T>(metrics) as u64).saturating_div(100)
			},
			_ => 0,
		};

		// Conditionally include storage usage score only for storage miners
		let storage_usage_score = match _node_type {
			NodeType::StorageMiner => {
				(Self::calculate_storage_usage_score::<T>(metrics) as u64).saturating_div(100)
			},
			_ => 0,
		};

		// Conditionally include compute RAM score only for compute miners
		let compute_ram_score = match _node_type {
			NodeType::ComputeMiner => {
				Self::calculate_compute_ram_score(metrics).saturating_div(100)
			},
			_ => 0,
		};

		// Conditionally include compute usage score only for compute miners
		// let compute_usage_score = match _node_type {
		//     NodeType::ComputeMiner => Self::calculate_compute_usage_score::<T>(metrics).saturating_div(100),
		//     _ => 0,
		// };
		let compute_usage_score: u64 = 0;

		let network_score = (Self::calculate_network_score(metrics) as u64).saturating_div(100);
		let diversity_score =
			(Self::calculate_diversity_score(metrics, geo_distribution) as u64).saturating_div(100);

		// Adjust weight calculation based on node type
		let base_weight = match _node_type {
			NodeType::StorageMiner => {
				// If capacity_score is 0 for StorageMiner, set base_weight to 0 and return early
				if capacity_score == 0 {
					return 0;
				}
				(
					availability_score
					.saturating_mul(15) // Reduced from 20
					.saturating_add(performance_score.saturating_mul(10)) // Unchanged
					.saturating_add(reliability_score.saturating_mul(10)) // Unchanged
					.saturating_add(capacity_score.saturating_mul(35)) // Increased from 25
					.saturating_add(storage_usage_score.saturating_mul(20)) // Increased from 15
					.saturating_add(network_score.saturating_mul(5)) // Reduced from 10
					.saturating_add(diversity_score.saturating_mul(5)) // Unchanged
				) as u32
			},
			NodeType::StorageS3 => {
				(availability_score
					.saturating_mul(35)
					.saturating_add(performance_score.saturating_mul(20))
					.saturating_add(reliability_score.saturating_mul(15))
					.saturating_add(network_score.saturating_mul(15))
					.saturating_add(diversity_score.saturating_mul(15))) as u32
			},
			NodeType::ComputeMiner => {
				(availability_score
					.saturating_mul(35)
					.saturating_add(performance_score.saturating_mul(5))
					.saturating_add(reliability_score.saturating_mul(10))
					.saturating_add(compute_ram_score.saturating_mul(15)) // New compute RAM score
					.saturating_add(compute_usage_score.saturating_mul(25)) // New compute usage score
					.saturating_add(network_score.saturating_mul(5))
					.saturating_add(diversity_score.saturating_mul(5))) as u32
			},
			NodeType::Validator => {
				(availability_score
					.saturating_mul(30)
					.saturating_add(performance_score.saturating_mul(25))
					.saturating_add(reliability_score.saturating_mul(25))
					.saturating_add(network_score.saturating_mul(15))
					.saturating_add(diversity_score.saturating_mul(5))) as u32
			},
			NodeType::GpuMiner => {
				(availability_score
					.saturating_mul(35)
					.saturating_add(performance_score.saturating_mul(5))
					.saturating_add(reliability_score.saturating_mul(10))
					.saturating_add(compute_ram_score.saturating_mul(15)) // New compute RAM score
					.saturating_add(compute_usage_score.saturating_mul(25)) // New compute usage score
					.saturating_add(network_score.saturating_mul(5))
					.saturating_add(diversity_score.saturating_mul(5))) as u32
			},
		};

		// Early return if base_weight is 0 (for other node types)
		if base_weight == 0 {
			return 0;
		}

		// Calculate modifiers
		let bonuses = (Self::calculate_bonuses(metrics) as u64).saturating_div(100);
		let penalties = (Self::calculate_penalties(metrics) as u64).saturating_div(100);

		let modifier = (Self::INTERNAL_SCALING as u64)
			.saturating_div(100)
			.saturating_add(bonuses)
			.saturating_sub(penalties) as u32;

		// Calculate scaling factors
		let network_scaling =
			Self::_get_network_scaling_factor(all_nodes_metrics).saturating_div(100);
		let relative_position =
			Self::_calculate_relative_position(metrics, all_nodes_metrics).saturating_div(100);

		// Calculate final weight with careful scaling
		let intermediate = (base_weight as u64)
			.saturating_mul(network_scaling as u64)
			.saturating_div((Self::INTERNAL_SCALING / 100) as u64) as u32;

		let with_modifier = (intermediate as u64)
			.saturating_mul(modifier as u64)
			.saturating_div((Self::INTERNAL_SCALING / 100) as u64) as u32;

		let positioned = (with_modifier as u64)
			.saturating_mul(relative_position as u64)
			.saturating_div((Self::INTERNAL_SCALING / 100) as u64) as u32;

		// Scale to 16-bit range with minimum value of 1
		let weight = (positioned as u64)
			.saturating_mul(Self::MAX_SCORE as u64)
			.saturating_div(Self::INTERNAL_SCALING as u64)
			.max(1)
			.min(Self::MAX_SCORE as u64) as u32;

		weight
	}

	fn calculate_availability_score(metrics: &NodeMetricsData) -> u32 {
		if metrics.total_pin_checks == 0 || metrics.total_pin_checks < 10 {
			return 0;
		}

		let base_score = (metrics.successful_pin_checks as u64)
			.saturating_mul(Self::INTERNAL_SCALING as u64)
			.saturating_div(metrics.total_pin_checks as u64) as u32;

		// Apply minimum checks requirement
		let check_frequency_score = if metrics.total_pin_checks < 100 {
			base_score.saturating_mul(8).saturating_div(10) // 20% penalty
		} else {
			base_score
		};

		// Add long-term reliability bonus
		let final_score = if metrics.consecutive_reliable_days > 30 {
			check_frequency_score.saturating_mul(12).saturating_div(10) // 20% bonus
		} else {
			check_frequency_score
		};

		final_score
	}

	fn calculate_performance_score(metrics: &NodeMetricsData) -> u32 {
		// Response time component
		let response_score = Self::INTERNAL_SCALING
			.saturating_mul(100)
			.saturating_div(metrics.avg_response_time_ms.max(1).min(Self::MAX_RESPONSE_TIME_MS));

		// Bandwidth scoring with uplink data consideration
		let bandwidth_score = if let Some(network_interface) = &metrics.primary_network_interface {
			// Convert uplink_mb to Mbps equivalent
			let uplink_mbps = (network_interface.uplink_mb * 8 / (1024 * 1024)) as u64; // Convert MB to Mbps and cast to u64
			(uplink_mbps.min(10000))
				.saturating_mul(Self::INTERNAL_SCALING.into()) // Convert INTERNAL_SCALING to u64
				.saturating_div(10000)
		} else {
			// Fallback to existing bandwidth_mbps if no primary network interface
			(metrics.bandwidth_mbps.min(10000) as u64)
				.saturating_mul(Self::INTERNAL_SCALING.into()) // Convert INTERNAL_SCALING to u64
				.saturating_div(10000)
		} as u32; // Cast back to u32 for final result

		// Storage proof scoring
		let storage_proof_score = Self::INTERNAL_SCALING
			.saturating_mul(100)
			.saturating_div(metrics.storage_proof_time_ms.max(1).min(1000));

		// Weighted combination
		let final_score = response_score
			.saturating_mul(40)
			.saturating_add(bandwidth_score.saturating_mul(40))
			.saturating_add(storage_proof_score.saturating_mul(20))
			.saturating_div(100);

		final_score
	}

	fn calculate_reliability_score(metrics: &NodeMetricsData) -> u32 {
		// Uptime scoring
		let uptime_score = if metrics.total_minutes > 0 {
			let base_score = (metrics.uptime_minutes as u64)
				.saturating_mul(Self::INTERNAL_SCALING as u64)
				.saturating_div(metrics.total_minutes as u64) as u32;

			if (metrics.uptime_minutes * 100 / metrics.total_minutes) < Self::MIN_UPTIME_PERCENTAGE
			{
				base_score.saturating_mul(7).saturating_div(10) // 30% penalty
			} else {
				base_score
			}
		} else {
			0
		};

		// Challenge success scoring
		let challenge_score = if metrics.total_challenges > 0 {
			let base_score = (metrics.successful_challenges as u64)
				.saturating_mul(Self::INTERNAL_SCALING as u64)
				.saturating_div(metrics.total_challenges as u64) as u32;

			if metrics.failed_challenges_count > Self::MAX_CONSECUTIVE_FAILURES {
				base_score.saturating_mul(6).saturating_div(10) // 40% penalty
			} else {
				base_score
			}
		} else {
			0
		};

		// Stability scoring
		let stability_score =
			if metrics.recent_downtime_hours == 0 && metrics.consecutive_reliable_days > 7 {
				Self::INTERNAL_SCALING / 5 // 20% bonus
			} else {
				0
			};

		let final_score = uptime_score
			.saturating_mul(50)
			.saturating_add(challenge_score.saturating_mul(30))
			.saturating_add(stability_score.saturating_mul(20))
			.saturating_div(100);

		final_score
	}

	// New method to calculate storage usage score
	fn calculate_storage_usage_score<T: ipfs_pallet::Config>(metrics: &NodeMetricsData) -> u64 {
		// Add the condition to check for ipfs_storage_max against ipfs_zfs_pool_size
		if metrics.ipfs_storage_max as u128 > metrics.ipfs_zfs_pool_size {
			return 0; // Set final score to 0
		}

		// Use the new method to get total file size for the miner
		let total_storage_usage =
			ipfs_pallet::Pallet::<T>::get_total_file_size_by_miner(metrics.miner_id.clone());

		// Define weights for storage scoring
		const TOTAL_STORAGE_WEIGHT: u64 = 50; // 50% weight to total storage
		const PINNED_FILES_WEIGHT: u64 = 30; // 30% weight to number of pinned files
		const FILE_COUNT_WEIGHT: u64 = 20; // 20% weight to total file count

		// Calculate storage score based on total storage usage
		// Normalize the storage usage score
		let storage_usage_score =
			(total_storage_usage as f64 / metrics.ipfs_storage_max as f64 * 100.0) as u64;

		// You might want to add additional logic for pinned files and file count
		// This is a placeholder and might need adjustment based on your specific requirements
		let pinned_files_score: u64 = 0; // Replace with actual pinned files calculation
		let file_count_score: u64 = 0; // Replace with actual file count calculation

		// Weighted score calculation
		let normalized_score = (storage_usage_score.saturating_mul(TOTAL_STORAGE_WEIGHT)
			+ pinned_files_score.saturating_mul(PINNED_FILES_WEIGHT)
			+ file_count_score.saturating_mul(FILE_COUNT_WEIGHT))
			/ (TOTAL_STORAGE_WEIGHT + PINNED_FILES_WEIGHT + FILE_COUNT_WEIGHT);

		normalized_score
	}

	fn calculate_capacity_score<T: ipfs_pallet::Config>(metrics: &NodeMetricsData) -> u32 {
		// Add the condition to check for ipfs_storage_max against ipfs_zfs_pool_size
		if metrics.ipfs_storage_max as u128 > metrics.ipfs_zfs_pool_size {
			return 0; // Set final score to 0
		}

		// Minimum storage requirement
		if metrics.ipfs_storage_max < (Self::MIN_STORAGE_GB as u64 * 1024 * 1024 * 1024) {
			return 0;
		}

		// Use the new method to get total file size for the miner
		let total_used_storage =
			ipfs_pallet::Pallet::<T>::get_total_file_size_by_miner(metrics.miner_id.clone());

		// Usage scoring
		let usage_score = if metrics.ipfs_storage_max > 0 {
			let usage_percent = (total_used_storage * 100) / metrics.ipfs_storage_max as u128;
			let base_score = (total_used_storage as u32)
				.saturating_mul(Self::INTERNAL_SCALING)
				.saturating_div(metrics.ipfs_storage_max as u32);

			if usage_percent >= Self::OPTIMAL_STORAGE_USAGE_MIN.into()
				&& usage_percent <= Self::OPTIMAL_STORAGE_USAGE_MAX.into()
			{
				base_score.saturating_add(Self::INTERNAL_SCALING / 5) // 20% bonus
			} else {
				base_score
			}
		} else {
			0
		};

		// Growth rate scoring
		let growth_score = (metrics.storage_growth_rate as u32)
			.saturating_div(1024 * 1024 * 1024) // Convert to GB
			.min(100)
			.saturating_mul(Self::INTERNAL_SCALING)
			.saturating_div(100);

		// Free space scoring
		let free_space_score = if metrics.ipfs_storage_max > 0 {
			let free_percent = ((metrics.ipfs_storage_max - total_used_storage as u64) * 100)
				/ metrics.ipfs_storage_max;

			if free_percent < 10 {
				0 // Critical low space
			} else if free_percent > 50 {
				Self::INTERNAL_SCALING / 2 // Underutilization
			} else {
				Self::INTERNAL_SCALING
			}
		} else {
			0
		};

		let final_score = usage_score
			.saturating_mul(40)
			.saturating_add(growth_score.saturating_mul(30))
			.saturating_add(free_space_score.saturating_mul(30))
			.saturating_div(100);

		final_score
	}

	fn calculate_network_score(metrics: &NodeMetricsData) -> u32 {
		// Peer scoring with progressive rewards
		let peer_score = if metrics.peer_count < Self::MIN_PEER_COUNT {
			(metrics.peer_count as u32)
				.saturating_mul(Self::INTERNAL_SCALING)
				.saturating_div(Self::MIN_PEER_COUNT)
				.saturating_mul(5)
				.saturating_div(10) // 50% penalty
		} else {
			let base_score = Self::INTERNAL_SCALING;
			let bonus = (metrics.peer_count.saturating_sub(Self::MIN_PEER_COUNT) as u32)
				.saturating_mul(Self::INTERNAL_SCALING)
				.saturating_div(1000) // 0.1% bonus per extra peer
				.min(Self::INTERNAL_SCALING / 2);

			base_score.saturating_add(bonus)
		};

		// Latency scoring
		let latency_score = Self::INTERNAL_SCALING
			.saturating_mul(100)
			.saturating_div(metrics.latency_ms.max(1).min(1000));

		// Connection stability
		let stability_score = if metrics.consecutive_reliable_days > 30
			&& metrics.peer_count >= Self::MIN_PEER_COUNT
		{
			Self::INTERNAL_SCALING / 10
		} else {
			0
		};

		let final_score = peer_score
			.saturating_mul(35)
			.saturating_add(latency_score.saturating_mul(35))
			.saturating_add(stability_score.saturating_mul(30))
			.saturating_div(100);

		final_score
	}

	fn calculate_diversity_score(
		metrics: &NodeMetricsData,
		geo_distribution: &BTreeMap<Vec<u8>, u32>,
	) -> u32 {
		if geo_distribution.is_empty() {
			return Self::INTERNAL_SCALING / 2;
		}

		let total_nodes = geo_distribution.values().sum::<u32>();
		let location_count = geo_distribution.get(&metrics.geolocation).unwrap_or(&0);

		if total_nodes == 0 {
			return Self::INTERNAL_SCALING / 2;
		}

		// Distribution quality
		let distribution_score =
			if geo_distribution.len() >= Self::GEOGRAPHIC_DIVERSITY_TARGET as usize {
				Self::INTERNAL_SCALING
			} else {
				(Self::INTERNAL_SCALING * geo_distribution.len() as u32)
					/ Self::GEOGRAPHIC_DIVERSITY_TARGET
			};

		// Location uniqueness
		let uniqueness_score = Self::INTERNAL_SCALING.saturating_sub(
			location_count
				.saturating_mul(Self::INTERNAL_SCALING)
				.saturating_div(total_nodes),
		);

		// Regional balance
		let balance_score = if *location_count > total_nodes / 3 {
			Self::INTERNAL_SCALING / 2
		} else {
			Self::INTERNAL_SCALING
		};

		let final_score = distribution_score
			.saturating_mul(40)
			.saturating_add(uniqueness_score.saturating_mul(40))
			.saturating_add(balance_score.saturating_mul(20))
			.saturating_div(100);

		final_score
	}

	fn calculate_bonuses(metrics: &NodeMetricsData) -> u32 {
		// Longevity bonus - reward consistent uptime
		let uptime_bonus = (metrics.consecutive_reliable_days as u32)
			.saturating_mul(1000) // 0.1% per day
			.min(200_000); // Cap at 20%

		// Challenge success streak bonus
		let challenge_bonus =
			if metrics.successful_challenges > 1000 && metrics.failed_challenges_count == 0 {
				50_000 // 5% bonus for perfect long-term performance
			} else {
				0
			};

		// Storage growth bonus
		let growth_bonus = if metrics.storage_growth_rate > 0
			&& metrics.current_storage_bytes > metrics.total_storage_bytes / 2
		{
			50_000 // 5% bonus for healthy growth
		} else {
			0
		};

		let total_bonus = uptime_bonus.saturating_add(challenge_bonus).saturating_add(growth_bonus);

		total_bonus
	}

	fn calculate_penalties(metrics: &NodeMetricsData) -> u32 {
		// Downtime penalties: Unchanged, reliability matters
		let downtime_penalty = (metrics.recent_downtime_hours as u32)
			.saturating_mul(10_000) // 1% per hour
			.min(500_000); // Max 50%

		// Failed challenges penalty: Unchanged, reliability matters
		let challenge_penalty = (metrics.failed_challenges_count as u32)
			.saturating_mul(5_000) // 0.5% per failure
			.min(300_000); // Max 30%

		// Storage utilization penalty: Adjusted to support empty disks
		let storage_penalty = if metrics.total_storage_bytes > 0 {
			let usage_percent = (metrics.current_storage_bytes * 100) / metrics.total_storage_bytes;
			if usage_percent < 5 {
				50_000 // 5% penalty only for extremely low usage (<5%)
			} else if usage_percent > 90 {
				50_000 // 5% penalty for overuse (>90%)
			} else {
				0 // No penalty between 5% and 90%, including empty disks
			}
		} else {
			500_000 // Unchanged, but irrelevant due to earlier checks
		};

		let total_penalty = downtime_penalty
			.saturating_add(challenge_penalty)
			.saturating_add(storage_penalty)
			.min(800_000); // Maximum 80% total penalty

		total_penalty
	}

	fn _get_network_scaling_factor(all_metrics: &[NodeMetricsData]) -> u32 {
		if all_metrics.is_empty() {
			return Self::INTERNAL_SCALING;
		}

		let mut total_uptime = 0u64;
		let mut total_success = 0u64;

		for m in all_metrics {
			if m.total_minutes > 0 {
				total_uptime += (m.uptime_minutes as u64)
					.saturating_mul(Self::INTERNAL_SCALING as u64)
					.saturating_div(m.total_minutes as u64);
			}
			if m.total_challenges > 0 {
				total_success += (m.successful_challenges as u64)
					.saturating_mul(Self::INTERNAL_SCALING as u64)
					.saturating_div(m.total_challenges as u64);
			}
		}

		let avg_uptime = (total_uptime / all_metrics.len() as u64) as u32;
		let avg_success = (total_success / all_metrics.len() as u64) as u32;

		let score = avg_uptime
			.saturating_mul(60)
			.saturating_add(avg_success.saturating_mul(40))
			.saturating_div(100);

		score
	}

	fn _calculate_relative_position(
		metrics: &NodeMetricsData,
		all_metrics: &[NodeMetricsData],
	) -> u32 {
		if all_metrics.is_empty() {
			return Self::INTERNAL_SCALING;
		}

		let node_score = Self::_get_composite_score(metrics);
		let mut better_than = 0;
		let mut total_valid = 0;
		let mut closest_better_score = Self::INTERNAL_SCALING;

		for other in all_metrics {
			let other_score = Self::_get_composite_score(other);
			if other_score > 0 {
				total_valid += 1;
				if node_score > other_score {
					better_than += 1;
				} else if other_score > node_score {
					closest_better_score = closest_better_score.min(other_score - node_score);
				}
			}
		}

		if total_valid == 0 {
			return Self::INTERNAL_SCALING / 2;
		}

		// Calculate position score
		let position_base = (Self::INTERNAL_SCALING / 4) // 25% minimum
			.saturating_add(
				(better_than as u32)
					.saturating_mul(Self::INTERNAL_SCALING * 3 / 4)
					.saturating_div(total_valid as u32),
			);

		// Apply competitiveness modifier
		let competitive_mod = if closest_better_score < Self::INTERNAL_SCALING / 100 {
			Self::INTERNAL_SCALING + (Self::INTERNAL_SCALING / 10) // 10% bonus for close competition
		} else {
			Self::INTERNAL_SCALING
		};

		let final_position = position_base
			.saturating_mul(competitive_mod)
			.saturating_div(Self::INTERNAL_SCALING);

		final_position
	}

	fn _get_composite_score(metrics: &NodeMetricsData) -> u32 {
		// Calculate individual components
		let uptime_score = if metrics.total_minutes > 0 {
			(metrics.uptime_minutes as u64)
				.saturating_mul(Self::INTERNAL_SCALING as u64)
				.saturating_div(metrics.total_minutes as u64) as u32
		} else {
			0
		};

		let challenge_score = if metrics.total_challenges > 0 {
			(metrics.successful_challenges as u64)
				.saturating_mul(Self::INTERNAL_SCALING as u64)
				.saturating_div(metrics.total_challenges as u64) as u32
		} else {
			0
		};

		let response_score =
			Self::INTERNAL_SCALING.saturating_div(metrics.avg_response_time_ms.max(1).min(1000));

		let storage_score = if metrics.total_storage_bytes > 0 {
			(metrics.current_storage_bytes as u32)
				.saturating_mul(Self::INTERNAL_SCALING)
				.saturating_div(metrics.total_storage_bytes as u32)
		} else {
			0
		};

		// Combine scores with weights
		let weighted_score = uptime_score
			.saturating_mul(30)
			.saturating_add(challenge_score.saturating_mul(30))
			.saturating_add(response_score.saturating_mul(20))
			.saturating_add(storage_score.saturating_mul(20))
			.saturating_div(100);

		// Apply penalties
		let penalty_multiplier = Self::INTERNAL_SCALING.saturating_sub(
			metrics
				.failed_challenges_count
				.saturating_mul(50_000) // 5% per failure
				.min(500_000), // Max 50% penalty
		);

		let final_score = weighted_score
			.saturating_mul(penalty_multiplier)
			.saturating_div(Self::INTERNAL_SCALING);

		final_score
	}
}

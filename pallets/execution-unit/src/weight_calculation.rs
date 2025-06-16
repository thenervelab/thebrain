
pub use crate::types::NodeMetricsData;
use pallet_registration::NodeType;
use sp_std::{collections::btree_map::BTreeMap, vec::Vec};
use scale_info::prelude::string::String;
use frame_support::BoundedVec;
use sp_runtime::traits::ConstU32;

impl NodeMetricsData {
    // Configuration constants
    const MIN_STORAGE_GB: u32 = 2048; // Minimum 2TB storage
    const MAX_SCORE: u32 = 65535; // 16-bit maximum
    const INTERNAL_SCALING: u32 = 1_000_000;
    const MIN_PIN_CHECKS: u32 = 1; // Minimum pin checks for valid scoring
    const SLASH_THRESHOLD: u32 = 1; // Number of failed storage proofs before slashing
    const REPUTATION_NEUTRAL: u32 = 1000; // Neutral reputation points
    const REPUTATION_BOOST_NEW: u32 = 1100; // Initial boost for new coldkeys
    const MAX_FILE_SIZE: u64 = 1024 * 1024 * 1024 ; // 1GB as max file size for scoring

    fn calculate_storage_proof_score<T: ipfs_pallet::Config>(
        metrics: &NodeMetricsData,
        total_pin_checks: u32,
        total_successful_pin_checks: u32,
        miner_id : Vec<u8>
    ) -> u64 {
        if total_pin_checks == 0 || metrics.total_pin_checks < Self::MIN_PIN_CHECKS {
            return 0; // Avoid division by zero or insufficient pin checks
        }

        // Convert Vec<u8> to BoundedVec
        let bounded_miner_id = match BoundedVec::<u8, ConstU32<64>>::try_from(miner_id) {
            Ok(id) => id,
            Err(_) => return 0, // if conversion fails
        };

        if !ipfs_pallet::Pallet::<T>::has_miner_profile(&bounded_miner_id) {
            return 0;
        }

        // Check if miner has a profile
        if ipfs_pallet::Pallet::<T>::miner_profile(bounded_miner_id).is_empty() {
            return 0;
        }

        // Pin check success rate (70% weight)
        let pin_success_score = (total_successful_pin_checks as u64)
            .saturating_mul(Self::INTERNAL_SCALING as u64)
            .saturating_div(total_pin_checks as u64);
    
        // Storage usage score (30% weight)
        let storage_usage_score = if metrics.ipfs_storage_max > 0 {
            let usage_percent = (metrics.ipfs_repo_size * 100) / metrics.ipfs_storage_max;
            if usage_percent >= 5 && usage_percent <= 90 {
                (metrics.current_storage_bytes as u64)
                    .saturating_mul(Self::INTERNAL_SCALING as u64)
                    .saturating_div(metrics.ipfs_storage_max as u64)
            } else {
                0 // Penalize extreme usage
            }
        } else {
            0
        };
    
        // Weighted combination
        (pin_success_score.saturating_mul(30) + storage_usage_score.saturating_mul(70))
            .saturating_div(100)
    }

    fn calculate_ping_score(
        total_ping_checks: u32,
        total_successful_ping_checks: u32,
    ) -> u64 {
        if total_ping_checks == 0 {
            return 0;
        }

        (total_successful_ping_checks as u64)
            .saturating_mul(Self::INTERNAL_SCALING as u64)
            .saturating_div(total_ping_checks as u64)
    }

    fn calculate_overall_pin_score<T: ipfs_pallet::Config>(
        total_overall_pin_checks: u32,
        total_overall_successful_pin_checks: u32,
        miner_id : Vec<u8>
    ) -> u64 {
        if total_overall_pin_checks == 0 {
            return 0;
        }

        // Convert Vec<u8> to BoundedVec
        let bounded_miner_id = match BoundedVec::<u8, ConstU32<64>>::try_from(miner_id) {
            Ok(id) => id,
            Err(_) => return 0, // if conversion fails
        };

        if !ipfs_pallet::Pallet::<T>::has_miner_profile(&bounded_miner_id) {
            return 0;
        }

        // Check if miner has a profile
        if ipfs_pallet::Pallet::<T>::miner_profile(bounded_miner_id).is_empty() {
            return 0;
        }

        
        (total_overall_successful_pin_checks as u64)
            .saturating_mul(Self::INTERNAL_SCALING as u64)
            .saturating_div(total_overall_pin_checks as u64)
    }
    
    fn calculate_reputation_bonus(reputation_points: u32) -> u64 {
        match reputation_points {
            0 => 0,             // No bonus
            1..=499 => 20,       // Small bonus
            500..=999 => 50,     // Medium bonus
            1000..=1499 => 100,  // Good bonus
            1500..=1999 => 150,  // Better bonus
            _ => 200,            // Max bonus
        }
    }


    pub fn calculate_weight<T: pallet_marketplace::Config + ipfs_pallet::Config + pallet_registration::Config + crate::Config + pallet_rankings::Config>(
        _node_type: NodeType,
        metrics: &NodeMetricsData,
        all_nodes_metrics: &[NodeMetricsData],
        geo_distribution: &BTreeMap<Vec<u8>, u32>,
        coldkey: &T::AccountId,
    ) -> u32 {
        if _node_type != NodeType::StorageMiner {
            return 0; // Only handle storage miners
        }

        let linked_nodes = pallet_registration::Pallet::<T>::linked_nodes(metrics.miner_id.clone());
        // calculate total pin checks per epoch
        let (mut total_pin_checks, mut successful_pin_checks) = linked_nodes.clone().into_iter().fold(
            (0u32, 0u32),
            |(total, successful), node_id| {
                let total_epoch = crate::Pallet::<T>::total_pin_checks_per_epoch(&node_id);
                let success_epoch = crate::Pallet::<T>::successful_pin_checks_per_epoch(&node_id);
        
                (
                    total.saturating_add(total_epoch),
                    successful.saturating_add(success_epoch),
                )
            },
        );
        // calculate total ping checks per epoch
        let (mut total_ping_checks, mut successful_ping_checks) = linked_nodes.clone().into_iter().fold(
            (0u32, 0u32),
            |(total, successful), node_id| {
                let total_epoch = crate::Pallet::<T>::total_ping_checks_per_epoch(&node_id);
                let success_epoch = crate::Pallet::<T>::successful_ping_checks_per_epoch(&node_id);
        
                (
                    total.saturating_add(total_epoch),
                    successful.saturating_add(success_epoch),
                )
            },
        );

        // calculate overall total pin checks per epoch
        let (mut total_overall_pin_score, mut total_successfull_overall_pin_score) = linked_nodes.into_iter().fold(
            (0u32, 0u32),
            |(total, successful), node_id| {
                let linked_node_metrics = crate::Pallet::<T>::get_node_metrics(node_id);
                if linked_node_metrics.is_some() {
                    let metrics_data = linked_node_metrics.unwrap();
                    (
                        total.saturating_add(metrics_data.total_pin_checks),
                        successful.saturating_add(metrics_data.successful_pin_checks),
                    )
                } else {
                    // Return the accumulator unchanged if metrics are None
                    (total, successful)
                }
            },
        );
        
        total_pin_checks = total_pin_checks + crate::Pallet::<T>::total_pin_checks_per_epoch(&metrics.miner_id);
        successful_pin_checks = successful_pin_checks + crate::Pallet::<T>::successful_pin_checks_per_epoch(&metrics.miner_id);
        total_ping_checks = total_ping_checks + crate::Pallet::<T>::total_ping_checks_per_epoch(&metrics.miner_id);
        successful_ping_checks = successful_ping_checks + crate::Pallet::<T>::successful_ping_checks_per_epoch(&metrics.miner_id);
        total_overall_pin_score = total_overall_pin_score + metrics.total_pin_checks;
        total_successfull_overall_pin_score = total_successfull_overall_pin_score + metrics.successful_pin_checks;

        log::info!("Total pin checks across linked nodes: {}", total_pin_checks);
        log::info!("Successful pin checks across linked nodes: {}", successful_pin_checks);

        // // Early return for invalid metrics
        // if metrics.ipfs_storage_max < (Self::MIN_STORAGE_GB as u64 * 1024 * 1024 * 1024)
        //     || metrics.bandwidth_mbps < 125
        //     || metrics.primary_network_interface.is_none()
        //     || metrics.disks.is_empty()
        // {
        //     return 0;
        // }

        // Calculate storage proof score (main component)
        let storage_proof_score = Self::calculate_storage_proof_score::<T>(
            metrics, 
            total_pin_checks, 
            successful_pin_checks,
            metrics.miner_id.clone(),
        ).saturating_div(100);
        log::info!("storage_proof_score: {}", storage_proof_score);

        // Calculate ping score separately
        let ping_score = Self::calculate_ping_score(
            total_ping_checks,
            successful_ping_checks,
        ).saturating_div(100);
        log::info!("ping_score: {}", ping_score);

        let overall_pin_score = Self::calculate_overall_pin_score::<T>(
            total_overall_pin_score,
            total_successfull_overall_pin_score,
            metrics.miner_id.clone(),
        ).saturating_div(100);

        // Calculate file size score
        let miner_id_bounded: BoundedVec<u8, ConstU32<64>> = metrics.miner_id.clone()
        .try_into()
        .unwrap_or_default();
        let file_size = ipfs_pallet::Pallet::<T>::miner_files_size(miner_id_bounded)
            .unwrap_or(0);
        let file_size_score = Self::calculate_file_size_score(file_size).saturating_div(100);
        log::info!("file_size_score: {}", file_size_score);        
     
        // Get reputation points and calculate modifier
        let reputation_points = ipfs_pallet::Pallet::<T>::reputation_points(coldkey);
        let reputation_modifier = Self::calculate_reputation_bonus(reputation_points);

     
        // Calculate diversity score (unchanged)
        let _diversity_score =
            (Self::calculate_diversity_score(metrics, geo_distribution) as u64).saturating_div(100);
     
        // New base weight calculation: 60% storage proof, 10% ping score, overall_pin_score 5% 
        let base_weight = (storage_proof_score.saturating_mul(60) + ping_score.saturating_mul(10) + 
                           overall_pin_score.saturating_mul(5) + file_size_score.saturating_mul(25))
        .saturating_div(100);

        // Apply reputation modifier
        let final_weight = (base_weight)
            .saturating_add(reputation_modifier) // modifier is now a u32 bonus
            .max(1)
            .min(Self::MAX_SCORE as u64);


        let previous_rankings = pallet_rankings::Pallet::<T>::get_node_ranking(metrics.miner_id.clone());
        // Blend with previous weight using integer arithmetic (30% new, 70% old) if previous ranking exists
        let updated_weight = match previous_rankings {
            Some(rankings) => {
                // Ensure at least weight of 1 after blending
                (((1 * final_weight as u32) + (9 * rankings.weight as u32)) / 10).max(1)
            }
            None => final_weight as u32,
        };        
        log::info!("updated_weight: {}, metrics miner id: {}", updated_weight, String::from_utf8_lossy(&metrics.miner_id));

        updated_weight
    }

    // Unchanged functions (included for completeness)
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

        let distribution_score =
            if geo_distribution.len() >= 5 {
                Self::INTERNAL_SCALING
            } else {
                (Self::INTERNAL_SCALING * geo_distribution.len() as u32) / 5
            };

        let uniqueness_score = Self::INTERNAL_SCALING.saturating_sub(
            location_count
                .saturating_mul(Self::INTERNAL_SCALING)
                .saturating_div(total_nodes),
        );

        let balance_score = if *location_count > total_nodes / 3 {
            Self::INTERNAL_SCALING / 2
        } else {
            Self::INTERNAL_SCALING
        };

        distribution_score
            .saturating_mul(40)
            .saturating_add(uniqueness_score.saturating_mul(40))
            .saturating_add(balance_score.saturating_mul(20))
            .saturating_div(100)
    }

    fn calculate_file_size_score(
        file_size: u128,
    ) -> u64 {
        if file_size == 0 {
            return 0;
        }

        // Normalize file size against MAX_FILE_SIZE
        let file_size_score = (file_size as u64)
            .saturating_mul(Self::INTERNAL_SCALING as u64)
            .saturating_div(Self::MAX_FILE_SIZE);
        
        // Cap the score to prevent over-weighting
        file_size_score.min(Self::INTERNAL_SCALING as u64)
    }

}
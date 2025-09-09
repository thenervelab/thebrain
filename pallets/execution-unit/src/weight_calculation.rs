pub use crate::types::NodeMetricsData;
use pallet_registration::NodeType;
use sp_std::{collections::btree_map::BTreeMap, vec::Vec};
use frame_support::BoundedVec;
use sp_runtime::traits::ConstU32;

impl NodeMetricsData {
    // Configuration constants
    const MIN_STORAGE_GB: u32 = 2048; // Minimum 2TB storage
    const MAX_SCORE: u32 = 65535; // 16-bit maximum
    const INTERNAL_SCALING: u32 = 1_000_000;
    const MIN_PIN_CHECKS: u32 = 1; // Minimum pin checks for valid scoring

    fn calculate_storage_proof_score<T: ipfs_pallet::Config>(
        metrics: &NodeMetricsData,
        total_pin_checks: u32,
        total_successful_pin_checks: u32,
        miner_id: Vec<u8>
    ) -> u64 {
        if total_pin_checks == 0 || metrics.total_pin_checks < Self::MIN_PIN_CHECKS {
            return 0; // Avoid division by zero or insufficient pin checks
        }

        // Convert Vec<u8> to BoundedVec
        let bounded_miner_id = match BoundedVec::<u8, ConstU32<64>>::try_from(miner_id) {
            Ok(id) => id,
            Err(_) => return 0, // if conversion fails
        };

        let main_has_profile = ipfs_pallet::Pallet::<T>::has_miner_profile(&bounded_miner_id)
            && !ipfs_pallet::Pallet::<T>::miner_profile(bounded_miner_id.clone()).is_empty();

        let linked_nodes = pallet_registration::Pallet::<T>::linked_nodes(bounded_miner_id.clone());
        let any_linked_has_profile = linked_nodes.iter().any(|node_id| {
            if let Ok(bounded_node_id) = BoundedVec::<u8, ConstU32<64>>::try_from(node_id.clone()) {
                ipfs_pallet::Pallet::<T>::has_miner_profile(&bounded_node_id)
                    && !ipfs_pallet::Pallet::<T>::miner_profile(bounded_node_id).is_empty()
            } else {
                false
            }
        });

        if !main_has_profile && !any_linked_has_profile {
            return 0;
        }

        // Pin success rate (scaled to 0-700, 70% weight)
        let pin_success_rate = if total_pin_checks > 0 {
            (total_successful_pin_checks as u64 * 700) / total_pin_checks as u64
        } else {
            0
        };

        // Storage efficiency (scaled to 0-300, 30% weight)
        let storage_efficiency = if metrics.ipfs_storage_max > 0 {
            let usage_percent = (metrics.ipfs_repo_size * 100) / metrics.ipfs_storage_max;
            if usage_percent >= 5 && usage_percent <= 90 {
                let used_percent = (metrics.current_storage_bytes as u64 * 100) / metrics.ipfs_storage_max as u64;
                (300 * (100 - used_percent.min(100))) / 100 // Inverse usage, max at 300 when near 0% used
            } else {
                0 // Penalize extreme usage
            }
        } else {
            0
        };

        // Combine with 70/30 weight, cap at 1000, ensure min 1
        let total_score = pin_success_rate + storage_efficiency;
        if total_score > 1000 {
            1000
        } else if total_score == 0 && (pin_success_rate > 0 || storage_efficiency > 0) {
            1
        } else {
            total_score
        }
    }

    fn calculate_ping_score(
        total_ping_checks: u32,
        total_successful_ping_checks: u32,
    ) -> u64 {
        if total_ping_checks == 0 {
            return 0;
        }

        // Scale to 1-1000 range directly
        let raw_score = (total_successful_ping_checks as u64 * 1000) / total_ping_checks as u64;
        if raw_score > 1000 {
            1000
        } else if raw_score == 0 && total_successful_ping_checks > 0 {
            1
        } else {
            raw_score
        }
    }

    fn calculate_overall_pin_score<T: ipfs_pallet::Config>(
        total_overall_pin_checks: u32,
        total_overall_successful_pin_checks: u32,
        miner_id: Vec<u8>
    ) -> u64 {
        if total_overall_pin_checks == 0 {
            return 0;
        }

        // Convert Vec<u8> to BoundedVec
        let bounded_miner_id = match BoundedVec::<u8, ConstU32<64>>::try_from(miner_id) {
            Ok(id) => id,
            Err(_) => return 0, // if conversion fails
        };

        let main_has_profile = ipfs_pallet::Pallet::<T>::has_miner_profile(&bounded_miner_id)
            && !ipfs_pallet::Pallet::<T>::miner_profile(bounded_miner_id.clone()).is_empty();

        let linked_nodes = pallet_registration::Pallet::<T>::linked_nodes(bounded_miner_id.clone());
        let any_linked_has_profile = linked_nodes.iter().any(|node_id| {
            if let Ok(bounded_node_id) = BoundedVec::<u8, ConstU32<64>>::try_from(node_id.clone()) {
                ipfs_pallet::Pallet::<T>::has_miner_profile(&bounded_node_id)
                    && !ipfs_pallet::Pallet::<T>::miner_profile(bounded_node_id).is_empty()
            } else {
                false
            }
        });

        if !main_has_profile && !any_linked_has_profile {
            return 0;
        }

        // Scale to 1-1000 range directly
        let raw_score = (total_overall_successful_pin_checks as u64 * 1000) / total_overall_pin_checks as u64;
        if raw_score > 1000 {
            1000
        } else if raw_score == 0 && total_overall_successful_pin_checks > 0 {
            1
        } else {
            raw_score
        }
    }

    fn calculate_reputation_bonus(reputation_points: u32) -> u64 {
        match reputation_points {
            0 => 0,              // No bonus
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
        let (mut total_overall_pin_score, mut total_successfull_overall_pin_score) = linked_nodes.clone().into_iter().fold(
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


        let overall_pin_score = Self::calculate_overall_pin_score::<T>(
            total_overall_pin_score,
            total_successfull_overall_pin_score,
            metrics.miner_id.clone(),
        );

        // Calculate total file size across miner and linked nodes
        let total_file_size = {
            // Start with the main miner's file size
            let miner_id_bounded: BoundedVec<u8, ConstU32<64>> = metrics.miner_id.clone()
                .try_into()
                .unwrap_or_default();
            let mut total = ipfs_pallet::Pallet::<T>::miner_files_size(miner_id_bounded)
                .unwrap_or(0);

            // Add file sizes from all linked nodes
            for node_id in linked_nodes.iter() {
                let node_id_bounded: BoundedVec<u8, ConstU32<64>> = node_id.clone()
                    .try_into()
                    .unwrap_or_default();
                total += ipfs_pallet::Pallet::<T>::miner_files_size(node_id_bounded)
                    .unwrap_or(0);
            }

            total
        };
        let file_size_score = Self::calculate_file_size_score::<T>(total_file_size);
        
        let base_weight = (
            overall_pin_score.saturating_mul(20) + file_size_score.saturating_mul(80))
        .saturating_div(100);
        
        let final_weight = (base_weight)
            .max(1)
            .min(Self::MAX_SCORE as u64);

        let previous_rankings = pallet_rankings::Pallet::<T>::get_node_ranking(metrics.miner_id.clone());

        // Blend with previous weight using integer arithmetic (30% new, 70% old) if previous ranking exists
        let updated_weight = match previous_rankings {
            Some(rankings) => {
                // Ensure at least weight of 1 after blending
                (((3 * final_weight as u32) + (7 * rankings.weight as u32)) / 10).max(1)
            }
            None => final_weight as u32,
        };

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


    fn calculate_file_size_score<T: ipfs_pallet::Config>(
        file_size_bytes: u128,
    ) -> u64 {
        const MIN_SCORE: u64 = 500;     // Baseline score for any storage
        const MAX_SCORE: u64 = 60000;   // Aiming for a high max score to allow final weights up to 15k-20k+
        const SCALE_FACTOR: u128 = 1_000_000; // 1,000,000 represents 100%

        if file_size_bytes == 0 {
            return MIN_SCORE; // Even with 0 bytes, a baseline is given.
        }

        let total_network_storage = ipfs_pallet::Pallet::<T>::get_total_network_storage();
        if total_network_storage == 0 {
            return MIN_SCORE;
        }

        // Calculate percentage of total network storage (0-1_000_000 for 0% to 100%)
        let percentage_scaled = (file_size_bytes * SCALE_FACTOR) / total_network_storage;

        let score: u64;

        // More granular piecewise function for smoother score curve
        if percentage_scaled <= 1_000 { // 0% to 0.1% of network (0 - 1,000 PPM)
            // From MIN_SCORE (500) to 1,000 (500 points increase over 0.1%)
            score = MIN_SCORE + (percentage_scaled as u64 * 500) / 1_000;
        } else if percentage_scaled <= 10_000 { // 0.1% to 1% of network (1,001 - 10,000 PPM)
            // From 1,000 to 5,000 (4,000 points increase over 0.9%)
            score = 1_000 + ((percentage_scaled - 1_000) as u64 * 4_000) / 9_000;
        } else if percentage_scaled <= 100_000 { // 1% to 10% of network (10,001 - 100,000 PPM)
            // From 5,000 to 15,000 (10,000 points increase over 9%)
            score = 5_000 + ((percentage_scaled - 10_000) as u64 * 10_000) / 90_000;
        } else if percentage_scaled <= 300_000 { // 10% to 30% of network (100,001 - 300,000 PPM)
            // From 15,000 to 25,000 (10,000 points increase over 20%)
            score = 15_000 + ((percentage_scaled - 100_000) as u64 * 10_000) / 200_000;
        } else if percentage_scaled <= 500_000 { // 30% to 50% of network (300,001 - 500,000 PPM)
            // From 25,000 to 35,000 (10,000 points increase over 20%)
            score = 25_000 + ((percentage_scaled - 300_000) as u64 * 10_000) / 200_000;
        } else if percentage_scaled <= 700_000 { // 50% to 70% of network (500,001 - 700,000 PPM)
            // From 35,000 to 45,000 (10,000 points increase over 20%)
            score = 35_000 + ((percentage_scaled - 500_000) as u64 * 10_000) / 200_000;
        } else if percentage_scaled <= 850_000 { // 70% to 85% of network (700,001 - 850,000 PPM)
            // From 45,000 to 52,500 (7,500 points increase over 15%)
            score = 45_000 + ((percentage_scaled - 700_000) as u64 * 7_500) / 150_000;
        } else { // 85% to 100% of network (850,001 - 1,000,000 PPM)
            // From 52,500 to MAX_SCORE (60,000) (7,500 points increase over 15%)
            score = 52_500 + ((percentage_scaled - 850_000) as u64 * 7_500) / 150_000;
        }
 
        // Ensure we stay within the defined MIN_SCORE and MAX_SCORE
        score.max(MIN_SCORE).min(MAX_SCORE)
    }


}
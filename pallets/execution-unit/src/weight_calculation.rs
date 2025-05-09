pub use crate::types::NodeMetricsData;
use pallet_registration::NodeType;
use sp_std::{collections::btree_map::BTreeMap, vec::Vec};
use scale_info::prelude::string::String;

impl NodeMetricsData {
    // Configuration constants
    const MIN_STORAGE_GB: u32 = 2048; // Minimum 2TB storage
    const MAX_SCORE: u32 = 65535; // 16-bit maximum
    const INTERNAL_SCALING: u32 = 1_000_000;
    const MIN_PIN_CHECKS: u32 = 5; // Minimum pin checks for valid scoring
    const SLASH_THRESHOLD: u32 = 3; // Number of failed storage proofs before slashing
    const REPUTATION_NEUTRAL: u32 = 1000; // Neutral reputation points
    const REPUTATION_BOOST_NEW: u32 = 1100; // Initial boost for new coldkeys

    fn calculate_storage_proof_score(
        metrics: &NodeMetricsData,
        total_pin_checks: u32,
        total_successful_pin_checks: u32
    ) -> u64 {
        if total_pin_checks == 0 || total_pin_checks < Self::MIN_PIN_CHECKS {
            return 1; // Avoid division by zero or insufficient pin checks
        }
    
        // Pin check success rate (70% weight)
        let pin_success_score = (total_successful_pin_checks as u64)
            .saturating_mul(Self::INTERNAL_SCALING as u64)
            .saturating_div(total_pin_checks as u64);
    
        // Storage usage score (30% weight)
        let storage_usage_score = if metrics.ipfs_storage_max > 0 {
            let usage_percent = (metrics.current_storage_bytes * 100) / metrics.total_storage_bytes;
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
        (pin_success_score.saturating_mul(70) + storage_usage_score.saturating_mul(30))
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

    fn calculate_overall_pin_score(
        total_overall_pin_checks: u32,
        total_overall_successful_pin_checks: u32,
    ) -> u64 {
        if total_overall_pin_checks == 0 {
            return 0;
        }

        (total_overall_successful_pin_checks as u64)
            .saturating_mul(Self::INTERNAL_SCALING as u64)
            .saturating_div(total_overall_pin_checks as u64)
    }
    
    // Calculate reputation modifier based on coldkey reputation points
    fn calculate_reputation_modifier(reputation_points: u32) -> u64 {
        // Map reputation points to a multiplier between 0.5 and 1.5
        let base = Self::INTERNAL_SCALING as u64;
        if reputation_points < 500 {
            base / 2 // 0.5x for low reputation
        } else if reputation_points < 1000 {
            base * 3 / 4 // 0.75x
        } else if reputation_points < 1500 {
            base // 1.0x
        } else if reputation_points < 2000 {
            base * 5 / 4 // 1.25x
        } else {
            base * 3 / 2 // 1.5x cap
        }
    }

    // Update reputation points (called externally by pallet logic)
    fn update_reputation_points<T: ipfs_pallet::Config>(
        metrics: &NodeMetricsData,
        coldkey: &T::AccountId,
        total_pin_checks: u32,
        total_successful_pin_checks: u32,
        
    ) -> u32 {
        let mut reputation_points = ipfs_pallet::Pallet::<T>::reputation_points(coldkey);

        // Increase points for successful storage proofs
        if total_pin_checks >= Self::MIN_PIN_CHECKS {
            let success_rate = (total_successful_pin_checks * 100) / total_pin_checks;
            if success_rate >= 95 {
                reputation_points = reputation_points.saturating_add(50); // +50 for high success
            } else if success_rate >= 80 {
                reputation_points = reputation_points.saturating_add(20); // +20 for good success
            }
        }

		// Slash points for failed storage proofs
		let failed_count = total_pin_checks - total_successful_pin_checks;
        if failed_count >= Self::SLASH_THRESHOLD {
            reputation_points = reputation_points.saturating_mul(9).saturating_div(10); // 10% slash
        }

        // Slash points for significant downtime
        if metrics.recent_downtime_hours > 24 {
            reputation_points = reputation_points.saturating_mul(8).saturating_div(10); // 20% slash
        }

        // Slash points for false capacity claims
        if metrics.ipfs_storage_max as u128 > metrics.ipfs_zfs_pool_size
            || (metrics.ipfs_storage_max > 0
                && metrics.current_storage_bytes < metrics.ipfs_storage_max / 20)
        {
            reputation_points = reputation_points.saturating_mul(5).saturating_div(10); // 50% slash
        }

        // Cap reputation points
        reputation_points = reputation_points.min(3000).max(100);

        // Store updated points
        ipfs_pallet::Pallet::<T>::set_reputation_points(coldkey, reputation_points);
        reputation_points
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
        let storage_proof_score = Self::calculate_storage_proof_score(
            metrics, 
            total_pin_checks, 
            successful_pin_checks,
        ).saturating_div(100);
        log::info!("storage_proof_score: {}", storage_proof_score);

        // Calculate ping score separately
        let ping_score = Self::calculate_ping_score(
            total_ping_checks,
            successful_ping_checks,
        ).saturating_div(100);
        log::info!("ping_score: {}", ping_score);

        let overall_pin_score = Self::calculate_overall_pin_score(
            total_overall_pin_score,
            total_successfull_overall_pin_score
        ).saturating_div(100);
     
        // Get reputation points and calculate modifier
        let reputation_points = Self::update_reputation_points::<T>(metrics, coldkey, total_pin_checks, successful_pin_checks);
        let reputation_modifier = Self::calculate_reputation_modifier(reputation_points);
     
        // Calculate diversity score (unchanged)
        let diversity_score =
            (Self::calculate_diversity_score(metrics, geo_distribution) as u64).saturating_div(100);
     
        // Base weight: storage proof (80%), diversity (20%)
        // let base_weight = (storage_proof_score.saturating_mul(80)
        //     + diversity_score.saturating_mul(20))
        //     .saturating_div(100);

        // New base weight calculation: 70% storage proof, 30% ping score
        // let base_weight = (storage_proof_score.saturating_mul(70) + ping_score.saturating_mul(20) + overall_pin_score.saturating_mul(10))
        // .saturating_div(100);
        let base_weight = storage_proof_score;

        // Apply reputation modifier
        let final_weight = (base_weight as u64)
            .saturating_mul(reputation_modifier)
            .saturating_div(Self::INTERNAL_SCALING as u64)
            .max(1)
            .min(Self::MAX_SCORE as u64) as u32;

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
}
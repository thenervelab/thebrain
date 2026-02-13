pub use crate::types::NodeMetricsData;
use core::convert::TryFrom;
use frame_support::BoundedVec;
use pallet_registration::NodeType;
use sp_arithmetic::{FixedPointNumber, FixedU128};
use sp_runtime::traits::ConstU32;
use sp_std::{
    collections::btree_set::BTreeSet,
    vec::Vec,
};
use sp_runtime::Saturating;
use num_traits::One;
use log::{info, warn};
use num_traits::CheckedDiv;
use num_traits::Zero;
use num_traits::float::FloatCore;

#[derive(Clone, Copy)]
struct PoolContext {
    available_pool: f64,
    miners_pool: f64,
    uid_zero_pool: f64,
    burn_percentage: f64,
}

impl NodeMetricsData {
    const MAX_SCORE: u32 = 65_535;
    const DISTRIBUTION_PERCENT: u32 = 100;
    const ONE_HUNDRED_PERCENT: u128 = 100;
    const EMISSION_PERIOD: u128 = 50; 
    const WEIGHT_SCALE: u128 = 100;
    const BYTES_PER_GB: u128 = 1_073_741_824;
    const TOKEN_DECIMALS: u32 = 18;

    fn pool_context(
        total_network_storage: u128,
        price_per_gb: u128,
        alpha_price: u128,
    ) -> Option<PoolContext> {
        info!(
            "pool_context - raw price_per_gb: {}, total_network_storage: {}, alpha_price: {}",
            price_per_gb, total_network_storage, alpha_price
        );
    
        if price_per_gb == 0 || (total_network_storage == 0 ) || alpha_price == 0 {
            warn!("price_per_gb, total_network_storage/bandwidth, or alpha_price is 0");
            return None;
        }

        // ---- 1) Convert storage and bandwidth to GB ----
        let storage_gb = total_network_storage as f64 / Self::BYTES_PER_GB as f64;
        let total_resource_gb = storage_gb;
        info!("storage_gb: {}, total_resource_gb: {}", storage_gb, total_resource_gb);
    
        // ---- 2) Convert prices to fractional tokens ----
        let price_scale = 10u128.pow(Self::TOKEN_DECIMALS);
        let price_per_gb_token = price_per_gb as f64 / price_scale as f64;
        let alpha_price_token = alpha_price as f64 / price_scale as f64;
        info!("price_per_gb_token: {}", price_per_gb_token);
        info!("alpha_price_token: {}", alpha_price_token);
    
        // ---- 3) Calculate COST and EMISSIONS ----
        let cost = total_resource_gb * price_per_gb_token * 250.0;
        info!("cost: {}", cost);
        let emissions = alpha_price_token * (Self::EMISSION_PERIOD as f64);
        info!("emissions: {}", emissions);
    
        // ---- 4) Calculate burn amount and percentage ----
        let burn_number = if emissions > cost { emissions - cost } else { 0.0 };
        info!("burn_number: {}", burn_number);
    
        let burn_percentage = if emissions != 0.0 { burn_number / emissions } else { 0.2 };
        info!("burn_percentage: {}", burn_percentage);
    
        // ---- 5) Calculate pool distribution ----
        let max_score = Self::MAX_SCORE as f64;
        let uid_zero_pool = max_score * burn_percentage;
        let miners_pool = max_score - uid_zero_pool;
        info!("uid_zero_pool: {}", uid_zero_pool);
        info!("miners_pool: {}", miners_pool);
    
        Some(PoolContext {
            available_pool: max_score,
            miners_pool,
            uid_zero_pool,
            burn_percentage,
        })
    }    

    pub fn calculate_weight<
        T: pallet_marketplace::Config
            + ipfs_pallet::Config
            + pallet_registration::Config
            + crate::Config
            + pallet_rankings::Config
            + pallet_credits::Config
            + pallet_arion::Config,
    >(
        node_type: NodeType,
        miner_id: &Vec<u8>,
    ) -> u32 {
        info!(
            "calculate_weight - node_type: {:?}, miner_id: {:?}",
            node_type,
            hex::encode(miner_id)
        );

        if node_type != NodeType::StorageMiner {
            return 0;
        }

        let price_per_gb = pallet_marketplace::Pallet::<T>::get_price_per_gb();
        let network_totals = pallet_arion::CurrentNetworkTotals::<T>::get();
        let total_network_storage = network_totals.total_shard_data_bytes;
        let alpha_price = pallet_credits::Pallet::<T>::alpha_price();

        let context = if let Some(ctx) = Self::pool_context(total_network_storage, price_per_gb, alpha_price) {
            ctx
        } else {
            warn!("Failed to get pool context, returning 0 weight");
            return 0;
        };

        let miners_pool_inner = context.miners_pool;
        if miners_pool_inner == 0.0 || total_network_storage == 0 {
            warn!("miners_pool is 0 or total_network_storage is 0, returning 0 weight");
            return 0;
        }

      // Get miner owner and their Arion weight
      let node_info = pallet_registration::Pallet::<T>::get_node_registration_info(miner_id.clone());
      if node_info.is_none() {
          warn!("Node registration info not found for miner");
          return 0;
      }
      let owner = node_info.unwrap().owner;
      let arion_weight = pallet_arion::FamilyWeight::<T>::get(owner);
      let total_arion_weight = pallet_arion::Pallet::<T>::get_total_family_weight();

      if total_arion_weight == 0 {
          warn!("Total Arion weight is 0");
          return 0;
      }

      let final_weight = Self::calculate_final_weight(arion_weight as u32, total_arion_weight as u32, miners_pool_inner as u32);
      final_weight
    }

    pub fn uid_zero_weight<
        T: pallet_marketplace::Config + ipfs_pallet::Config + pallet_credits::Config + pallet_arion::Config,
    >() -> u16 {
        let price_per_gb = pallet_marketplace::Pallet::<T>::get_price_per_gb();
        let network_totals = pallet_arion::CurrentNetworkTotals::<T>::get();
        let total_network_storage = network_totals.total_shard_data_bytes;
        let alpha_price = pallet_credits::Pallet::<T>::alpha_price();

        let context = if let Some(ctx) = Self::pool_context(total_network_storage, price_per_gb, alpha_price) {
            ctx
        } else {
            warn!("Failed to get pool context for UID zero, returning 0");
            return 0;
        };

        let uid_inner = context.uid_zero_pool;
        info!("raw UID zero weight: {}", uid_inner);

        let capped_f64 = uid_inner.min(Self::MAX_SCORE as f64);
        let final_weight = capped_f64 as u16; 
        
        info!("final UID zero weight: {}", final_weight);
        final_weight
    }

    fn calculate_final_weight(miner_points: u32, total_points: u32, pool_amount: u32) -> u32 {
        if total_points > 0 {
            let exact = (miner_points as f64 / total_points as f64) * pool_amount as f64;
            exact.round() as u32
        } else {
            0
        }
    }
}
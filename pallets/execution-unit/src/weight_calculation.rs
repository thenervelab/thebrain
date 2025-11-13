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

#[derive(Clone, Copy)]
struct PoolContext {
    available_pool: f64,
    miners_pool: f64,
    uid_zero_pool: f64,
    burn_percentage: f64, // NEW: track the actual burn percentage
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
    
        if price_per_gb == 0 || total_network_storage == 0 || alpha_price == 0 {
            warn!("price_per_gb, total_network_storage, or alpha_price is 0");
            return None;
        }
    
        // ---- 1) Convert storage to GB ----
        let storage_gb = total_network_storage as f64 / Self::BYTES_PER_GB as f64;
        info!("storage_gb: {}", storage_gb);
    
        // ---- 2) Convert prices to fractional tokens ----
        let price_scale = 10u128.pow(Self::TOKEN_DECIMALS);
        let price_per_gb_token = price_per_gb as f64 / price_scale as f64;
        let alpha_price_token = alpha_price as f64 / price_scale as f64;
        info!("price_per_gb_token: {}", price_per_gb_token);
        info!("alpha_price_token: {}", alpha_price_token);
    
        // ---- 3) Calculate COST and EMISSIONS ----
        let cost = storage_gb * price_per_gb_token;
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
            + pallet_credits::Config,
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
        let total_network_storage = ipfs_pallet::Pallet::<T>::get_total_network_storage();
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

        // Calculate miner's storage share
        let linked_nodes = pallet_registration::Pallet::<T>::linked_nodes(miner_id.clone());

        let mut seen: BTreeSet<Vec<u8>> = BTreeSet::new();
        let mut miner_total_storage: u128 = 0;

        let mut collect_storage = |node_id: &Vec<u8>| {
            if !seen.insert(node_id.clone()) {
                return;
            }

            if let Ok(bounded_id) = BoundedVec::<u8, ConstU32<64>>::try_from(node_id.clone()) {
                let files_size = ipfs_pallet::Pallet::<T>::miner_files_size(bounded_id).unwrap_or(0);
                miner_total_storage = miner_total_storage.saturating_add(files_size);
            }
        };

        collect_storage(miner_id);
        for node_id in linked_nodes.iter() {
            collect_storage(node_id);
        }

        if miner_total_storage == 0 {
            warn!("miner_total_storage is 0, returning 0 weight");
            return 0;
        }

        info!("miner_total_storage: {}, total_network_storage: {}", miner_total_storage, total_network_storage);

        let share = (miner_total_storage as f64 / total_network_storage as f64).min(1.0);
        info!("miner storage share: {}", share);
        
        // Miner weight
        let miner_weight = context.miners_pool * share;
        let final_weight = miner_weight.min(Self::MAX_SCORE as f64) as u32;
        info!("final miner weight: {}", final_weight);        

        // Convert to u32 (this should now be properly scaled)
        let capped_f64 = miner_weight.min(Self::MAX_SCORE as f64);
        let final_weight = capped_f64 as u32;   // safe because `MAX_SCORE` = 65_535
        
        info!("final miner weight: {}", final_weight);
        final_weight
    }

    pub fn uid_zero_weight<
        T: pallet_marketplace::Config + ipfs_pallet::Config + pallet_credits::Config,
    >() -> u16 {
        let price_per_gb = pallet_marketplace::Pallet::<T>::get_price_per_gb();
        let total_network_storage = ipfs_pallet::Pallet::<T>::get_total_network_storage();
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
}
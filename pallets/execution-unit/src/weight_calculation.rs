pub use crate::types::NodeMetricsData;
use core::convert::TryFrom;
use frame_support::BoundedVec;
use pallet_registration::NodeType;
use sp_arithmetic::{FixedPointNumber, FixedU128};
use sp_runtime::traits::ConstU32;
use sp_std::{
    collections::{btree_map::BTreeMap, btree_set::BTreeSet},
    vec::Vec,
};
use sp_runtime::Saturating;
use num_traits::One;
use num_traits::CheckedDiv;
use log::{info, warn, error};
use num_traits::Zero;

#[derive(Clone, Copy)]
struct PoolContext {
    available_pool: FixedU128,
    miners_pool: FixedU128,
    uid_zero_pool: FixedU128,
}

impl NodeMetricsData {
    const MAX_SCORE: u32 = 65_535;
    const DISTRIBUTION_PERCENT: u32 = 100;
    const ONE_HUNDRED_PERCENT: u128 = 100;
    const EMISSION_PERIOD: u128 = 50; 
    const WEIGHT_SCALE: u128 = 100; // Preserve two decimals of precision
    const BYTES_PER_GB: u128 = 1_073_741_824;
    const TOKEN_DECIMALS: u32 = 18; // 18 decimal places for the token

    // NEW: how much of the pool goes to UID0 vs miners
    const UID_ZERO_PERCENT: u32 = 20;   // 20% to UID0/burn
    const MINERS_PERCENT: u32 = 80;     // 80% to miners  (must sum to 100)

    fn pool_context(
        total_network_storage: u128,
        price_per_gb: u128, // in smallest unit (10^18)
        alpha_price: u128,  // still passed, we just log for now
    ) -> Option<PoolContext> {
        info!(
            "pool_context - raw price_per_gb: {}, total_network_storage: {}, alpha_price: {}",
            price_per_gb, total_network_storage, alpha_price
        );

        if price_per_gb == 0 || total_network_storage == 0 {
            warn!("price_per_gb or total_network_storage is 0");
            return None;
        }

        // ---- 1) Convert storage to GB in FixedU128 ----
        // storage_gb ≈ total_network_storage / BYTES_PER_GB
        let storage_gb = FixedU128::saturating_from_rational(
            total_network_storage,
            Self::BYTES_PER_GB,
        );
        info!("storage_gb inner: {}", storage_gb.into_inner());

        // ---- 2) Convert price_per_gb (18 decimals) into FixedU128 tokens/GB ----
        let price_scale = 10u128.pow(Self::TOKEN_DECIMALS);
        let price_per_gb_token =
            FixedU128::saturating_from_rational(price_per_gb, price_scale);
        info!(
            "price_per_gb_token (FixedU128) inner: {}",
            price_per_gb_token.into_inner()
        );

        // ---- 3) EMISSION_PERIOD as FixedU128 ----
        let emission_period_fixed =
            FixedU128::saturating_from_integer(Self::EMISSION_PERIOD);

        // ---- 4) Base global pool: storage_gb * price_per_gb_token * EMISSION_PERIOD ----
        // This is done fully in FixedU128 to avoid overflow.
        let mut global_pool = storage_gb
            .saturating_mul(price_per_gb_token)
            .saturating_mul(emission_period_fixed);

        // ---- 5) Apply DISTRIBUTION_PERCENT (usually 100%, but kept for flexibility) ----
        let distribution_ratio = FixedU128::saturating_from_rational(
            Self::DISTRIBUTION_PERCENT as u128,
            100u128,
        );
        global_pool = global_pool.saturating_mul(distribution_ratio);

        let global_inner = global_pool.into_inner();
        info!("global_inner (final pool inner): {}", global_inner);

        if global_inner == 0 {
            warn!("global_inner is 0 after calculation, returning None");
            return None;
        }

        // ---- 6) Optional: log alpha_price as FixedU128 (not used in split yet) ----
        let alpha_price_fixed =
            FixedU128::saturating_from_rational(alpha_price, price_scale);
        info!(
            "alpha_price_fixed inner: {}",
            alpha_price_fixed.into_inner()
        );

        // ---- 7) Split pool between UID0 and miners with clear percentages ----
        // NOTE: we do *not* subtract twice anymore.
        let uid_zero_inner = global_inner
            .saturating_mul(Self::UID_ZERO_PERCENT as u128)
            .checked_div(Self::ONE_HUNDRED_PERCENT)
            .unwrap_or(0);

        let mut miners_inner = global_inner.saturating_sub(uid_zero_inner);

        info!(
            "uid_zero_inner: {}, miners_inner before cap: {}",
            uid_zero_inner, miners_inner
        );

        // Cap miners_inner to MAX_SCORE so we don't exceed scoring range.
        miners_inner = miners_inner.min(Self::MAX_SCORE as u128);
        info!("miners_inner after cap: {}", miners_inner);

        Some(PoolContext {
            // available_pool here is simply the whole pool;
            // if you need a different semantics later, you can adjust.
            available_pool: FixedU128::from_inner(global_inner),
            miners_pool: FixedU128::from_inner(miners_inner),
            uid_zero_pool: FixedU128::from_inner(uid_zero_inner),
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

        let context =
            if let Some(ctx) = Self::pool_context(total_network_storage, price_per_gb, alpha_price) {
                ctx
            } else {
                return 0;
            };

        let miners_pool_inner = context.miners_pool.into_inner();
        if miners_pool_inner == 0 || total_network_storage == 0 {
            return 0;
        }

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
            return 0;
        }

        let share = FixedU128::saturating_from_rational(miner_total_storage, total_network_storage)
            .min(FixedU128::one());

        let miner_weight_fixed = context.miners_pool.saturating_mul(share);
        let miner_weight = miner_weight_fixed.into_inner();

        let capped = miner_weight.min(Self::MAX_SCORE as u128);
        u32::try_from(capped).unwrap_or(Self::MAX_SCORE)
    }

    pub fn uid_zero_weight<
        T: pallet_marketplace::Config + ipfs_pallet::Config + pallet_credits::Config,
    >() -> u16 {
        let price_per_gb = pallet_marketplace::Pallet::<T>::get_price_per_gb();
        let total_network_storage = ipfs_pallet::Pallet::<T>::get_total_network_storage();
        let alpha_price = pallet_credits::Pallet::<T>::alpha_price();

        let context =
            if let Some(ctx) = Self::pool_context(total_network_storage, price_per_gb, alpha_price)
            {
                ctx
            } else {
                return 0;
            };

        let uid_inner = context.uid_zero_pool.into_inner();
        if uid_inner == 0 {
            return 0;
        }

        let capped = uid_inner.min(Self::MAX_SCORE as u128);
        u16::try_from(capped).unwrap_or(Self::MAX_SCORE as u16)
    }
}

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
use log::{info, warn, error};

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
	const EMISSION_PERIOD: u128 = 2;
	const WEIGHT_SCALE: u128 = 100; // Preserve two decimals of precision
	const BYTES_PER_GB: u128 = 1_073_741_824;
	const TOKEN_DECIMALS: u32 = 18; // 18 decimal places for the token

	fn pool_context(
		total_network_storage: u128,
		price_per_gb: u128, // This is expected to be in the smallest unit (10^18)
		alpha_price: u128,
	) -> Option<PoolContext> {
		info!("pool_context - raw price_per_gb: {}, total_network_storage: {}, alpha_price: {}", 
		    price_per_gb, total_network_storage, alpha_price);

		if price_per_gb == 0 || total_network_storage == 0 {
			warn!("price_per_gb or total_network_storage is 0");
			return None;
		}

		// Convert price from 18 decimals to the correct scale for calculations
		let price_per_gb_fixed = FixedU128::saturating_from_rational(price_per_gb, 10u128.pow(Self::TOKEN_DECIMALS));
		info!(
			"Adjusted price_per_gb (removed {} decimals):  (inner: {})", 
			Self::TOKEN_DECIMALS, 
			price_per_gb_fixed.into_inner()
		);

		let total_storage_fixed = FixedU128::saturating_from_integer(total_network_storage);
		let bytes_per_gb_fixed = FixedU128::saturating_from_integer(Self::BYTES_PER_GB);

		let global_pool = {
			let total_storage_inner = total_storage_fixed.into_inner();
			let price_inner = price_per_gb_fixed.into_inner();
			let bytes_per_gb_inner = bytes_per_gb_fixed.into_inner();
			
			info!(
				"Raw calculation: ({} * {}) / {}",
				total_storage_inner, price_inner, bytes_per_gb_inner
			);
			
			// Avoid overflow: (total_storage * price) / BYTES_PER_GB
			let numerator = total_storage_inner.saturating_mul(price_inner);
			info!("Numerator: {}", numerator);
			
			let pool_inner = if bytes_per_gb_inner == 0 {
				warn!("bytes_per_gb_inner is 0");
				0
			} else {
				let result = numerator.checked_div(bytes_per_gb_inner).unwrap_or_else(|| {
					error!("Division by zero or overflow in pool_inner calculation");
					0
				});
				info!("Division result: {}", result);
				result
			};
			
			info!("Final pool_inner: {}", pool_inner);
			
			// Apply distribution ratio: pool * DISTRIBUTION_PERCENT / 100
			let distributed_inner = pool_inner
				.saturating_mul(Self::DISTRIBUTION_PERCENT as u128)
				.checked_div(100)
				.unwrap_or_else(|| {
					error!("Division by zero in distributed_inner calculation");
					0
				});
			
			info!(
				"After distribution ({}% of pool): {}", 
				Self::DISTRIBUTION_PERCENT, 
				distributed_inner
			);
			
			FixedU128::from_inner(distributed_inner)
		};

		let global_inner = global_pool.into_inner();
		// After this line:
		info!("global_inner: {}", global_inner);

		// Convert alpha_price from 18 decimals to the correct scale for calculations
		let alpha_price_fixed = FixedU128::saturating_from_rational(alpha_price, 10u128.pow(Self::TOKEN_DECIMALS));
		info!(
			"Adjusted alpha_price (removed {} decimals): {} (inner: {})", 
			Self::TOKEN_DECIMALS,
			alpha_price,
			alpha_price_fixed.into_inner()
		);

		// Calculate burn_requested as alpha_price * EMISSION_PERIOD with proper fixed-point handling
		let burn_requested = alpha_price.saturating_mul(Self::EMISSION_PERIOD as u128);
		let burn_requested_inner = burn_requested;
		info!("burn_requested: {} (raw inner value: {})", 
			burn_requested, burn_requested_inner);

		let burn_inner = burn_requested_inner.min(global_inner);
		info!("burn_inner: {}", burn_inner);

		let available_inner = global_inner.saturating_sub(burn_inner);
		info!("available_inner: {}", available_inner);

		let uid_zero_inner = burn_inner;

		let miners_inner = available_inner.saturating_sub(uid_zero_inner);
		info!("miners_inner: {}", miners_inner);

		// Cap the miners_inner to MAX_SCORE if it exceeds
		let miners_inner = miners_inner.min(Self::MAX_SCORE as u128);
		info!("Miners pool after capping: {}", miners_inner);

		Some(PoolContext {
			available_pool: FixedU128::from_inner(available_inner),
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
		info!("calculate_weight - node_type: {:?}, miner_id: {:?}", node_type, hex::encode(miner_id));

		if node_type != NodeType::StorageMiner {
			info!("Node is not a StorageMiner, returning weight 0");
			return 0;
		}

		let price_per_gb = pallet_marketplace::Pallet::<T>::get_price_per_gb();
		let total_network_storage = ipfs_pallet::Pallet::<T>::get_total_network_storage();
		let alpha_price = pallet_credits::Pallet::<T>::alpha_price();

		info!(
			"Fetched values - price_per_gb: {}, total_network_storage: {}, alpha_price: {}",
			price_per_gb, total_network_storage, alpha_price
		);

		let context = if let Some(ctx) = Self::pool_context(total_network_storage, price_per_gb, alpha_price) {
			info!("Successfully created pool context");
			ctx
		} else {
			warn!("Failed to create pool context, returning weight 0");
			return 0;
		};

		let miners_pool_inner = context.miners_pool.into_inner();
		info!("miners_pool inner value: {}", miners_pool_inner);

		if miners_pool_inner == 0 || total_network_storage == 0 {
			warn!(
				"miners_pool is 0 ({}) or total_network_storage is 0 ({}), returning weight 0",
				miners_pool_inner, total_network_storage
			);
			return 0;
		}

		let linked_nodes = pallet_registration::Pallet::<T>::linked_nodes(miner_id.clone());
		info!("Found {} linked nodes for miner", linked_nodes.len());

		let mut seen: BTreeSet<Vec<u8>> = BTreeSet::new();
		let mut miner_total_storage: u128 = 0;

		let mut collect_storage = |node_id: &Vec<u8>| {
			if !seen.insert(node_id.clone()) {
				return;
			}

			if let Ok(bounded_id) = BoundedVec::<u8, ConstU32<64>>::try_from(node_id.clone()) {
				let files_size = ipfs_pallet::Pallet::<T>::miner_files_size(bounded_id).unwrap_or(0);
				info!(
					"Node {} has storage: {}",
					hex::encode(node_id),
					files_size
				);
				miner_total_storage = miner_total_storage.saturating_add(files_size);
			}
		};

		info!("Collecting storage for main miner node");
		collect_storage(&miner_id);
		
		info!("Collecting storage for linked nodes");
		for (i, node_id) in linked_nodes.iter().enumerate() {
			info!("Processing linked node {}/{}", i + 1, linked_nodes.len());
			collect_storage(node_id);
		}

		info!("Total storage for miner and linked nodes: {}", miner_total_storage);

		if miner_total_storage == 0 {
			warn!("Miner has no storage, returning weight 0");
			return 0;
		}

		let share = FixedU128::saturating_from_rational(miner_total_storage, total_network_storage)
			.min(FixedU128::one());
		info!(
			"Storage share: {} (miner_storage: {}, total_network_storage: {})",
			share, miner_total_storage, total_network_storage
		);

		let miner_weight_fixed = context.miners_pool.saturating_mul(share);
		info!(
			"Miner weight before scaling: {} (miners_pool: {}, share: {})",
			miner_weight_fixed, context.miners_pool, share
		);

		let miner_weight = miner_weight_fixed.into_inner();
		info!("Miner weight: {}", miner_weight);

		// Ensure we don't exceed MAX_SCORE
		let capped = miner_weight.min(Self::MAX_SCORE as u128);
		info!("Final weight (capped at {}): {}", Self::MAX_SCORE, capped);

		u32::try_from(capped).unwrap_or_else(|_| {
			warn!(
				"Failed to convert weight {} to u32, using MAX_SCORE",
				capped
			);
			Self::MAX_SCORE
		})
	}

	pub fn uid_zero_weight<
		T: pallet_marketplace::Config + ipfs_pallet::Config + pallet_credits::Config,
	>() -> u16 {
		let price_per_gb = pallet_marketplace::Pallet::<T>::get_price_per_gb();
		let total_network_storage = ipfs_pallet::Pallet::<T>::get_total_network_storage();
		let alpha_price = pallet_credits::Pallet::<T>::alpha_price();

		let context =
			if let Some(ctx) =
				Self::pool_context(total_network_storage, price_per_gb, alpha_price)
			{
				ctx
			} else {
				return 0;
			};

		let uid_inner = context.uid_zero_pool.into_inner();
		if uid_inner == 0 {
			return 0;
		}

		// Cap the UID zero weight to MAX_SCORE
		let capped = uid_inner.min(Self::MAX_SCORE as u128);
		u16::try_from(capped).unwrap_or(Self::MAX_SCORE as u16)
	}
}

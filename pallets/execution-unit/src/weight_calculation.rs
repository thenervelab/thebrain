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

impl NodeMetricsData {
	const MAX_SCORE: u32 = 65_535;
	const DISTRIBUTION_PERCENT: u32 = 80;
	const WEIGHT_SCALE: u128 = 100; // Preserve two decimals of precision
	const BYTES_PER_GB: u128 = 1_073_741_824;

	pub fn calculate_weight<
		T: pallet_marketplace::Config
			+ ipfs_pallet::Config
			+ pallet_registration::Config
			+ crate::Config
			+ pallet_rankings::Config,
	>(
		node_type: NodeType,
		metrics: &NodeMetricsData,
		_all_nodes_metrics: &[NodeMetricsData],
		_geo_distribution: &BTreeMap<Vec<u8>, u32>,
		_coldkey: &T::AccountId,
	) -> u32 {
		if node_type != NodeType::StorageMiner {
			return 0;
		}

		let price_per_gb = pallet_marketplace::Pallet::<T>::get_price_per_gb();
		if price_per_gb == 0 {
			return 0;
		}

		let total_network_storage = ipfs_pallet::Pallet::<T>::get_total_network_storage();
		if total_network_storage == 0 {
			return 0;
		}

		let linked_nodes = pallet_registration::Pallet::<T>::linked_nodes(metrics.miner_id.clone());
		let mut seen: BTreeSet<Vec<u8>> = BTreeSet::new();
		let mut miner_total_storage: u128 = 0;

		let mut collect_storage = |node_id: &Vec<u8>| {
			if !seen.insert(node_id.clone()) {
				return;
			}

			if let Ok(bounded_id) = BoundedVec::<u8, ConstU32<64>>::try_from(node_id.clone()) {
				miner_total_storage = miner_total_storage.saturating_add(
					ipfs_pallet::Pallet::<T>::miner_files_size(bounded_id).unwrap_or(0),
				);
			}
		};

		collect_storage(&metrics.miner_id);
		for node_id in linked_nodes.iter() {
			collect_storage(node_id);
		}

		if miner_total_storage == 0 {
			return 0;
		}

		let total_storage_fixed = FixedU128::saturating_from_integer(total_network_storage);
		let miner_storage_fixed = FixedU128::saturating_from_integer(miner_total_storage);
		let price_fixed = FixedU128::saturating_from_integer(price_per_gb);
		let bytes_per_gb_fixed = FixedU128::saturating_from_integer(Self::BYTES_PER_GB);

		let global_pool = total_storage_fixed
			.saturating_mul(price_fixed)
			.saturating_div(bytes_per_gb_fixed);

		if global_pool.into_inner() == 0 {
			return 0;
		}

		let distribution_ratio =
			FixedU128::saturating_from_rational(Self::DISTRIBUTION_PERCENT.into(), 100u128);
		let distribution_pool = global_pool.saturating_mul(distribution_ratio);

		let accuracy_per_scale = FixedU128::accuracy()
			.checked_div(Self::WEIGHT_SCALE)
			.unwrap_or(FixedU128::accuracy());

		let target_total_scaled = distribution_pool.into_inner() / accuracy_per_scale;
		if target_total_scaled == 0 {
			return 0;
		}

		let share = miner_storage_fixed
			.saturating_div(total_storage_fixed)
			.min(FixedU128::one());
		let miner_weight_fixed = distribution_pool.saturating_mul(share);
		let mut miner_weight_scaled = miner_weight_fixed.into_inner() / accuracy_per_scale;

		if target_total_scaled > Self::MAX_SCORE as u128 {
			let scale_factor =
				FixedU128::saturating_from_rational(Self::MAX_SCORE as u128, target_total_scaled);
			miner_weight_scaled = FixedU128::saturating_from_integer(miner_weight_scaled)
				.saturating_mul(scale_factor)
				.into_inner()
				/ FixedU128::accuracy();
		}

		let capped = miner_weight_scaled.min(Self::MAX_SCORE as u128);
		u32::try_from(capped).unwrap_or(Self::MAX_SCORE)
	}
}


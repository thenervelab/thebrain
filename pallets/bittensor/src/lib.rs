#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

#[frame_support::pallet]
pub mod pallet {
    use frame_support::pallet_prelude::*;
    use frame_system::pallet_prelude::*;
    use pallet_execution_unit::{
        Pallet as ExecutionPallet,
        types::NodeMetricsData,
        weight_calculation::NodeMetricsData as WeightCalculation,
    };
    use pallet_metagraph::Pallet as MetagraphPallet;
    use pallet_rankings::Pallet as RankingsPallet;
    use pallet_registration::{Pallet as RegistrationPallet,  NodeType, NodeInfo};
    use pallet_metagraph::{UID, Role};
    use pallet_utils::Pallet as UtilsPallet;
    use scale_info::prelude::string::String;
    use sp_core::crypto::Ss58Codec;
    use sp_io;
    use pallet_registration::LinkedNodes;
    use sp_runtime::{
        format,
        traits::Zero,
        offchain::{http, Duration},
        AccountId32,
    };
    use sp_std::{
        collections::btree_map::BTreeMap,
        prelude::*,
    };
    use serde_json::Value;
    use codec::alloc::string::ToString;

    #[pallet::config]
    pub trait Config: frame_system::Config + pallet_session::Config 
                      + pallet_registration::Config + pallet_execution_unit::Config 
                      + pallet_metagraph::Config + pallet_rankings::Config 
                      + pallet_rankings::Config 
                    //   + pallet_rankings::Config<pallet_rankings::Instance2>
                      + pallet_rankings::Config<pallet_rankings::Instance3>
                    //   + pallet_rankings::Config<pallet_rankings::Instance4> 
                    //   + pallet_rankings::Config<pallet_rankings::Instance5> 
                      {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        #[pallet::constant]
        type FinneyRpcUrl: Get<&'static str>;

        #[pallet::constant]
        type VersionKeyStorageKey: Get<&'static str>;
        
        #[pallet::constant]
        type BittensorCallSubmission: Get<u32>; // Add this line for the new constant

        #[pallet::constant]
        type NetUid: Get<u16>; 

        #[pallet::constant]
        type Versionkey: Get<u32>;

        #[pallet::constant]
        type DefaultSpecVersion: Get<u32>;
    
        #[pallet::constant]
        type DefaultGenesisHash: Get<&'static str>;
    }

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::event]
    // #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {

    }

    #[pallet::error]
    pub enum Error<T> {
        NoneValue,
        StorageOverflow,
        SubmissionDisabled,
    }

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        fn offchain_worker(block_number: BlockNumberFor<T>) {
            if block_number %  T::BittensorCallSubmission::get().into() != Zero::zero() {
                log::info!("Skipping bittensor call submission");
                return;
            }

            match UtilsPallet::<T>::fetch_node_id() {
                Ok(node_id) => {
                    let node_info = RegistrationPallet::<T>::get_node_registration_info(node_id.clone());	
                    if node_info.is_some() {
                        if node_info.unwrap().node_type == NodeType::Validator {
                            if let Err(e) = Self::submit_weight_extrinsic(block_number) {
                                log::error!("❌ Failed to submit weights: {:?}", e);
                            } else {
                                log::info!("✅ Successfully submitted weights!");
                            }  
                        }
                    }
                }
				Err(e) => {
					log::error!("Error fetching node identity inside bittensor pallet: {:?}", e);
				}
            }
        }
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        // #[pallet::call_index(0)]
        // #[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
        // pub fn set_value(origin: OriginFor<T>, new_value: Vec<u8>) -> DispatchResult {
        //     let sender = ensure_signed(origin)?;

        //     // Update the storage
        //     StoredData::<T>::put(new_value.clone());

        //     Ok(())
        // }
    }

    impl<T: Config> Pallet<T> {
        // New method to calculate weights specifically for storage miners
        fn calculate_storage_miner_weights(
            all_miners: &Vec<NodeInfo<BlockNumberFor<T>, T::AccountId>>, 
            all_nodes_metrics: &Vec<NodeMetricsData>, 
            uids: &Vec<UID>
        ) -> (Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>, Vec<u16>, Vec<u16>,
                Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>) {

            let mut storage_weights: Vec<u16> = Vec::new();
            let mut storage_nodes_ss58: Vec<Vec<u8>> = Vec::new();
            let mut storage_miners_node_id: Vec<Vec<u8>> = Vec::new();
            let mut storage_miners_node_types: Vec<NodeType> = Vec::new();
            let mut all_uids_on_bittensor: Vec<u16> = Vec::new();
            let mut all_weights_on_bitensor: Vec<u16> = Vec::new();
        
            let mut linked_storage_miners_weights: Vec<u16> = Vec::new();
            let mut linked_storage_miners_ss58: Vec<Vec<u8>> = Vec::new();
            let mut linked_storage_miners_node_id: Vec<Vec<u8>> = Vec::new();
            let mut linked_storage_miners_node_types: Vec<NodeType> = Vec::new();
        
            let mut geo_distribution: BTreeMap<Vec<u8>, u32> = BTreeMap::new();
        
            for miner in all_miners {
                if miner.node_type != NodeType::StorageMiner {
                    continue;
                }

                // Retrieve linked nodes for the main node
                let mut linked_node_ids = LinkedNodes::<T>::get(&miner.node_id);
                let mut total_weight = 0u32;

                if !linked_node_ids.is_empty() { // Check if there are linked nodes
                    // Remove linked nodes without metrics
                    linked_node_ids.retain(|linked_node_id| ExecutionPallet::<T>::get_node_metrics(linked_node_id.clone()).is_some());

                    // Calculate weights for linked nodes
                    for linked_node_id in &linked_node_ids {
                        if let Some(metrics) = ExecutionPallet::<T>::get_node_metrics(linked_node_id.clone()) {
                            geo_distribution.insert(metrics.geolocation.clone(), 1);    
                            let weight = WeightCalculation::calculate_weight::<T>(NodeType::StorageMiner, &metrics, all_nodes_metrics, &geo_distribution);
                            total_weight += weight as u32; // Accumulate weight

                            let current_block_number = <frame_system::Pallet<T>>::block_number();
                            let buffer = 300u32;
                            let blocks_online = ExecutionPallet::<T>::block_numbers(miner.node_id.clone());
                            
                            if let Some(blocks) = blocks_online {
                                if let Some(&last_block) = blocks.last() {
                                    let difference = current_block_number - last_block;
                                    if difference > buffer.into() { // Ensure buffer is of the correct type
                                        total_weight -= weight as u32; // Accumulate weight
                                    }
                                }
                            }
                            // Retrieve node info using the linked node ID
                            if let Some(linked_node_info) = RegistrationPallet::<T>::get_node_registration_info(linked_node_id.clone().to_vec()) {
                                let linked_miner_ss58 = AccountId32::new(linked_node_info.owner.encode().try_into().unwrap_or_default()).to_ss58check();
                                linked_storage_miners_ss58.push(linked_miner_ss58.into());
                                linked_storage_miners_node_id.push(linked_node_id.clone());
                                linked_storage_miners_node_types.push(NodeType::StorageMiner);
                                linked_storage_miners_weights.push(weight as u16);
                            }
                        }
                    }
                    // Normalize weight for the main node based on linked nodes
                    let normalized_weight = total_weight / linked_node_ids.len() as u32;
                    storage_weights.push(normalized_weight as u16);
                } else {
                    // Handle the case where there are no linked nodes
                    if let Some(metrics) = ExecutionPallet::<T>::get_node_metrics(miner.node_id.clone()) {
                        geo_distribution.insert(metrics.geolocation.clone(), 1);    
                        let mut weight = WeightCalculation::calculate_weight::<T>(NodeType::StorageMiner, &metrics, all_nodes_metrics, &geo_distribution);
                        let current_block_number = <frame_system::Pallet<T>>::block_number();
                        let buffer = 300u32;
                        let blocks_online = ExecutionPallet::<T>::block_numbers(miner.node_id.clone());
                        
                        if let Some(blocks) = blocks_online {
                            if let Some(&last_block) = blocks.last() {
                                let difference = current_block_number - last_block;
                                if difference > buffer.into() { // Ensure buffer is of the correct type
                                    weight = 0; // Accumulate weight
                                }
                            }
                        }
                        storage_weights.push(weight as u16);
                    } else {
                        log::info!("Node metrics not found for storage miner: {:?}", miner.node_id);
                    }
                }

                // Other logic remains the same...
                let miner_ss58 = AccountId32::new(miner.owner.encode().try_into().unwrap_or_default()).to_ss58check();
                storage_miners_node_id.push(miner.node_id.clone());
                storage_miners_node_types.push(miner.node_type.clone());
                storage_nodes_ss58.push(miner_ss58.clone().into());
        
                // Update Bittensor UIDs
                for uid in uids.iter() {
                    if uid.substrate_address.to_ss58check() == miner_ss58 {
                        all_uids_on_bittensor.push(uid.id);
                        all_weights_on_bitensor.push(*storage_weights.last().unwrap_or(&0) as u16); // Use the last calculated weight
                    }
                }
            }
        
            (storage_weights, storage_nodes_ss58, storage_miners_node_id, 
             storage_miners_node_types, all_uids_on_bittensor, all_weights_on_bitensor,
             linked_storage_miners_weights, linked_storage_miners_ss58, linked_storage_miners_node_id, 
             linked_storage_miners_node_types, 
            )
        }

        // New method to calculate weights specifically for storage S3 miners
        fn calculate_storage_s3_weights(
            all_miners: &Vec<NodeInfo<BlockNumberFor<T>, T::AccountId>>, 
            all_nodes_metrics: &Vec<NodeMetricsData>, 
            uids: &Vec<UID>
        ) -> (Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>, Vec<u16>, Vec<u16>,
            Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>) {
            
            let mut storage_s3_weights: Vec<u16> = Vec::new();
            let mut storage_s3_nodes_ss58: Vec<Vec<u8>> = Vec::new();
            let mut storage_s3_miners_node_id: Vec<Vec<u8>> = Vec::new();
            let mut storage_s3_miners_node_types: Vec<NodeType> = Vec::new();
            let mut all_uids_on_bittensor: Vec<u16> = Vec::new();
            let mut all_weights_on_bitensor: Vec<u16> = Vec::new();

            let mut linked_s3_miners_ss58: Vec<Vec<u8>> = Vec::new();
            let mut linked_s3_miners_node_id: Vec<Vec<u8>> = Vec::new();
            let mut linked_s3_miners_node_types: Vec<NodeType> = Vec::new();
            let mut linked_s3_miners_weights: Vec<u16> = Vec::new();

            let mut geo_distribution: BTreeMap<Vec<u8>, u32> = BTreeMap::new();

            for miner in all_miners {
                if miner.node_type != NodeType::StorageS3 {
                    continue;
                }

                // Retrieve linked nodes for the main node
                let mut linked_node_ids = LinkedNodes::<T>::get(&miner.node_id);
                let mut total_weight = 0u32;

                if !linked_node_ids.is_empty() { // Check if there are linked nodes
                    // Remove linked nodes without metrics
                    linked_node_ids.retain(|linked_node_id| ExecutionPallet::<T>::get_node_metrics(linked_node_id.clone()).is_some());

                    // Calculate weights for linked nodes
                    for linked_node_id in &linked_node_ids {
                        if let Some(metrics) = ExecutionPallet::<T>::get_node_metrics(linked_node_id.clone()) {
                            geo_distribution.insert(metrics.geolocation.clone(), 1);
                            let weight = WeightCalculation::calculate_weight::<T>(NodeType::StorageS3, &metrics, all_nodes_metrics, &geo_distribution);
                            total_weight += weight as u32; // Accumulate weight

                            let current_block_number = <frame_system::Pallet<T>>::block_number();
                            let buffer = 300u32;
                            let blocks_online = ExecutionPallet::<T>::block_numbers(miner.node_id.clone());
                            
                            if let Some(blocks) = blocks_online {
                                if let Some(&last_block) = blocks.last() {
                                    let difference = current_block_number - last_block;
                                    if difference > buffer.into() { // Ensure buffer is of the correct type
                                        total_weight -= weight as u32; // Accumulate weight
                                    }
                                }
                            }                   
                            // Retrieve node info using the linked node ID
                            if let Some(linked_node_info) = RegistrationPallet::<T>::get_node_registration_info(linked_node_id.clone().to_vec()) {
                                let linked_miner_ss58 = AccountId32::new(linked_node_info.owner.encode().try_into().unwrap_or_default()).to_ss58check();
                                linked_s3_miners_ss58.push(linked_miner_ss58.into());
                                linked_s3_miners_node_id.push(linked_node_id.clone());
                                linked_s3_miners_node_types.push(NodeType::StorageS3);
                                linked_s3_miners_weights.push(weight as u16);
                            }
                        }
                    }
                    // Normalize weight for the main node based on linked nodes
                    let normalized_weight = total_weight / linked_node_ids.len() as u32;
                    storage_s3_weights.push(normalized_weight as u16);
                } else {
                    // Handle the case where there are no linked nodes
                    if let Some(metrics) = ExecutionPallet::<T>::get_node_metrics(miner.node_id.clone()) {
                        geo_distribution.insert(metrics.geolocation.clone(), 1);
                        let mut weight = WeightCalculation::calculate_weight::<T>(NodeType::StorageS3, &metrics, all_nodes_metrics, &geo_distribution);
                        let current_block_number = <frame_system::Pallet<T>>::block_number();
                        let buffer = 300u32;
                        let blocks_online = ExecutionPallet::<T>::block_numbers(miner.node_id.clone());
                        
                        if let Some(blocks) = blocks_online {
                            if let Some(&last_block) = blocks.last() {
                                let difference = current_block_number - last_block;
                                if difference > buffer.into() { // Ensure buffer is of the correct type
                                    weight = 0; // Accumulate weight
                                }
                            }
                        }
                        storage_s3_weights.push(weight as u16);
                    } else {
                        log::info!("Node metrics not found for storage S3 miner: {:?}", miner.node_id);
                    }
                }

                // Other logic remains the same...
                let miner_ss58 = AccountId32::new(miner.owner.encode().try_into().unwrap_or_default()).to_ss58check();
                storage_s3_miners_node_id.push(miner.node_id.clone());
                storage_s3_miners_node_types.push(miner.node_type.clone());
                storage_s3_nodes_ss58.push(miner_ss58.clone().into());

                // Update Bittensor UIDs
                for uid in uids.iter() {
                    if uid.substrate_address.to_ss58check() == miner_ss58 {
                        all_uids_on_bittensor.push(uid.id);
                        all_weights_on_bitensor.push(*storage_s3_weights.last().unwrap_or(&0) as u16); // Use the last calculated weight
                    }
                }
            }

            (storage_s3_weights, storage_s3_nodes_ss58, storage_s3_miners_node_id, 
            storage_s3_miners_node_types, all_uids_on_bittensor, all_weights_on_bitensor,
            linked_s3_miners_weights, linked_s3_miners_ss58, linked_s3_miners_node_id, linked_s3_miners_node_types)
        }   

        // New method to calculate weights specifically for compute miners
        fn calculate_validator_weights(
            all_miners: &Vec<NodeInfo<BlockNumberFor<T>, T::AccountId>>, 
            all_nodes_metrics: &Vec<NodeMetricsData>, 
            uids: &Vec<UID>
        ) -> (Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>, Vec<u16>, Vec<u16>,
            Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>) {
            
            let mut validator_weights: Vec<u16> = Vec::new();
            let mut validator_nodes_ss58: Vec<Vec<u8>> = Vec::new();
            let mut validator_miners_node_id: Vec<Vec<u8>> = Vec::new();
            let mut validator_miners_node_types: Vec<NodeType> = Vec::new();
            let mut all_uids_on_bittensor: Vec<u16> = Vec::new();
            let mut all_weights_on_bitensor: Vec<u16> = Vec::new();

            let mut linked_validator_ss58: Vec<Vec<u8>> = Vec::new();
            let mut linked_validator_node_id: Vec<Vec<u8>> = Vec::new();
            let mut linked_validator_node_types: Vec<NodeType> = Vec::new();
            let mut linked_validator_weights: Vec<u16> = Vec::new();
            let mut geo_distribution: BTreeMap<Vec<u8>, u32> = BTreeMap::new();

            for miner in all_miners {
                if miner.node_type != NodeType::Validator {
                    continue;
                }

                // Retrieve linked nodes for the main node
                let mut linked_node_ids = LinkedNodes::<T>::get(&miner.node_id);
                let mut total_weight = 0u32;

                if !linked_node_ids.is_empty() { // Check if there are linked nodes
                    // Remove linked nodes without metrics
                    linked_node_ids.retain(|linked_node_id| ExecutionPallet::<T>::get_node_metrics(linked_node_id.clone()).is_some());

                    // Calculate weights for linked nodes
                    for linked_node_id in &linked_node_ids {
                        if let Some(metrics) = ExecutionPallet::<T>::get_node_metrics(linked_node_id.clone()) {
                            geo_distribution.insert(metrics.geolocation.clone(), 1);    
                            let weight = WeightCalculation::calculate_weight::<T>(NodeType::Validator, &metrics, all_nodes_metrics, &geo_distribution);
                            total_weight += weight as u32; // Accumulate weight

                            let current_block_number = <frame_system::Pallet<T>>::block_number();
                            let buffer = 300u32;
                            let blocks_online = ExecutionPallet::<T>::block_numbers(miner.node_id.clone());
                            
                            if let Some(blocks) = blocks_online {
                                if let Some(&last_block) = blocks.last() {
                                    let difference = current_block_number - last_block;
                                    if difference > buffer.into() { // Ensure buffer is of the correct type
                                        total_weight -= weight as u32; // Accumulate weight
                                    }
                                }
                            }

                            // Retrieve node info using the linked node ID
                            if let Some(linked_node_info) = RegistrationPallet::<T>::get_node_registration_info(linked_node_id.clone().to_vec()) {
                                let linked_miner_ss58 = AccountId32::new(linked_node_info.owner.encode().try_into().unwrap_or_default()).to_ss58check();
                                linked_validator_ss58.push(linked_miner_ss58.into());
                                linked_validator_node_id.push(linked_node_id.clone());
                                linked_validator_node_types.push(NodeType::Validator);
                                linked_validator_weights.push(weight as u16);
                            }
                        }
                    }
                    // Normalize weight for the main node based on linked nodes
                    let normalized_weight = total_weight / linked_node_ids.len() as u32;
                    validator_weights.push(normalized_weight as u16);
                } else {
                    // Handle the case where there are no linked nodes
                    if let Some(metrics) = ExecutionPallet::<T>::get_node_metrics(miner.node_id.clone()) {
                        geo_distribution.insert(metrics.geolocation.clone(), 1);    
                        let mut weight = WeightCalculation::calculate_weight::<T>(NodeType::Validator, &metrics, all_nodes_metrics, &geo_distribution);
                        let current_block_number = <frame_system::Pallet<T>>::block_number();
                        let buffer = 300u32;
                        let blocks_online = ExecutionPallet::<T>::block_numbers(miner.node_id.clone());
                        
                        if let Some(blocks) = blocks_online {
                            if let Some(&last_block) = blocks.last() {
                                let difference = current_block_number - last_block;
                                if difference > buffer.into() { // Ensure buffer is of the correct type
                                    weight = 0; // Accumulate weight
                                }
                            }
                        }
                        validator_weights.push(weight as u16);
                    } else {
                        log::info!("Node metrics not found for validator miner: {:?}", miner.node_id);
                    }
                }

                // Other logic remains the same...
                let miner_ss58 = AccountId32::new(miner.owner.encode().try_into().unwrap_or_default()).to_ss58check();
                validator_miners_node_id.push(miner.node_id.clone());
                validator_miners_node_types.push(miner.node_type.clone());
                validator_nodes_ss58.push(miner_ss58.clone().into());

                // Update Bittensor UIDs
                for uid in uids.iter() {
                    if uid.substrate_address.to_ss58check() == miner_ss58 {
                        all_uids_on_bittensor.push(uid.id);
                        all_weights_on_bitensor.push(*validator_weights.last().unwrap_or(&0) as u16); // Use the last calculated weight
                    }
                }
            }

            (validator_weights, validator_nodes_ss58, validator_miners_node_id, 
            validator_miners_node_types, all_uids_on_bittensor, all_weights_on_bitensor,
            linked_validator_weights, linked_validator_ss58, linked_validator_node_id, linked_validator_node_types)
        }

        // New method to calculate weights specifically for GPU miners
        fn calculate_gpu_miner_weights(
            all_miners: &Vec<NodeInfo<BlockNumberFor<T>, T::AccountId>>, 
            all_nodes_metrics: &Vec<NodeMetricsData>, 
            uids: &Vec<UID>
        ) -> (Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>, Vec<u16>, Vec<u16>,
            Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>) {

            let mut gpu_weights: Vec<u16> = Vec::new();
            let mut gpu_nodes_ss58: Vec<Vec<u8>> = Vec::new();
            let mut gpu_miners_node_id: Vec<Vec<u8>> = Vec::new();
            let mut gpu_miners_node_types: Vec<NodeType> = Vec::new();
            let mut all_uids_on_bittensor: Vec<u16> = Vec::new();
            let mut all_weights_on_bitensor: Vec<u16> = Vec::new();

            let mut linked_gpu_miners_ss58: Vec<Vec<u8>> = Vec::new();
            let mut linked_gpu_miners_node_id: Vec<Vec<u8>> = Vec::new();
            let mut linked_gpu_miners_node_types: Vec<NodeType> = Vec::new();
            let mut linked_gpu_miners_weights: Vec<u16> = Vec::new();
            let mut geo_distribution: BTreeMap<Vec<u8>, u32> = BTreeMap::new();

            for miner in all_miners {
                if miner.node_type != NodeType::GpuMiner {
                    continue;
                }

                // Retrieve linked nodes for the main node
                let mut linked_node_ids = LinkedNodes::<T>::get(&miner.node_id);
                let mut total_weight = 0u32;

                if !linked_node_ids.is_empty() { // Check if there are linked nodes
                    // Remove linked nodes without metrics
                    linked_node_ids.retain(|linked_node_id| ExecutionPallet::<T>::get_node_metrics(linked_node_id.clone()).is_some());

                    // Calculate weights for linked nodes
                    for linked_node_id in &linked_node_ids {
                        if let Some(metrics) = ExecutionPallet::<T>::get_node_metrics(linked_node_id.clone()) {
                            geo_distribution.insert(metrics.geolocation.clone(), 1);    
                            let weight = WeightCalculation::calculate_weight::<T>(NodeType::GpuMiner, &metrics, all_nodes_metrics, &geo_distribution);
                            total_weight += weight as u32; // Accumulate weight

                            let current_block_number = <frame_system::Pallet<T>>::block_number();
                            let buffer = 300u32;
                            let blocks_online = ExecutionPallet::<T>::block_numbers(miner.node_id.clone());
                            
                            if let Some(blocks) = blocks_online {
                                if let Some(&last_block) = blocks.last() {
                                    let difference = current_block_number - last_block;
                                    if difference > buffer.into() { // Ensure buffer is of the correct type
                                        total_weight -= weight as u32; // Accumulate weight
                                    }
                                }
                            }

                            // Retrieve node info using the linked node ID
                            if let Some(linked_node_info) = RegistrationPallet::<T>::get_node_registration_info(linked_node_id.clone().to_vec()) {
                                let linked_miner_ss58 = AccountId32::new(linked_node_info.owner.encode().try_into().unwrap_or_default()).to_ss58check();
                                linked_gpu_miners_ss58.push(linked_miner_ss58.into());
                                linked_gpu_miners_node_id.push(linked_node_id.clone());
                                linked_gpu_miners_node_types.push(NodeType::GpuMiner);
                                linked_gpu_miners_weights.push(weight as u16);
                            }
                        }
                    }
                    // Normalize weight for the main node based on linked nodes
                    let normalized_weight = total_weight / linked_node_ids.len() as u32;
                    gpu_weights.push(normalized_weight as u16);
                } else {
                    // Handle the case where there are no linked nodes
                    if let Some(metrics) = ExecutionPallet::<T>::get_node_metrics(miner.node_id.clone()) {
                        geo_distribution.insert(metrics.geolocation.clone(), 1);    
                        let mut weight = WeightCalculation::calculate_weight::<T>(NodeType::GpuMiner, &metrics, all_nodes_metrics, &geo_distribution);
                        let current_block_number = <frame_system::Pallet<T>>::block_number();
                        let buffer = 300u32;
                        let blocks_online = ExecutionPallet::<T>::block_numbers(miner.node_id.clone());
                        
                        if let Some(blocks) = blocks_online {
                            if let Some(&last_block) = blocks.last() {
                                let difference = current_block_number - last_block;
                                if difference > buffer.into() { // Ensure buffer is of the correct type
                                    weight = 0; // Accumulate weight
                                }
                            }
                        }
                        gpu_weights.push(weight as u16);
                    } else {
                        log::info!("Node metrics not found for GPU miner: {:?}", miner.node_id);
                    }
                }

                // Other logic remains the same...
                let miner_ss58 = AccountId32::new(miner.owner.encode().try_into().unwrap_or_default()).to_ss58check();
                gpu_miners_node_id.push(miner.node_id.clone());
                gpu_miners_node_types.push(miner.node_type.clone());
                gpu_nodes_ss58.push(miner_ss58.clone().into());

                // Update Bittensor UIDs
                for uid in uids.iter() {
                    if uid.substrate_address.to_ss58check() == miner_ss58 {
                        all_uids_on_bittensor.push(uid.id);
                        all_weights_on_bitensor.push(*gpu_weights.last().unwrap_or(&0) as u16); // Use the last calculated weight
                    }
                }
            }

            (gpu_weights, gpu_nodes_ss58, gpu_miners_node_id, 
            gpu_miners_node_types, all_uids_on_bittensor, all_weights_on_bitensor,
            linked_gpu_miners_weights, linked_gpu_miners_ss58, linked_gpu_miners_node_id, linked_gpu_miners_node_types)
        }

        // New method to calculate weights specifically for compute miners
        fn calculate_compute_miner_weights(
            all_miners: &Vec<NodeInfo<BlockNumberFor<T>, T::AccountId>>, 
            all_nodes_metrics: &Vec<NodeMetricsData>, 
            uids: &Vec<UID>
        ) -> (Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>, Vec<u16>, Vec<u16>,
            Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>) {
            
            let mut compute_weights: Vec<u16> = Vec::new();
            let mut compute_nodes_ss58: Vec<Vec<u8>> = Vec::new();
            let mut compute_miners_node_id: Vec<Vec<u8>> = Vec::new();
            let mut compute_miners_node_types: Vec<NodeType> = Vec::new();
            let mut all_uids_on_bittensor: Vec<u16> = Vec::new();
            let mut all_weights_on_bitensor: Vec<u16> = Vec::new();

            let mut linked_compute_miners_ss58: Vec<Vec<u8>> = Vec::new();
            let mut linked_compute_miners_node_id: Vec<Vec<u8>> = Vec::new();
            let mut linked_compute_miners_node_types: Vec<NodeType> = Vec::new();
            let mut linked_compute_miners_weights: Vec<u16> = Vec::new();
            let mut geo_distribution: BTreeMap<Vec<u8>, u32> = BTreeMap::new();

            for miner in all_miners {
                if miner.node_type != NodeType::ComputeMiner {
                    continue;
                }

                // Retrieve linked nodes for the main node
                let mut linked_node_ids = LinkedNodes::<T>::get(&miner.node_id);
                let mut total_weight = 0u32;

                if !linked_node_ids.is_empty() { // Check if there are linked nodes
                    // Remove linked nodes without metrics
                    linked_node_ids.retain(|linked_node_id| ExecutionPallet::<T>::get_node_metrics(linked_node_id.clone()).is_some());

                    // Calculate weights for linked nodes
                    for linked_node_id in &linked_node_ids {
                        if let Some(metrics) = ExecutionPallet::<T>::get_node_metrics(linked_node_id.clone()) {
                            geo_distribution.insert(metrics.geolocation.clone(), 1);    
                            let weight = WeightCalculation::calculate_weight::<T>(NodeType::ComputeMiner, &metrics, all_nodes_metrics, &geo_distribution);
                            total_weight += weight as u32; // Accumulate weight

                            let current_block_number = <frame_system::Pallet<T>>::block_number();
                            let buffer = 300u32;
                            let blocks_online = ExecutionPallet::<T>::block_numbers(miner.node_id.clone());
                            
                            if let Some(blocks) = blocks_online {
                                if let Some(&last_block) = blocks.last() {
                                    let difference = current_block_number - last_block;
                                    if difference > buffer.into() { // Ensure buffer is of the correct type
                                        total_weight -= weight as u32; // Accumulate weight
                                    }
                                }
                            }

                            // Retrieve node info using the linked node ID
                            if let Some(linked_node_info) = RegistrationPallet::<T>::get_node_registration_info(linked_node_id.clone().to_vec()) {
                                let linked_miner_ss58 = AccountId32::new(linked_node_info.owner.encode().try_into().unwrap_or_default()).to_ss58check();
                                linked_compute_miners_ss58.push(linked_miner_ss58.into());
                                linked_compute_miners_node_id.push(linked_node_id.clone());
                                linked_compute_miners_node_types.push(NodeType::ComputeMiner);
                                linked_compute_miners_weights.push(weight as u16);
                            }
                        }
                    }
                    // Normalize weight for the main node based on linked nodes
                    let normalized_weight = total_weight / linked_node_ids.len() as u32;
                    compute_weights.push(normalized_weight as u16);
                } else {
                    // Handle the case where there are no linked nodes
                    if let Some(metrics) = ExecutionPallet::<T>::get_node_metrics(miner.node_id.clone()) {
                        geo_distribution.insert(metrics.geolocation.clone(), 1);    
                        let mut weight = WeightCalculation::calculate_weight::<T>(NodeType::ComputeMiner, &metrics, all_nodes_metrics, &geo_distribution);
                        let current_block_number = <frame_system::Pallet<T>>::block_number();
                        let buffer = 300u32;
                        let blocks_online = ExecutionPallet::<T>::block_numbers(miner.node_id.clone());
                        
                        if let Some(blocks) = blocks_online {
                            if let Some(&last_block) = blocks.last() {
                                let difference = current_block_number - last_block;
                                if difference > buffer.into() { // Ensure buffer is of the correct type
                                    weight = 0; // Accumulate weight
                                }
                            }
                        }
                        compute_weights.push(weight as u16);
                    } else {
                        log::info!("Node metrics not found for compute miner: {:?}", miner.node_id);
                    }
                }

                // Other logic remains the same...
                let miner_ss58 = AccountId32::new(miner.owner.encode().try_into().unwrap_or_default()).to_ss58check();
                compute_miners_node_id.push(miner.node_id.clone());
                compute_miners_node_types.push(miner.node_type.clone());
                compute_nodes_ss58.push(miner_ss58.clone().into());

                // Update Bittensor UIDs
                for uid in uids.iter() {
                    if uid.substrate_address.to_ss58check() == miner_ss58 {
                        all_uids_on_bittensor.push(uid.id);
                        all_weights_on_bitensor.push(*compute_weights.last().unwrap_or(&0) as u16); // Use the last calculated weight
                    }
                }
            }

            (compute_weights, compute_nodes_ss58, compute_miners_node_id, 
            compute_miners_node_types, all_uids_on_bittensor, all_weights_on_bitensor,
            linked_compute_miners_weights, linked_compute_miners_ss58, linked_compute_miners_node_id, linked_compute_miners_node_types)
        }

        // Refactored main weight calculation method
        pub fn calculate_weights_for_nodes() -> (Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>,
            Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>, 
            Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>, 
            Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>, 
            Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>, 
            Vec<u16>, Vec<u16>,
            Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>,
            Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>,
            Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>,
            Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>,
            Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>,
        ) {
            let mut uids = MetagraphPallet::<T>::get_uids();
            let mut all_nodes = RegistrationPallet::<T>::get_all_nodes_with_min_staked();

            // Collect metrics for all miners
            let mut all_nodes_metrics: Vec<NodeMetricsData> = Vec::new();
            all_nodes.retain(|node| {
                if let Some(metrics) = ExecutionPallet::<T>::get_node_metrics(node.node_id.clone()) {
                    all_nodes_metrics.push(metrics);
                    true // Keep the node
                } else {
                    if let Ok(node_id_str) = String::from_utf8(node.node_id.clone()) {
                        log::info!("Node metrics not found for miner: {:?}", node_id_str);
                    }
                    false // Remove the node
                }
            });

            // Calculate weights for different miner types
            let (storage_weights, storage_nodes_ss58, storage_miners_node_id, storage_miners_node_types, 
                 storage_uids, storage_weights_on_bittensor,
                 linked_storage_miners_weights, linked_storage_miners_ss58, linked_storage_miners_node_id, linked_storage_miners_node_types
                ) = Self::calculate_storage_miner_weights(&all_nodes, &all_nodes_metrics, &uids);

            let (compute_weights, compute_nodes_ss58, compute_miners_node_id, compute_miners_node_types, 
                 compute_uids, compute_weights_on_bittensor,
                 linked_compute_miners_weights, linked_compute_miners_ss58, linked_compute_miners_node_id, linked_compute_miners_node_types
                ) = Self::calculate_compute_miner_weights(&all_nodes, &all_nodes_metrics, &uids);

            let (gpu_weights, gpu_nodes_ss58, gpu_miners_node_id, gpu_miners_node_types, 
                gpu_uids, gpu_weights_on_bittensor,
                linked_gpu_miners_weights, linked_gpu_miners_ss58, linked_gpu_miners_node_id, linked_gpu_miners_node_types
            ) = Self::calculate_gpu_miner_weights(&all_nodes, &all_nodes_metrics, &uids);
   
            // Calculate weights for different validator types
            let (validator_weights, validator_nodes_ss58, validator_miners_node_id, validator_miners_node_types, 
                validator_uids, _validator_weights_on_bittensor,
                linked_validator_miners_weights, linked_validator_miners_ss58, linked_validator_miners_node_id, linked_validator_miners_node_types
            ) = Self::calculate_validator_weights(&all_nodes, &all_nodes_metrics, &uids);

            let (storage_s3_weights, storage_s3_nodes_ss58, storage_s3_miners_node_id, storage_s3_miners_node_types, 
                storage_s3_uids, storage_s3_weights_on_bittensor,
                linked_storage_s3_miners_weights, linked_storage_s3_miners_ss58, linked_storage_s3_miners_node_id, linked_storage_s3_miners_node_types
            ) = Self::calculate_storage_s3_weights(&all_nodes, &all_nodes_metrics, &uids);
   
            let mut all_uids_on_bittensor: Vec<u16> = [storage_uids, compute_uids, gpu_uids, storage_s3_uids].concat();
            let mut all_weights_on_bitensor: Vec<u16> = [storage_weights_on_bittensor, compute_weights_on_bittensor, gpu_weights_on_bittensor, storage_s3_weights_on_bittensor].concat();

            // remove validator uids
            uids.retain(|uid| !validator_uids.contains(&uid.id));

            // Remove UIDs with role equal to Validator
            uids.retain(|uid| uid.role != Role::Validator);
         
            // After checking all miners, add unmatched UIDs with weight 0
            for uid in uids.iter() {
                if !all_uids_on_bittensor.contains(&uid.id) {
                    all_uids_on_bittensor.push(uid.id);
                    all_weights_on_bitensor.push(0);
                }
            }
            // Combine results
            (
                storage_weights, storage_nodes_ss58, storage_miners_node_id, storage_miners_node_types,
                compute_weights, compute_nodes_ss58, compute_miners_node_id, compute_miners_node_types,
                gpu_weights, gpu_nodes_ss58, gpu_miners_node_id, gpu_miners_node_types,
                validator_weights, validator_nodes_ss58, validator_miners_node_id, validator_miners_node_types,
                storage_s3_weights, storage_s3_nodes_ss58, storage_s3_miners_node_id, storage_s3_miners_node_types,
                all_uids_on_bittensor, all_weights_on_bitensor,
                linked_storage_miners_weights, linked_storage_miners_ss58, linked_storage_miners_node_id, linked_storage_miners_node_types,
                linked_compute_miners_weights, linked_compute_miners_ss58, linked_compute_miners_node_id, linked_compute_miners_node_types,
                linked_gpu_miners_weights, linked_gpu_miners_ss58, linked_gpu_miners_node_id, linked_gpu_miners_node_types,
                linked_validator_miners_weights, linked_validator_miners_ss58, linked_validator_miners_node_id, linked_validator_miners_node_types,
                linked_storage_s3_miners_weights, linked_storage_s3_miners_ss58, linked_storage_s3_miners_node_id, linked_storage_s3_miners_node_types
            )
        }

        pub fn submit_weight_extrinsic(block_number: BlockNumberFor<T>) -> Result<(), &'static str> {
            // Get RPC URL from config
            let rpc_url = T::FinneyRpcUrl::get();
        
            // Call get_signed weight hex to get the hex string
            let hex_result = Self::get_signed_weight_hex("http://127.0.0.1:9944", block_number).map_err(|e| {
                log::error!("❌ Failed to get signed weight hex: {:?}", e);
                "Failed to get signed weight hex"
            })?;
        
            // Check if submission is enabled before proceeding
            if UtilsPallet::<T>::weight_submission_enabled() {
                // Now use the hex_result in the submit_to_chain function
                match Self::submit_to_chain(&rpc_url, &hex_result) {
                    Ok(_) => {
                        log::info!("✅ Successfully submitted the signed extrinsic for weights");
                        Ok(())
                    }
                    Err(e) => {
                        log::error!("❌ Failed to submit the extrinsic for weights: {:?}", e);
                        Err("Failed to submit the extrinsic")
                    }
                }
            }else {
                log::info!("❌ Weight submission is disabled");
                Err("Weight submission is disabled")
            }
        }

        /// Fetch versionkey from the Finney API
        pub fn fetch_version_key() -> Result<u16, http::Error> {
            let url = T::FinneyRpcUrl::get();

            let json_payload = format!(r#"{{
                "id": 1,
                "jsonrpc": "2.0",
                "method": "state_getStorage",
                "params": ["{}"]
            }}"#, T::VersionKeyStorageKey::get());

            let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(2_000));

            let body = vec![json_payload];
            let request = sp_runtime::offchain::http::Request::post(url, body);

            let pending = request
                .add_header("Content-Type", "application/json")
                .deadline(deadline)
                .send()
                .map_err(|err| {
                    log::error!("❌ Error making Request: {:?}", err);
                    sp_runtime::offchain::http::Error::IoError
                })?;

            let response = pending
                .try_wait(deadline)
                .map_err(|err| {
                    log::error!("❌ Error getting Response: {:?}", err);
                    sp_runtime::offchain::http::Error::DeadlineReached
                })??;

            if response.code != 200 {
                log::error!("❌ Unexpected status code: {}", response.code);
                return Err(http::Error::Unknown);
            }

            let body = response.body().collect::<Vec<u8>>();
            let body_str = sp_std::str::from_utf8(&body).map_err(|_| {
                log::error!("❌ Response body is not valid UTF-8");
                http::Error::Unknown
            })?;

            // Parse the JSON response to extract the "result" field
            let result_start = body_str.find("\"result\":\"").ok_or_else(|| {
                log::error!("❌ 'result' field not found in JSON fetching version key");
                http::Error::Unknown
            })? + 10;

            let result_end = body_str[result_start..].find("\"").ok_or_else(|| {
                log::error!("❌ End of 'result' field not found");
                http::Error::Unknown
            })? + result_start;

            let result_str = &body_str[result_start..result_end];

            // Decode the hex string and reverse the bytes
            let hex_str = result_str.strip_prefix("0x").unwrap_or(result_str); // Remove "0x" prefix if it exists
            let bytes = hex::decode(hex_str).map_err(|_| {
                log::error!("❌ Failed to decode hex string");
                http::Error::Unknown
            })?;

            // Convert the first 2 bytes (little-endian) to u16
            if bytes.len() < 2 {
                log::error!("❌ Not enough bytes to parse u16");
                return Err(http::Error::Unknown);
            }

            let value = u16::from_le_bytes([bytes[0], bytes[1]]); // Interpret the first two bytes as little-endian
  
            Ok(value)
        }

        pub fn get_signed_weight_hex(rpc_url: &str, block_number: BlockNumberFor<T>) -> Result<String, http::Error> {
            let (
                weights, 
                storage_nodes_ss58,  
                storage_miners_node_id,
                storage_miners_node_types, 

                compute_weights, 
                compute_all_nodes_ss58,  
                compute_all_miners_node_id,
                compute_all_miners_node_types, 

                gpu_weights, 
                gpu_all_nodes_ss58,  
                gpu_all_miners_node_id,
                gpu_all_miners_node_types, 

                validator_weights, 
                validator_all_nodes_ss58,  
                validator_all_miners_node_id,
                validator_all_miners_node_types, 

                storage_s3_weights, 
                storage_s3_all_nodes_ss58,  
                storage_s3_all_miners_node_id,
                storage_s3_all_miners_node_types, 

                all_dests_on_bittensor, 
                all_weights_on_bitensor,

                linked_storage_miners_weights, linked_storage_miners_ss58, linked_storage_miners_node_id, linked_storage_miners_node_types,
                linked_compute_miners_weights, linked_compute_miners_ss58, linked_compute_miners_node_id, linked_compute_miners_node_types,
                linked_gpu_miners_weights, linked_gpu_miners_ss58, linked_gpu_miners_node_id, linked_gpu_miners_node_types,
                linked_validator_miners_weights, linked_validator_miners_ss58, linked_validator_miners_node_id, linked_validator_miners_node_types,
                linked_storage_s3_miners_weights, linked_storage_s3_miners_ss58, linked_storage_s3_miners_node_id, linked_storage_s3_miners_node_types
                
            ) :
                (Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>,
                Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>,
                Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>,
                Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>,
                Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>,
                Vec<u16>, Vec<u16>,
                Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>,
                Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>,
                Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>,
                Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>,
                Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>,
                
            ) = 
                Self::calculate_weights_for_nodes();
            
            // update rankings in ranking pallet for both instances
            let _ = RankingsPallet::<T>::save_rankings_update([weights.clone(), linked_storage_miners_weights.clone()].concat(),
                        [storage_nodes_ss58.clone(), linked_storage_miners_ss58.clone()].concat(), 
                        [storage_miners_node_id.clone(), linked_storage_miners_node_id.clone()].concat(), 
                        [storage_miners_node_types.clone(), linked_storage_miners_node_types.clone()].concat(), block_number, 1u32);

            // let _ = RankingsPallet::<T, pallet_rankings::Instance2>::save_rankings_update([compute_weights.clone(), linked_compute_miners_weights.clone()].concat(), 
            //             [compute_all_nodes_ss58.clone(), linked_compute_miners_ss58.clone()].concat(), [compute_all_miners_node_id.clone(), linked_compute_miners_node_id.clone()].concat(), [compute_all_miners_node_types.clone(), linked_compute_miners_node_types.clone()].concat(), block_number, 2u32);

            let _ = RankingsPallet::<T, pallet_rankings::Instance3>::save_rankings_update([validator_weights.clone(), linked_validator_miners_weights.clone()].concat(), 
                    [validator_all_nodes_ss58.clone(), linked_validator_miners_ss58.clone()].concat(), [validator_all_miners_node_id.clone(), linked_validator_miners_node_id.clone()].concat(), [validator_all_miners_node_types.clone(), linked_validator_miners_node_types.clone()].concat(), block_number, 3u32); 

            // let _ = RankingsPallet::<T, pallet_rankings::Instance4>::save_rankings_update([gpu_weights.clone(), linked_gpu_miners_weights.clone()].concat(), 
            //         [gpu_all_nodes_ss58.clone(), linked_gpu_miners_ss58.clone()].concat(), [gpu_all_miners_node_id.clone(), linked_gpu_miners_node_id.clone()].concat(), [gpu_all_miners_node_types.clone(), linked_gpu_miners_node_types.clone()].concat(), block_number,  4u32);

            // let _ = RankingsPallet::<T, pallet_rankings::Instance5>::save_rankings_update([storage_s3_weights.clone(), linked_storage_s3_miners_weights.clone()].concat(), 
            //         [storage_s3_all_nodes_ss58.clone(), linked_storage_s3_miners_ss58.clone()].concat(), [storage_s3_all_miners_node_id.clone(), linked_storage_s3_miners_node_id.clone()].concat(), [storage_s3_all_miners_node_types.clone(), linked_storage_s3_miners_node_types.clone()].concat(), block_number, 5u32);

            // Ensure both vectors have the same length and are greater than 1
            if all_dests_on_bittensor.len() != all_weights_on_bitensor.len() {
                log::error!("❌ Destinations and weights must have the same length ");
                return Err(http::Error::Unknown);
            }

            // Ensure both vectors are greater than 0
            if all_dests_on_bittensor.len() <= 0 {
                log::error!("❌ Destinations and weights must be greater than 1");
                return Err(http::Error::Unknown);
            }

            let version_key_res = match Self::fetch_version_key() {
                Ok(key) => key,
                Err(_) => T::Versionkey::get().try_into().unwrap_or(0), // Convert to u16 if needed
            };

            let weights_string = all_weights_on_bitensor.iter().map(|w| w.to_string()).collect::<Vec<_>>().join(", ");
            let dests_string = all_dests_on_bittensor.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(", ");
            let net_uid = T::NetUid::get();
            let net_uid_string = format!("{}", net_uid);
            let version_key = format!("{}", version_key_res);
            let default_spec_version = T::DefaultSpecVersion::get();
            let default_genesis_hash = T::DefaultGenesisHash::get();            
            let finney_rpc_url = T::FinneyRpcUrl::get(); // Get the Finney RPC URL
            
            let rpc_payload = format!(
                r#"{{
                    "jsonrpc": "2.0",
                    "method": "submit_weights",
                    "params": [{{
                        "netuid": {},
                        "dests": [{}],
                        "weights": [{}],
                        "version_key": {},
                        "default_spec_version": {},
                        "default_genesis_hash": "{}",
                        "finney_api_url": "{}"
                    }}],
                    "id": 1
                }}"#,
                net_uid_string, dests_string, weights_string, version_key, default_spec_version, default_genesis_hash, finney_rpc_url
            );

            // Convert the JSON value to a string
            let rpc_payload_string = rpc_payload.to_string();

            let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(2_000));
            
            let body = vec![rpc_payload_string];
            let request = sp_runtime::offchain::http::Request::post(rpc_url, body);

            let pending = request
                .add_header("Content-Type", "application/json")
                .deadline(deadline)
                .send()
                .map_err(|err| {
                    log::error!("❌ Error making Request: {:?}", err);
                    sp_runtime::offchain::http::Error::IoError
                })?;

            let response = pending
                .try_wait(deadline)
                .map_err(|err| {
                    log::error!("❌ Error getting Response: {:?}", err);
                    sp_runtime::offchain::http::Error::DeadlineReached
                })??;

            if response.code != 200 {
                log::error!("❌ RPC call failed with status code: {}", response.code);
                return Err(http::Error::Unknown);
            }

            let body = response.body().collect::<Vec<u8>>();
            let body_str = sp_std::str::from_utf8(&body).map_err(|_| {
                log::error!("❌ Response body is not valid UTF-8");
                http::Error::Unknown
            })?;

            // Parse the JSON response
            let json_response: Value = serde_json::from_str(body_str).map_err(|_| {
                log::error!("❌ Failed to parse JSON response");
                http::Error::Unknown
            })?;

            // Extract the hex string from the result field
            let hex_result = json_response.get("result")
                .and_then(Value::as_str) // Get the result as a string
                .ok_or_else(|| {
                    log::error!("❌ 'result' field not found in response");
                    http::Error::Unknown
                })?;


            // Return the hex string
            Ok(hex_result.to_string())
        }        

        pub fn submit_to_chain(rpc_url: &str, encoded_call_data: &str) -> Result<(), http::Error> {
            let rpc_payload = format!(r#"{{
                "id": 1,
                "jsonrpc": "2.0",
                "method": "author_submitExtrinsic",
                "params": ["{}"]
            }}"#, encoded_call_data);

            let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(2_000));
            
            let body = vec![rpc_payload];
            let request = sp_runtime::offchain::http::Request::post(rpc_url, body);

            let pending = request
                .add_header("Content-Type", "application/json")
                .deadline(deadline)
                .send()
                .map_err(|err| {
                    log::error!("❌ Error making Request: {:?}", err);
                    sp_runtime::offchain::http::Error::IoError
                })?;

            let response = pending
                .try_wait(deadline)
                .map_err(|err| {
                    log::error!("❌ Error getting Response: {:?}", err);
                    sp_runtime::offchain::http::Error::DeadlineReached
                })??;

            if response.code != 200 {
                log::error!("❌ RPC call failed with status code: {}", response.code);
                return Err(http::Error::Unknown);
            }

            let body = response.body().collect::<Vec<u8>>();
            let body_str = sp_std::str::from_utf8(&body).map_err(|_| {
                log::error!("❌ Response body is not valid UTF-8");
                http::Error::Unknown
            })?;

            log::info!("response of weight submission is {:?}", body_str);

            Ok(())
        }        
    }
}
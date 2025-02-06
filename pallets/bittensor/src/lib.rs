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
    use pallet_registration::{Pallet as RegistrationPallet, NodeRegistration, NodeType};
    use pallet_utils::Pallet as UtilsPallet;
    use scale_info::prelude::string::String;
    use sp_core::crypto::Ss58Codec;
    use sp_io;
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
                      + pallet_rankings::Config<pallet_rankings::Instance2>
                      + pallet_rankings::Config<pallet_rankings::Instance3> {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        #[pallet::constant]
        type FinneyRpcUrl: Get<&'static str>;

        #[pallet::constant]
        type VersionKeyStorageKey: Get<&'static str>;
        
        #[pallet::constant]
        type BittensorCallSubmission: Get<u32>; // Add this line for the new constant

        #[pallet::constant]
        type NetUid: Get<u16>; 
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
                    let node_info = NodeRegistration::<T>::get(&node_id);	
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
        pub fn calculate_weights_for_miners() -> (Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>,Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>, Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>, Vec<u16>, Vec<u16>) {
            let uids = MetagraphPallet::<T>::get_uids();
            let all_miners = RegistrationPallet::<T>::get_all_miners_with_min_staked();

            let mut storage_weights: Vec<u16> = Vec::new();
            let mut storage_nodes_ss58: Vec<Vec<u8>> = Vec::new();
            let mut storage_miners_node_id: Vec<Vec<u8>> = Vec::new();
            let mut storage_miners_node_types: Vec<NodeType> = Vec::new();

            let mut compute_weights: Vec<u16> = Vec::new();
            let mut compute_nodes_ss58: Vec<Vec<u8>> = Vec::new();
            let mut compute_miners_node_id: Vec<Vec<u8>> = Vec::new();
            let mut compute_miners_node_types: Vec<NodeType> = Vec::new();

            let mut validator_weights: Vec<u16> = Vec::new();
            let mut validator_nodes_ss58: Vec<Vec<u8>> = Vec::new();
            let mut validator_miners_node_id: Vec<Vec<u8>> = Vec::new();
            let mut validator_miners_node_types: Vec<NodeType> = Vec::new();

            let mut all_uids_on_bittensor: Vec<u16> = Vec::new();
            let mut all_weights_on_bitensor: Vec<u16> = Vec::new();

            // Collect metrics for all miners
            let mut all_nodes_metrics: Vec<NodeMetricsData> = Vec::new();

            for miner in &all_miners {
                if let Some(metrics) = ExecutionPallet::<T>::get_node_metrics(miner.node_id.clone()) {
                    all_nodes_metrics.push(metrics);
                } else {
                    if let Ok(node_id_str) = String::from_utf8(miner.node_id.clone()) {
                        log::info!("Node metrics not found for miner: {:?}", node_id_str);
                    }
                }
            }

            let all_miners = RegistrationPallet::<T>::get_all_miners_with_min_staked();

            for miner in &all_miners {
                let miner_ss58 = AccountId32::new(miner.owner.encode().try_into().unwrap_or_default()).to_ss58check();
                if let Some(metrics) = ExecutionPallet::<T>::get_node_metrics(miner.node_id.clone()) {
                    let mut geo_distribution: BTreeMap<Vec<u8>, u32> = BTreeMap::new(); // Populate this as needed
                    geo_distribution.insert(metrics.geolocation.clone(), 1);    
                    let weight = WeightCalculation::calculate_weight(&metrics, &all_nodes_metrics, &geo_distribution);
                    if miner.node_type == NodeType::StorageMiner{
                        storage_weights.push(weight as u16); // Assuming you want to store the weight as u16
                        storage_miners_node_id.push(miner.node_id.clone());
                        storage_miners_node_types.push(miner.node_type.clone());
                        storage_nodes_ss58.push(miner_ss58.clone().into());
                    }else if miner.node_type == NodeType::ComputeMiner{
                        compute_weights.push(weight as u16); // Assuming you want to store the weight as u16
                        compute_miners_node_id.push(miner.node_id.clone());
                        compute_miners_node_types.push(miner.node_type.clone());
                        compute_nodes_ss58.push(miner_ss58.clone().into());
                    }else {
                        validator_weights.push(weight as u16); // Assuming you want to store the weight as u16
                        validator_miners_node_id.push(miner.node_id.clone());
                        validator_miners_node_types.push(miner.node_type.clone());
                        validator_nodes_ss58.push(miner_ss58.clone().into());
                    }

                    for uid in uids.iter() {
                        if uid.substrate_address.to_ss58check() == miner_ss58 {
                            all_uids_on_bittensor.push(uid.id);
                            all_weights_on_bitensor.push(weight as u16);
                        }
                    }
                } else {
                    if let Ok(node_id_str) = String::from_utf8(miner.node_id.clone()) {
                        log::info!("Node metrics not found for miner: {:?}", node_id_str);
                    }
                }
            }

            (storage_weights, storage_nodes_ss58,  storage_miners_node_id, storage_miners_node_types, 
             compute_weights, compute_nodes_ss58, compute_miners_node_id, compute_miners_node_types, 
             validator_weights, validator_nodes_ss58, validator_miners_node_id, validator_miners_node_types, 
             all_uids_on_bittensor, all_weights_on_bitensor)
        }

        pub fn submit_weight_extrinsic(block_number: BlockNumberFor<T>) -> Result<(), &'static str> {
            // Get RPC URL from config
            let rpc_url = T::FinneyRpcUrl::get();
        
            // Call get_signed weight hex to get the hex string
            let hex_result = Self::get_signed_weight_hex("http://127.0.0.1:9944", block_number).map_err(|e| {
                log::error!("❌ Failed to get signed weight hex: {:?}", e);
                "Failed to get signed weight hex"
            })?;
        
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

                validator_weights, 
                validator_all_nodes_ss58,  
                validator_all_miners_node_id,
                validator_all_miners_node_types, 

                all_dests_on_bittensor, 
                all_weights_on_bitensor) :
                (Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>, Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>, Vec<u16>, Vec<Vec<u8>>, Vec<Vec<u8>>, Vec<NodeType>, Vec<u16>, Vec<u16>) = 
                Self::calculate_weights_for_miners();
            
            // update rankings in ranking pallet for both instances
            let _ = RankingsPallet::<T>::save_rankings_update(weights.clone(),
                        storage_nodes_ss58.clone(), storage_miners_node_id.clone(), storage_miners_node_types.clone(), block_number);
                        
            let _ = RankingsPallet::<T, pallet_rankings::Instance2>::save_rankings_update(compute_weights.clone(), 
                        compute_all_nodes_ss58.clone(), compute_all_miners_node_id.clone(), compute_all_miners_node_types.clone(), block_number);

            let _ = RankingsPallet::<T, pallet_rankings::Instance3>::save_rankings_update(validator_weights.clone(), 
                    validator_all_nodes_ss58.clone(), validator_all_miners_node_id.clone(), validator_all_miners_node_types.clone(), block_number);

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
                Err(_) => 0,
            };

            let weights_string = all_weights_on_bitensor.iter().map(|w| w.to_string()).collect::<Vec<_>>().join(", ");
            let dests_string = all_dests_on_bittensor.iter().map(|d| d.to_string()).collect::<Vec<_>>().join(", ");
            let net_uid = T::NetUid::get();
            let net_uid_string = format!("{}", net_uid);
            let version_key = format!("{}", version_key_res);

            let rpc_payload = format!(
                r#"{{
                    "jsonrpc": "2.0",
                    "method": "submit_weights",
                    "params": [{{
                        "netuid": {},
                        "dests": [{}],
                        "weights": [{}],
                        "version_key": {}
                    }}],
                    "id": 1
                }}"#,
                net_uid_string, dests_string, weights_string, version_key
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
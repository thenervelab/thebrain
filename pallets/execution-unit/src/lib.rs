#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;
pub mod weights;
use sp_std::vec::Vec;
pub use types::*;
mod system_info; 
pub mod types;
use sp_runtime::SaturatedConversion;
pub mod weight_calculation;
use sp_core::offchain::KeyTypeId;

/// Defines application identifier for crypto keys of this module.
///
/// Every module that deals with signatures needs to declare its unique identifier for
/// its crypto keys.
/// When offchain worker is signing transactions it's going to request keys of type
/// `KeyTypeId` from the keystore and use the ones it finds to sign the transaction.
/// The keys can be inserted manually via RPC (see `author_insertKey`).
pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"hips");

/// Based on the above `KeyTypeId` we need to generate a pallet-specific crypto type wrappers.
/// We can use from supported crypto kinds (`sr25519`, `ed25519` and `ecdsa`) and augment
/// the types with this pallet-specific identifier.
pub mod crypto {
	use super::KEY_TYPE;
	use sp_core::sr25519::Signature as Sr25519Signature;
	use sp_runtime::{
		app_crypto::{app_crypto, sr25519},
		traits::Verify,
		MultiSignature, MultiSigner,
	};
	app_crypto!(sr25519, KEY_TYPE);

	pub struct TestAuthId;

	impl frame_system::offchain::AppCrypto<MultiSigner, MultiSignature> for TestAuthId {
		type RuntimeAppPublic = Public;
		type GenericSignature = sp_core::sr25519::Signature;
		type GenericPublic = sp_core::sr25519::Public;
	}

	impl frame_system::offchain::AppCrypto<<Sr25519Signature as Verify>::Signer, Sr25519Signature>
		for TestAuthId
	{
		type RuntimeAppPublic = Public;
		type GenericSignature = sp_core::sr25519::Signature;
		type GenericPublic = sp_core::sr25519::Public;
	}
}

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use num_traits::float::FloatCore;
	use crate::weights::WeightInfo;
	use frame_support::{
		pallet_prelude::*,
		traits::Randomness,
	};
	use frame_system::{
		offchain::{
			AppCrypto, SendTransactionTypes, SendUnsignedTransaction, 
			Signer, SigningTypes, 
		},
		pallet_prelude::*,
	};
	use pallet_babe::RandomnessFromOneEpochAgo;
	use pallet_ipfs_pin::Pallet as IpfsPinPallet;
	use pallet_metagraph::UIDs;
	use pallet_registration::{NodeRegistration, NodeType};
	use sp_core::offchain::StorageKind;
	use sp_runtime::{
		format,
		offchain::{http, Duration, storage_lock::{StorageLock, BlockAndTime}},
		traits::Zero,
		AccountId32,
	};
	use sp_std::prelude::*;
	// use pallet_credits::Pallet as CreditsPallet;
	use pallet_rankings::Pallet as RankingsPallet;
	use sp_std::collections::btree_map::BTreeMap;

	const STORAGE_KEY: &[u8] = b"execution-unit::last-run";
	const DUMMY_REQUEST_BODY: &[u8; 78] = b"{\"id\": 10, \"jsonrpc\": \"2.0\", \"method\": \"chain_getFinalizedHead\", \"params\": []}";

	#[pallet::config]
	pub trait Config: frame_system::Config + 
					  pallet_metagraph::Config + 
					  pallet_babe::Config + 
					  pallet_ipfs_pin::Config + 
					  pallet_marketplace::Config + 
					  pallet_timestamp::Config + 
					  SendTransactionTypes<Call<Self>> + 
					  frame_system::offchain::SigningTypes +
					  pallet_credits::Config + 
					  pallet_compute::Config +
					  pallet_balances::Config +
					  pallet_rankings::Config 
		{
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		/// The identifier type for an offchain worker.
		type AuthorityId: AppCrypto<Self::Public, Self::Signature>;
		
		/// The identifier type for an offchain worker.
		type WeightInfo: WeightInfo;

		#[pallet::constant]
		type LocalRpcUrl: Get<&'static str>;
		
		#[pallet::constant]
		type SystemInfoRpcMethod: Get<&'static str>;

		// Define block time as a runtime parameter
		type BlockTime: Get<u64>;

		// Define block time as a runtime parameter
		type BlockCheckInterval: Get<u32>;
		
		#[pallet::constant]
		type GetReadProofRpcMethod: Get<&'static str>; 

		#[pallet::constant]
		type SystemHealthRpcMethod: Get<&'static str>;

		#[pallet::constant]
		type IPFSBaseUrl: Get<&'static str>;

		#[pallet::constant]
		type UnregistrationBuffer: Get<u32>;
	}

	#[pallet::pallet]
	#[pallet::without_storage_info]
	pub struct Pallet<T>(_);

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		BenchmarkStarted {
			node_id: Vec<u8>,
		},
		BenchmarkCompleted {
			node_id: Vec<u8>,
			metrics: BenchmarkMetrics,
			final_score: u32
		},
		BenchmarkFailed {
			node_id: Vec<u8>,
			error: BenchmarkError,
		},
		NodeSpecsStored{
			node_id: Vec<u8>,
		},
		SignedPayloadProcessed {
			signer: [u8; 32],
			payload: Vec<u8>,
			signature: Vec<u8>,
			node_id: Vec<u8>,
		},
		PinCheckMetricsUpdated {
			node_id: Vec<u8>,
		},
	}

	#[derive(Debug, Encode, Decode, Clone, PartialEq, Eq, TypeInfo)]
	pub enum BenchmarkError {
		LockAcquisitionFailed,
		HardwareCheckFailed,
		BenchmarkExecutionFailed,
		MetricsNotFound
	}

	#[pallet::storage]
	#[pallet::getter(fn block_numbers)]
	/// A vector storing block numbers for each block processed.
	pub type BlockNumbers<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, Vec<BlockNumberFor<T>>, OptionQuery>;

	#[pallet::storage]
	pub(super) type NodeMetrics<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, NodeMetricsData, OptionQuery>;

	#[pallet::storage]
	pub(super) type DowntimeStatus<T: Config> = StorageValue<_, Vec<OfflineStatus<BlockNumberFor<T>>>, ValueQuery>;
	
	// #[pallet::storage]
	// pub type NodeSpecs<T: Config> = StorageMap<
	// 	_, 
	// 	Blake2_128Concat, 
	// 	Vec<u8>,     
	// 	SystemInfo,
	// 	OptionQuery  // Changed from ValueQuery to OptionQuery
	// >;

	const LOCK_BLOCK_EXPIRATION: u32 = 3;
    const LOCK_TIMEOUT_EXPIRATION: u32 = 10000;

	#[pallet::storage]
    #[pallet::getter(fn benchmark_results)]
    pub(super) type BenchmarkResults<T: Config> = StorageMap<
        _, 
        Blake2_128Concat, 
        Vec<u8>,    
        BenchmarkResult<BlockNumberFor<T>>,
        ValueQuery        
    >;

	#[pallet::error]
	pub enum Error<T> {
		MetricsNotFound,
	}

	#[pallet::validate_unsigned]
    impl<T: Config> ValidateUnsigned for Pallet<T> {
        type Call = Call<T>;
        fn validate_unsigned(_source: TransactionSource, call: &Self::Call) -> TransactionValidity {
            match call {
                Call::put_node_specs { node_id, system_info: _, signature: _, node_type: _ } => {
					let current_block = frame_system::Pallet::<T>::block_number();
					// Create a unique hash combining all relevant data
					let mut data = Vec::new();
					data.extend_from_slice(&current_block.encode());
					data.extend_from_slice(node_id);
					let unique_hash = sp_io::hashing::blake2_256(&data);
	
					ValidTransaction::with_tag_prefix("ExecutionUnitOffchain")
						.priority(TransactionPriority::max_value())
						.and_provides(("put_node_specs", unique_hash))
						.longevity(5)
						.propagate(true)
						.build()
                },
                Call::update_pin_check_metrics { node_id, signature: _, total_pin_checks: _, successful_pin_checks: _ } => {
					let current_block = frame_system::Pallet::<T>::block_number();
					// Create a unique hash combining all relevant data
					let mut data = Vec::new();
					data.extend_from_slice(&current_block.encode());
					data.extend_from_slice(node_id);
					let unique_hash = sp_io::hashing::blake2_256(&data);

					ValidTransaction::with_tag_prefix("ExecutionUnitOffchain")
						.priority(TransactionPriority::max_value())
						.and_provides(("update_pin_check_metrics", unique_hash))
						.longevity(5)
						.propagate(true)
						.build()
                },
                Call::update_metrics_data { node_id, signature: _, 
					storage_proof_time_ms: _,
					latency_ms: _,
					peer_count: _,
					failed_challenges_count: _,
					successful_challenges: _,
					total_challenges: _,
					uptime_minutes: _, 
					total_minutes: _, 
					consecutive_reliable_days: _, 
					recent_downtime_hours: _,  
					node_type: _,
				} => {
					let current_block = frame_system::Pallet::<T>::block_number();
					// Create a unique hash combining all relevant data
					let mut data = Vec::new();
					data.extend_from_slice(&current_block.encode());
					data.extend_from_slice(node_id);
					
					let unique_hash = sp_io::hashing::blake2_256(&data);

					ValidTransaction::with_tag_prefix("ExecutionUnitOffchain")
						.priority(TransactionPriority::max_value())
						.and_provides(("update_metrics_data", unique_hash))
						.longevity(5)
						.propagate(true)
						.build()
                },
                Call::update_block_time { node_id, signature: _, block_number: _ } => {
					let current_block = frame_system::Pallet::<T>::block_number();
					// Create a unique hash combining all relevant data
					let mut data = Vec::new();
					data.extend_from_slice(&current_block.encode());
					data.extend_from_slice(node_id);
					let unique_hash = sp_io::hashing::blake2_256(&data);

					ValidTransaction::with_tag_prefix("ExecutionUnitOffchain")
						.priority(TransactionPriority::max_value())
						.and_provides(("update_block_time", unique_hash))
						.longevity(5)
						.propagate(true)
						.build()
                },
				Call::store_offline_status_of_miner { node_id, miner_id, signature: _, block_number } => {
                    // Create a unique hash combining all relevant data
					let mut data = Vec::new();
					data.extend_from_slice(&block_number.encode());
					data.extend_from_slice(node_id);
					data.extend_from_slice(miner_id);
					let unique_hash = sp_io::hashing::blake2_256(&data);
                    ValidTransaction::with_tag_prefix("ExecutionUnitOffchain")
                        .priority(TransactionPriority::max_value())
                        .and_provides(("store_offline_status_of_miner", unique_hash))
                        .longevity(5)
                        .propagate(true)
                        .build()
                },
                _ => InvalidTransaction::Call.into(),
            }
        }
    }

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_initialize(_n: BlockNumberFor<T>) -> Weight {

            Self::handle_incorrect_registration(_n);

            Weight::zero()
        }
		
		fn offchain_worker(block_number: BlockNumberFor<T>) {
			match IpfsPinPallet::<T>::fetch_node_id() {
				Ok(node_id) => {
					let node_info = NodeRegistration::<T>::get(&node_id);    
	
					if node_info.is_some() {
						// Get BABE randomness
						let random_seed = Self::get_babe_randomness();
						
						// update blocktime for uptime tracking
						let check_intetrval = <T as pallet::Config>::BlockCheckInterval::get();
						if block_number % check_intetrval.into() == Zero::zero() {
							Self::call_update_block_time(node_id.clone(), block_number);
						}
	
						let current_block = block_number.saturated_into::<u32>();
						
						// Use BABE randomness instead of block hash for determining execution
						let should_run = Self::should_execute_offchain(current_block, random_seed);
						if should_run {
							// Update last run time
							sp_io::offchain::local_storage_set(
								sp_core::offchain::StorageKind::PERSISTENT,
								STORAGE_KEY,
								&current_block.to_be_bytes(),
							);
							let node_type = node_info.unwrap().node_type;
							// Execute tasks with BABE randomness
							Self::save_hardware_info(node_id.clone(), node_type.clone());
							Self::do_update_metrics_data(node_id.clone(), node_type.clone());

							// commenting purging of nodes bacuse we will have other nodes as well
							// Self::purge_nodes_if_deregistered_on_bittensor(node_id.clone());							
							if node_type == NodeType::Validator {
								Self::process_pending_compute_requests();
								Self::process_pending_storage_requests(node_id.clone(), block_number);
								Self::perform_pin_checks_to_miners(node_id.clone());
								Self::perform_ping_checks_to_miners(node_id.clone());
							}
							log::info!("✅ Executed offchain worker tasks at block {}", current_block);
						} else {
							log::info!("Skipping execution at block {}", current_block);
						}
					}
				}
				Err(e) => {
					log::error!("Error fetching node identity: {:?}", e);
				}
			};
		}
	}
	
	#[pallet::call]
	impl<T: Config> Pallet<T> {
		// #[pallet::call_index(0)]
		// #[pallet::weight(<T as pallet::Config>::WeightInfo::trigger_benchmark())]
		// pub fn trigger_benchmark(origin: OriginFor<T>, node_id: Vec<u8>) -> DispatchResult {
		// 	ensure_signed(origin)?;

		// 	Self::execute_benchmark(node_id)
		// 		.map_err(|_| DispatchError::Other("Benchmark execution failed"))?;

		// 	Ok(())
		// }

		#[pallet::call_index(1)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn put_node_specs(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			system_info: SystemInfo,
			signature: <T as SigningTypes>::Signature,
			_node_type: NodeType
		) -> DispatchResultWithPostInfo {
            ensure_none(origin)?;
			let _signature = signature;
			
			// Check if specs already exist and are the same
			// if let Some(existing_specs) = NodeSpecs::<T>::get(&node_id) {
			// 	if existing_specs == system_info {
			// 		log::info!("✅ System specs unchanged, skipping update");
			// 		return Ok(().into());
			// 	}
			// }

			// Function to create default metrics data
			let create_default_metrics = || {
				let geolocation = system_info.primary_network_interface.as_ref()
					.and_then(|interface| interface.network_details.as_ref())
					.map(|details| details.loc.clone())
					.unwrap_or_default(); // Use default if None
				NodeMetricsData {
					miner_id: node_id.clone(),
					bandwidth_mbps: system_info.network_bandwidth_mb_s,
					// converting mbs into bytes 
					current_storage_bytes: (system_info.storage_total_mb * 1024 * 1024)  - (system_info.storage_free_mb * 1024 * 1024),
					total_storage_bytes: system_info.storage_total_mb * 1024 * 1024,
					geolocation: geolocation.unwrap_or_default(),
					primary_network_interface: system_info.primary_network_interface.clone(), 
					disks: system_info.disks.clone(),
					ipfs_repo_size: system_info.ipfs_repo_size,
					ipfs_storage_max: system_info.ipfs_storage_max,
					cpu_model: system_info.cpu_model.clone(),
					cpu_cores: system_info.cpu_cores,
					memory_mb: system_info.memory_mb,
					free_memory_mb: system_info.free_memory_mb,
					is_sev_enabled: system_info.is_sev_enabled,
					zfs_info: system_info.zfs_info,
					ipfs_zfs_pool_size: system_info.ipfs_zfs_pool_size,
					ipfs_zfs_pool_alloc: system_info.ipfs_zfs_pool_alloc,
					ipfs_zfs_pool_free: system_info.ipfs_zfs_pool_free,
					raid_info: system_info.raid_info,
					vm_count: system_info.vm_count,
					gpu_name: system_info.gpu_name.clone(),
					gpu_memory_mb: system_info.gpu_memory_mb.clone(),
					hypervisor_disk_type: system_info.hypervisor_disk_type.clone(),
					vm_pool_disk_type: system_info.vm_pool_disk_type.clone(),
					disk_info: system_info.disk_info.clone(),
					..Default::default()
				}
			};

			// Check if there is an existing record in storage
			let metrics = NodeMetrics::<T>::get(&node_id).map_or_else(
				create_default_metrics, // Create default metrics if none exist
				|mut existing_metrics| {
					// Update existing metrics
					existing_metrics.bandwidth_mbps = system_info.network_bandwidth_mb_s;
					// converting mbs into bytes 
					existing_metrics.current_storage_bytes = (system_info.storage_total_mb * 1024 * 1024) - (system_info.storage_free_mb * 1024 * 1024);
					existing_metrics.total_storage_bytes = system_info.storage_total_mb * 1024 * 1024;

					 // Calculate storage growth rate
					existing_metrics.storage_growth_rate = if existing_metrics.uptime_minutes == 0 {
						existing_metrics.storage_growth_rate // Avoid division by zero (remains unchanged)
					} else {
						(existing_metrics.current_storage_bytes / existing_metrics.uptime_minutes as u64) as u32
					};

					// Update geolocation only if available
					if let Some(geolocation) = system_info.primary_network_interface.as_ref()
						.and_then(|interface| interface.network_details.as_ref())
						.map(|details| details.loc.clone()) {
						existing_metrics.geolocation = geolocation.unwrap_or_default(); // Use default if None
					}
					existing_metrics.ipfs_zfs_pool_size= system_info.ipfs_zfs_pool_size;
					existing_metrics.ipfs_zfs_pool_alloc= system_info.ipfs_zfs_pool_alloc;
					existing_metrics.ipfs_zfs_pool_free= system_info.ipfs_zfs_pool_free;
					existing_metrics.primary_network_interface= system_info.primary_network_interface.clone(); 
					existing_metrics.disks= system_info.disks.clone();
					existing_metrics.ipfs_repo_size= system_info.ipfs_repo_size;
					existing_metrics.ipfs_storage_max= system_info.ipfs_storage_max;
					existing_metrics.cpu_model= system_info.cpu_model.clone();
					existing_metrics.cpu_cores= system_info.cpu_cores;
					existing_metrics.memory_mb= system_info.memory_mb;
					existing_metrics.free_memory_mb= system_info.free_memory_mb;
					existing_metrics.vm_count= system_info.vm_count;
					existing_metrics.disk_info= system_info.disk_info.clone();
					existing_metrics
				},
			);

			// Insert the updated or new metrics data into storage
			NodeMetrics::<T>::insert(node_id.clone(), metrics);
			// NodeSpecs::<T>::insert(node_id.clone(), system_info);
			log::info!("✅ Storage updated with Specs");

			Self::deposit_event(Event::NodeSpecsStored { node_id });
			Ok(().into())
		}


		#[pallet::call_index(2)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn update_pin_check_metrics(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			_signature: <T as SigningTypes>::Signature,
			total_pin_checks: u32,
			successful_pin_checks: u32,
		) -> DispatchResultWithPostInfo {
			ensure_none(origin)?; // Ensure the call is unsigned

			// Fetch existing metrics
			let mut metrics = NodeMetrics::<T>::get(&node_id).ok_or(Error::<T>::MetricsNotFound)?;

			// Update the metrics
			metrics.total_pin_checks += total_pin_checks;
			metrics.successful_pin_checks += successful_pin_checks;

			// Insert the updated metrics back into storage
			NodeMetrics::<T>::insert(node_id.clone(), metrics);
			log::info!("✅ Updated pin check metrics for node ID {:?}", node_id);

			Self::deposit_event(Event::PinCheckMetricsUpdated { node_id });
			Ok(().into())
		}


		#[pallet::call_index(3)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn update_metrics_data(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			_signature: <T as SigningTypes>::Signature,
			storage_proof_time_ms: u32,
			latency_ms: u32,
			peer_count: u32,
			failed_challenges_count: u32,
			successful_challenges: u32,
			total_challenges: u32,
			uptime_minutes: u32, 
			total_minutes: u32, 
			consecutive_reliable_days: u32,
			recent_downtime_hours: u32, 
			_node_type: NodeType
		) -> DispatchResultWithPostInfo {
			ensure_none(origin)?; // Ensure the call is unsigned

			// Fetch existing metrics
			let mut metrics = NodeMetrics::<T>::get(&node_id).ok_or(Error::<T>::MetricsNotFound)?;
			let current_block_number = <frame_system::Pallet<T>>::block_number();

			// Update the metrics
			metrics.storage_proof_time_ms = storage_proof_time_ms;
			
			metrics.storage_growth_rate = 
			((sp_core::U256::from(metrics.current_storage_bytes) / (current_block_number.into() * <T as pallet::Config>::BlockTime::get()))
			.low_u64()) as u32;
		
			metrics.latency_ms = latency_ms;
			metrics.total_latency_ms =  metrics.total_latency_ms + latency_ms;
			metrics.total_times_latency_checked += 1;
			metrics.avg_response_time_ms =  metrics.total_latency_ms / metrics.total_times_latency_checked;
			metrics.peer_count = peer_count;
			metrics.failed_challenges_count += failed_challenges_count;
			metrics.successful_challenges += successful_challenges;
			metrics.total_challenges += total_challenges;
			if uptime_minutes != 0 {
				metrics.uptime_minutes = uptime_minutes; 
			}
			if total_minutes != 0 {
				metrics.total_minutes = total_minutes;
			}
			if consecutive_reliable_days != 0 {
				metrics.consecutive_reliable_days = consecutive_reliable_days;
			}
			if recent_downtime_hours != 0 {
				metrics.recent_downtime_hours = recent_downtime_hours;
			}

			// Insert the updated metrics back into storage
			NodeMetrics::<T>::insert(node_id.clone(), metrics);
			log::info!("✅ Updated metrics Extrinsic data sucessfully");

			Self::deposit_event(Event::PinCheckMetricsUpdated { node_id });
			Ok(().into())
		}

		#[pallet::call_index(4)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn update_block_time(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			_signature: <T as SigningTypes>::Signature,
			block_number: BlockNumberFor<T>
		) -> DispatchResultWithPostInfo {
			ensure_none(origin)?; // Ensure the call is unsigned

			// Fetch the existing vector of block numbers or initialize a new one
			let mut blocks = BlockNumbers::<T>::get(node_id.clone()).unwrap_or_else(|| Vec::new());

			let check_intetrval = <T as pallet::Config>::BlockCheckInterval::get();
			// Add the current block number
			// Push the current block number and the preceding ones
			for i in (0..check_intetrval).rev() {
				let block_to_push = block_number - i.into();
				blocks.push(block_to_push);
			}

			BlockNumbers::<T>::insert(node_id,blocks);

			Ok(().into())
		}

		#[pallet::call_index(5)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn store_offline_status_of_miner(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			miner_id: Vec<u8>,
			_signature: <T as SigningTypes>::Signature,
			block_number: BlockNumberFor<T>,
		) -> DispatchResultWithPostInfo {
			ensure_none(origin)?; // Ensure the call is unsigned
		
			// Create the new OfflineStatus entry
			let offline_status = OfflineStatus {
				miner_node_id: miner_id.clone(),
				reportor_node_id: node_id,
				at_block: block_number,
			};
		
			// Retrieve the existing downtime status, or initialize an empty vector if it does not exist
			let mut current_downtime_status = DowntimeStatus::<T>::get();
		
			// Add the new OfflineStatus to the list
			current_downtime_status.push(offline_status);
		
			// Store the updated list back in the storage
			DowntimeStatus::<T>::put(current_downtime_status);

			pallet_registration::Pallet::<T>::do_unregister_node(miner_id.clone());
			
			// Return the result
			Ok(().into())
		}
		

	}

	impl<T: Config> Pallet<T> {
		fn fetch_hardware_info(node_type: NodeType) -> Result<SystemInfo, http::Error> {
			let url = <T as pallet::Config>::LocalRpcUrl::get();
			let method = <T as pallet::Config>::SystemInfoRpcMethod::get();

			let json_payload = format!(
				r#"{{
					"id": 1,
					"jsonrpc": "2.0",
					"method": "{}",
					"params": []
				}}"#,
				method
			);

			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(20_000));
				
			let body = vec![json_payload];
			let request = sp_runtime::offchain::http::Request::post(url, body);
		
			let pending = request
				.add_header("Content-Type", "application/json")
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::info!("Error making Request: {:?}", err);
					sp_runtime::offchain::http::Error::IoError
				})?;
		
			let response = pending
				.try_wait(deadline)
				.map_err(|err| {
					log::info!("Error getting Response: {:?}", err);
					sp_runtime::offchain::http::Error::DeadlineReached
				})??;
			
			if response.code != 200 {
				log::info!("Unexpected status code: {}", response.code);
				return Err(http::Error::Unknown);
			}
			
			let body = response.body().collect::<Vec<u8>>();	

			let body_str = sp_std::str::from_utf8(&body).map_err(|_| {
				log::info!("Response body is not valid UTF-8");
				http::Error::Unknown
			})?;

			let mut system_info: SystemInfo = body_str.parse().map_err(|_| {
				log::error!("Failed to parse system info");
				http::Error::Unknown
			})?;

			if system_info.cpu_cores == 0 || system_info.memory_mb == 0 {
				log::error!("Invalid system info: CPU cores or memory cannot be 0");
				return Err(http::Error::Unknown);  
			}
		
			// If the node type is Validator, read from off-chain storage
			if node_type == NodeType::Validator {
				// Read from off-chain storage
				let offchain_db = sp_io::offchain::local_storage_get(StorageKind::PERSISTENT, b"bandwidth_mbps");
				let bandwidth_mbps: Option<f32> = if let Some(value) = offchain_db {
					if value.len() == 4 { // Check for 4 bytes for f32
						// Convert the value back to f32
						let metric_value = f32::from_le_bytes([value[0], value[1], value[2], value[3]]);
						Some(metric_value) // Return the value wrapped in Some
					} else {
						None // Return None if the length is unexpected
					}
				} else {
					None // Return None if reading fails
				};

				// Include the off-chain bandwidth if it is not None
				if let Some(offchain_bandwidth) = bandwidth_mbps {
					system_info.network_bandwidth_mb_s = (system_info.network_bandwidth_mb_s as f32 * offchain_bandwidth) as u32; // Convert u32 to f32 for multiplication
				}

				// Read from off-chain storage for storage bytes
				let offchain_db_storage = sp_io::offchain::local_storage_get(StorageKind::PERSISTENT, b"storage_bytes");
				let storage_bytes: Option<u64> = if let Some(value) = offchain_db_storage {
					if value.len() == 8 { // Check for 8 bytes for u64
						// Convert the value back to u64
						let bytes_value = u64::from_le_bytes([value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7]]);
						Some(bytes_value) // Return the value wrapped in Some
					} else {
						None // Return None if the length is unexpected
					}
				} else {
					None // Return None if reading fails
				};

				// Include the off-chain storage bytes if it is not None
				if let Some(offchain_storage) = storage_bytes {
					system_info.storage_total_mb *= offchain_storage; // Add off-chain storage bytes
					system_info.free_memory_mb *= offchain_storage; // Add off-chain storage bytes to total as well
				}
			}
			
			Ok(system_info)
		}

		// fn benchmark_cpu(system_info: &SystemInfo) -> Result<u32, ()> {
		// 	let iterations = 2000000 / system_info.cpu_cores as u32;
		// 	let matrix_size = 50;
		// 	let mut result = 0u32;

		// 	for _ in 0..iterations {
		// 		for i in 0..matrix_size {
		// 			for j in 0..matrix_size {
		// 				result = result.wrapping_add((i * j) as u32);
		// 			}
		// 		}
		// 	}

		// 	Ok(result.saturating_div(100_000_000))
		// }

		// fn benchmark_memory(system_info: &SystemInfo) -> Result<u32, ()> {
		// 	let mut score = 0u32;
		
		// 	let sizes = [system_info.memory_mb / 1024, system_info.memory_mb / 256, system_info.memory_mb / 64];
			
		// 	for size in sizes.iter() {
		// 		if *size < 1 { continue; }
		// 		let mut vec = Vec::with_capacity(*size as usize);
		
		// 		for i in 0..*size as usize {
		// 			vec.push((i % 256) as u8);
		// 		}
		
		// 		for chunk in vec.chunks(256) {
		// 			score = score.wrapping_add(chunk.iter().map(|&x| x as u32).sum::<u32>());
		// 		}
		// 	}
		
		// 	Ok(score.saturating_div(1_000_000))
		// }

		// fn benchmark_storage(system_info: &SystemInfo) -> Result<u32, ()> {
		// 	let mut score = 0u32;
		
		// 	let sizes = [64, 256, (system_info.storage_total_mb / 10) as usize, (system_info.storage_total_mb / 4) as usize];
		
		// 	for size in sizes.iter() {
		// 		if *size < 1 { continue; }
		// 		let data = (0..*size).map(|i| (i % 256) as u8).collect::<Vec<u8>>();
		
		// 		for i in 0..100 {
		// 			let key = (i as u32).to_be_bytes();
		// 			sp_io::storage::set(&key, &data);
		// 		}
		
		// 		for i in 0..100 {
		// 			let key = (i as u32).to_be_bytes();
		// 			if let Some(value) = sp_io::storage::get(&key) {
		// 				score = score.wrapping_add(value.len() as u32);
		// 			}
		// 			sp_io::storage::clear(&key);
		// 		}
		// 	}
		
		// 	Ok(score.saturating_div(10_000))
		// }

		// fn execute_benchmark(
		// 	node_id: Vec<u8>,
		// ) -> Result<(), BenchmarkError> {
		// 	let system_info = NodeSpecs::<T>::get(&node_id).ok_or(BenchmarkError::HardwareCheckFailed)?;
			
		// 	let mut final_score = 0u32;
		
		// 	let cpu_score = Self::benchmark_cpu(&system_info).map_err(|_| {
		// 		log::error!(target: "execution-unit", "CPU benchmark failed");
		// 		BenchmarkError::BenchmarkExecutionFailed
		// 	})?;
		// 	final_score = final_score.wrapping_add(cpu_score);
		
		// 	let memory_score = Self::benchmark_memory(&system_info).map_err(|_| {
		// 		log::error!(target: "execution-unit", "Memory benchmark failed");
		// 		BenchmarkError::BenchmarkExecutionFailed
		// 	})?;
		// 	final_score = final_score.wrapping_add(memory_score);
		
		// 	let storage_score = Self::benchmark_storage(&system_info).map_err(|_| {
		// 		log::error!(target: "execution-unit", "Storage benchmark failed");
		// 		BenchmarkError::BenchmarkExecutionFailed
		// 	})?;
		// 	final_score = final_score.wrapping_add(storage_score);
		
		// 	let current_block_number = <frame_system::Pallet<T>>::block_number();

		// 	let metrics = BenchmarkMetrics {
		// 		cpu_score,
		// 		memory_score,
		// 		storage_score,
		// 		disk_score: 0,
		// 		network_score: 0
		// 	};

		// 	let results = BenchmarkResult {
		// 	    final_score,
		// 		timestamp: current_block_number,
		// 		trigger_type: TriggerType::Random,
		// 		metrics: metrics.clone(), 	
		// 	};

		// 	BenchmarkResults::<T>::insert(node_id.clone(), results);

		// 	Self::deposit_event(Event::BenchmarkCompleted {
		// 		node_id,
		// 		metrics,
		// 		final_score
		// 	});
		
		// 	Ok(())
		// }

		pub fn do_update_metrics_data(node_id: Vec<u8>, node_type: NodeType) {
			// Initialize counters
			let mut failed_challenges_count = 0;
			let mut successful_challenges = 0;
			let mut total_challenges = 0;
		
			// Fetch latency
			total_challenges += 1;
			let latency_ms = match Self::fetch_latency_ms() {
				Ok(value) => {
					successful_challenges += 1;
					value
				}
				Err(e) => {
					log::error!("Failed to fetch latency: {:?}", e);
					failed_challenges_count += 1;
					0 // Default value instead of early return
				}
			};
		
			// Fetch peer count
			total_challenges += 1;
			let peer_count = match Self::get_peer_count_from_health() {
				Ok(value) => {
					successful_challenges += 1;
					value
				}
				Err(e) => {
					log::error!("Failed to fetch peer count: {:?}", e);
					failed_challenges_count += 1;
					0 // Default value instead of early return
				}
			};
		
			// Fetch storage proof time
			total_challenges += 1;
			let storage_proof_time_ms = match Self::fetch_storage_proof_time_ms() {
				Ok(value) => {
					successful_challenges += 1;
					value
				}
				Err(e) => {
					log::error!("Failed to fetch storage proof time: {:?}", e);
					failed_challenges_count += 1;
					0 // Default value instead of early return
				}
			};
		
			// Modify uptime calculation to handle potential None case more gracefully
			let (uptime_minutes, total_uptime_minutes, consecutive_reliable_days, downtime_hours) = 
				Self::calculate_uptime_and_recent_downtime(node_id.clone())
					.unwrap_or((0, 0, 0, 0));
		
			// Modify latency calculation for validator
			let mut adjusted_latency_ms = latency_ms;
			if node_type == NodeType::Validator {
				if let Some(value) = sp_io::offchain::local_storage_get(StorageKind::PERSISTENT, b"latency_bytes") {
					if value.len() == 4 {
						let latency_bytes = f32::from_le_bytes([value[0], value[1], value[2], value[3]]);
						adjusted_latency_ms = (latency_ms as f32 * latency_bytes) as u128;
					} else {
						log::error!("Unexpected latency bytes length: {:?}", value.len());
					}
				}
			}
		
			// Similar modification for peer count
			let mut adjusted_peer_count = peer_count;
			if node_type == NodeType::Validator {
				if let Some(value) = sp_io::offchain::local_storage_get(StorageKind::PERSISTENT, b"peers_bytes") {
					if value.len() == 4 {
						let peers_bytes = f32::from_le_bytes([value[0], value[1], value[2], value[3]]);
						adjusted_peer_count = (peer_count as f32 * peers_bytes) as u32;
					} else {
						log::error!("Unexpected peers bytes length: {:?}", value.len());
					}
				}
			}
		
			// Call update metrics with adjusted values
			Self::call_update_metrics_data(
				node_id, 
				storage_proof_time_ms as u32, 
				adjusted_latency_ms as u32, 
				adjusted_peer_count, 
				failed_challenges_count,
				successful_challenges, 
				total_challenges,
				uptime_minutes as u32, 
				total_uptime_minutes as u32, 
				consecutive_reliable_days,
				downtime_hours as u32,  
				node_type
			);
		}

		
		pub fn call_update_metrics_data(
			node_id: Vec<u8>,
			storage_proof_time_ms: u32,
			latency_ms: u32,
			peer_count: u32,
			failed_challenges_count: u32,
			successful_challenges: u32,
			total_challenges: u32,
			uptime_minutes: u32,
			total_minutes: u32,
			consecutive_reliable_days: u32,
			recent_downtime_hours: u32,
			node_type: NodeType,
		) {
			// Create a unique lock for the update metrics data operation
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"executionunit::update_metrics_data_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);
		
			if let Ok(_guard) = lock.try_lock() {
				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T, <T as pallet::Config>::AuthorityId>::all_accounts();
		
				if !signer.can_sign() {
					log::warn!("No accounts available for signing in signer.");
					return;
				}
		
				// Prepare and sign the payload
				let results = signer.send_unsigned_transaction(
					|account| {
						UpdateMetricsDataPayload {
							node_id: node_id.clone(),
							storage_proof_time_ms,
							latency_ms,
							peer_count,
							failed_challenges_count,
							successful_challenges,
							total_challenges,
							uptime_minutes,
							total_minutes,
							consecutive_reliable_days,
							recent_downtime_hours,
							node_type: node_type.clone(),
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, signature| {
						Call::update_metrics_data {
							node_id: payload.node_id,
							signature,
							storage_proof_time_ms: payload.storage_proof_time_ms,
							latency_ms: payload.latency_ms,
							peer_count: payload.peer_count,
							failed_challenges_count: payload.failed_challenges_count,
							successful_challenges: payload.successful_challenges,
							total_challenges: payload.total_challenges,
							uptime_minutes: payload.uptime_minutes,
							total_minutes: payload.total_minutes,
							consecutive_reliable_days: payload.consecutive_reliable_days,
							recent_downtime_hours: payload.recent_downtime_hours,
							node_type: payload.node_type,
						}
					},
				);
		
				// Process results of the transaction submission
				for (acc, res) in &results {
					match res {
						Ok(()) => log::info!("[{:?}] Successfully submitted metrics data update", acc.id),
						Err(e) => log::error!("[{:?}] Error submitting metrics data update: {:?}", acc.id, e),
					}
				}
			} else {
				log::error!("❌ Could not acquire lock for updating metrics data");
			};
		}
		

		pub fn call_store_offline_status_of_miner(
			node_id: Vec<u8>,
			miner_id: Vec<u8>,
			block_number: BlockNumberFor<T>,
		) {
			// Create a unique lock for the operation
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"executionunit::store_offline_status_of_miner_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);
		
			if let Ok(_guard) = lock.try_lock() {
				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T, <T as pallet::Config>::AuthorityId>::all_accounts();
		
				if !signer.can_sign() {
					log::warn!("No accounts available for signing in signer.");
					return;
				}
		
				// Prepare and sign the payload
				let results = signer.send_unsigned_transaction(
					|account| {
						StoreOfflineStatusPayload {
							node_id: node_id.clone(),
							miner_id: miner_id.clone(),
							block_number,
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, signature| {
						Call::store_offline_status_of_miner {
							node_id: payload.node_id,
							miner_id: payload.miner_id,
							signature,
							block_number: payload.block_number,
						}
					},
				);
		
				// Process results of the transaction submission
				for (acc, res) in &results {
					match res {
						Ok(()) => log::info!(
							"[{:?}] Successfully submitted tx for storing offline status of miner",
							acc.id
						),
						Err(e) => log::error!(
							"[{:?}] Error submitting tx for storing offline status of miner: {:?}",
							acc.id,
							e
						),
					}
				}
			} else {
				log::error!("❌ Could not acquire lock for storing offline status of miner");
			};
		}
		

		// 
		pub fn call_update_block_time(
			node_id: Vec<u8>,
			block_number: BlockNumberFor<T>,
		) {
			// Create a unique lock for the update block time operation
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"executionunit::update_block_time_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);
		
			if let Ok(_guard) = lock.try_lock() {
				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T, <T as pallet::Config>::AuthorityId>::all_accounts();
		
				if !signer.can_sign() {
					log::warn!("No accounts available for signing in signer.");
					return;
				}
		
				// Prepare and sign the payload
				let results = signer.send_unsigned_transaction(
					|account| {
						UpdateBlockTimePayload {
							node_id: node_id.clone(),
							block_number,
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, signature| {
						Call::update_block_time {
							node_id: payload.node_id,
							signature,
							block_number: payload.block_number,
						}
					},
				);
		
				// Process results of the transaction submission
				for (acc, res) in &results {
					match res {
						Ok(()) => log::info!("[{:?}] Successfully submitted block time update", acc.id),
						Err(e) => log::error!("[{:?}] Error submitting block time update: {:?}", acc.id, e),
					}
				}
			} else {
				log::error!("❌ Could not acquire lock for updating block time");
			};
		}
		
		
		pub fn call_update_pin_check_metrics(
			node_id: Vec<u8>,
			total_pin_checks: u32,
			successful_pin_checks: u32,
		) {
			// Create a unique lock for the update pin check metrics operation
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"executionunit::update_pin_check_metrics_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);
		
			if let Ok(_guard) = lock.try_lock() {
				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T, <T as pallet::Config>::AuthorityId>::all_accounts();
		
				if !signer.can_sign() {
					log::warn!("No accounts available for signing in signer.");
					return;
				}
		
				// Prepare and sign the payload
				let results = signer.send_unsigned_transaction(
					|account| {
						UpdatePinCheckMetricsPayload {
							node_id: node_id.clone(),
							total_pin_checks,
							successful_pin_checks,
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, signature| {
						Call::update_pin_check_metrics {
							node_id: payload.node_id,
							signature,
							total_pin_checks: payload.total_pin_checks,
							successful_pin_checks: payload.successful_pin_checks,
						}
					},
				);
		
				// Process results of the transaction submission
				for (acc, res) in &results {
					match res {
						Ok(()) => log::info!("[{:?}] Successfully submitted pin check metrics update", acc.id),
						Err(e) => log::error!("[{:?}] Error submitting metrics update: {:?}", acc.id, e),
					}
				}
			} else {
				log::error!("❌ Could not acquire lock for updating metrics");
			};
		}
		

		pub fn save_hardware_info(node_id: Vec<u8>, node_type: NodeType) {
			// Create a unique lock for the save hardware info operation
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"executionunit::save_hardware_info_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);
		
			if let Ok(_guard) = lock.try_lock() {
				match Self::fetch_hardware_info(node_type.clone()) {
					Ok(hardware_info) => {
						// Fetch signer accounts using AuthorityId
						let signer = Signer::<T, <T as pallet::Config>::AuthorityId>::all_accounts();
		
						if !signer.can_sign() {
							log::warn!("No accounts available for signing in signer.");
							return;
						}
		
						// Prepare and sign the payload
						let results = signer.send_unsigned_transaction(
							|account| {
								SaveHardwareInfoPayload {
									node_id: node_id.clone(),
									system_info: hardware_info.clone(),
									public: account.public.clone(),
									node_type: node_type.clone(),
									_marker: PhantomData,
								}
							},
							|payload, signature| {
								Call::put_node_specs {
									node_id: payload.node_id,
									system_info: payload.system_info,
									signature,
									node_type: payload.node_type,
								}
							},
						);
		
						// Process results of the transaction submission
						for (acc, res) in &results {
							match res {
								Ok(()) => log::info!("[{:?}] Successfully submitted signed hardware update", acc.id),
								Err(e) => log::error!("[{:?}] Error submitting hardware update: {:?}", acc.id, e),
							}
						}
					}
					Err(e) => {
						log::error!("❌ Error fetching hardware info: {:?}", e);
					}
				}
			} else {
				log::error!("❌ Could not acquire lock for saving hardware info");
			};
		}
		
		fn fetch_ipfs_file_size(file_hash_vec: Vec<u8>) -> Result<u32, http::Error> {
		
			let file_hash = hex::decode(file_hash_vec).map_err(|_| {
				log::error!("Failed to decode file hash");
				http::Error::Unknown
			})?;

			let hash_str = sp_std::str::from_utf8(&file_hash).map_err(|_| {
				log::error!("Failed to convert hash to string");
				http::Error::Unknown
			})?;
		
			let url = format!("{}/api/v0/dag/stat?arg={}", T::IPFSBaseUrl::get(), hash_str);

			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(5_000));
			
			let request = sp_runtime::offchain::http::Request::post(&url, vec![DUMMY_REQUEST_BODY.to_vec()]);
			
			let pending = request
				.add_header("Content-Type", "application/json")
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::info!("Error making Request: {:?}", err);
					sp_runtime::offchain::http::Error::IoError
				})?;

			let response = pending
				.try_wait(deadline)
				.map_err(|err| {
					log::info!("Error getting Response: {:?}", err);
					sp_runtime::offchain::http::Error::DeadlineReached
				})??;

			if response.code != 200 {
				log::info!("Unexpected status code: {}", response.code);
				return Err(http::Error::Unknown);
			}

			let body = response.body().collect::<Vec<u8>>();    

			let body_str = sp_std::str::from_utf8(&body).map_err(|_| {
				log::error!("IPFS response not valid UTF-8");
				http::Error::Unknown
			})?;

			if let Some(size_start) = body_str.find("\"Size\":") {
				let size_str = &body_str[size_start + 7..];
				if let Some(size_end) = size_str.find(',') {
					let size_value = &size_str[..size_end];
					return size_value.trim().parse::<u32>().map_err(|_| {
						log::error!("Failed to parse file size");
						http::Error::Unknown
					});
				}
			}
		
			log::error!("Failed to parse IPFS response");
			Err(http::Error::Unknown)
		}

		fn fetch_cid_pinned_nodes(cid: &Vec<u8>) -> Result<Vec<Vec<u8>>, http::Error> {

			let file_hash = hex::decode(cid).map_err(|_| {
				log::error!("Failed to decode file hash");
				http::Error::Unknown
			})?;
			
			let hash_str = sp_std::str::from_utf8(&file_hash).map_err(|_| {
				log::error!("Failed to convert hash to string");
				http::Error::Unknown
			})?;
		
			let url = format!("{}/api/v0/routing/findprovs?arg={}", T::IPFSBaseUrl::get(), hash_str); // Updated to use the constant
		
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(8_000));
			
			let request = sp_runtime::offchain::http::Request::post(&url, vec![DUMMY_REQUEST_BODY.to_vec()]);

			let pending = request
				.add_header("Content-Type", "application/json")
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::info!("Error making Request: {:?}", err);
					sp_runtime::offchain::http::Error::IoError
				})?;

			let response = pending
				.try_wait(deadline)
				.map_err(|err| {
					log::info!("Error getting Response: {:?}", err);
					sp_runtime::offchain::http::Error::DeadlineReached
				})??;

			if response.code != 200 {
				log::info!("Unexpected status code: {}", response.code);
				return Err(http::Error::Unknown);
			}

			let body = response.body().collect::<Vec<u8>>();
		
			let body_str = sp_std::str::from_utf8(&body).map_err(|_| {
				http::Error::Unknown
			})?;

			let json_strings: Vec<&str> = body_str.split('\n').collect(); // Split by newlines

			let mut node_ids = Vec::new();
		
			for json_str in json_strings {
				if json_str.trim().is_empty() {
					continue; // Skip empty strings
				}
		
				// Parse the JSON response
				let parsed_response: serde_json::Value = serde_json::from_str(json_str).map_err(|e| {
					log::error!("Failed to parse JSON response: {:?}", e);
					http::Error::Unknown
				})?;
		
				// Extract the node IDs from the JSON response
				if let Some(responses) = parsed_response["Responses"].as_array() {
					for response in responses {
						if let Some(id) = response["ID"].as_str() {
							node_ids.push(id.as_bytes().to_vec());
						}
					}
				}
			}
		
			if node_ids.is_empty() {
				log::info!("No nodes found");
				return Err(http::Error::Unknown);
			}

			Ok(node_ids)
		}
		
		// Helper function to ping an IPFS node to track uptime
		fn ping_node(node_id_bytes: Vec<u8>) -> Result<bool, http::Error> {
			let node_id = sp_std::str::from_utf8(&node_id_bytes).map_err(|_| {
				log::error!("Failed to convert hash to string");
				http::Error::Unknown
			})?;
			log::info!("Pinging node: {}", node_id);
			// Update the URL to include the count parameter
			let url = format!("{}/api/v0/ping?arg={}&count=5", T::IPFSBaseUrl::get(), node_id);
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(15_000));

			// Use POST instead of GET, as ping typically requires a POST request
			let request = sp_runtime::offchain::http::Request::post(&url, vec![DUMMY_REQUEST_BODY.to_vec()]);

			let pending = request
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::info!("Error sending ping request: {:?}", err);
					sp_runtime::offchain::http::Error::IoError
				})?;

			let response = pending
				.try_wait(deadline)
				.map_err(|err| {
					log::info!("Error waiting for ping response: {:?}", err);
					sp_runtime::offchain::http::Error::DeadlineReached
				})??;

			log::info!("Ping response: {:?}", response);
			
			// Check if the response code is 200 (OK)
			if response.code == 200 {
				let response_body = response.body().collect::<Vec<u8>>();
				let body_str = core::str::from_utf8(&response_body).unwrap_or("");

				// Split the response body into lines to count success messages
				let success_count = body_str.lines().filter(|line| line.contains("\"Success\":true")).count();

				// Check if we have at least 5 successful pings
				if success_count >= 5 {
					log::info!("Node {} pinged successfully at least 5 times.", node_id);
					return Ok(true);
				} else {
					log::info!("Node {} did not respond successfully 5 times: {}", node_id, body_str);
					return Ok(false);
				}
			} else {
				let response_body = response.body().collect::<Vec<u8>>();
				let body_str = core::str::from_utf8(&response_body).unwrap_or("");
				log::info!("Node {} returned an error (status code: {}): {}", node_id, response.code, body_str);
				return Ok(false);
			}
		}

		pub fn process_pending_compute_requests() {
			let active_compute_miners = pallet_registration::Pallet::<T>::get_all_active_compute_miners();
			let pending_compute_requests = pallet_compute::Pallet::<T>::get_pending_compute_requests();
			
			for compute_request in pending_compute_requests {
				// Parse the plan technical description (assuming JSON format)
				let plan_specs = match sp_std::str::from_utf8(&compute_request.plan_technical_description) {
					Ok(specs_str) => {
						match serde_json::from_str::<serde_json::Value>(specs_str) {
							Ok(json) => json,
							Err(e) => {
								log::error!(
									"Failed to parse plan technical description JSON: {:?}, raw string: {}",
									e,
									specs_str
								);
								continue; // Skip if JSON parsing fails
							}
						}
					},
					Err(e) => {
						log::error!(
							"Failed to convert plan technical description to UTF-8: {:?}, raw bytes: {:?}",
							e,
							compute_request.plan_technical_description
						);
						continue; // Skip if UTF-8 conversion fails
					}
				};

				// Find a suitable miner with matching or exceeding specs
				if let Some(suitable_miner) = active_compute_miners.iter().find(|miner| {
					// Check if a specific miner ID is requested
					let miner_id_match = match &compute_request.miner_id {
						Some(requested_miner_id) => miner.node_id == *requested_miner_id,
						None => true, // No specific miner requested, so all miners are valid
					};

					// Retrieve node metrics
					let node_metrics_match = match Self::get_node_metrics(miner.node_id.clone()) {
						Some(node_metrics) if miner_id_match => {
							// Define the minimum resource reservation percentage
							const MIN_RESERVED_PERCENTAGE: f64 = 0.1; // 10%

							// Check CPU cores requirement with 10% reservation
							let total_cpu_cores = node_metrics.cpu_cores as f64;
							let cpu_cores_match = plan_specs["cpu_cores"].as_u64()
								.map_or(true, |req_cores| {
									let requested_cores = req_cores as f64;
									(total_cpu_cores - requested_cores) / total_cpu_cores >= MIN_RESERVED_PERCENTAGE
								});

							// Check RAM requirement with 10% reservation (convert MB to GB)
							let total_ram_gb = (node_metrics.free_memory_mb / 1024) as f64;
							let ram_match = plan_specs["ram_gb"].as_u64()
								.map_or(true, |req_ram| {
									let requested_ram = req_ram as f64;
									(total_ram_gb - requested_ram) / total_ram_gb >= MIN_RESERVED_PERCENTAGE
								});

							// Check storage requirement with 10% reservation
							let current_storage_gb = (node_metrics.current_storage_bytes / (1024 * 1024 * 1024)) as f64;
							let storage_match = plan_specs["storage_gb"].as_u64()
								.map_or(true, |req_storage| {
									let requested_storage = req_storage as f64;
									(current_storage_gb - requested_storage) / current_storage_gb >= MIN_RESERVED_PERCENTAGE
								});

							// New check for SEV
							let sev_match = plan_specs["is_sev_enabled"].as_bool()
							.map_or(true, |req_sev| {
								!req_sev || node_metrics.is_sev_enabled
							});

							// Return true if all specified requirements are met and 10% resources remain
							cpu_cores_match && ram_match && storage_match && sev_match
						},
						_ => false // No metrics available or miner ID mismatch
					};

					node_metrics_match
				}) {

					let failed_requests = pallet_compute::Pallet::<T>::get_miner_compute_requests_with_failure(compute_request.request_id);

					// Check if the suitable miner's node ID is not in the failed requests
					let is_miner_failed = failed_requests.iter().any(|req| 
						req.miner_node_id == suitable_miner.node_id
					);
					
                    // only assign if not already assigned
					if !is_miner_failed {
					    // Assign the compute request to the suitable miner
					    pallet_compute::Pallet::<T>::save_compute_request(
					    	suitable_miner.node_id.clone(), 
					    	compute_request.plan_id, 
					    	compute_request.request_id,
					    	compute_request.owner
					    );						
					}

				}
			}
		}

		pub fn get_node_metrics(node_id: Vec<u8>) -> Option<NodeMetricsData> {
			NodeMetrics::<T>::get(node_id)
		}

		// perform health checks and see if the miner is pinned the file / updates the node metrics fro miners
		pub fn perform_pin_checks_to_miners(node_id: Vec<u8>) {
			let fullfilled_req  = IpfsPinPallet::<T>::get_all_fullfilled_requests();
			// for each fullfilled request 
			for request in fullfilled_req {
				// all active Ipfs nodes that has that hash pinned (http req)
				let ipfs_nodes_who_pinned: Vec<Vec<u8>> = Self::fetch_cid_pinned_nodes(&request.file_hash.clone())
					.expect("Failed to fetch pinned nodes");
				// all miners who have pinned this file 
				let request_pinned_by_miners = IpfsPinPallet::<T>::get_pin_requests_by_file_hash(&request.file_hash.clone());
				for miner_request in request_pinned_by_miners {
					let miner_node_id: Vec<u8> = miner_request.miner_node_id;
					match pallet_registration::Pallet::<T>::get_registered_node(miner_node_id.clone()) {
						Ok(node_info) => {
							let mut total_pin_checks = 0;
							let mut successful_pin_checks = 0;		

							// Check if ipfs_node_id is present in ipfs_nodes_who_pinned
							if let Some(ipfs_node_id) = node_info.ipfs_node_id {
								if ipfs_nodes_who_pinned.contains(&ipfs_node_id) {
									// found pinned the file 
									total_pin_checks += 1;
									successful_pin_checks += 1;
								} else {
									// not found pinned file
									// update request storage  and delete this fileRequest
									total_pin_checks += 1;
									let mut updated_req = request.clone();
									updated_req.fullfilled_replicas = updated_req.fullfilled_replicas - 1;
									// we just need to update this and offchian worker will do assigning stuff
									pallet_ipfs_pin::Pallet::<T>::update_storage_usage_request(request.owner.clone(), request.file_hash.clone(), Some(updated_req) , node_id.clone());

									let mut files_stored_for_miner = pallet_ipfs_pin::Pallet::<T>::get_files_stored(miner_node_id.clone());
									files_stored_for_miner.retain(|stored_request| {
										stored_request.file_hash != miner_request.file_hash // Adjust this condition based on your criteria
									});
									pallet_ipfs_pin::Pallet::<T>::update_ipfs_request_storage(miner_node_id.clone(), files_stored_for_miner);
								}
							} else {
								log::info!("Node ID {:?} does not have an associated IPFS node ID",sp_std::str::from_utf8(&node_info.node_id).unwrap_or("<Invalid UTF-8>"));
							}

							// update node metrics
							Self::call_update_pin_check_metrics(miner_node_id, total_pin_checks, successful_pin_checks)
						},
						Err(err) => log::info!("Failed to get node info: {}", err),
					}
				}
			}
		}

		// perform health checks and see if the miner is pinned the file / updates the node metrics fro miners
		pub fn perform_ping_checks_to_miners(node_id: Vec<u8>) {
			let active_miners  = pallet_registration::Pallet::<T>::get_all_active_storage_miners();
			for miner in active_miners {
				let miner_node_id: Vec<u8> = miner.node_id;
				match pallet_registration::Pallet::<T>::get_registered_node(miner_node_id.clone()) {
					Ok(node_info) => {
						// Check if ipfs_node_id is present in ipfs_nodes_who_pinned
						if let Some(ipfs_node_id) = node_info.ipfs_node_id {
							// check if the miner's ipfs node  is online or not 
							match Self::ping_node(ipfs_node_id.clone()) {
								Ok(is_online) => {
									if !is_online {
										let current_block = frame_system::Pallet::<T>::block_number();
										Self::call_store_offline_status_of_miner(node_id.clone(), miner_node_id.clone(),current_block);
									}
								},
								Err(err) => {
									log::error!("Ipfs for Error pinging node {}: {:?}", sp_std::str::from_utf8(&miner_node_id).unwrap_or("<Invalid UTF-8>"), err);
								}
							}
						}
					},
					Err(err) => log::info!("Failed to get node info: {}", err),
				}
			}
		}
		
		pub fn process_pending_storage_requests(node_id: Vec<u8>, block_number: BlockNumberFor<T>) {
			let active_storage_miners = pallet_registration::Pallet::<T>::get_all_storage_miners_with_min_staked();
			let pending_storage_requests = pallet_ipfs_pin::Pallet::<T>::get_pending_storage_requests();

			// Retrieve the ranked list of nodes
			let ranked_list = RankingsPallet::<T>::get_ranked_list();

			let mut rank_map: BTreeMap<Vec<u8>, u32> = BTreeMap::new();
			for ranking in ranked_list.iter() {
				rank_map.insert(ranking.node_id.clone(), ranking.rank);
			}
			
			// Sort active storage miners by rank
			let mut sorted_miners: Vec<_> = active_storage_miners
			.iter()
			.filter_map(|miner| {
				if let Some(rank) = rank_map.get(&miner.node_id) {
					Some((miner, *rank)) // Pair miner with its rank
				} else {
					None
				}
			})
			.collect();

			// Sort miners by rank (ascending)
			sorted_miners.sort_by_key(|(_, rank)| *rank);
		
			for storage_request in pending_storage_requests {
				let file_hash = storage_request.file_hash.clone();								
				// should not be blaclisted 
				if !pallet_ipfs_pin::Pallet::<T>::is_file_blacklisted(&file_hash) {
					// should be an approved request
					if storage_request.is_approved{
						// First, try to assign to miners specified in miner_ids
						let mut assigned = false;
						if let Some(request_miner_ids) = &storage_request.miner_ids {

							for (miner, _) in sorted_miners.iter() {
								// Check if this miner's node_id is in the requested miner_ids
								if request_miner_ids.iter().any(|id| *id == miner.node_id) {
									let is_pinned = pallet_ipfs_pin::Pallet::<T>::is_file_already_pinned_by_storage_miner(
										miner.node_id.clone(), 
										file_hash.clone() 
									);
									if !is_pinned {
										// Retrieve node metrics to check available storage
										if let Some(node_metrics) = Self::get_node_metrics(miner.node_id.clone()) {

											let ipfs_zfs_pool_size = node_metrics.ipfs_zfs_pool_size;
											let ipfs_storage_max = node_metrics.ipfs_storage_max;
											if ipfs_zfs_pool_size < (ipfs_storage_max as u128) {
												// degrade miner 
												pallet_registration::Pallet::<T>::call_node_status_to_degraded_unsigned(miner.node_id.clone());
											}else{
												// Calculate available storage using IPFS metrics
												let available_storage = ipfs_storage_max.saturating_sub(node_metrics.ipfs_repo_size);
												// Fetch file size
												match Self::fetch_ipfs_file_size(file_hash.clone()) {
													Ok(file_size) => {
														// Check if miner has enough storage
														if (file_size as u64) <= available_storage  {
															if let (Some(file_hash_str), Some(miner_id_str)) = (
																sp_std::str::from_utf8(&file_hash).ok(),
																sp_std::str::from_utf8(&miner.node_id).ok(),
															) {
																log::info!(
																	"Assigning file {:?} to specified miner {:?}. Available storage: {} bytes, File size: {} bytes", 
																	file_hash_str, 
																	miner_id_str,
																	available_storage,
																	file_size
																);
															}
															pallet_ipfs_pin::Pallet::<T>::store_ipfs_pin_request(
																storage_request.owner.clone(),
																file_hash.clone(), 
																miner.node_id.clone(),
																file_size
															);
															assigned = true;
															break;
														} else {
															log::info!(
																"Miner {:?} does not have enough storage. Available: {} bytes, Required: {} bytes", 
																sp_std::str::from_utf8(&miner.node_id).unwrap_or("Unknown"),
																available_storage,
																file_size
															);
														}
													},
													Err(e) => {
														log::error!("Failed to fetch file size: {:?}", e);
														continue;
													}
												}
											}


										} else {
											log::warn!(
												"No metrics found for miner {:?}", 
												sp_std::str::from_utf8(&miner.node_id).unwrap_or("Unknown")
											);
										}
									}
								}
							}
						}
						// If not assigned to specified miners, fall back to general assignment
						if !assigned {
							for (miner, _) in sorted_miners.iter() {
								let is_pinned = pallet_ipfs_pin::Pallet::<T>::is_file_already_pinned_by_storage_miner(
									miner.node_id.clone(), 
									file_hash.clone()
								);										
								if !is_pinned { 
									// Retrieve node metrics to check available storage
									if let Some(node_metrics) = Self::get_node_metrics(miner.node_id.clone()) {
										// Calculate available storage
										let available_storage = node_metrics.total_storage_bytes.saturating_sub(node_metrics.current_storage_bytes);

										// Fetch file size
										match Self::fetch_ipfs_file_size(file_hash.clone()) {
											Ok(file_size) => {
												// Check if miner has enough storage
												if (file_size as u64) <= available_storage {
													if let (Some(file_hash_str), Some(miner_id_str)) = (
														sp_std::str::from_utf8(&file_hash).ok(),
														sp_std::str::from_utf8(&miner.node_id).ok(),
													) {
														log::info!(
															"Assigning file {:?} to specified miner {:?}. Available storage: {} bytes, File size: {} bytes", 
															file_hash_str, 
															miner_id_str,
															available_storage,
															file_size
														);
													}

													pallet_ipfs_pin::Pallet::<T>::store_ipfs_pin_request(
														storage_request.owner.clone(),
														file_hash.clone(), 
														miner.node_id.clone(),
														file_size
													);
													assigned = true;
													break;
												} else {
													log::info!(
														"Miner {:?} does not have enough storage. Available: {} bytes, Required: {} bytes", 
														sp_std::str::from_utf8(&miner.node_id).unwrap_or("Unknown"),
														available_storage,
														file_size
													);
												}
											},
											Err(e) => {
												log::error!("Failed to fetch file size: {:?}", e);
												continue;
											}
										}
									} else {
										log::warn!(
											"No metrics found for miner {:?}", 
											sp_std::str::from_utf8(&miner.node_id).unwrap_or("Unknown")
										);
									}
								}
							}
						}						
					}else{
						// check if file size is less then free space or not 
						match Self::fetch_ipfs_file_size(file_hash.clone()) {
							Ok(file_size) => {
								// since the file size in bytes so convertng
								let one_gb_into_bytes = 1_073_741_824.0;

								// Get the current price per GB from the marketplace pallet
								let price_per_gb = pallet_marketplace::Pallet::<T>::get_price_per_gb();
								
								// Calculate the file size in GB with ceiling rounding
								let file_size_in_gb = file_size as f64 / one_gb_into_bytes;

								// Calculate the total cost for storing the file
								// Round up to the nearest whole number of GBs
								let rounded_gbs = ((file_size_in_gb).floor() as u128) + 1;
								let storage_cost = price_per_gb * rounded_gbs;                    

								let free_credits = pallet_credits::Pallet::<T>::free_credits(storage_request.owner.clone());
												
								if free_credits >= storage_cost {									
									// approve the stautus of the request and update storage 
									let mut updated_req = storage_request.clone();
									updated_req.is_approved = true;
									updated_req.last_charged_at = block_number;

									let _ = pallet_marketplace::Pallet::<T>::call_storage_request_approval_charging(storage_request.owner.clone(), storage_cost);
									
									pallet_ipfs_pin::Pallet::<T>::update_storage_usage_request(updated_req.owner.clone(), file_hash.clone(), Some(updated_req) , node_id.clone());
								}else{
									// not enough space left delete storage
									pallet_ipfs_pin::Pallet::<T>::update_storage_usage_request(storage_request.owner.clone(), file_hash.clone(), None , node_id.clone());
								}
							},
							Err(e) => {
								log::error!("Failed to fetch file size: {:?}", e);
								continue;
							}
						}
					}
				}
			}
		}

		/// Fetch the storage proof time in milliseconds
		pub fn fetch_storage_proof_time_ms() -> Result<u128, http::Error> {
			let url = <T as pallet::Config>::LocalRpcUrl::get();
			let method = T::GetReadProofRpcMethod::get(); 

			let json_payload = format!(r#"
			{{
				"id": 1,
				"jsonrpc": "2.0",
				"method": "{}",
				"params": [[], null]
			}}
			"#, method); 

			// state_getReadProof
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(5_000));

			let body = vec![json_payload.as_bytes().to_vec()];
			let request = sp_runtime::offchain::http::Request::post(url, body);

			let start_time = sp_io::offchain::timestamp();

			let pending = request
				.add_header("Content-Type", "application/json")
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::info!("Error making Request: {:?}", err);
					sp_runtime::offchain::http::Error::IoError
				})?;

			let response = pending
				.try_wait(deadline)
				.map_err(|err| {
					log::info!("Error getting Response: {:?}", err);
					sp_runtime::offchain::http::Error::DeadlineReached
				})??;

			if response.code != 200 {
				log::info!("Unexpected status code: {}", response.code);
				return Err(http::Error::Unknown);
			}

			let end_time = sp_io::offchain::timestamp();
			let storage_proof_time_ms = end_time.unix_millis() - start_time.unix_millis();

			// Return the storage proof time in milliseconds
			Ok(storage_proof_time_ms.into())
		}

		/// Fetch the latency in milliseconds
		pub fn fetch_latency_ms() -> Result<u128, http::Error> {
			let url = <T as pallet::Config>::LocalRpcUrl::get();
			let method = T::SystemHealthRpcMethod::get(); 

			let json_payload = format!(r#"
			{{
				"id": 1,
				"jsonrpc": "2.0",
				"method": "{}",
				"params": [[], null]
			}}
			"#, method); 

			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(5_000));

			let body = vec![json_payload.as_bytes().to_vec()];
			let request = sp_runtime::offchain::http::Request::post(url, body);

			let start_time = sp_io::offchain::timestamp();

			let pending = request
				.add_header("Content-Type", "application/json")
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::info!("Error making Request: {:?}", err);
					sp_runtime::offchain::http::Error::IoError
				})?;

			let response = pending
				.try_wait(deadline)
				.map_err(|err| {
					log::info!("Error getting Response: {:?}", err);
					sp_runtime::offchain::http::Error::DeadlineReached
				})??;

			if response.code != 200 {
				log::info!("Unexpected status code: {}", response.code);
				return Err(http::Error::Unknown);
			}

			let end_time = sp_io::offchain::timestamp();
			let latency_ms = end_time.unix_millis() - start_time.unix_millis();

			// Return the latency in milliseconds
			Ok(latency_ms.into())
		}

		fn get_peer_count_from_health() -> Result<u32, http::Error> {
			let url = <T as pallet::Config>::LocalRpcUrl::get();
			let method = T::SystemHealthRpcMethod::get(); 

			let json_payload = format!(r#"
			{{
				"id": 1,
				"jsonrpc": "2.0",
				"method": "{}",
				"params": [[], null]
			}}
			"#, method); 


			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(5_000));

			let body = vec![json_payload.as_bytes().to_vec()];
			let request = sp_runtime::offchain::http::Request::post(url, body);

			let pending = request
			.add_header("Content-Type", "application/json")
			.deadline(deadline)
			.send()
			.map_err(|err| {
				log::info!("Error making Request: {:?}", err);
				sp_runtime::offchain::http::Error::IoError
			})?;

			let response = pending
			.try_wait(deadline)
			.map_err(|err| {
				log::info!("Error getting Response: {:?}", err);
				sp_runtime::offchain::http::Error::DeadlineReached
			})??;

			if response.code != 200 {
				log::info!("Unexpected status code: {}", response.code);
				return Err(http::Error::Unknown);
			}

			let response_body = response.body().collect::<Vec<u8>>(); // Collect the response body
			let response_json: serde_json::Value = serde_json::from_slice(&response_body).map_err(|_| {
				log::error!("Failed to parse response body as JSON");
				http::Error::Unknown
			})?;

			// Access the peers from the parsed JSON
			if let Some(peers) = response_json["result"]["peers"].as_u64() {
				Ok(peers as u32)
			} else {
				Err(http::Error::Unknown)
			}
		}

		fn block_time() -> u64 {
			<T as pallet::Config>::BlockTime::get()
		}

		/// Helper function to calculate uptime, total uptime, consecutive reliable days, and recent downtime hours
		pub fn calculate_uptime_and_recent_downtime(
			miner_node_id: Vec<u8>, // Key for the storage map
		) -> Option<(u64, u64, u32, u64)> {
			// Fetch the stored block numbers
			let block_numbers = BlockNumbers::<T>::get(&miner_node_id)?;

			if block_numbers.is_empty() {
				return None; // No blocks recorded
			}

			// Fetch the block time
			let block_time: u64 = Self::block_time();

			let mut uptime_seconds = 0u64;
			let mut total_uptime_seconds = 0u64;
			let mut recent_downtime_seconds = 0u64;

			let mut consecutive_reliable_days = 0u32;
			let mut current_day_uptime = 0u64;

			// Calculate metrics
			for i in 1..block_numbers.len() {
				let previous_block = block_numbers[i - 1].saturated_into::<u64>();
				let current_block = block_numbers[i].saturated_into::<u64>();

				// Difference in blocks
				let block_diff = current_block.saturating_sub(previous_block);

				// Calculate uptime
				if block_diff == 1 {
					uptime_seconds += block_time;
					current_day_uptime += block_time;
				} else {
					// Calculate recent downtime (only consider the last gap)
					recent_downtime_seconds = (block_diff - 1) * block_time;
					current_day_uptime = 0; // Reset for gaps
				}

				// Add to total uptime (including gaps)
				total_uptime_seconds += block_diff * block_time;

				// Check if a full day of uptime is reached
				if current_day_uptime >= 86400 { // 86400 seconds = 1 day
					consecutive_reliable_days += 1;
					current_day_uptime = 0; // Reset for next day
				}
			}

			// Convert recent downtime seconds to hours
			let recent_downtime_hours = recent_downtime_seconds / 3600;

			let uptime_minutes = uptime_seconds / 60;
			let total_uptime_minutes = total_uptime_seconds / 60;

			Some((uptime_minutes, total_uptime_minutes, consecutive_reliable_days, recent_downtime_hours))
		}

		// // Helper function purge miners if deregistered on bittensor 
		// pub fn purge_nodes_if_deregistered_on_bittensor(node_id: Vec<u8>){
		// 	// get all nodes 
		// 	let active_stroage_miners = pallet_registration::Pallet::<T>::get_all_active_nodes();

		// 	// Retrieve UIDs from storage and match miners with uids
		// 	// Iterate through all active miners
		// 	for miner in active_stroage_miners {
		// 		// Check if the miner's owner is in UIDs
		// 		let is_registered = Self::is_owner_in_uids(&miner.owner);

		// 		// if not ,change status to degraded 
		// 		// perform unpin and assign to other miners 		
		// 		if !is_registered {

		// 			// get all files pinned by this miner
		// 			let files_pinned_by_miners = pallet_ipfs_pin::Pallet::<T>::get_files_stored(miner.node_id.clone());
					
		// 			// degrading by calling unisgned tx
		// 			pallet_registration::Pallet::<T>::call_node_status_to_degraded_unsigned(miner.node_id);

		// 			// all storage requests fullfilled replicas should be -1 and then update storage;
		// 			for files_pinned in files_pinned_by_miners {
		// 				// Attempt to get the storage request
		// 				if let Some(mut req) = pallet_ipfs_pin::Pallet::<T>::get_storage_request_by_hash(files_pinned.file_hash.clone()) {
		// 					// Update the fulfilled replicas count
		// 					req.fullfilled_replicas = req.fullfilled_replicas.saturating_sub(1); // Ensure no underflow
							
		// 					// Update the storage usage request
		// 					pallet_ipfs_pin::Pallet::<T>::update_storage_usage_request(
		// 						files_pinned.file_hash.clone(),
		// 						Some(req),
		// 						node_id.clone(),
		// 					);
		// 				}
		// 			}
		// 		}
		// 	}
		// }

		pub fn is_owner_in_uids(owner: &T::AccountId) -> bool {
			// Convert `owner` to bytes
			let account_bytes = owner.encode(); // This gives a Vec<u8>

			// Attempt to create AccountId32 from bytes
			let owner_account32 = match AccountId32::try_from(account_bytes.as_slice()) {
				Ok(account) => account,
				Err(_) => {
					log::error!("Failed to convert AccountId to AccountId32");
					return false;
				}
			};

			// Retrieve UIDs from storage
			let uids_vec = UIDs::<T>::get();

			// Check if the owner is present in the UIDs
			uids_vec.iter().any(|uid| uid.substrate_address == owner_account32)
		}

    /// Get randomness from BABE
    fn get_babe_randomness() -> [u8; 32] {
        // Get current subject for randomness
        let subject = sp_io::hashing::blake2_256(b"babe_randomness");
        
        // Try to get randomness from one epoch ago
        let (random_hash, _block_number) = <RandomnessFromOneEpochAgo<T> as Randomness<T::Hash, BlockNumberFor<T>>>::random(&subject);
        let random_seed = random_hash.as_ref();
        
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&random_seed[..32]);
        bytes
    }

	fn handle_incorrect_registration(current_block_number: BlockNumberFor<T>) {
		let unregistration_period: BlockNumberFor<T> = T::UnregistrationBuffer::get().into();
		let min_period = 50u32;
		let max_period = 70u32;
		let range = max_period - min_period;
	
		// Use block number to create a pseudo-random offset
		let pseudo_random_offset = (current_block_number % range.into()) + min_period.into();
		let adjusted_period = unregistration_period + pseudo_random_offset - (range / 2).into();
	
		// Check if current block matches the adjusted period
		if current_block_number % adjusted_period == 0u32.into() {
			let active_miners = pallet_registration::Pallet::<T>::get_all_active_nodes();
			for miner in active_miners {
				if current_block_number - miner.registered_at > 36u32.into() {
					let metrics = Self::get_node_metrics(miner.node_id.clone());
					if metrics.is_none() {
						// If node metrics are not there, delete it
						pallet_registration::Pallet::<T>::do_unregister_node(miner.node_id.clone());
					} else {
						let registered_at = miner.registered_at;
						let uptime_minutes = metrics.unwrap().uptime_minutes;
						// Since each block is 6 seconds
						let uptime_in_blocks = (uptime_minutes * 60) / 6;
						// Buffer period before unregistration
						let buffer = 300;
						if current_block_number - registered_at > (uptime_in_blocks + buffer).into() {
							pallet_registration::Pallet::<T>::do_unregister_node(miner.node_id.clone());
						}
					}
				}
			}
		}
	}

	/// Determine if we should execute offchain worker tasks based on BABE randomness
	fn should_execute_offchain(current_block: u32, _random_seed: [u8; 32]) -> bool {
		// Get last execution time
		let last_run = sp_io::offchain::local_storage_get(
			sp_core::offchain::StorageKind::PERSISTENT,
			STORAGE_KEY,
		);

		match last_run {
			Some(last_block_bytes) => {
				let last_block = u32::from_be_bytes(last_block_bytes.try_into().unwrap_or([0; 4]));
				let blocks_passed = current_block.saturating_sub(last_block);

				// Minimum gap of 7 blocks
				// Maximum gap of 15 blocks
				blocks_passed >= 7 && blocks_passed <= 15 // Execute if within range
			},
			None => true, // If there's no last run, allow execution
		}
	}

		/// Get node metrics for multiple node IDs
		pub fn get_node_metrics_batch(node_ids: Vec<Vec<u8>>) -> Vec<Option<NodeMetricsData>> {
			node_ids
				.into_iter()
				.map(|node_id| {
					NodeMetrics::<T>::get(node_id)
				})
				.collect()
		}


		pub fn get_active_nodes_metrics_by_type(node_type: NodeType) -> Vec<Option<NodeMetricsData>> {
			// First, get all active nodes of the specified type
			let active_node_ids = pallet_registration::Pallet::<T>::get_active_nodes_by_type(node_type);

			// Then, fetch metrics for these active nodes
			Self::get_node_metrics_batch(active_node_ids)
		}
	}
}
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
	use crate::weights::WeightInfo;
	use scale_codec::alloc::string::ToString;
	use scale_info::prelude::string::String;
	use ipfs_pallet::{FileHash, MAX_NODE_ID_LENGTH};
	use ipfs_pallet::MinerProfileItem;
	use frame_support::{pallet_prelude::*, traits::Randomness};
	use frame_system::{
		offchain::{
			AppCrypto, SendTransactionTypes, SendUnsignedTransaction, Signer, SigningTypes,
		},
		pallet_prelude::*,
	};
	use ipfs_pallet::Pallet as IpfsPallet;
	use num_traits::float::FloatCore;
	use pallet_babe::RandomnessFromOneEpochAgo;
	use pallet_metagraph::UIDs;
	use pallet_registration::NodeType;
	use pallet_registration::Pallet as RegistrationPallet;
	use pallet_utils::Pallet as UtilsPallet;
	use sp_core::crypto::Ss58Codec;
	use sp_core::offchain::StorageKind;
	use sp_runtime::{
		format,
		offchain::{
			http,
			storage_lock::{BlockAndTime, StorageLock},
			Duration,
		},
		traits::Zero,
		AccountId32,
	};
	use sp_std::prelude::*;
	// use ipfs_pallet::FileHash;
	// use ipfs_pallet::MinerProfileItem;
	// use serde_json::to_string;
	use sp_runtime::Saturating;
	use ipfs_pallet::AssignmentEnabled;
	use pallet_registration::NodeInfo;
	use serde_json::Value;
	// use pallet_credits::Pallet as CreditsPallet;
	use ipfs_pallet::MinerProfile;
	use pallet_rankings::Pallet as RankingsPallet;
	// use ipfs_pallet::MAX_FILE_HASH_LENGTH;
	use ipfs_pallet::MinerPinRequest;
	use ipfs_pallet::StorageRequest;
	use ipfs_pallet::UserProfile;
	use sp_std::collections::btree_map::BTreeMap;
	use ipfs_pallet::StorageUnpinRequest;

	const STORAGE_KEY: &[u8] = b"execution-unit::last-run";

	#[pallet::config]
	pub trait Config: frame_system::Config + 
					  pallet_metagraph::Config + 
					  pallet_babe::Config + 
					  pallet_marketplace::Config +
					  pallet_timestamp::Config + 
					  SendTransactionTypes<Call<Self>> + 
					  frame_system::offchain::SigningTypes +
					  pallet_credits::Config + 
					  ipfs_pallet::Config + 
					//   pallet_compute::Config +
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
		type BlockTime: Get<u32>;

		// Define block time as a runtime parameter
		type BlockCheckInterval: Get<u32>;
		
		#[pallet::constant]
		type GetReadProofRpcMethod: Get<&'static str>; 

		#[pallet::constant]
		type SystemHealthRpcMethod: Get<&'static str>;

		#[pallet::constant]
		type UnregistrationBuffer: Get<u32>;

		#[pallet::constant]
		type MaxOffchainRequestsPerPeriod: Get<u32>;

	    #[pallet::constant]
	    type RequestsClearInterval: Get<u32>;

		#[pallet::constant]
		type MaxOffchainHardwareSubmitRequestsPerPeriod: Get<u32>;

	    #[pallet::constant]
	    type HardwareSubmitRequestsClearInterval: Get<u32>;

		#[pallet::constant]
		type IpfsServiceUrl: Get<&'static str>;
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
			final_score: u32,
		},
		BenchmarkFailed {
			node_id: Vec<u8>,
			error: BenchmarkError,
		},
		NodeSpecsStored {
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
		PurgeDeregisteredNodesStatusChanged {
			enabled: bool,
		},
	}

	#[derive(Debug, Encode, Decode, Clone, PartialEq, Eq, TypeInfo)]
	pub enum BenchmarkError {
		LockAcquisitionFailed,
		HardwareCheckFailed,
		BenchmarkExecutionFailed,
		MetricsNotFound,
	}

	#[pallet::storage]
	#[pallet::getter(fn block_numbers)]
	/// A vector storing block numbers for each block processed.
	pub type BlockNumbers<T: Config> =
		StorageMap<_, Blake2_128Concat, Vec<u8>, Vec<BlockNumberFor<T>>, OptionQuery>;

	#[pallet::storage]
	pub(super) type NodeMetrics<T: Config> =
		StorageMap<_, Blake2_128Concat, Vec<u8>, NodeMetricsData, OptionQuery>;

	#[pallet::storage]
	pub(super) type DowntimeStatus<T: Config> =
		StorageValue<_, Vec<OfflineStatus<BlockNumberFor<T>>>, ValueQuery>;

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
	pub(super) type BenchmarkResults<T: Config> =
		StorageMap<_, Blake2_128Concat, Vec<u8>, BenchmarkResult<BlockNumberFor<T>>, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn purge_deregistered_nodes_enabled)]
	pub type PurgeDeregisteredNodesEnabled<T: Config> = StorageValue<_, bool, ValueQuery>;

	#[pallet::error]
	pub enum Error<T> {
		MetricsNotFound,
		InvalidJson,
		InvalidCid,
		StorageOverflow,
		IpfsError,
		TooManyRequests,
	}

	#[pallet::storage]
	#[pallet::getter(fn requests_count)]
	pub type RequestsCount<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, u32, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn hardware_requests_count)]
	pub type HardwareRequestsCount<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, u32, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn hardware_requests_last_block)]
	pub type HardwareRequestsLastBlock<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, BlockNumberFor<T>, ValueQuery>;

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
				Call::update_pin_check_metrics {
					node_id,
					signature: _,
					total_pin_checks: _,
					successful_pin_checks: _,
				} => {
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
				Call::update_metrics_data {
					node_id,
					signature: _,
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
					block_number: _,
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
				_ => InvalidTransaction::Call.into(),
			}
		}
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_initialize(_n: BlockNumberFor<T>) -> Weight {
			let clear_interval = <T as pallet::Config>::RequestsClearInterval::get();

			// Clear entries every 10 blocks
			if _n % clear_interval.into() == 0u32.into() {
				// Clear all entries; limit is u32::MAX to ensure we get them all
				let _result = RequestsCount::<T>::clear(u32::MAX, None);
			}

			let hardware_clear_interval = <T as pallet::Config>::HardwareSubmitRequestsClearInterval::get();

			// Clear entries every 150 blocks
			if _n % hardware_clear_interval.into() == 0u32.into() {
				// Iterate through all entries in HardwareRequestsCount
				HardwareRequestsCount::<T>::iter().for_each(|(node_id, _count)| {
					let last_request_block = HardwareRequestsLastBlock::<T>::get(&node_id);
					
					// Check if 150 blocks have passed since the last request
					if _n.saturating_sub(last_request_block) >= hardware_clear_interval.into() {
						// Reset the requests count and last block for this node
						HardwareRequestsCount::<T>::remove(&node_id);
						HardwareRequestsLastBlock::<T>::remove(&node_id);
					}
				});
			}

			Self::handle_incorrect_registration(_n);

			// Only purge if the feature is enabled
			if Self::purge_deregistered_nodes_enabled() {
				Self::purge_nodes_if_deregistered_on_bittensor();
			}

			Weight::zero()
		}

		fn offchain_worker(block_number: BlockNumberFor<T>) {
			match UtilsPallet::<T>::fetch_node_id() {
				Ok(node_id) => {
					let node_info =
						RegistrationPallet::<T>::get_node_registration_info(node_id.clone());
					if node_info.is_some() {
						let node_info = node_info.unwrap();
						let node_type = node_info.node_type.clone();

						// update blocktime for uptime tracking
						let check_intetrval = <T as pallet::Config>::BlockCheckInterval::get();
						// last metrics updated at
						if block_number % check_intetrval.into() == Zero::zero() {
							Self::call_update_block_time(node_id.clone(), block_number);
							Self::do_update_metrics_data(
								node_id.clone(),
								node_type.clone(),
								block_number,
							);
							Self::save_hardware_info(node_id.clone(), node_type.clone());
						}

					}
				},
				Err(e) => {
					log::error!("Error fetching node identity: {:?}", e);
				},
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
			_node_type: NodeType,
		) -> DispatchResultWithPostInfo {
			ensure_none(origin)?;
			let _signature = signature;

			// Rate limit: maximum storage requests per block per user
			let max_requests_per_block = <T as pallet::Config>::MaxOffchainHardwareSubmitRequestsPerPeriod::get();
			let user_requests_count = HardwareRequestsCount::<T>::get(&node_id);
			ensure!(user_requests_count + 1 <= max_requests_per_block, Error::<T>::TooManyRequests);

			// Update user's storage requests count
			HardwareRequestsCount::<T>::insert(&node_id, user_requests_count + 1);

			// Update last request block
			HardwareRequestsLastBlock::<T>::insert(&node_id, <frame_system::Pallet<T>>::block_number());

			// Check if specs already exist and are the same
			// if let Some(existing_specs) = NodeSpecs::<T>::get(&node_id) {
			// 	if existing_specs == system_info {
			// 		log::info!("✅ System specs unchanged, skipping update");
			// 		return Ok(().into());
			// 	}
			// }

			// Function to create default metrics data
			let create_default_metrics = || {
				let geolocation = system_info
					.primary_network_interface
					.as_ref()
					.and_then(|interface| interface.network_details.as_ref())
					.map(|details| details.loc.clone())
					.unwrap_or_default(); // Use default if None
				NodeMetricsData {
					miner_id: node_id.clone(),
					bandwidth_mbps: system_info.network_bandwidth_mb_s,
					// converting mbs into bytes
					current_storage_bytes: (system_info.storage_total_mb * 1024 * 1024)
						- (system_info.storage_free_mb * 1024 * 1024),
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
					existing_metrics.current_storage_bytes =
						(system_info.storage_total_mb * 1024 * 1024)
							- (system_info.storage_free_mb * 1024 * 1024);
					existing_metrics.total_storage_bytes =
						system_info.storage_total_mb * 1024 * 1024;

					// Calculate storage growth rate
					existing_metrics.storage_growth_rate = if existing_metrics.uptime_minutes == 0
						|| existing_metrics.uptime_minutes == u32::MAX
					{
						existing_metrics.storage_growth_rate // Avoid division by zero or max value (remains unchanged)
					} else {
						(existing_metrics.current_storage_bytes
							/ existing_metrics.uptime_minutes as u64) as u32
					};

					// Update geolocation only if available
					if let Some(geolocation) = system_info
						.primary_network_interface
						.as_ref()
						.and_then(|interface| interface.network_details.as_ref())
						.map(|details| details.loc.clone())
					{
						existing_metrics.geolocation = geolocation.unwrap_or_default(); // Use default if None
					}
					existing_metrics.ipfs_zfs_pool_size = system_info.ipfs_zfs_pool_size;
					existing_metrics.ipfs_zfs_pool_alloc = system_info.ipfs_zfs_pool_alloc;
					existing_metrics.ipfs_zfs_pool_free = system_info.ipfs_zfs_pool_free;
					existing_metrics.primary_network_interface =
						system_info.primary_network_interface.clone();
					existing_metrics.disks = system_info.disks.clone();
					existing_metrics.ipfs_repo_size = system_info.ipfs_repo_size;
					existing_metrics.ipfs_storage_max = system_info.ipfs_storage_max;
					existing_metrics.cpu_model = system_info.cpu_model.clone();
					existing_metrics.cpu_cores = system_info.cpu_cores;
					existing_metrics.memory_mb = system_info.memory_mb;
					existing_metrics.free_memory_mb = system_info.free_memory_mb;
					existing_metrics.vm_count = system_info.vm_count;
					existing_metrics.disk_info = system_info.disk_info.clone();
					existing_metrics
				},
			);

			// Insert the updated or new metrics data into storage
			NodeMetrics::<T>::insert(node_id.clone(), metrics);

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

			// Rate limit: maximum storage requests per block per user
			let max_requests_per_block = <T as pallet::Config>::MaxOffchainRequestsPerPeriod::get();
			let user_requests_count = RequestsCount::<T>::get(&node_id);
			ensure!(user_requests_count + 1 <= max_requests_per_block, Error::<T>::TooManyRequests);

			// Update user's storage requests count
			RequestsCount::<T>::insert(&node_id, user_requests_count + 1);

			// Fetch existing metrics
			let mut metrics = NodeMetrics::<T>::get(&node_id).ok_or(Error::<T>::MetricsNotFound)?;

			// Update the metrics
			metrics.total_pin_checks += total_pin_checks;
			metrics.successful_pin_checks += successful_pin_checks;

			// Insert the updated metrics back into storage
			NodeMetrics::<T>::insert(node_id.clone(), metrics);

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
			_node_type: NodeType,
			_block_number: u32,
		) -> DispatchResultWithPostInfo {
			ensure_none(origin)?; // Ensure the call is unsigned

			// Rate limit: maximum storage requests per block per user
			let max_requests_per_block = <T as pallet::Config>::MaxOffchainRequestsPerPeriod::get();
			let user_requests_count = RequestsCount::<T>::get(&node_id);
			ensure!(user_requests_count + 1 <= max_requests_per_block, Error::<T>::TooManyRequests);

			// Update user's storage requests count
			RequestsCount::<T>::insert(&node_id, user_requests_count + 1);

			// Fetch existing metrics
			let mut metrics = NodeMetrics::<T>::get(&node_id).ok_or(Error::<T>::MetricsNotFound)?;
			let current_block_number = <frame_system::Pallet<T>>::block_number();

			// Update the metrics
			metrics.storage_proof_time_ms = storage_proof_time_ms;

			metrics.storage_growth_rate = ((sp_core::U256::from(metrics.current_storage_bytes)
				/ (current_block_number.into() * <T as pallet::Config>::BlockTime::get()))
			.low_u64()) as u32;

			metrics.latency_ms = latency_ms;
			metrics.total_latency_ms = metrics.total_latency_ms + latency_ms;
			metrics.total_times_latency_checked += 1;
			metrics.avg_response_time_ms =
				metrics.total_latency_ms / metrics.total_times_latency_checked;
			metrics.peer_count = peer_count;
			metrics.failed_challenges_count += failed_challenges_count;
			metrics.successful_challenges += successful_challenges;
			metrics.total_challenges += total_challenges;
			if uptime_minutes != 0 && uptime_minutes != u32::MAX {
				metrics.uptime_minutes = uptime_minutes;
			}
			if total_minutes != 0 && total_minutes != u32::MAX {
				metrics.total_minutes = total_minutes;
			}
			if consecutive_reliable_days != 0 && consecutive_reliable_days != u32::MAX {
				metrics.consecutive_reliable_days = consecutive_reliable_days;
			}
			if recent_downtime_hours != 0 && recent_downtime_hours != u32::MAX {
				metrics.recent_downtime_hours = recent_downtime_hours;
			}

			// Insert the updated metrics back into storage
			NodeMetrics::<T>::insert(node_id.clone(), metrics);

			Ok(().into())
		}

		#[pallet::call_index(4)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn update_block_time(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			_signature: <T as SigningTypes>::Signature,
			block_number: BlockNumberFor<T>,
		) -> DispatchResultWithPostInfo {
			ensure_none(origin)?; // Ensure the call is unsigned

			// Rate limit: maximum storage requests per block per user
			let max_requests_per_block = <T as pallet::Config>::MaxOffchainRequestsPerPeriod::get();
			let user_requests_count = RequestsCount::<T>::get(&node_id);
			ensure!(user_requests_count + 1 <= max_requests_per_block, Error::<T>::TooManyRequests);

			// Update user's storage requests count
			RequestsCount::<T>::insert(&node_id, user_requests_count + 1);

			// Fetch the existing vector of block numbers or initialize a new one
			let blocks_vec = BlockNumbers::<T>::get(node_id.clone()).unwrap_or_else(|| Vec::new());

			// Convert the existing blocks into a BTreeMap to remove duplicates
			let mut blocks: BTreeMap<BlockNumberFor<T>, ()> =
				blocks_vec.into_iter().map(|block| (block, ())).collect();

			let check_interval = <T as pallet::Config>::BlockCheckInterval::get();
			// Push the current block number and the preceding ones
			for i in (0..check_interval).rev() {
				let block_to_push = block_number - i.into();
				// Check if the block is already present in the storage
				if !blocks.contains_key(&block_to_push) {
					blocks.insert(block_to_push, ()); // Only add if it's not already present
				}
			}

			// Convert the BTreeMap back to a Vec for storage
			let unique_blocks: Vec<_> = blocks.keys().cloned().collect();
			BlockNumbers::<T>::insert(node_id, unique_blocks);

			Ok(().into())
		}

		/// Sudo function to enable purging of deregistered nodes
		#[pallet::call_index(5)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn sudo_enable_purge_deregistered_nodes(origin: OriginFor<T>) -> DispatchResult {
			ensure_root(origin)?;

			PurgeDeregisteredNodesEnabled::<T>::put(true);

			Self::deposit_event(Event::PurgeDeregisteredNodesStatusChanged { enabled: true });
			Ok(())
		}

		/// Sudo function to disable purging of deregistered nodes
		#[pallet::call_index(6)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn sudo_disable_purge_deregistered_nodes(origin: OriginFor<T>) -> DispatchResult {
			ensure_root(origin)?;

			PurgeDeregisteredNodesEnabled::<T>::put(false);

			Self::deposit_event(Event::PurgeDeregisteredNodesStatusChanged { enabled: false });
			Ok(())
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

			let response = pending.try_wait(deadline).map_err(|err| {
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
				let offchain_db =
					sp_io::offchain::local_storage_get(StorageKind::PERSISTENT, b"bandwidth_mbps");
				let bandwidth_mbps: Option<f32> = if let Some(value) = offchain_db {
					if value.len() == 4 {
						// Check for 4 bytes for f32
						// Convert the value back to f32
						let metric_value =
							f32::from_le_bytes([value[0], value[1], value[2], value[3]]);
						Some(metric_value) // Return the value wrapped in Some
					} else {
						None // Return None if the length is unexpected
					}
				} else {
					None // Return None if reading fails
				};

				// Include the off-chain bandwidth if it is not None
				if let Some(offchain_bandwidth) = bandwidth_mbps {
					system_info.network_bandwidth_mb_s =
						(system_info.network_bandwidth_mb_s as f32 * offchain_bandwidth) as u32; // Convert u32 to f32 for multiplication
				}

				// Read from off-chain storage for storage bytes
				let offchain_db_storage =
					sp_io::offchain::local_storage_get(StorageKind::PERSISTENT, b"storage_bytes");
				let storage_bytes: Option<u64> = if let Some(value) = offchain_db_storage {
					if value.len() == 8 {
						// Check for 8 bytes for u64
						// Convert the value back to u64
						let bytes_value = u64::from_le_bytes([
							value[0], value[1], value[2], value[3], value[4], value[5], value[6],
							value[7],
						]);
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

		pub fn do_update_metrics_data(
			node_id: Vec<u8>,
			node_type: NodeType,
			block_number: BlockNumberFor<T>,
		) {
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
				},
				Err(e) => {
					log::error!("Failed to fetch latency: {:?}", e);
					failed_challenges_count += 1;
					0 // Default value instead of early return
				},
			};

			// Fetch peer count
			total_challenges += 1;
			let peer_count = match Self::get_peer_count_from_health() {
				Ok(value) => {
					successful_challenges += 1;
					value
				},
				Err(e) => {
					log::error!("Failed to fetch peer count: {:?}", e);
					failed_challenges_count += 1;
					0 // Default value instead of early return
				},
			};

			// Fetch storage proof time
			total_challenges += 1;
			let storage_proof_time_ms = match Self::fetch_storage_proof_time_ms() {
				Ok(value) => {
					successful_challenges += 1;
					value
				},
				Err(e) => {
					log::error!("Failed to fetch storage proof time: {:?}", e);
					failed_challenges_count += 1;
					0 // Default value instead of early return
				},
			};

			// Modify uptime calculation to handle potential None case more gracefully
			let (uptime_minutes, total_uptime_minutes, consecutive_reliable_days, downtime_hours) =
				Self::calculate_uptime_and_recent_downtime(node_id.clone()).unwrap_or((0, 0, 0, 0));

			// Modify latency calculation for validator
			let mut adjusted_latency_ms = latency_ms;
			if node_type == NodeType::Validator {
				if let Some(value) =
					sp_io::offchain::local_storage_get(StorageKind::PERSISTENT, b"latency_bytes")
				{
					if value.len() == 4 {
						let latency_bytes =
							f32::from_le_bytes([value[0], value[1], value[2], value[3]]);
						adjusted_latency_ms = (latency_ms as f32 * latency_bytes) as u128;
					} else {
						log::error!("Unexpected latency bytes length: {:?}", value.len());
					}
				}
			}

			// Similar modification for peer count
			let mut adjusted_peer_count = peer_count;
			if node_type == NodeType::Validator {
				if let Some(value) =
					sp_io::offchain::local_storage_get(StorageKind::PERSISTENT, b"peers_bytes")
				{
					if value.len() == 4 {
						let peers_bytes =
							f32::from_le_bytes([value[0], value[1], value[2], value[3]]);
						adjusted_peer_count = (peer_count as f32 * peers_bytes) as u32;
					} else {
						log::error!("Unexpected peers bytes length: {:?}", value.len());
					}
				}
			}

			let block: u32 = block_number.saturated_into::<u32>();

			// Call update metrics with adjusted values
			Self::call_update_metrics_data(
				node_id,
				storage_proof_time_ms as u32,
				adjusted_latency_ms as u32,
				adjusted_peer_count,
				failed_challenges_count,
				successful_challenges,
				total_challenges,
				uptime_minutes,
				total_uptime_minutes,
				consecutive_reliable_days,
				downtime_hours,
				node_type,
				block,
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
			block_number: u32,
		) {
			// Create a unique lock for the update metrics data operation
			let mut lock =
				StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
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
					|account| UpdateMetricsDataPayload {
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
						block_number: block_number.clone(),
						public: account.public.clone(),
						_marker: PhantomData,
					},
					|payload, signature| Call::update_metrics_data {
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
						block_number: payload.block_number,
					},
				);

				// Process results of the transaction submission
				for (acc, res) in &results {
					match res {
						Ok(()) => {
							log::info!("[{:?}] Successfully submitted metrics data update", acc.id)
						},
						Err(e) => log::error!(
							"[{:?}] Error submitting metrics data update: {:?}",
							acc.id,
							e
						),
					}
				}
			} else {
				log::error!("❌ Could not acquire lock for updating metrics data");
			};
		}

		pub fn call_update_block_time(node_id: Vec<u8>, block_number: BlockNumberFor<T>) {
			// Create a unique lock for the update block time operation
			let mut lock =
				StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
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
					|account| UpdateBlockTimePayload {
						node_id: node_id.clone(),
						block_number,
						public: account.public.clone(),
						_marker: PhantomData,
					},
					|payload, signature| Call::update_block_time {
						node_id: payload.node_id,
						signature,
						block_number: payload.block_number,
					},
				);

				// Process results of the transaction submission
				for (acc, res) in &results {
					match res {
						Ok(()) => {
							log::info!("[{:?}] Successfully submitted block time update", acc.id)
						},
						Err(e) => log::error!(
							"[{:?}] Error submitting block time update: {:?}",
							acc.id,
							e
						),
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
			let mut lock =
				StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
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
					|account| UpdatePinCheckMetricsPayload {
						node_id: node_id.clone(),
						total_pin_checks,
						successful_pin_checks,
						public: account.public.clone(),
						_marker: PhantomData,
					},
					|payload, signature| Call::update_pin_check_metrics {
						node_id: payload.node_id,
						signature,
						total_pin_checks: payload.total_pin_checks,
						successful_pin_checks: payload.successful_pin_checks,
					},
				);

				// Process results of the transaction submission
				for (acc, res) in &results {
					match res {
						Ok(()) => log::info!(
							"[{:?}] Successfully submitted pin check metrics update",
							acc.id
						),
						Err(e) => {
							log::error!("[{:?}] Error submitting metrics update: {:?}", acc.id, e)
						},
					}
				}
			} else {
				log::error!("❌ Could not acquire lock for updating metrics");
			};
		}

		pub fn save_hardware_info(node_id: Vec<u8>, node_type: NodeType) {
			// Create a unique lock for the save hardware info operation
			let mut lock =
				StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
					b"executionunit::save_hardware_info_lock",
					LOCK_BLOCK_EXPIRATION,
					Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
				);

			if let Ok(_guard) = lock.try_lock() {
				match Self::fetch_hardware_info(node_type.clone()) {
					Ok(hardware_info) => {
						// Fetch signer accounts using AuthorityId
						let signer =
							Signer::<T, <T as pallet::Config>::AuthorityId>::all_accounts();

						if !signer.can_sign() {
							log::warn!("No accounts available for signing in signer.");
							return;
						}

						// Prepare and sign the payload
						let results = signer.send_unsigned_transaction(
							|account| SaveHardwareInfoPayload {
								node_id: node_id.clone(),
								system_info: hardware_info.clone(),
								public: account.public.clone(),
								node_type: node_type.clone(),
								_marker: PhantomData,
							},
							|payload, signature| Call::put_node_specs {
								node_id: payload.node_id,
								system_info: payload.system_info,
								signature,
								node_type: payload.node_type,
							},
						);

						// Process results of the transaction submission
						for (acc, res) in &results {
							match res {
								Ok(()) => log::info!(
									"[{:?}] Successfully submitted signed hardware update",
									acc.id
								),
								Err(e) => log::error!(
									"[{:?}] Error submitting hardware update: {:?}",
									acc.id,
									e
								),
							}
						}
					},
					Err(e) => {
						log::error!("❌ Error fetching hardware info: {:?}", e);
					},
				}
			} else {
				log::error!("❌ Could not acquire lock for saving hardware info");
			};
		}

		// pub fn process_pending_compute_requests() {
		// 	let active_compute_miners = pallet_registration::Pallet::<T>::get_all_active_compute_miners();
		// 	let pending_compute_requests = pallet_compute::Pallet::<T>::get_pending_compute_requests();

		// 	for compute_request in pending_compute_requests {
		// 		// Parse the plan technical description (assuming JSON format)
		// 		let plan_specs = match sp_std::str::from_utf8(&compute_request.plan_technical_description) {
		// 			Ok(specs_str) => {
		// 				match serde_json::from_str::<serde_json::Value>(specs_str) {
		// 					Ok(json) => json,
		// 					Err(e) => {
		// 						log::error!(
		// 							"Failed to parse plan technical description JSON: {:?}, raw string: {}",
		// 							e,
		// 							specs_str
		// 						);
		// 						continue; // Skip if JSON parsing fails
		// 					}
		// 				}
		// 			},
		// 			Err(e) => {
		// 				log::error!(
		// 					"Failed to convert plan technical description to UTF-8: {:?}, raw bytes: {:?}",
		// 					e,
		// 					compute_request.plan_technical_description
		// 				);
		// 				continue; // Skip if UTF-8 conversion fails
		// 			}
		// 		};

		// 		// Find a suitable miner with matching or exceeding specs
		// 		if let Some(suitable_miner) = active_compute_miners.iter().find(|miner| {
		// 			// Check if a specific miner ID is requested
		// 			let miner_id_match = match &compute_request.miner_id {
		// 				Some(requested_miner_id) => miner.node_id == *requested_miner_id,
		// 				None => true, // No specific miner requested, so all miners are valid
		// 			};

		// 			// Retrieve node metrics
		// 			let node_metrics_match = match Self::get_node_metrics(miner.node_id.clone()) {
		// 				Some(node_metrics) if miner_id_match => {
		// 					// Define the minimum resource reservation percentage
		// 					const MIN_RESERVED_PERCENTAGE: f64 = 0.1; // 10%

		// 					// Check CPU cores requirement with 10% reservation
		// 					let total_cpu_cores = node_metrics.cpu_cores as f64;
		// 					let cpu_cores_match = plan_specs["cpu_cores"].as_u64()
		// 						.map_or(true, |req_cores| {
		// 							let requested_cores = req_cores as f64;
		// 							(total_cpu_cores - requested_cores) / total_cpu_cores >= MIN_RESERVED_PERCENTAGE
		// 						});

		// 					// Check RAM requirement with 10% reservation (convert MB to GB)
		// 					let total_ram_gb = (node_metrics.free_memory_mb / 1024) as f64;
		// 					let ram_match = plan_specs["ram_gb"].as_u64()
		// 						.map_or(true, |req_ram| {
		// 							let requested_ram = req_ram as f64;
		// 							(total_ram_gb - requested_ram) / total_ram_gb >= MIN_RESERVED_PERCENTAGE
		// 						});

		// 					// Check storage requirement with 10% reservation
		// 					let current_storage_gb = (node_metrics.current_storage_bytes / (1024 * 1024 * 1024)) as f64;
		// 					let storage_match = plan_specs["storage_gb"].as_u64()
		// 						.map_or(true, |req_storage| {
		// 							let requested_storage = req_storage as f64;
		// 							(current_storage_gb - requested_storage) / current_storage_gb >= MIN_RESERVED_PERCENTAGE
		// 						});

		// 					// New check for SEV
		// 					let sev_match = plan_specs["is_sev_enabled"].as_bool()
		// 					.map_or(true, |req_sev| {
		// 						!req_sev || node_metrics.is_sev_enabled
		// 					});

		// 					// Return true if all specified requirements are met and 10% resources remain
		// 					cpu_cores_match && ram_match && storage_match && sev_match
		// 				},
		// 				_ => false // No metrics available or miner ID mismatch
		// 			};

		// 			node_metrics_match
		// 		}) {

		// 			let failed_requests = pallet_compute::Pallet::<T>::get_miner_compute_requests_with_failure(compute_request.request_id);

		// 			// Check if the suitable miner's node ID is not in the failed requests
		// 			let is_miner_failed = failed_requests.iter().any(|req|
		// 				req.miner_node_id == suitable_miner.node_id
		// 			);

		//             // only assign if not already assigned
		// 			if !is_miner_failed {
		// 			    // Assign the compute request to the suitable miner
		// 			    pallet_compute::Pallet::<T>::save_compute_request(
		// 			    	suitable_miner.node_id.clone(),
		// 			    	compute_request.plan_id,
		// 			    	compute_request.request_id,
		// 			    	compute_request.owner
		// 			    );
		// 			}

		// 		}
		// 	}
		// }

		pub fn get_node_metrics(node_id: Vec<u8>) -> Option<NodeMetricsData> {
			NodeMetrics::<T>::get(node_id)
		}

		/// Fetch the storage proof time in milliseconds
		pub fn fetch_storage_proof_time_ms() -> Result<u128, http::Error> {
			let url = <T as pallet::Config>::LocalRpcUrl::get();
			let method = T::GetReadProofRpcMethod::get();

			let json_payload = format!(
				r#"
			{{
				"id": 1,
				"jsonrpc": "2.0",
				"method": "{}",
				"params": [[], null]
			}}
			"#,
				method
			);

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

			let response = pending.try_wait(deadline).map_err(|err| {
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

			let json_payload = format!(
				r#"
			{{
				"id": 1,
				"jsonrpc": "2.0",
				"method": "{}",
				"params": [[], null]
			}}
			"#,
				method
			);

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

			let response = pending.try_wait(deadline).map_err(|err| {
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

			let json_payload = format!(
				r#"
			{{
				"id": 1,
				"jsonrpc": "2.0",
				"method": "{}",
				"params": [[], null]
			}}
			"#,
				method
			);

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

			let response = pending.try_wait(deadline).map_err(|err| {
				log::info!("Error getting Response: {:?}", err);
				sp_runtime::offchain::http::Error::DeadlineReached
			})??;

			if response.code != 200 {
				log::info!("Unexpected status code: {}", response.code);
				return Err(http::Error::Unknown);
			}

			let response_body = response.body().collect::<Vec<u8>>();

			let response_json: serde_json::Value =
				serde_json::from_slice(&response_body).map_err(|_| {
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

		fn block_time() -> u32 {
			<T as pallet::Config>::BlockTime::get()
		}

		/// Helper function to calculate uptime, total uptime, consecutive reliable days, and recent downtime hours
		pub fn calculate_uptime_and_recent_downtime(
			miner_node_id: Vec<u8>, // Key for the storage map
		) -> Option<(u32, u32, u32, u32)> {
			// Fetch the stored block numbers
			let block_numbers = BlockNumbers::<T>::get(&miner_node_id)?;

			if block_numbers.is_empty() {
				return None; // No blocks recorded
			}

			// Fetch the block time
			let block_time: u32 = Self::block_time();

			let mut uptime_seconds = 0u32;
			let mut total_uptime_seconds = 0u32;
			let mut recent_downtime_seconds = 0u32;

			let mut consecutive_reliable_days = 0u32;
			let mut current_day_uptime = 0u32;

			// Calculate metrics
			for mut i in 1..block_numbers.len() {
				let previous_block = block_numbers[i - 1].saturated_into::<u32>();
				let mut current_block = block_numbers[i].saturated_into::<u32>();

				// Skip until we find a current block that is greater than previous block
				while previous_block >= current_block && i < block_numbers.len() - 1 {
					i += 1; // Move to the next block
					current_block = block_numbers[i].saturated_into::<u32>();
				}

				// If we exit the loop and the current block is still not greater, skip this iteration
				if previous_block >= current_block {
					continue;
				}

				// Difference in blocks
				let block_diff = current_block.saturating_sub(previous_block);

				// Calculate uptime
				if block_diff == 1 {
					// Uptime is continuous
					uptime_seconds += block_time;
					total_uptime_seconds += block_time; // Only add to total uptime when there is actual uptime
					current_day_uptime += block_time;

					// Check if a full day of uptime is reached
					if current_day_uptime >= 86400 {
						// 86400 seconds = 1 day
						consecutive_reliable_days += 1;
						current_day_uptime = 0; // Reset for next day
					}
				} else {
					// Calculate recent downtime for gaps
					recent_downtime_seconds += (block_diff - 1) * block_time;
					current_day_uptime = 0; // Reset for gaps
				}
			}

			// Convert recent downtime seconds to hours
			let recent_downtime_hours = recent_downtime_seconds / 3600;

			let uptime_minutes = uptime_seconds / 60;
			let total_uptime_minutes = total_uptime_seconds / 60;

			Some((
				uptime_minutes,
				total_uptime_minutes,
				consecutive_reliable_days,
				recent_downtime_hours,
			))
		}

		// Helper function purge miners if deregistered on bittensor
		pub fn purge_nodes_if_deregistered_on_bittensor() {
			// get all nodes
			let active_stroage_miners =
				pallet_registration::Pallet::<T>::get_all_coldkey_active_nodes();

			// Retrieve UIDs from storage and match miners with uids
			// Iterate through all active miners
			for miner in active_stroage_miners {
				// Check if the miner's owner is in UIDs
				let is_registered = Self::is_owner_in_uids(&miner.owner);

				// update storage request and remove files
				if !is_registered {
					// unRegister and check if storage miner than unpin and update storage
					let _ = ipfs_pallet::Pallet::<T>::clear_miner_profile(miner.node_id);
				}
			}
		}

		pub fn is_owner_in_uids(owner: &T::AccountId) -> bool {
			if let Ok(account_bytes) = owner.encode().try_into() {
				// Retrieve UIDs from storage
				let uids = UIDs::<T>::get();
				let account = AccountId32::new(account_bytes);
				let owner_ss58 = AccountId32::new(account.encode().try_into().unwrap_or_default())
					.to_ss58check();
				// Check if validator is in UIDs or matches the keep address
				let is_in_uids =
					uids.iter().any(|uid| uid.substrate_address.to_ss58check() == owner_ss58);
				return is_in_uids;
			}
			false
		}

		/// Get randomness from BABE
		fn get_babe_randomness() -> [u8; 32] {
			// Get current subject for randomness
			let subject = sp_io::hashing::blake2_256(b"babe_randomness");

			// Try to get randomness from one epoch ago
			let (random_hash, _block_number) = <RandomnessFromOneEpochAgo<T> as Randomness<
				T::Hash,
				BlockNumberFor<T>,
			>>::random(&subject);
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
					// Check if the miner has been registered for more than 36 blocks
					if current_block_number - miner.registered_at > 36u32.into() {
						let blocks_online = Self::block_numbers(miner.node_id.clone());

						if let Some(blocks) = blocks_online {
							if let Some(&last_block) = blocks.last() {
								let difference = current_block_number - last_block;
								// Check if the difference exceeds 500 for deregistration
								if difference > 1500u32.into() {
									// Call your deregistration logic here
									Self::unregister_and_remove_metrics(miner.node_id.clone());
								}
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
					let last_block =
						u32::from_be_bytes(last_block_bytes.try_into().unwrap_or([0; 4]));
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
			node_ids.into_iter().map(|node_id| NodeMetrics::<T>::get(node_id)).collect()
		}

		pub fn get_active_nodes_metrics_by_type(
			node_type: NodeType,
		) -> Vec<Option<NodeMetricsData>> {
			// First, get all active nodes of the specified type
			let active_node_ids =
				pallet_registration::Pallet::<T>::get_active_nodes_by_type(node_type);

			// Then, fetch metrics for these active nodes
			Self::get_node_metrics_batch(active_node_ids)
		}

		pub fn unregister_and_remove_metrics(node_id: Vec<u8>) {
			pallet_registration::Pallet::<T>::do_unregister_node(node_id.clone());
			NodeMetrics::<T>::remove(&node_id);
			BlockNumbers::<T>::remove(&node_id);
		}

		// pub fn handle_unpin_request_assignment(
		// 	node_info: NodeInfo<BlockNumberFor<T>, T::AccountId>,
		// 	block_number: BlockNumberFor<T>,
		// ) -> Result<(), DispatchError> {
		// 	let initial_unpin_requests =
		// 		IpfsPallet::<T>::get_unassigned_unpin_requests_for_validator(
		// 			node_info.owner.clone(),
		// 		);	

		// 		// Call the API if there are unpin requests
		// 		if !initial_unpin_requests.is_empty() {
		// 			log::info!("Processing unpin requests for node: {:?}", node_info.owner);
		// 			Self::process_unpin_request_with_service(initial_unpin_requests)
		// 				.map_err(|e| {
		// 					log::error!("Failed to process unpin requests: {:?}", e);
		// 					DispatchError::Other("Failed to process unpin requests")
		// 				})?;
		// 		}

		// 	Ok(())
		// }

		// pub fn process_unpin_request_with_service(
		// 	unpin_requests: Vec<StorageUnpinRequest<T::AccountId>>,
		// ) -> Result<(), http::Error> {
		// 	let api_url = format!("{}/api/ipfs/unpin", T::IpfsServiceUrl::get());
		
		// 	// Convert unpin requests to API format
		// 	let mut api_unpin_items = Vec::new();

		// 	for request in unpin_requests {
		// 		if let Ok(owner_bytes) = request.owner.encode().try_into() {
		// 			let owner_account = AccountId32::new(owner_bytes);
		// 			let owner_ss58 = owner_account.to_ss58check();
					
		// 			// Convert file hash to a consistent format for storage lookup
		// 			let file_hash_key = hex::decode(request.file_hash.clone()).unwrap_or_else(|e| {
		// 				log::error!("Failed to decode file_hash {:?}: {:?}", request.file_hash, e);
		// 				Vec::new()
		// 			});
					
		// 			let update_hash_vec: Vec<u8> = file_hash_key.into();
		// 			let file_hash = String::from_utf8_lossy(&update_hash_vec).into_owned();
					
		// 			api_unpin_items.push(ApiUnpinItem {
		// 				cid: file_hash,
		// 				owner: owner_ss58,
		// 			});
		// 		}
		// 	}
		
		// 	let api_unpin_request = ApiUnpinRequest {
		// 		items: api_unpin_items,
		// 	};
		
		// 	// Serialize the unpin request to JSON
		// 	let json_payload = serde_json::to_string(&api_unpin_request)
		// 		.map_err(|_| {
		// 			log::error!("Failed to serialize unpin request");
		// 			http::Error::Unknown
		// 		})?;
			
		// 	log::info!("Unpin request payload: {:?}", json_payload);
		
		// 	let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(30_000));

		// 	let request = sp_runtime::offchain::http::Request::post(
		// 		&api_url,
		// 		vec![json_payload.into_bytes()]
		// 	);
		
		// 	let pending = request
		// 		.add_header("Content-Type", "application/json")
		// 		.deadline(deadline)
		// 		.send()
		// 		.map_err(|err| {
		// 			log::error!("Error making unpin request: {:?}", err);
		// 			http::Error::IoError
		// 		})?;
		
		// 	let response = pending
		// 		.try_wait(deadline)
		// 		.map_err(|err| {
		// 			log::error!("Error getting unpin response: {:?}", err);
		// 			http::Error::DeadlineReached
		// 		})??;
		
		// 	if response.code != 200 {
		// 		log::error!("Unexpected status code for unpin request: {}", response.code);
		// 	}
		
		// 	// Read response body
		// 	let response_body = response.body().collect::<Vec<u8>>();
			
		// 	// Log the raw response body
		// 	if let Ok(body_str) = sp_std::str::from_utf8(&response_body) {
		// 		log::info!("Unpin Response Body: {}", body_str);
		// 	} else {
		// 		log::error!("Failed to convert unpin response body to UTF-8");
		// 	}
		
		// 	Ok(())
		// }

		pub fn process_storage_request_with_service(
			storage_request: &mut StorageRequest<T::AccountId, BlockNumberFor<T>>,
			main_req_hash: &mut Vec<u8>,
		) -> Result<(), http::Error> {
			let api_url = format!("{}/api/ipfs/process-storage", T::IpfsServiceUrl::get());

			if let Ok(account_bytes) = storage_request.owner.encode().try_into() {
				if let Ok(vali_account_bytes) = storage_request.selected_validator.encode().try_into() {
					let account = AccountId32::new(account_bytes);
					let owner_ss58 =
						AccountId32::new(account.encode().try_into().unwrap_or_default())
							.to_ss58check();
					let vali_account = AccountId32::new(vali_account_bytes);
					let vali_owner_ss58 =
						AccountId32::new(vali_account.encode().try_into().unwrap_or_default())
							.to_ss58check();

					// Convert BoundedVec fields to String
					let api_storage_request = ApiStorageRequest {
						total_replicas: storage_request.total_replicas,
						owner: owner_ss58.clone(),
						file_hash: String::from_utf8_lossy(&storage_request.file_hash).into_owned(),
						file_name: String::from_utf8_lossy(&storage_request.file_name).into_owned(),
						main_req_hash: String::from_utf8_lossy(&main_req_hash).into_owned(),
						last_charged_at: storage_request.last_charged_at.try_into().unwrap_or(0),
						created_at: storage_request.created_at.try_into().unwrap_or(0),
						miner_ids: storage_request.miner_ids.as_ref().map(|ids| 
							ids.iter().map(|id| String::from_utf8_lossy(id).into_owned()).collect()
						),
						selected_validator: vali_owner_ss58.clone(),
						is_assigned: storage_request.is_assigned,
					};

					// Serialize the storage request to JSON
					let json_payload = serde_json::to_string(&api_storage_request)
						.map_err(|_| {
							log::error!("Failed to serialize storage request");
							http::Error::Unknown
						})?;
					log::info!("json payload is : {:?}",api_storage_request);
				
					let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(30_000));
					
					let request = sp_runtime::offchain::http::Request::post(
						&api_url, 
						vec![json_payload.into_bytes()]
					);
					
					let pending = request
						.add_header("Content-Type", "application/json")
						.deadline(deadline)
						.send()
						.map_err(|err| {
							log::error!("Error making Request: {:?}", err);
							http::Error::IoError
						})?;
				
					let response = pending
						.try_wait(deadline)
						.map_err(|err| {
							log::error!("Error getting Response: {:?}", err);
							http::Error::DeadlineReached
						})??;
				
					if response.code != 200 {
						log::error!("Unexpected status code: {}", response.code);
					}
				
					// Read response body
					let response_body = response.body().collect::<Vec<u8>>();
					
					// Log the raw response body as a string
					if let Ok(body_str) = sp_std::str::from_utf8(&response_body) {
						log::info!("Response Body: {}", body_str);
					} else {
						log::error!("Failed to convert response body to UTF-8");
					}
				}
			}
		
			Ok(())
		}

		pub fn pin_profiles_via_service() -> Result<(), http::Error> {
			let api_url = format!("{}/api/ipfs/pin-profiles", T::IpfsServiceUrl::get());
			
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(30_000));
			
			let request = sp_runtime::offchain::http::Request::get(&api_url);
			
			let pending = request
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::error!("Error making Request: {:?}", err);
					http::Error::IoError
				})?;
			
			let response = pending
				.try_wait(deadline)
				.map_err(|err| {
					log::error!("Error getting Response: {:?}", err);
					http::Error::DeadlineReached
				})??;
			
			if response.code != 200 {
				log::error!("Unexpected status code: {}", response.code);
				return Err(http::Error::Unknown);
			}
			
			// Read response body
			let response_body = response.body().collect::<Vec<u8>>();
			
			// Log the raw response body as a string
			if let Ok(body_str) = sp_std::str::from_utf8(&response_body) {
				log::info!("Response Body for Profile pinning is : {}", body_str);
			} else {
				log::error!("Failed to convert response body to UTF-8");
			}

			Ok(())
		}

		pub fn update_all_miners_with_profiles(
			_selected_validator: T::AccountId, // Kept for signature compatibility, but unused
		) -> Result<(), DispatchError> {
			// Collect all CIDs from MinerProfile
			let mut all_cids: Vec<FileHash> = Vec::new();
			MinerProfile::<T>::iter().for_each(|(_miner_id, cid)| {
				let file_hash = BoundedVec::try_from(cid.to_vec())
					.unwrap_or_else(|_| {
						BoundedVec::try_from(vec![0; MAX_NODE_ID_LENGTH as usize]).unwrap()
					});
				all_cids.push(file_hash);
			});
		
			// Collect all CIDs from UserProfile
			UserProfile::<T>::iter().for_each(|(_user_id, cid)| {
				all_cids.push(cid);
			});
		
			// Pin each CID locally
			for cid in all_cids {
				// Convert CID to string for IPFS pinning
				let cid_vec = cid.to_vec();
				let cid_str = sp_std::str::from_utf8(&cid_vec)
					.map_err(|_| Error::<T>::InvalidCid)?;
		
				// Pin the CID using the existing IPFS pallet function
				match IpfsPallet::<T>::pin_file_to_ipfs(cid_str) {
					Ok(_) => {
						log::info!("Successfully pinned profile CID: {}", cid_str);
					},
					Err(e) => {
						log::warn!("Failed to pin profile CID {}: {:?}", cid_str, e);
						// Continue with the next CID instead of failing the entire function
					},
				}
			}
			Ok(())
		}

		pub fn perform_pin_checks_to_miners() {
			// Iterate over all miners in MinerProfile
			MinerProfile::<T>::iter().for_each(|(miner_node_id, cid)| {
				// Fetch the CID content (JSON array of pin requests)
				if let Ok(cid_str) = sp_std::str::from_utf8(&cid) {
					match IpfsPallet::<T>::fetch_ipfs_content(cid_str) {
						Ok(content) => {
							if let Ok(pin_requests) = serde_json::from_slice::<Vec<serde_json::Value>>(&content) {
								// Process each pin request in the miner's profile
								for pin_request in pin_requests {
									if let Some(file_hash_str) = pin_request.get("file_hash").and_then(|v| v.as_str()) {
										// Since file_hash in JSON is hex-encoded 
										let file_hash_vec = hex::decode(file_hash_str).unwrap_or_else(|e| {
											log::error!("Failed to decode file_hash {}: {:?}", file_hash_str, e);
											Vec::new()
										});
										
										if file_hash_vec.is_empty() {
											continue;
										}
		
										// Fetch all IPFS nodes that have pinned this file hash
										let ipfs_nodes_who_pinned: Vec<Vec<u8>> = IpfsPallet::<T>::fetch_cid_pinned_nodes(&file_hash_vec)
											.unwrap_or_else(|e| {
												log::error!("Failed to fetch pinned nodes for file_hash {:?}: {:?}", file_hash_str, e);
												Vec::new()
											});
		
										// Get miner registration info
										match pallet_registration::Pallet::<T>::get_registered_node(miner_node_id.clone().into()) {
											Ok(node_info) => {
												let mut total_pin_checks = 0;
												let mut successful_pin_checks = 0;
		
												// Check if the miner's IPFS node has pinned the file
												if let Some(ipfs_node_id) = node_info.ipfs_node_id {
													total_pin_checks += 1;
													if ipfs_nodes_who_pinned.contains(&ipfs_node_id) {
														successful_pin_checks += 1;
													} else {
														// Miner failed to pin the file
														// We need to update MinerProfile by removing this pin request
														let bounded_miner_node_id = miner_node_id.clone();
														let existing_cid = MinerProfile::<T>::get(&bounded_miner_node_id);
														
														if !existing_cid.is_empty() {
															if let Ok(existing_cid_str) = sp_std::str::from_utf8(&existing_cid) {
																if let Ok(existing_content) = IpfsPallet::<T>::fetch_ipfs_content(existing_cid_str) {
																	if let Ok(mut existing_requests) = serde_json::from_slice::<Vec<serde_json::Value>>(&existing_content) {
																		// Remove the failed pin request
																		existing_requests.retain(|req| {
																			req.get("file_hash").and_then(|v| v.as_str()) != Some(file_hash_str)
																		});
		
																		// Update MinerProfile if there are still requests
																		if !existing_requests.is_empty() {
																			let updated_json = serde_json::to_string(&existing_requests)
																				.unwrap_or_else(|e| {
																					log::error!("Failed to serialize updated requests: {:?}", e);
																					String::new()
																				});
																			
																			if let Ok(new_cid) = IpfsPallet::<T>::pin_file_to_ipfs(&updated_json) {
																				let new_cid_bounded = BoundedVec::try_from(new_cid.into_bytes())
																					.unwrap_or_default();
																				MinerProfile::<T>::insert(&bounded_miner_node_id, new_cid_bounded);
																			}
																		} else {
																			// Remove the entry if no requests remain
																			MinerProfile::<T>::remove(&bounded_miner_node_id);
																		}
																	}
																}
															}
														}
		
														log::info!(
															"Miner {:?} failed to pin file_hash {:?}, removed from profile",
															sp_std::str::from_utf8(&miner_node_id).unwrap_or("<Invalid UTF-8>"),
															file_hash_str
														);
													}
												} else {
													log::info!(
														"Node ID {:?} does not have an associated IPFS node ID",
														sp_std::str::from_utf8(&node_info.node_id).unwrap_or("<Invalid UTF-8>")
													);
												}
		
												// Update node metrics
												Self::call_update_pin_check_metrics(
													miner_node_id.clone().into(),
													total_pin_checks,
													successful_pin_checks,
												);
												log::info!("Successfully performed min checks to miners using profile");
											},
											Err(err) => log::info!("Failed to get node info for miner {:?}: {}", miner_node_id, err),
										}
									} else {
										log::error!("Pin request missing file_hash for miner {:?}", miner_node_id);
									}
								}
							} else {
								log::error!("Failed to parse miner profile JSON for miner {:?}", miner_node_id);
							}
						},
						Err(e) => log::error!("Failed to fetch CID content for miner {:?}: {:?}", miner_node_id, e),
					}
				} else {
					log::error!("Invalid CID in MinerProfile for miner {:?}", miner_node_id);
				}
			});
		}
	}
}

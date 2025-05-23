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
use pallet_utils::MetricsInfoProvider;

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
	use frame_support::{pallet_prelude::*, traits::Randomness};
	use frame_system::{
		offchain::{
			AppCrypto, SendTransactionTypes, SendUnsignedTransaction, Signer, SigningTypes,
		},
		pallet_prelude::*,
	};
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
			Duration,
		},
		traits::Zero,
		AccountId32,
	};
	use sp_std::prelude::*;
	use sp_runtime::Saturating;
	use serde_json::Value;
	use sp_std::collections::btree_map::BTreeMap;
	use pallet_registration::ColdkeyNodeRegistration;
	use pallet_registration::NodeRegistration;

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

		#[pallet::constant]
        type LocalDefaultSpecVersion: Get<u32>;
    
        #[pallet::constant]
        type LocalDefaultGenesisHash: Get<&'static str>;

		type ConsensusPeriod: Get<BlockNumberFor<Self>>;
		
		#[pallet::constant]
        type ConsensusThreshold: Get<u32>;

		#[pallet::constant]
		type ConsensusSimilarityThreshold: Get<u32>; // Percentage (e.g., 85 for 85%)

		#[pallet::constant]
		type EpochDuration: Get<u32>;

		/// The block interval at which to update miner reputations
		#[pallet::constant]
		type ReputationUpdateInterval: Get<u32>;
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
		/// Emitted when storage size is below 2TB.
		StorageBelowTwoTB { node_id: Vec<u8> },
		/// Emitted when primary network interface is not provided.
		NoPrimaryNetworkInterface { node_id: Vec<u8> },
		/// Emitted when disks array is empty.
		EmptyDisksArray { node_id: Vec<u8> },
		MemoryExceedsFiveTB { node_id: Vec<u8> },
		ConsensusReached { miner_id: Vec<u8>, total_pin_checks: u32, successful_pin_checks: u32 },
        ConsensusFailed { miner_id: Vec<u8> },
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
	#[pallet::getter(fn purge_deregistered_nodes_enabled)]
	pub type PurgeDeregisteredNodesEnabled<T: Config> = StorageValue<_, bool, ValueQuery>;

	#[pallet::storage]
    pub(super) type TemporaryPinReports<T: Config> =
        StorageDoubleMap<_, Blake2_128Concat, Vec<u8>, Blake2_128Concat, T::AccountId, MinerPinMetrics, OptionQuery>;

	#[pallet::error]
	pub enum Error<T> {
		MetricsNotFound,
		InvalidJson,
		InvalidCid,
		StorageOverflow,
		IpfsError,
		TooManyRequests,
		NodeNotRegistered,
		InvalidNodeType,
		StorageBelowTwoTB,
		/// Primary network interface is not provided.
		NoPrimaryNetworkInterface,
		/// Disks array is empty.
		EmptyDisksArray,
		MemoryExceedsFiveTB,
		ConsensusNotReached,
		SuccessfulPinsExceedTotal
	}

	#[pallet::storage]
	#[pallet::getter(fn requests_count)]
	pub type RequestsCount<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, u32, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn hardware_requests_count)]
	pub type HardwareRequestsCount<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, u32, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn total_pin_checks_per_epoch)]
	pub type TotalPinChecksPerEpoch<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, u32, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn successful_pin_checks_per_epoch)]
	pub type SuccessfulPinChecksPerEpoch<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, u32, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn total_ping_checks_per_epoch)]
	pub type TotalPingChecksPerEpoch<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, u32, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn successful_ping_checks_per_epoch)]
	pub type SuccessfulPingChecksPerEpoch<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, u32, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn hardware_requests_last_block)]
	pub type HardwareRequestsLastBlock<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, BlockNumberFor<T>, ValueQuery>;

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

			// Clear entries every 1500 blocks
			if _n % hardware_clear_interval.into() == 0u32.into() {
				// Iterate through all entries in HardwareRequestsCount
				HardwareRequestsCount::<T>::iter().for_each(|(node_id, _count)| {
					let last_request_block = HardwareRequestsLastBlock::<T>::get(&node_id);
					
					// Check if 1500 blocks have passed since the last request
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


			let consensus_period = T::ConsensusPeriod::get();
            if _n % consensus_period == 0u32.into() {
                Self::apply_consensus();
            }

			let epoch_clear_interval = <T as pallet::Config>::EpochDuration::get();
			if _n % epoch_clear_interval.into() == 0u32.into() {
				// Clear per-epoch pin stats for all miners
				let _ = TotalPinChecksPerEpoch::<T>::clear(u32::MAX, None);
				let _ = SuccessfulPinChecksPerEpoch::<T>::clear(u32::MAX, None);
				let _ = TotalPingChecksPerEpoch::<T>::clear(u32::MAX, None);
				let _ = SuccessfulPingChecksPerEpoch::<T>::clear(u32::MAX, None);
			}

			// Clean up NodeMetrics if not registered
			for (node_id, _) in NodeMetrics::<T>::iter() {
				let is_registered = ColdkeyNodeRegistration::<T>::contains_key(&node_id)
					|| NodeRegistration::<T>::contains_key(&node_id);
				if !is_registered {
					NodeMetrics::<T>::remove(&node_id);
				}
			}

			// Clean up BlockNumbers if node not registered
			for (node_id, _) in BlockNumbers::<T>::iter() {
				let is_registered = ColdkeyNodeRegistration::<T>::contains_key(&node_id)
					|| NodeRegistration::<T>::contains_key(&node_id);
				if !is_registered {
					BlockNumbers::<T>::remove(&node_id);
				}
			}

			let reputation_update_interval = T::ReputationUpdateInterval::get();
			if _n % reputation_update_interval.into() == 0u32.into() {
				Self::update_all_active_miners_reputation();
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
							let _ = Self::do_update_metrics_data(
								node_id.clone(),
								node_type.clone(),
								block_number,
							);
							let _ = Self::save_hardware_info(node_id.clone(), node_type.clone());
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
		#[pallet::call_index(0)]
		#[pallet::weight((0, Pays::No))]
		pub fn add_hardware_info(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			system_info: SystemInfo,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Check if the node is registered
			let node_info = RegistrationPallet::<T>::get_registered_node_for_owner(&who);
			ensure!(node_info.is_some(), Error::<T>::NodeNotRegistered);

			// Unwrap safely after checking it's Some
			let node_info = node_info.unwrap();

			// Check if the node type is Validator
			ensure!(node_info.node_id == node_id, Error::<T>::NodeNotRegistered);

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

			// // Define 2TB in bytes (2TB = 2 * 1024 * 1024 * 1024 * 1024 bytes)
			// const TWO_TB_IN_BYTES: u64 = 2 * 1024 * 1024 * 1024 * 1024;

			// // Define 5TB in megabytes (5TB = 5 * 1024 * 1024 MB)
			// const FIVE_TB_IN_MB: u64 = 5 * 1024 * 1024;

			// // Calculate storage values
			// let current_storage_bytes = (system_info.storage_total_mb * 1024 * 1024)
			// - (system_info.storage_free_mb * 1024 * 1024);
			// let total_storage_bytes = system_info.storage_total_mb * 1024 * 1024;
		
			// if current_storage_bytes < TWO_TB_IN_BYTES || total_storage_bytes < TWO_TB_IN_BYTES {
			// 	Self::deposit_event(Event::StorageBelowTwoTB { node_id: node_id.clone() });
			// 	return Err(Error::<T>::StorageBelowTwoTB.into());
			// }

			// // Check if primary_network_interface is None
			// if system_info.primary_network_interface.is_none() {
			// 	Self::deposit_event(Event::NoPrimaryNetworkInterface { node_id: node_id.clone() });
			// 	return Err(Error::<T>::NoPrimaryNetworkInterface.into());
			// }
		
			// // Check if disks array is empty
			// if system_info.disks.is_empty() {
			// 	Self::deposit_event(Event::EmptyDisksArray { node_id: node_id.clone() });
			// 	return Err(Error::<T>::EmptyDisksArray.into());
			// }

			// // Check if memory or free memory exceeds 5TB
			// if system_info.memory_mb > FIVE_TB_IN_MB || system_info.free_memory_mb > FIVE_TB_IN_MB {
			// 	Self::deposit_event(Event::MemoryExceedsFiveTB { node_id: node_id.clone() });
			// 	return Err(Error::<T>::MemoryExceedsFiveTB.into());
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
			log::info!("✅ Successfully updated hardware info ");
			Ok(().into())
		}

		#[pallet::call_index(1)]
		#[pallet::weight((0, Pays::No))]
		pub fn metrics_data_update(
			origin: OriginFor<T>,
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
			block_number: u32,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Check if the node is registered
			let node_info = RegistrationPallet::<T>::get_registered_node_for_owner(&who);
			ensure!(node_info.is_some(), Error::<T>::NodeNotRegistered);

			// Unwrap safely after checking it's Some
			let node_info = node_info.unwrap();

			// Check if the node type is Validator
			ensure!(node_info.node_id == node_id, Error::<T>::NodeNotRegistered);

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

			// // Fetch the existing vector of block numbers or initialize a new one
			// let blocks_vec = BlockNumbers::<T>::get(node_id.clone()).unwrap_or_else(|| Vec::new());

			// // Convert the existing blocks into a BTreeMap to remove duplicates
			// let mut blocks: BTreeMap<BlockNumberFor<T>, ()> =
			// 	blocks_vec.into_iter().map(|block| (block, ())).collect();
					
			// let check_interval = <T as pallet::Config>::BlockCheckInterval::get();
			// // Push the current block number and the preceding ones
			// for i in (0..check_interval).rev() {
			// 	let block_to_push = block_number - i;
			// 	// Check if the block is already present in the storage
			// 	if !blocks.contains_key(&block_to_push.into()) {
			// 		blocks.insert(block_to_push.into(), ()); // Only add if it's not already present
			// 	}
			// }

			// // Convert the BTreeMap back to a Vec for storage
			// let unique_blocks: Vec<_> = blocks.keys().cloned().collect();
			// BlockNumbers::<T>::insert(node_id, unique_blocks);

			// Store only the latest block number as a single-element vector
			let block_vec : Vec<BlockNumberFor<T>> = vec![block_number.into()];
			BlockNumbers::<T>::insert(node_id, block_vec);
			log::info!("✅ Successfully updated metrics");
			log::info!("✅ Successfully updated block numbers");
			Ok(().into())
		}

		#[pallet::call_index(2)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn update_pin_check_metrics(
			origin: OriginFor<T>,
			miners_metrics: Vec<MinerPinMetrics>,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;
		
			// Check if the node is registered
			let node_info = RegistrationPallet::<T>::get_registered_node_for_owner(&who);
			ensure!(node_info.is_some(), Error::<T>::NodeNotRegistered);
		
			// Unwrap safely after checking it's Some
			let node_info = node_info.unwrap();
		
			// Check if the node type is Validator
			ensure!(node_info.node_type == NodeType::Validator, Error::<T>::InvalidNodeType);
		
			// Rate limit: maximum storage requests per block per user
			let max_requests_per_block = <T as pallet::Config>::MaxOffchainRequestsPerPeriod::get();
			let user_requests_count = RequestsCount::<T>::get(node_info.node_id.clone());
			ensure!(
				user_requests_count + 1u32 <= max_requests_per_block,
				Error::<T>::TooManyRequests
			);
			
			// Validate metrics and update storage
			for miner in miners_metrics.iter() {
				ensure!(
					miner.successful_pin_checks <= miner.total_pin_checks,
					Error::<T>::SuccessfulPinsExceedTotal
				);
			}
		
			// Update user's storage requests count
			RequestsCount::<T>::insert(node_info.node_id.clone(), user_requests_count + miners_metrics.len() as u32);
		
            for miner in miners_metrics {
				log::info!("submitting miner id {}", String::from_utf8_lossy(&miner.node_id));
                TemporaryPinReports::<T>::insert(&miner.node_id, &who, miner.clone());
            }
		
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

		pub fn do_update_metrics_data(
			node_id: Vec<u8>,
			node_type: NodeType,
			block_number: BlockNumberFor<T>,
		)  -> Result<(), &'static str> {
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
				block_number,
			)
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
			block_number: BlockNumberFor<T>,
		)  -> Result<(), &'static str> {

			// Call fn to get Signed Hex
			let hex_result = Self::get_hex_for_submit_metrics(node_id, storage_proof_time_ms, latency_ms, peer_count, failed_challenges_count, successful_challenges, total_challenges, uptime_minutes, total_minutes, consecutive_reliable_days, recent_downtime_hours, block_number).map_err(|e| {
				log::error!("❌ Failed to get signed metrics hex: {:?}", e);
				"Failed to get signed metrics hex"
			})?;

			let local_rpc_url = <T as pallet::Config>::LocalRpcUrl::get();
			// Now use the hex_result in the function
			UtilsPallet::<T>::submit_to_chain(&local_rpc_url, &hex_result)
				.map_err(|e| {
					log::error!("❌ Failed to submit the extrinsic for hardware info: {:?}", e);
					"Failed to submit the extrinsic for hardware info"
				})?;
			Ok(())
		}

		// update_block_time_update
		pub fn save_hardware_info(node_id: Vec<u8>, node_type: NodeType) -> Result<(), &'static str> {
			match Self::fetch_hardware_info(node_type.clone()) {
				Ok(hardware_info) => {
					// Call fn to get Signed Hex
					let hex_result = Self::get_hex_for_submit_hardware(node_id, hardware_info).map_err(|e| {
						log::error!("❌ Failed to get signed weight hex: {:?}", e);
						"Failed to get signed weight hex"
					})?;
		
					let local_rpc_url = <T as pallet::Config>::LocalRpcUrl::get();
					// Now use the hex_result in the function
					UtilsPallet::<T>::submit_to_chain(&local_rpc_url, &hex_result)
						.map_err(|e| {
							log::error!("❌ Failed to submit the extrinsic for hardware info: {:?}", e);
							"Failed to submit the extrinsic for hardware info"
						})?;
						
					log::info!("✅ Successfully submitted the signed extrinsic for hardware info");
					Ok(())
				}
				Err(e) => {
					log::error!("❌ Error fetching hardware info: {:?}", e);
					Err("Error fetching hardware info")
				}
			}
		}

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

			// Since we only store the last block number, take the last element and clone to get owned value
			let last_block = block_numbers.last()?.clone().try_into().ok()?; // Convert BlockNumberFor<T> to u32
			
			// Get the current block number
			let current_block = <frame_system::Pallet<T>>::block_number().saturated_into::<u32>();

			// Fetch the block time
			let block_time: u32 = Self::block_time();

			let mut uptime_seconds = 0u32;
			let mut total_uptime_seconds = 0u32;
			let mut recent_downtime_seconds = 0u32;
			let mut consecutive_reliable_days = 0u32;

			// Calculate the block difference
			let block_diff = current_block.saturating_sub(last_block);
			let check_intetrval = <T as pallet::Config>::BlockCheckInterval::get();
			// If the last block is within 300 blocks (since updates happen every 300 blocks)
			if block_diff <= check_intetrval {
				// Assume the node was up for the entire period since the last recorded block
				uptime_seconds = block_diff * block_time;
				total_uptime_seconds = uptime_seconds;

				// Calculate consecutive reliable days
				let total_uptime_days = uptime_seconds / 86400; // 86400 seconds = 1 day
				consecutive_reliable_days = total_uptime_days;
			} else {
				// If the difference is more than 300 blocks, assume the node was only up at the last recorded block
				// and has been down since the next expected update (last_block + 300)
				uptime_seconds = block_time; // Uptime only for the recorded block
				total_uptime_seconds = uptime_seconds;

				// Calculate downtime from (last_block + 300) to current_block
				let downtime_blocks = block_diff.saturating_sub(300);
				recent_downtime_seconds = downtime_blocks * block_time;

				// No consecutive reliable days since the node missed an update
				consecutive_reliable_days = 0;
			}

			// Convert recent downtime seconds to hours
			let recent_downtime_hours = recent_downtime_seconds / 3600;

			// Convert uptime to minutes
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
					let _ = ipfs_pallet::Pallet::<T>::clear_miner_profile(miner.node_id.clone());

					// unregister node , Hotkey nodes and LinkedNodes
					pallet_registration::Pallet::<T>::do_unregister_main_node(miner.node_id.clone());
					// Remove node metrics and block numbers
					Self::do_remove_metrics(miner.node_id.clone());
				}
			}
		}

		pub fn do_remove_metrics(node_id: Vec<u8>) {
			NodeMetrics::<T>::remove(&node_id.clone());
			BlockNumbers::<T>::remove(&node_id.clone());
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
			Self::do_remove_metrics(node_id);
		}

		fn system_info_to_json_string(system_info: &SystemInfo) -> String {
			format!(
				r#"{{
					"memory_mb": {},
					"free_memory_mb": {},
					"storage_total_mb": {},
					"storage_free_mb": {},
					"network_bandwidth_mb_s": {},
					"primary_network_interface": {},
					"disks": [{}],
					"ipfs_repo_size": {},
					"ipfs_storage_max": {},
					"cpu_model": [{}],
					"cpu_cores": {},
					"is_sev_enabled": {},
					"zfs_info": [{}],
					"ipfs_zfs_pool_size": {},
					"ipfs_zfs_pool_alloc": {},
					"ipfs_zfs_pool_free": {},
					"raid_info": [{}],
					"vm_count": {},
					"gpu_name": {},
					"gpu_memory_mb": {},
					"hypervisor_disk_type": {},
					"vm_pool_disk_type": {},
					"disk_info": [{}]
				}}"#,
				system_info.memory_mb,
				system_info.free_memory_mb,
				system_info.storage_total_mb,
				system_info.storage_free_mb,
				system_info.network_bandwidth_mb_s,
				// Primary network interface
				system_info.primary_network_interface.as_ref().map_or("null".to_string(), |nif| 
					format!("{{\"name\": [{}], \"mac_address\": {}, \"uplink_mb\": {}, \"downlink_mb\": {}, \"network_details\": {}}}",
						nif.name.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", "),
						nif.mac_address.as_ref().map_or("null".to_string(), |mac| 
							format!("[{}]", mac.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", "))
						),
						nif.uplink_mb,
						nif.downlink_mb,
						nif.network_details.as_ref().map_or("null".to_string(), |nd| 
							format!("{{\"network_type\": \"{}\", \"city\": {}, \"region\": {}, \"country\": {}, \"loc\": {}}}",
								match nd.network_type {
									NetworkType::Private => "Private",
									NetworkType::Public => "Public"
								},
								nd.city.as_ref().map_or("null".to_string(), |c| 
									format!("[{}]", c.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", "))
								),
								nd.region.as_ref().map_or("null".to_string(), |r| 
									format!("[{}]", r.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", "))
								),
								nd.country.as_ref().map_or("null".to_string(), |c| 
									format!("[{}]", c.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", "))
								),
								nd.loc.as_ref().map_or("null".to_string(), |l| 
									format!("[{}]", l.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", "))
								)
							)
						)
					)
				),
				// Disks
				system_info.disks.iter()
					.map(|disk| format!("{{\"name\": [{}], \"disk_type\": [{}], \"total_space_mb\": {}, \"free_space_mb\": {}}}",
						disk.name.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", "),
						disk.disk_type.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", "),
						disk.total_space_mb,
						disk.free_space_mb
					))
					.collect::<Vec<_>>().join(", "),
				system_info.ipfs_repo_size,
				system_info.ipfs_storage_max,
				system_info.cpu_model.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", "), 
				system_info.cpu_cores,
				system_info.is_sev_enabled,
				system_info.zfs_info.iter()
					.map(|info| format!("[{}]", info.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", ")))
					.collect::<Vec<_>>().join(", "),
				system_info.ipfs_zfs_pool_size,
				system_info.ipfs_zfs_pool_alloc,
				system_info.ipfs_zfs_pool_free,
				system_info.raid_info.iter()
					.map(|info| format!("[{}]", info.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", ")))
					.collect::<Vec<_>>().join(", "),
				system_info.vm_count,
				system_info.gpu_name.as_ref().map_or("null".to_string(), |gpu| 
					format!("[{}]", gpu.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", "))
				),
				system_info.gpu_memory_mb.unwrap_or(0),
				system_info.hypervisor_disk_type.as_ref().map_or("null".to_string(), |hdt| 
					format!("[{}]", hdt.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", "))
				),
				system_info.vm_pool_disk_type.as_ref().map_or("null".to_string(), |vpdt| 
					format!("[{}]", vpdt.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", "))
				),
				// Disk info
				system_info.disk_info.iter()
					.map(|disk| format!("{{\"name\": [{}], \"serial\": [{}], \"model\": [{}], \"size\": [{}], \"is_rotational\": {}, \"disk_type\": [{}]}}",
						disk.name.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", "),
						disk.serial.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", "),
						disk.model.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", "),
						disk.size.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", "),
						disk.is_rotational,
						disk.disk_type.iter().map(|&b| b.to_string()).collect::<Vec<_>>().join(", ")
					))
					.collect::<Vec<_>>().join(", ")
			)
		}
		
		pub fn get_hex_for_submit_metrics(
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
			block_number: BlockNumberFor<T>,
		) -> Result<String, http::Error> {
			let local_default_spec_version = <T as pallet::Config>::LocalDefaultSpecVersion::get();
			let local_default_genesis_hash = <T as pallet::Config>::LocalDefaultGenesisHash::get();
			let local_rpc_url = <T as pallet::Config>::LocalRpcUrl::get();
		
			// Convert node_id to a comma-separated string of numbers
			let node_id_string = node_id
				.iter()
				.map(|&b| b.to_string())
				.collect::<Vec<_>>()
				.join(", ");
			let block_number_updated: u32 = block_number.try_into().unwrap_or(0);
		
			let rpc_payload = format!(
				r#"{{
					"jsonrpc": "2.0",
					"method": "submit_metrics",
					"params": [{{
						"node_id": [{}],
						"storage_proof_time_ms": {},
						"latency_ms": {},
						"peer_count": {},
						"failed_challenges_count": {},
						"successful_challenges": {},
						"total_challenges": {},
						"uptime_minutes": {},
						"total_minutes": {},
						"consecutive_reliable_days": {},
						"recent_downtime_hours": {},
						"block_number": {},
						"default_spec_version": {},
						"default_genesis_hash": "{}",
						"local_rpc_url": "{}"
					}}],
					"id": 1
				}}"#,
				node_id_string,
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
				block_number_updated,
				local_default_spec_version,
				local_default_genesis_hash,
				local_rpc_url
			);
		
			// Convert the JSON value to a string
			let rpc_payload_string = rpc_payload.to_string();
		
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(2_000));

			let body = vec![rpc_payload_string];
			let request = sp_runtime::offchain::http::Request::post(local_rpc_url, body);

			let pending = request
				.add_header("Content-Type", "application/json")
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::error!("❌ Error making Request: {:?}", err);
					sp_runtime::offchain::http::Error::IoError
				})?;

			let response = pending.try_wait(deadline).map_err(|err| {
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
			let hex_result = json_response
				.get("result")
				.and_then(Value::as_str) // Get the result as a string
				.ok_or_else(|| {
					log::error!("❌ 'result' field not found in response");
					http::Error::Unknown
				})?;

			// Return the hex string
			Ok(hex_result.to_string())		
		}
		
		pub fn get_hex_for_submit_hardware(
			node_id: Vec<u8>,
			system_info: SystemInfo
		) -> Result<String, http::Error> {
			let local_default_spec_version = <T as pallet::Config>::LocalDefaultSpecVersion::get();
			let local_default_genesis_hash = <T as pallet::Config>::LocalDefaultGenesisHash::get();
			let local_rpc_url = <T as pallet::Config>::LocalRpcUrl::get();
		
			// Convert node_id to a comma-separated string of numbers
			let node_id_string = node_id
				.iter()
				.map(|&b| b.to_string())
				.collect::<Vec<_>>()
				.join(", ");
		
			let rpc_payload = format!(
				r#"{{
					"jsonrpc": "2.0",
					"method": "submit_hardware",
					"params": [{{
						"node_id": [{}],
						"system_info": {},
						"default_spec_version": {},
						"default_genesis_hash": "{}",
						"local_rpc_url": "{}"
					}}],
					"id": 1
				}}"#,
				node_id_string,
				Self::system_info_to_json_string(&system_info),
				local_default_spec_version,
				local_default_genesis_hash,
				local_rpc_url
			);
		
			// Convert the JSON value to a string
			let rpc_payload_string = rpc_payload.to_string();
		
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(2_000));

			let body = vec![rpc_payload_string];
			let request = sp_runtime::offchain::http::Request::post(local_rpc_url, body);

			let pending = request
				.add_header("Content-Type", "application/json")
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::error!("❌ Error making Request: {:?}", err);
					sp_runtime::offchain::http::Error::IoError
				})?;

			let response = pending.try_wait(deadline).map_err(|err| {
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
			let hex_result = json_response
				.get("result")
				.and_then(Value::as_str) // Get the result as a string
				.ok_or_else(|| {
					log::error!("❌ 'result' field not found in response");
					http::Error::Unknown
				})?;

			// Return the hex string
			Ok(hex_result.to_string())		
		}

		fn apply_consensus() {
			let all_miners: Vec<Vec<u8>> = TemporaryPinReports::<T>::iter_keys()
				.map(|(miner_id, _)| miner_id)
				.collect::<Vec<_>>();
			let threshold = T::ConsensusThreshold::get();
		
			for miner_id in all_miners {
				let reports: Vec<MinerPinMetrics> = TemporaryPinReports::<T>::iter_prefix(&miner_id)
					.map(|(_, report)| report)
					.collect();
		
				if reports.is_empty() {
					continue;
				}
				let total_reports = reports.len() as u32;
				if total_reports >= threshold {
					let similarity_percentage = T::ConsensusSimilarityThreshold::get(); // e.g., 85 for 85%
		
					// Calculate sums for averaging
					let mut total_pin_checks_sum = 0u64;
					let mut successful_pin_checks_sum = 0u64;
					for report in &reports {
						total_pin_checks_sum += report.total_pin_checks as u64;
						successful_pin_checks_sum += report.successful_pin_checks as u64;
					}
		
					// Calculate averages (as u128 for precise arithmetic)
					let avg_total_pin_checks = (total_pin_checks_sum as u128 / total_reports as u128) as u32;
					let avg_successful_pin_checks = (successful_pin_checks_sum as u128 / total_reports as u128) as u32;
		
					let similarity = similarity_percentage as u128;
		
					// Define acceptable ranges with proper zero handling
					let total_pin_checks_min = if avg_total_pin_checks == 0 {
						0
					} else {
						((avg_total_pin_checks as u128 * similarity) / 100).max(1)
					};
					let total_pin_checks_max = if avg_total_pin_checks == 0 {
						0
					} else {
						((avg_total_pin_checks as u128 * 115) / 100).max(1)
					};
		
					let successful_pin_checks_min = if avg_successful_pin_checks == 0 {
						0
					} else {
						((avg_successful_pin_checks as u128 * similarity) / 100).max(1)
					};
					let successful_pin_checks_max = if avg_successful_pin_checks == 0 {
						0
					} else {
						((avg_successful_pin_checks as u128 * 115) / 100).max(1)
					};
		
					// Count validators with reports within acceptable ranges
					let agreeing_validators = reports.iter().filter(|report| {
						let total_pin = report.total_pin_checks as u128;
						let successful_pin = report.successful_pin_checks as u128;
		
						total_pin >= total_pin_checks_min &&
						total_pin <= total_pin_checks_max &&
						successful_pin >= successful_pin_checks_min &&
						successful_pin <= successful_pin_checks_max
					}).count() as u32;
		
					// Debug logging
					log::info!(
						"Miner: {:?}, Reports: {}, Avg Total: {}, Avg Success: {}, Total Range: [{}, {}], Success Range: [{}, {}], Agreeing: {}, Threshold: {}, Similarity: {}",
						miner_id,
						total_reports,
						avg_total_pin_checks,
						avg_successful_pin_checks,
						total_pin_checks_min,
						total_pin_checks_max,
						successful_pin_checks_min,
						successful_pin_checks_max,
						agreeing_validators,
						threshold,
						similarity_percentage
					);
		
					if agreeing_validators >= threshold {
						let mut metrics = NodeMetrics::<T>::get(&miner_id).unwrap_or_default();
						metrics.total_pin_checks += avg_total_pin_checks;
						metrics.successful_pin_checks += avg_successful_pin_checks;
						NodeMetrics::<T>::insert(&miner_id, metrics);
		
						// Update total pin checks per epoch
						let total_pin = TotalPinChecksPerEpoch::<T>::get(&miner_id);
						TotalPinChecksPerEpoch::<T>::insert(&miner_id, total_pin + avg_total_pin_checks);
						
						// Update total ping checks per epoch
						let total_ping = TotalPingChecksPerEpoch::<T>::get(&miner_id);
						TotalPingChecksPerEpoch::<T>::insert(&miner_id, total_ping + 1);

						// Update successful pin checks per epoch
						let successful_pin = SuccessfulPinChecksPerEpoch::<T>::get(&miner_id);
						SuccessfulPinChecksPerEpoch::<T>::insert(&miner_id, successful_pin + avg_successful_pin_checks);

						// if there is any successful pin check means miner is online 
						let successful_ping = SuccessfulPingChecksPerEpoch::<T>::get(&miner_id);
						if avg_successful_pin_checks > 0 {
							SuccessfulPingChecksPerEpoch::<T>::insert(&miner_id, successful_ping + 1);
						}else{
							SuccessfulPingChecksPerEpoch::<T>::insert(&miner_id, successful_ping);
						}

						Self::deposit_event(Event::ConsensusReached {
							miner_id: miner_id.clone(),
							total_pin_checks: avg_total_pin_checks,
							successful_pin_checks: avg_successful_pin_checks,
						});
					} else {
						Self::deposit_event(Event::ConsensusFailed { miner_id: miner_id.clone() });
					}
		
					// Clear temporary reports for this miner
					TemporaryPinReports::<T>::remove_prefix(&miner_id, None);
				}
			}
		}

		// Update reputation points (called externally by pallet logic)
		fn update_reputation_points(
			metrics: &NodeMetricsData,
			coldkey: &T::AccountId,		
		) -> u32 {
			// const MIN_PIN_CHECKS: u32 = 5; // Minimum pin checks for valid scoring
			// const SLASH_THRESHOLD: u32 = 3; // Number of failed storage proofs before slashing
			// let mut reputation_points = ipfs_pallet::Pallet::<T>::reputation_points(coldkey);
			let mut reputation_points = 0u32;
	
			// let linked_nodes = pallet_registration::Pallet::<T>::linked_nodes(metrics.miner_id.clone());
			// // calculate total pin checks per epoch
			// let (mut total_pin_checks, mut successful_pin_checks) = linked_nodes.clone().into_iter().fold(
			// 	(0u32, 0u32),
			// 	|(total, successful), node_id| {
			// 		let total_epoch = Self::total_pin_checks_per_epoch(&node_id);
			// 		let success_epoch = Self::successful_pin_checks_per_epoch(&node_id);
			
			// 		(
			// 			total.saturating_add(total_epoch),
			// 			successful.saturating_add(success_epoch),
			// 		)
			// 	},
			// );

			// total_pin_checks = total_pin_checks + Self::total_pin_checks_per_epoch(&metrics.miner_id);
			// successful_pin_checks = successful_pin_checks + Self::successful_pin_checks_per_epoch(&metrics.miner_id);

			// // Increase points for successful storage proofs
			// if total_pin_checks >= MIN_PIN_CHECKS {
			// 	let success_rate = (successful_pin_checks * 100) / total_pin_checks;
			// 	if success_rate >= 95 {
			// 		reputation_points = reputation_points.saturating_add(50); // +50 for high success
			// 	} else if success_rate >= 80 {
			// 		reputation_points = reputation_points.saturating_add(20); // +20 for good success
			// 	}
			// }
		
			// // Slash points for failed storage proofs
			// let failed_count = total_pin_checks - successful_pin_checks;
			// if failed_count >= SLASH_THRESHOLD {
			// 	reputation_points = reputation_points.saturating_mul(9).saturating_div(10); // 10% slash
			// }
		
			// // Slash points for significant downtime
			// if metrics.recent_downtime_hours > 24 {
			// 	reputation_points = reputation_points.saturating_mul(8).saturating_div(10); // 20% slash
			// }
		
			// // Slash points for false capacity claims
			// if metrics.ipfs_storage_max as u128 > metrics.ipfs_zfs_pool_size
			// 	|| (metrics.ipfs_storage_max > 0
			// 		&& metrics.current_storage_bytes < metrics.ipfs_storage_max / 20)
			// {
			// 	reputation_points = reputation_points.saturating_mul(5).saturating_div(10); // 50% slash
			// }
		
			// // Cap reputation points
			// reputation_points = reputation_points.min(3000).max(100);
	
			// Store updated points
			ipfs_pallet::Pallet::<T>::set_reputation_points(coldkey, reputation_points);
			reputation_points
		}	
		
		// Function to update reputation points for all active miners
		fn update_all_active_miners_reputation() {
			// Get all active miners
			let active_miners = pallet_registration::Pallet::<T>::get_all_coldkey_active_nodes();
			
			// Iterate through each miner
			for miner in active_miners.iter() {
				// Get node metrics for this miner
				if let Some(metrics) = Self::get_node_metrics(miner.node_id.clone()) {
					// Update reputation points if metrics exist
					Self::update_reputation_points(&metrics, &miner.owner);
				}
			}
		}
				
	}
}

impl<T: Config> MetricsInfoProvider<T> for Pallet<T> {
	fn remove_metrics(node_id: Vec<u8>) {
		Self::do_remove_metrics(node_id);
	}
}
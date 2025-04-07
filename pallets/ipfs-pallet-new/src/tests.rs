#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;
pub mod weights;
pub use weights::*;
pub use types::*;
use sp_core::offchain::KeyTypeId;

mod types;

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
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;
	use sp_std::vec::Vec;
	use frame_support::sp_runtime::SaturatedConversion;
    const STORAGE_VERSION: StorageVersion = StorageVersion::new(0);
	use sp_runtime::{
		format,
		offchain::{http, Duration},
	};
	use sp_std::vec;
	use pallet_utils::Pallet as UtilsPallet;
	use pallet_registration::Pallet as RegistrationPallet;
	use pallet_rankings::Pallet as RankingsPallet;
	use pallet_execution_unit::Pallet as ExecutionUnitPallet;
	use pallet_registration::NodeType;
	use pallet_registration::NodeInfo;
	use sp_runtime::offchain::storage_lock::StorageLock;
	use sp_runtime::offchain::storage_lock::BlockAndTime;
	use frame_system::offchain::Signer;
	use crate::types::{
		FileHash, 
		StorageRequest, 
		MAX_NODE_ID_LENGTH,
		MAX_BLACKLIST_ENTRIES,
		MAX_UNPIN_REQUESTS,
		FileInput,
		FileName,
	};
	use frame_support::BoundedVec;
	use sp_std::collections::btree_map::BTreeMap;
	use codec::alloc::string::ToString;
	use scale_info::prelude::string::String;
	use frame_system::offchain::SigningTypes;
	use frame_system::offchain::AppCrypto;
	use frame_system::offchain::SendUnsignedTransaction;
	use frame_system::offchain::SendTransactionTypes;
	use scale_info::prelude::collections;
	use serde_json::{Value, to_string};
	const DUMMY_REQUEST_BODY: &[u8; 78] = b"{\"id\": 10, \"jsonrpc\": \"2.0\", \"method\": \"chain_getFinalizedHead\", \"params\": []}";

	#[pallet::pallet]
	#[pallet::storage_version(STORAGE_VERSION)]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config + SendTransactionTypes<Call<Self>> + pallet_execution_unit::Config + pallet_staking::Config + pallet_registration::Config + pallet_rankings::Config + pallet_utils::Config {
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		#[pallet::constant]
		type IPFSBaseUrl: Get<&'static str>;

		#[pallet::constant]
		type GarbageCollectorInterval: Get<u32>;
\

		/// The identifier type for an offchain worker.
		type AuthorityId: AppCrypto<Self::Public, Self::Signature>;
	}

	const LOCK_BLOCK_EXPIRATION: u32 = 1;
    const LOCK_TIMEOUT_EXPIRATION: u32 = 3000;
	pub(crate) const LOCK_SLEEP_DURATION: u32 = LOCK_TIMEOUT_EXPIRATION / 3;

	#[pallet::validate_unsigned]
	impl<T: Config> ValidateUnsigned for Pallet<T> {
		type Call = Call<T>;
		
		fn validate_unsigned(_source: TransactionSource, call: &Self::Call) -> TransactionValidity {
			match call {
				// Handler for `update_pin_requests` unsigned transaction
				Call::update_pin_requests { node_identity, miner_pin_requests, storage_request, file_size, signature: _ } => {
					let current_block = frame_system::Pallet::<T>::block_number();

					// Create a unique hash combining all relevant data
					let mut data = Vec::new();
					data.extend_from_slice(&current_block.encode());
					data.extend_from_slice(node_identity);
					for request in miner_pin_requests {
						data.extend_from_slice(&request.miner_node_id.encode());
						data.extend_from_slice(&request.cid.encode());
					}
					data.extend_from_slice(&storage_request.file_hash);
					data.extend_from_slice(&storage_request.created_at.encode());
					data.extend_from_slice(&file_size.encode());
					
					let unique_hash = sp_io::hashing::blake2_256(&data);

					// Ensure unique transaction validity
					ValidTransaction::with_tag_prefix("IpfsOffchain")
						.priority(TransactionPriority::max_value())
						.and_provides(unique_hash)
						.longevity(3)
						.propagate(true)
						.build()
				},
				// Default case for invalid calls
				_ => InvalidTransaction::Call.into(),
			}
		}
	}


	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_initialize(_block_number: BlockNumberFor<T>) -> Weight {
			// Clear all entries; limit is u32::MAX to ensure we get them all
			let result = UserStorageRequestsCount::<T>::clear(u32::MAX, None);

			// Log it (optional)
			log::info!("Cleared UserStorageRequestsCount in on_initialize. Removed {} unique entries.", result.unique);
	
			// Return weight — adjust base_weight if needed
			T::DbWeight::get().writes(result.unique.into())
		}

		fn offchain_worker(_block_number: BlockNumberFor<T>) {
			let current_block = _block_number.saturated_into::<u32>();
						
			if current_block % <T as pallet::Config>::GarbageCollectorInterval::get() == 0 {
				if let Err(err) = Self::run_ipfs_gc() {
					log::error!("Failed to run IPFS garbage collection: {:?}", err);
				} else {
					log::info!("IPFS garbage collection executed successfully.");
				}
			}

			match UtilsPallet::<T>::fetch_node_id() {
                Ok(node_id) => {
                    let node_info = RegistrationPallet::<T>::get_node_registration_info(node_id.clone());	
                    if node_info.is_some() {
						let node_info = node_info.unwrap();
                        if node_info.node_type == NodeType::Validator {
							let _ = Self::handle_request_assignment(node_id, node_info);
						}
						else if node_info.node_type == NodeType::StorageMiner {
							let _ = Self::sync_pinned_files(node_id);
						}else{
							log::error!("skipping ipfs checks");
						}
					}
                }
				Err(e) => {
					log::error!("Error fetching node identity inside bittensor pallet: {:?}", e);
				}
            }
        
		}
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		SomethingStored { something: u32, who: T::AccountId },
		StorageRequestUpdated { owner: T::AccountId, file_hash: FileHash, file_size: u128 },
		AssignmentEnabledChanged { enabled: bool },
	}

	#[pallet::error]
	pub enum Error<T> {
		NoneValue,
		StorageOverflow,
		RequestDoesNotExists,
		OwnerNotFound,
		TooManyUnpinRequests,
		InvalidInput,
		RequestAlreadyExists,
		TooManyRequests,
		ValidatorSelectionFailed,
		NoValidatorsAvailable,
		NodeNotRegistered,
		NodeNotValidator,
		InvalidCid,
		InvalidJson,
		IpfsError
	}

	// the file size where the key is encoded file hash
	#[pallet::storage]
	#[pallet::getter(fn user_total_files_size)]
	pub type UserTotalFilesSize<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, u128>;

	// Saves the owners request to store the file, with AccountId and FileHash as keys
	#[pallet::storage]
	#[pallet::getter(fn user_storage_requests)]
	pub type UserStorageRequests<T: Config> = StorageDoubleMap<
		_,
		Blake2_128Concat, 
		T::AccountId,     // First key: Account ID of the owner
		Blake2_128Concat, 
		FileHash,         // Second key: File Hash
		Option<StorageRequest<T::AccountId,BlockNumberFor<T>>>,
		ValueQuery
	>;

	#[pallet::storage]
	#[pallet::getter(fn user_storage_requests_count)]
	pub type UserStorageRequestsCount<T: Config> = StorageMap<
		_, 
		Blake2_128Concat, 
		T::AccountId, 
		u32, 
		ValueQuery
	>;

	//  Blacklist storage item containing the blacklisted Files Hashes
	#[pallet::storage]
	pub type Blacklist<T: Config> = StorageValue<_, BoundedVec<FileHash, ConstU32<MAX_BLACKLIST_ENTRIES>>, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn unpin_requests)]
	pub(super) type UnpinRequests<T: Config> = StorageValue<_, BoundedVec<FileHash, ConstU32<MAX_UNPIN_REQUESTS>>, ValueQuery>;

	// Saves all the node ids who have pinned for that file hash
	#[pallet::storage]
	#[pallet::getter(fn miner_profile)]
	pub type MinerProfile<T: Config> = StorageMap<
		_, 
		Blake2_128Concat, 
		BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>, // node id     
		BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>, // cid
		ValueQuery
	>;


	#[pallet::storage]
    #[pallet::getter(fn assignment_enabled)]
    pub type AssignmentEnabled<T: Config> = StorageValue<_, bool, ValueQuery>;

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		// miners request to store a file given file hash 
		#[pallet::call_index(0)]
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn update_pin_requests(
			origin: OriginFor<T>,
			node_identity: Vec<u8>,
			miner_pin_requests: Vec<MinerProfileItem>, 
			storage_request: StorageRequest<T::AccountId, BlockNumberFor<T>>,
			file_size: u128,
			signature: <T as SigningTypes>::Signature
		) -> DispatchResultWithPostInfo {
            ensure_none(origin)?;
			let _signature = signature;

			// check if the node is already registered or not here
			let node_info = RegistrationPallet::<T>::get_node_registration_info(node_identity.clone());
			// Ensure the node is registered and its type is miner
			ensure!(
				node_info.is_some(),
				Error::<T>::NodeNotRegistered
			);
			
			// node should be a validator
			ensure!(
				matches!(node_info.unwrap().node_type, NodeType::Validator),
				Error::<T>::NodeNotValidator
			);

			// Update MinerProfile storage for each miner pin request
			for miner_profile in miner_pin_requests.iter() {
				// Update MinerProfile storage with node ID and CID
				<MinerProfile<T>>::insert(
					miner_profile.miner_node_id.clone(), 
					BoundedVec::<u8, ConstU32<MAX_NODE_ID_LENGTH>>::try_from(
						miner_profile.cid.clone().into_inner().to_vec()
					).unwrap_or_else(|v: Vec<u8>| 
						BoundedVec::truncate_from(v)
					)
				);
			}

			// Update UserStorageRequests to mark the request as fulfilled
			<UserStorageRequests<T>>::mutate(
				storage_request.owner.clone(), 
				storage_request.file_hash.clone(), 
				|request| {
					if let Some(ref mut req) = request {
						let mut updated_req = req.clone();
						updated_req.is_assigned = true;
						*request = Some(updated_req);
					}
				}
			);

			// Update or insert UserTotalFilesSize
			<UserTotalFilesSize<T>>::mutate(storage_request.owner.clone(), |total_size| {
				*total_size = Some(total_size.unwrap_or(0) + file_size);
			});

			// Deposit an event to log the pin request update
			Self::deposit_event(Event::StorageRequestUpdated {
				owner: storage_request.owner,
				file_hash: storage_request.file_hash,
				file_size: file_size,
			});

			Ok(().into())
		}

		/// Sudo function to enable or disable file assignments
		#[pallet::call_index(1)]
		#[pallet::weight(10_000)]
		pub fn set_assignment_enabled(
			origin: OriginFor<T>,
			enabled: bool
		) -> DispatchResult {
			// Ensure the origin is the root (sudo) origin
			ensure_root(origin)?;

			// Set the assignment enabled flag
			AssignmentEnabled::<T>::put(enabled);

			// Emit an event to log the change
			Self::deposit_event(Event::AssignmentEnabledChanged { enabled });

			Ok(())
		}

	}

	impl<T: Config> Pallet<T>{
		/// Helper function to handle the storage request logic
		pub fn process_storage_request(
			owner: T::AccountId,
			file_inputs: Vec<FileInput>,
			miner_ids: Option<Vec<Vec<u8>>>,
		) -> Result<(), Error<T>> {
			let current_block = frame_system::Pallet::<T>::block_number();

			// Validate input
			ensure!(!file_inputs.is_empty(), Error::<T>::InvalidInput);


			// Update user's storage requests count
			UserStorageRequestsCount::<T>::insert(&owner, user_requests_count + file_inputs.len() as u32);

			// Select a validator from current BABE validators
			let selected_validator = Self::select_validator()?;

			// Process each file input
			for file_input in file_inputs {
				// Convert file hash to a consistent format for storage lookup
				let file_hash_key = hex::encode(file_input.file_hash.clone());
				let update_hash_vec: Vec<u8> = file_hash_key.into();
				let update_hash: FileHash = BoundedVec::try_from(update_hash_vec)
					.map_err(|_| Error::<T>::StorageOverflow)?;

				// Convert file_name to BoundedVec
				let bounded_file_name: FileName = BoundedVec::try_from(file_input.file_name.clone())
					.map_err(|_| Error::<T>::StorageOverflow)?;

				// Check if the request already exists for this user and file hash
				ensure!(
					!UserStorageRequests::<T>::contains_key(&owner, &update_hash),
					Error::<T>::RequestAlreadyExists
				);

				// Create the storage request
				let request_info = StorageRequest {
					total_replicas: 1u32,  
					owner: owner.clone(),
					file_hash: update_hash.clone(),
					file_name: bounded_file_name,
					miner_ids: miner_ids.clone().map(|ids| 
						BoundedVec::try_from(
							ids.into_iter()
								.map(|id| BoundedVec::try_from(id).unwrap_or_default())
								.collect::<Vec<_>>()
						).unwrap_or_default()
					),
					last_charged_at: current_block,
					created_at: current_block,
					selected_validator: selected_validator.clone(),
					is_assigned: false,
				};

				// Store the request in the double map
				UserStorageRequests::<T>::insert(&owner, &update_hash, Some(request_info.clone()));
			}

			Ok(())
		}

		fn select_validator() -> Result<T::AccountId, Error<T>> {
			// Correct way to get active validators from staking pallet
			let validators = pallet_staking::Validators::<T>::iter()
				.map(|(validator, _prefs)| validator)
				.collect::<Vec<_>>();
			
			ensure!(!validators.is_empty(), Error::<T>::NoValidatorsAvailable);
		
			// Use current block number for pseudo-random selection
			let block_number = frame_system::Pallet::<T>::block_number();
			let validator_index = block_number.saturated_into::<usize>() % validators.len();
			
			Ok(validators[validator_index].clone())
		}

		// get all files stored my miners 
		pub fn get_storage_request_by_hash(owner: T::AccountId, encoded_hash: Vec<u8>) -> Option<StorageRequest<T::AccountId,BlockNumberFor<T>>>{
			let update_hash: FileHash = BoundedVec::try_from(encoded_hash).ok()?;
			UserStorageRequests::<T>::get(owner, update_hash)
		}

		/// Helper function to handle the unpin request logic
		pub fn process_unpin_request(
			who: T::AccountId,
			file_hash: Vec<u8>,
		) -> Result<FileHash, Error<T>> {
			let current_block = frame_system::Pallet::<T>::block_number();


			// Update user's unpin requests count
			UserStorageRequestsCount::<T>::insert(&who, user_unpin_requests_count + 1);

			// Convert file hash to a hex-encoded string
			let file_hash_encoded = hex::encode(&file_hash);
			let update_hash_vec: Vec<u8> = file_hash_encoded.clone().into();
			let update_hash: FileHash = BoundedVec::try_from(update_hash_vec)
				.map_err(|_| Error::<T>::StorageOverflow)?;

			// Check if the request exists in UserStorageRequests
			let storage_request = UserStorageRequests::<T>::get(&who, &update_hash)
				.ok_or(Error::<T>::RequestDoesNotExists)?;

			// Validate ownership
			ensure!(storage_request.owner == who, Error::<T>::OwnerNotFound);

			// Add to UnpinRequests if not already present
			UnpinRequests::<T>::try_mutate(|requests| -> Result<(), Error<T>> {
				if !requests.contains(&update_hash) {
					requests.try_push(update_hash.clone())
						.map_err(|_| Error::<T>::TooManyUnpinRequests)?;
				}
				Ok(())
			})?;

			// Remove the storage request
			UserStorageRequests::<T>::remove(&who, &update_hash);

			Ok(update_hash)
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

			
			// Check if the response code is 200 (OK)
			if response.code == 200 {
				let response_body = response.body().collect::<Vec<u8>>();
				let body_str = core::str::from_utf8(&response_body).unwrap_or("");

				// Split the response body into lines to count success messages
				let success_count = body_str.lines().filter(|line| line.contains("\"Success\":true")).count();

				// Check if we have at least 5 successful pings
				if success_count >= 5 {
					return Ok(true);
				} else {
					return Ok(false);
				}
			} else {
				return Ok(false);
			}
		}

		// Function to trigger IPFS garbage collection via HTTP API
		pub fn run_ipfs_gc() -> Result<(), http::Error> {
			// Base URL of the IPFS node
			let base_url = T::IPFSBaseUrl::get();
			let url = format!("{}/api/v0/repo/gc", base_url);
		
			// Request Timeout
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(5_000));
		
			// Dummy body since this is a POST request (IPFS API expects POST for GC)
			let body = vec![DUMMY_REQUEST_BODY];
			let request = sp_runtime::offchain::http::Request::post(&url, body);
		
			let pending = request
				.add_header("Content-Type", "application/json")
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::warn!("Error making request to run GC: {:?}", err);
					sp_runtime::offchain::http::Error::IoError
				})?;
		
			// Getting response
			let response = pending
				.try_wait(deadline)
				.map_err(|err| {
					log::warn!("Error waiting for GC response: {:?}", err);
					sp_runtime::offchain::http::Error::DeadlineReached
				})??;
		
			// Check the response status code
			if response.code != 200 {
				log::warn!("Unexpected status code from GC request: {}", response.code);
				return Err(http::Error::Unknown);
			}

			Ok(())
		}

		/// Retrieve all unassigned storage requests for a specific validator
		pub fn get_unassigned_storage_requests_for_validator(
			validator: T::AccountId,
		) -> Vec<StorageRequest<T::AccountId, BlockNumberFor<T>>> {
			UserStorageRequests::<T>::iter()
				.filter_map(|(_, _, request)| 
					request.filter(|r| !r.is_assigned && r.selected_validator == validator)
				)
				.collect()
		}
		
		// Pin a JSON string to IPFS and return its CID
		pub fn pin_file_to_ipfs(json_string: &str) -> Result<String, http::Error> {
			let url = format!("{}/api/v0/add", T::IPFSBaseUrl::get());
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(5_000));
		
			// Convert JSON string to bytes
			let json_bytes = json_string.as_bytes();
		
			// Create a simple multipart form body
			// Note: In a real-world scenario, you might want to use a proper multipart library,
			// but for simplicity, we'll construct it manually here.
			let boundary = "----WebKitFormBoundary7MA4YWxkTrZu0gW";
			let mut body = Vec::new();
			body.extend_from_slice(b"--");
			body.extend_from_slice(boundary.as_bytes());
			body.extend_from_slice(b"\r\n");
			body.extend_from_slice(b"Content-Disposition: form-data; name=\"file\"; filename=\"data.json\"\r\n");
			body.extend_from_slice(b"Content-Type: application/json\r\n\r\n");
			body.extend_from_slice(json_bytes);
			body.extend_from_slice(b"\r\n--");
			body.extend_from_slice(boundary.as_bytes());
			body.extend_from_slice(b"--\r\n");
		
			// Create and send the request
			let request = sp_runtime::offchain::http::Request::post(&url, vec![body]);
		
			let pending = request
				.add_header("Content-Type", &format!("multipart/form-data; boundary={}", boundary))
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::info!("Error making Request: {:?}", err);
					sp_runtime::offchain::http::Error::IoError
				})?;
		
			// Get the response
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
		
			// Parse the response body
			let body = response.body().collect::<Vec<u8>>();
			let body_str = sp_std::str::from_utf8(&body).map_err(|_| {
				log::error!("IPFS response not valid UTF-8");
				http::Error::Unknown
			})?;
		
			// Parse the JSON response
			let parsed_response: serde_json::Value = serde_json::from_str(body_str).map_err(|e| {
				log::error!("Failed to parse JSON response: {:?}", e);
				http::Error::Unknown
			})?;
		
			// Extract the CID from the JSON response
			if let Some(cid) = parsed_response["Hash"].as_str() {
				return Ok(cid.to_string());
			}
		
			log::error!("Failed to parse IPFS response: Hash field missing");
			Err(http::Error::Unknown)
		}

		/// A helper function to fetch the node ID and hardware info, then store it in `NodeSpecs`.
		pub fn update_ipfs_request_storage(node_id: Vec<u8>, pin_requests: Vec<MinerProfileItem>, storage_requests: StorageRequest<T::AccountId, BlockNumberFor<T>> , file_size: u128) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"IpfsPin::update_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);
		
			if let Ok(_guard) = lock.try_lock() {
				let signer = Signer::<T, <T as pallet::Config>::AuthorityId>::all_accounts();
		
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
					return;
				}
		
				let results = signer.send_unsigned_transaction(
					|account| {
						UpdateIpfsRequestPayload {
							node_id: node_id.clone(),
							miner_pin_requests: pin_requests.clone(),
							storage_requests: storage_requests.clone(),
							file_size: file_size.clone(),
							public: account.public.clone(),
							_marker: PhantomData, // Ensure compatibility with generic payload types
						}
					},
					|payload, signature| {
						Call::update_pin_requests {
							node_identity: payload.node_id,
							miner_pin_requests: payload.miner_pin_requests,
							storage_request: payload.storage_requests,
							file_size: payload.file_size,
							signature,
						}
					},
				);
		
				// Process the results of the transaction submission
				for (acc, res) in &results {
					match res {
						Ok(()) => log::info!("[{:?}] Successfully submitted IPFS request", acc.id),
						Err(e) => log::error!("[{:?}] Failed to submit IPFS request: {:?}", acc.id, e),
					}
				}
			} else {
				log::error!("❌ Could not acquire lock for updating IPFS request");
			};
		}

		fn fetch_ipfs_content(cid: &str) -> Result<Vec<u8>, http::Error> {
			let url = format!("{}/api/v0/cat?arg={}", T::IPFSBaseUrl::get(), cid);
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
	
			Ok(response.body().collect::<Vec<u8>>())
		}

		pub fn handle_request_assignment(
			node_id: Vec<u8>,
			node_info: NodeInfo<BlockNumberFor<T>, T::AccountId>,
		) -> Result<(), DispatchError> {
			if AssignmentEnabled::<T>::get() {
				let storage_requests = Self::get_unassigned_storage_requests_for_validator(node_info.owner.clone());
				let active_storage_miners = pallet_registration::Pallet::<T>::get_all_storage_miners_with_min_staked();
				let ranked_list = RankingsPallet::<T>::get_ranked_list();
	
				let mut rank_map: BTreeMap<Vec<u8>, u32> = BTreeMap::new();
				for ranking in ranked_list.iter() {
					rank_map.insert(ranking.node_id.clone(), ranking.rank);
				}
	
				for storage_request in storage_requests {
					let mut available_miners: Vec<_> = active_storage_miners
						.iter()
						.filter_map(|miner| {
							if let Some(node_metrics) = ExecutionUnitPallet::<T>::get_node_metrics(miner.node_id.clone()) {
								let ipfs_storage_max = node_metrics.ipfs_storage_max;
								let available_storage = ipfs_storage_max.saturating_sub(node_metrics.ipfs_repo_size);
								let storage_threshold = ipfs_storage_max.saturating_mul(10) / 100;
	
								if available_storage >= storage_threshold {
									if let Some(rank) = rank_map.get(&miner.node_id) {
										Some((miner.clone(), *rank, available_storage))
									} else {
										None
									}
								} else {
									None
								}
							} else {
								None
							}
						})
						.collect();
	
					available_miners.sort_by(|a, b| 
						a.1.cmp(&b.1).then(b.2.cmp(&a.2))
					);
	
					let mut selected_miners = Vec::new();
					let num_replicas = storage_request.total_replicas.min(available_miners.len() as u32);
	
					if let Some(requested_miners) = &storage_request.miner_ids {
						for requested_miner_id in requested_miners.iter() {
							if selected_miners.len() >= num_replicas as usize {
								break;
							}
							if let Some((miner, rank, available_storage)) = available_miners.iter().find(|m| m.0.node_id == requested_miner_id.clone().to_vec()) {
								selected_miners.push((miner.clone(), *rank, *available_storage));
							}
						}
					}
	
					if selected_miners.len() < num_replicas as usize {
						let remaining_needed = num_replicas as usize - selected_miners.len();
						let mut top_miners: Vec<_> = available_miners
							.into_iter()
							.filter(|m| !selected_miners.iter().any(|sm| sm.0.node_id == m.0.node_id))
							.take(remaining_needed)
							.collect();
	
						let seed = frame_system::Pallet::<T>::block_number().saturated_into::<u64>();
						for i in (1..top_miners.len()).rev() {
							let j = seed.wrapping_mul(i as u64) % (i + 1) as u64;
							top_miners.swap(i, j as usize);
						}
						selected_miners.extend(top_miners);
					}
	
					if selected_miners.len() == storage_request.total_replicas as usize {
						match Self::fetch_ipfs_file_size(storage_request.file_hash.clone().to_vec()) {
							Ok(file_size) => {
								log::info!("File size: {}", file_size);
								log::info!(
									"Selected {} miners for storage request with file hash {:?}:",
									selected_miners.len(),
									storage_request.file_hash
								);
	
								let current_block = frame_system::Pallet::<T>::block_number();
								let miner_pin_requests: Vec<MinerPinRequest<BlockNumberFor<T>>> = selected_miners
									.iter()
									.map(|(miner, _, _)| MinerPinRequest {
										miner_node_id: BoundedVec::try_from(miner.node_id.clone()).unwrap(),
										file_hash: storage_request.file_hash.clone(),
										created_at: current_block,
										file_size_in_bytes: file_size,
									})
									.collect();
	
								for pin_request in miner_pin_requests.iter() {
									let file_hash_vec = pin_request.file_hash.clone().to_vec();
									log::info!(
										"Pinning file for Miner Node ID: {:?}, File Hash: {:?}, File Size: {}",
										pin_request.miner_node_id,
										file_hash_vec,
										pin_request.file_size_in_bytes
									);
	
									// Check if CID exists in MinerProfile for this miner
									let miner_node_id = pin_request.miner_node_id.clone();
									let existing_cid = MinerProfile::<T>::get(&miner_node_id);
	
									let json_content = if !existing_cid.is_empty() {
										// Fetch existing content from IPFS
										let binding = existing_cid.to_vec();
										let cid_str = sp_std::str::from_utf8(&binding).map_err(|_| Error::<T>::InvalidCid)?;
											match Self::fetch_ipfs_content(cid_str) {
												Ok(content) => {
													let existing_data: Value = serde_json::from_slice(&content)
														.map_err(|_| Error::<T>::InvalidJson)?;
													let mut requests_array = if existing_data.is_array() {
														existing_data.as_array().unwrap().clone()
													} else {
														vec![existing_data]
													};
													// Append new pin request
													let new_request = serde_json::to_value(pin_request)
														.map_err(|_| Error::<T>::InvalidJson)?;
													requests_array.push(new_request);
													to_string(&requests_array).unwrap()
												}
												Err(e) => {
													log::error!("Failed to fetch existing CID content: {:?}", e);
													// Fallback to creating a new array
													to_string(&vec![pin_request]).unwrap()
												}
											}
									} else {
										// No existing CID, create a new array with this request
										to_string(&vec![pin_request]).unwrap()
									};
	
									// Pin the updated or new content to IPFS
									match Self::pin_file_to_ipfs(&json_content) {
										Ok(new_cid) => {
											let update_hash: BoundedVec<u8, ConstU32<MAX_FILE_HASH_LENGTH>> =
												BoundedVec::try_from(new_cid.clone().into_bytes())
													.map_err(|_| Error::<T>::StorageOverflow)?;
	
											let miner_profile_items: Vec<MinerProfileItem> = miner_pin_requests
												.iter()
												.map(|pin_request| MinerProfileItem {
													miner_node_id: pin_request.miner_node_id.clone(),
													cid: update_hash.clone(),
												})
												.collect();
	
											Self::update_ipfs_request_storage(
												node_id.clone(),
												miner_profile_items,
												storage_request.clone(),
												file_size as u128,
											);
											log::info!("Successfully pinned file with CID: {}", new_cid);
										}
										Err(e) => log::error!("Failed to pin file: {:?}", e),
									}
								}
							}
							Err(e) => {
								log::error!("Failed to fetch file size: {:?}", e);
							}
						}
					}
				}
			}
			Ok(())
		}
	

		/// Returns a vector of all unique users who have made storage requests
		pub fn get_storage_request_users() -> Vec<T::AccountId> {
			UserStorageRequests::<T>::iter_keys()
				.map(|(user, _)| user)
				.collect::<collections::BTreeSet<_>>()
				.into_iter()
				.collect()
		}

		// unpins the local ipfs node 
		pub fn unpin_file_from_ipfs(cid: &str) -> Result<(), http::Error> {

			let base_url = T::IpfsBaseUrl::get();
			let url = format!("{}/api/v0/pin/rm?arg={}", base_url, cid);
		  
			// Request Timeout
			let deadline =
				sp_io::offchain::timestamp().add(Duration::from_millis(2_000));
				
			let body = vec![DUMMY_REQUEST_BODY];
			// getting the list of all pinned Files from node
			let request = sp_runtime::offchain::http::Request::post(&url, body);
			let pending = request
				.add_header("Content-Type", "application/json")
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::warn!("Error making Request: {:?}", err);
					sp_runtime::offchain::http::Error::IoError
				})?;
		
			// getting response
			let response = pending
				.try_wait(deadline)
				.map_err(|err| {
					log::warn!("Error getting Response: {:?}", err);
					sp_runtime::offchain::http::Error::DeadlineReached
				})??;
			
			// Check the response status code
			if response.code != 200 {
				log::warn!("Unexpected status code: {}", response.code);
				return Err(http::Error::Unknown);
			}
			
			Ok(())
		}	

		// Fetch currently pinned CIDs from the local IPFS node
		fn get_pinned_cids() -> Result<Vec<String>, http::Error> {
			let url = format!("{}/api/v0/pin/ls", T::IPFSBaseUrl::get());
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(5_000));
	
			let request = sp_runtime::offchain::http::Request::post(&url, vec![DUMMY_REQUEST_BODY.to_vec()]);
			let pending = request
				.add_header("Content-Type", "application/json")
				.deadline(deadline)
				.send()
				.map_err(|_| http::Error::IoError)?;
			let response = pending.try_wait(deadline).map_err(|_| http::Error::DeadlineReached)??;
	
			if response.code != 200 {
				log::warn!("Unexpected status code: {}", response.code);
				return Err(http::Error::Unknown);
			}
	
			let body = response.body().collect::<Vec<u8>>();
			let body_str = sp_std::str::from_utf8(&body).map_err(|_| http::Error::Unknown)?;
			let parsed: Value = serde_json::from_str(body_str).map_err(|e| {
				log::error!("Failed to parse pinned CIDs: {:?}", e);
				http::Error::Unknown
			})?;
	
			let pins = parsed["Keys"].as_object().ok_or(http::Error::Unknown)?;
			let pinned_cids: Vec<String> = pins.keys().map(|k| k.to_string()).collect();
	
			Ok(pinned_cids)
		}
	
		// Sync pinned files for a miner based on MinerProfile
		pub fn sync_pinned_files(node_id: Vec<u8>) -> Result<(), DispatchError> {
			let bounded_node_id = BoundedVec::try_from(node_id.clone())
				.map_err(|_| Error::<T>::StorageOverflow)?;
	
			// Step 1: Retrieve the CID from MinerProfile
			let cid = MinerProfile::<T>::get(&bounded_node_id);
			if cid.is_empty() {
				log::info!("No CID found for node_id: {:?}", node_id);
				return Ok(());
			}

			let binding = cid.to_vec();
			let cid_str = sp_std::str::from_utf8(&binding).map_err(|_| Error::<T>::InvalidCid)?;
	
			// Step 2: Fetch content from IPFS
			let content = Self::fetch_ipfs_content(cid_str).map_err(|e| {
				log::error!("Failed to fetch content for CID {}: {:?}", cid_str, e);
				Error::<T>::IpfsError
			})?;
	
			// Step 3: Parse the JSON array and extract file hashes
			let requests: Vec<Value> = serde_json::from_slice(&content).map_err(|e| {
				log::error!("Failed to parse JSON content: {:?}", e);
				Error::<T>::InvalidJson
			})?;
	
			let file_hashes: Vec<String> = requests
				.into_iter()
				.filter_map(|request| {
					let file_hash_vec = request["file_hash"].as_array()?;
					let file_hash_bytes: Vec<u8> = file_hash_vec
						.iter()
						.filter_map(|v| v.as_u64().map(|n| n as u8))
						.collect();
					
					// Decode the file hash bytes
					let decoded_file_hash = hex::decode(file_hash_bytes).ok()?;
					
					sp_std::str::from_utf8(&decoded_file_hash)
						.map(|s| s.to_string())
						.ok()
				})
				.collect();
	
			// Step 4: Pin all file hashes
			for file_hash in &file_hashes {
				match Self::pin_file_from_ipfs(file_hash) {
					Ok(()) => log::info!("Pinned file_hash: {}", file_hash),
					Err(e) => log::error!("Failed to pin file_hash {}: {:?}", file_hash, e),
				}
			}
	
			// Step 5: Get currently pinned CIDs
			let pinned_cids = Self::get_pinned_cids().map_err(|e| {
				log::error!("Failed to fetch pinned CIDs: {:?}", e);
				Error::<T>::IpfsError
			})?;
	
			// Step 6: Unpin CIDs not in the file_hashes list (excluding the MinerProfile CID)
			for pinned_cid in pinned_cids {
				if pinned_cid != cid_str && !file_hashes.contains(&pinned_cid) {
					match Self::unpin_file_from_ipfs(&pinned_cid) {
						Ok(()) => log::info!("Unpinned CID: {}", pinned_cid),
						Err(e) => log::error!("Failed to unpin CID {}: {:?}", pinned_cid, e),
					}
				}
			}
	
			Ok(())
		}


		// pins the local ipfs node 
		pub fn pin_file_from_ipfs(cid: &str) -> Result<(), http::Error> {

			let base_url = T::IpfsBaseUrl::get();
			let url = format!("{}/api/v0/pin/add?arg={}", base_url, cid);
			

			// Request Timeout
			let deadline =
				sp_io::offchain::timestamp().add(Duration::from_millis(2_000));
				
			let body = vec![DUMMY_REQUEST_BODY];
			// getting the list of all pinned Files from node
			let request = sp_runtime::offchain::http::Request::post(&url, body);
			let pending = request
				.add_header("Content-Type", "application/json")
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::warn!("Error making Request: {:?}", err);
					sp_runtime::offchain::http::Error::IoError
				})?;
		
			// getting response
			let response = pending
				.try_wait(deadline)
				.map_err(|err| {
					log::warn!("Error getting Response: {:?}", err);
					sp_runtime::offchain::http::Error::DeadlineReached
				})??;
			
			// Check the response status code
			if response.code != 200 {
				log::warn!("Unexpected status code: {}", response.code);
				return Err(http::Error::Unknown);
			}
			
			Ok(())
		}
	}
}
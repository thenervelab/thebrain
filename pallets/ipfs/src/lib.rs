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
	use sp_runtime::Saturating;

	const DUMMY_REQUEST_BODY: &[u8; 78] = b"{\"id\": 10, \"jsonrpc\": \"2.0\", \"method\": \"chain_getFinalizedHead\", \"params\": []}";

	#[pallet::pallet]
	#[pallet::storage_version(STORAGE_VERSION)]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config + SendTransactionTypes<Call<Self>> +  pallet_staking::Config + pallet_registration::Config + pallet_rankings::Config + pallet_utils::Config {
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		#[pallet::constant]
		type IPFSBaseUrl: Get<&'static str>;

		#[pallet::constant]
		type GarbageCollectorInterval: Get<u32>;

		#[pallet::constant]
		type PinPinningInterval: Get<u32>;

		/// The identifier type for an offchain worker.
		type AuthorityId: AppCrypto<Self::Public, Self::Signature>;

		#[pallet::constant]
		type MaxOffchainRequestsPerPeriod: Get<u32>;

	    #[pallet::constant]
	    type RequestsClearInterval: Get<u32>;
	}

	#[pallet::storage]
	#[pallet::getter(fn requests_count)]
	pub type RequestsCount<T: Config> = StorageMap<
		_, 
		Blake2_128Concat, 
		BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>, 
		u32, 
		ValueQuery
	>;

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
				// Handler for `process_miners_to_remove` unsigned transaction
				Call::process_miners_to_remove { owner, file_hash_to_remove, miner_node_id, new_cid_bounded } => {
					let current_block = frame_system::Pallet::<T>::block_number();

					// Create a unique hash combining all relevant data
					let mut data = Vec::new();
					data.extend_from_slice(&current_block.encode());
					data.extend_from_slice(&owner.encode());
					data.extend_from_slice(&file_hash_to_remove.encode());
					data.extend_from_slice(&miner_node_id.encode());
					data.extend_from_slice(&new_cid_bounded.encode());
					
					let unique_hash = sp_io::hashing::blake2_256(&data);

					// Ensure unique transaction validity
					ValidTransaction::with_tag_prefix("IpfsOffchain")
						.priority(TransactionPriority::max_value())
						.and_provides(unique_hash)
						.longevity(3)
						.propagate(true)
						.build()
				},
				Call::update_user_profile { owner, node_identity, cid, .. } => {
					let current_block = frame_system::Pallet::<T>::block_number();

					// Create a unique hash combining all relevant data
					let mut data = Vec::new();
					data.extend_from_slice(&current_block.encode());
					data.extend_from_slice(&owner.encode());
					data.extend_from_slice(&cid.encode());
					
					let unique_hash = sp_io::hashing::blake2_256(&data);

					// Ensure unique transaction validity
					ValidTransaction::with_tag_prefix("IpfsOffchain")
						.priority(TransactionPriority::max_value())
						.and_provides(unique_hash)
						.longevity(3)
						.propagate(true)
						.build()
				},
				// Handler for `set_miner_state_locked` unsigned transaction
				Call::set_miner_state_locked { miner_node_id, .. } => {
					let current_block = frame_system::Pallet::<T>::block_number();

					// Create a unique hash combining all relevant data
					let mut data = Vec::new();
					data.extend_from_slice(&current_block.encode());
					data.extend_from_slice(&miner_node_id.encode());
					
					let unique_hash = sp_io::hashing::blake2_256(&data);

					// Ensure unique transaction validity
					ValidTransaction::with_tag_prefix("IpfsOffchain")
						.priority(TransactionPriority::max_value())
						.and_provides(unique_hash)
						.longevity(3)
						.propagate(true)
						.build()
				},
				// Handler for `mark_storage_request_assigned` unsigned transaction
				Call::mark_storage_request_assigned { owner, file_hash } => {
					let current_block = frame_system::Pallet::<T>::block_number();

					// Create a unique hash combining all relevant data
					let mut data = Vec::new();
					data.extend_from_slice(&current_block.encode());
					data.extend_from_slice(&owner.encode());
					data.extend_from_slice(&file_hash.encode());
					
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

		fn offchain_worker(_block_number: BlockNumberFor<T>) {
			let current_block = _block_number.saturated_into::<u32>();
						
			if current_block % <T as pallet::Config>::GarbageCollectorInterval::get() == 0 {
				if let Err(err) = Self::run_ipfs_gc() {
					log::error!("Failed to run IPFS garbage collection: {:?}", err);
				} else {
					log::info!("IPFS garbage collection executed successfully.");
				}
			}

			if current_block % <T as pallet::Config>::PinPinningInterval::get() == 0 {
				if Self::pinning_enabled() {
					match UtilsPallet::<T>::fetch_node_id() {
						Ok(node_id) => {
							let node_info = RegistrationPallet::<T>::get_node_registration_info(node_id.clone());	
							if node_info.is_some() {
								let node_info = node_info.unwrap();
								if node_info.node_type == NodeType::StorageMiner {
									let _ = Self::sync_pinned_files(node_id);
								}
							}
						}
						Err(e) => {
							log::error!("Error fetching node identity inside bittensor pallet: {:?}", e);
						}
					}
				}
			}        
		}

		fn on_initialize(current_block: BlockNumberFor<T>) -> Weight {

			// Clear entries every 10 blocks
			if current_block % T::RequestsClearInterval::get().into() == 0u32.into() {
				// Clear all entries; limit is u32::MAX to ensure we get them all
				let _result = RequestsCount::<T>::clear(u32::MAX, None);
			}
						
			// Remove storage requests older than 500 blocks
			Self::cleanup_old_storage_requests(current_block, BlockNumberFor::<T>::from(500u32));

			Weight::zero()
		}
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		SomethingStored { something: u32, who: T::AccountId },
		StorageRequestUpdated { owner: T::AccountId, file_hash: FileHash, file_size: u128 },
		PinningEnabledChanged { enabled: bool },
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
		IpfsError,
		MaxUnpinRequestsExceeded,
	}

	// the file size where the key is encoded file hash
	#[pallet::storage]
	#[pallet::getter(fn user_total_files_size)]
	pub type UserTotalFilesSize<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, u128>;

	// the file size where the key is encoded file hash
	#[pallet::storage]
	#[pallet::getter(fn miner_files_size)]
	pub type MinerTotalFilesSize<T: Config> = StorageMap<_, Blake2_128Concat, BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>, u128>;


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

	// Saves all the node ids who have pinned for that file hash
	#[pallet::storage]
	#[pallet::getter(fn user_profile)]
	pub type UserProfile<T: Config> = StorageMap<
		_, 
		Blake2_128Concat, 
		T::AccountId, // node id     
		FileHash, // cid
		ValueQuery
	>;	

	#[pallet::storage]
    #[pallet::getter(fn pinning_enabled)]
    pub type PinningEnabled<T: Config> = StorageValue<_, bool, ValueQuery>;

	#[pallet::storage]
    #[pallet::getter(fn assignment_enabled)]
    pub type AssignmentEnabled<T: Config> = StorageValue<_, bool, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn miner_state)]
	pub type MinerStates<T: Config> = StorageMap<
		_, 
		Blake2_128Concat, 
		BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>, // Miner ID
		MinerState,  // Miner's current state
		ValueQuery   // Default to Free if not set
	>;

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

			// Rate limit: maximum storage requests per block per user
			let max_requests_per_block = T::MaxOffchainRequestsPerPeriod::get();
			let user_requests_count = RequestsCount::<T>::get(&BoundedVec::truncate_from(node_identity.clone()));
			ensure!(user_requests_count + 1 <= max_requests_per_block, Error::<T>::TooManyRequests);

			// Update user's storage requests count
			RequestsCount::<T>::insert(&BoundedVec::truncate_from(node_identity.clone()), user_requests_count + 1);

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

			// Update or insert UserTotalFilesSize
			<UserTotalFilesSize<T>>::mutate(storage_request.owner.clone(), |total_size| {
				*total_size = Some(total_size.unwrap_or(0) + file_size);
			});

			// Set all miners free after processing the pin request
			for miner_profile in miner_pin_requests.iter() {
				// Set the miner's state to Free
				MinerStates::<T>::insert(&miner_profile.miner_node_id.clone(), MinerState::Free);
			}
			
			// Update MinerTotalFilesSize for each miner
			for miner_profile in miner_pin_requests.iter() {
				<MinerTotalFilesSize<T>>::mutate(&miner_profile.miner_node_id, |total_size| {
					*total_size = Some(total_size.unwrap_or(0) + file_size);
				});
			}

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
		pub fn set_pinning_enabled(
			origin: OriginFor<T>,
			enabled: bool
		) -> DispatchResult {
			// Ensure the origin is the root (sudo) origin
			ensure_root(origin)?;

			// Set the assignment enabled flag
			PinningEnabled::<T>::put(enabled);

			// Emit an event to log the change
			Self::deposit_event(Event::PinningEnabledChanged { enabled });

			Ok(())
		}

		/// Sudo function to enable or disable file assignments
		#[pallet::call_index(2)]
		#[pallet::weight(10_000)]
		pub fn set_assignment_enabled(
			origin: OriginFor<T>,
			enabled: bool
		) -> DispatchResult {
			// Ensure the origin is the root (sudo) origin
			ensure_root(origin)?;

			// Set the assignment enabled flag
			AssignmentEnabled::<T>::put(enabled);

			Ok(())
		}

		/// Unsigned transaction to process miners to be removed
		#[pallet::call_index(3)]
		#[pallet::weight((10_000, DispatchClass::Operational, Pays::No))]
		pub fn process_miners_to_remove(
			origin: OriginFor<T>,
			owner: T::AccountId,
			file_hash_to_remove: FileHash,
			miner_node_id: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>,
			new_cid_bounded: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>
		) -> DispatchResultWithPostInfo {
			// Ensure this is an unsigned transaction
			ensure_none(origin)?;

			// Rate limit: maximum storage requests per block per user
			let max_requests_per_block = T::MaxOffchainRequestsPerPeriod::get();
			let user_requests_count = RequestsCount::<T>::get(&miner_node_id);
			ensure!(user_requests_count + 1 <= max_requests_per_block, Error::<T>::TooManyRequests);

			// Update user's storage requests count
			RequestsCount::<T>::insert(&miner_node_id, user_requests_count + 1);

			let miner_node_id_clone = miner_node_id.clone();
			let file_hash_to_remove_clone = file_hash_to_remove.clone();
			let new_cid_bounded_clone = new_cid_bounded.clone();

			MinerProfile::<T>::insert(&miner_node_id_clone, new_cid_bounded_clone);

			// UnpinRequests::<T>::mutate(|requests| {
				// requests.retain(|fh| *fh != file_hash_to_remove_clone);
			// });

			// Set the miner's state to Free
			MinerStates::<T>::insert(&miner_node_id.clone(), MinerState::Free);

			Ok(().into())
		}


		/// Unsigned transaction to set a miner's state to Locked
		#[pallet::call_index(5)]
		#[pallet::weight((10_000, DispatchClass::Operational, Pays::No))]
		pub fn set_miner_state_locked(
			origin: OriginFor<T>,
			miner_node_id: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>
		) -> DispatchResultWithPostInfo {
			// Ensure this is an unsigned transaction
			ensure_none(origin)?;

			// Rate limit: maximum storage requests per block per user
			let max_requests_per_block = T::MaxOffchainRequestsPerPeriod::get();
			let user_requests_count = RequestsCount::<T>::get(&miner_node_id);
			ensure!(user_requests_count + 1 <= max_requests_per_block, Error::<T>::TooManyRequests);

			// Update user's storage requests count
			RequestsCount::<T>::insert(&miner_node_id, user_requests_count + 1);

			// Set the miner's state to Locked
			MinerStates::<T>::insert(&miner_node_id, MinerState::Locked);

			Ok(().into())
		}

		/// Unsigned call to update UserProfile
		#[pallet::call_index(6)]  // Replace X with the next available call index
		#[pallet::weight(10_000)]
		pub fn update_user_profile(
			origin: OriginFor<T>,
			owner: T::AccountId,
			node_identity: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>,
			cid: FileHash,
		) -> DispatchResultWithPostInfo {
			ensure_none(origin)?;

			// Rate limit: maximum storage requests per block per user
			let max_requests_per_block = T::MaxOffchainRequestsPerPeriod::get();
			let user_requests_count = RequestsCount::<T>::get(&node_identity);
			ensure!(user_requests_count + 1 <= max_requests_per_block, Error::<T>::TooManyRequests);

			// Update user's storage requests count
			RequestsCount::<T>::insert(&node_identity, user_requests_count + 1);

			// Update UserProfile storage
			UserProfile::<T>::insert(&owner, cid);

			Ok(().into())
		}

		/// Unsigned transaction to mark a storage request as assigned
		#[pallet::call_index(7)]
		#[pallet::weight(10_000)]
		pub fn mark_storage_request_assigned(
			origin: OriginFor<T>,
			owner: T::AccountId,
			file_hash: FileHash,
		) -> DispatchResultWithPostInfo {
			ensure_none(origin)?;

			// Mark the storage request as assigned
			<UserStorageRequests<T>>::mutate(
				owner, 
				file_hash, 
				|request| {
					if let Some(ref mut req) = request {
						let mut updated_req = req.clone();
						updated_req.is_assigned = true;
						*request = Some(updated_req);
					}
				}
			);

			Ok(().into())
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

		/// Helper function to set a miner's state to Locked using unsigned transactions
		pub fn call_set_miner_state_locked(miner_node_id: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"IpfsPin::update_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);

			if let Ok(_guard) = lock.try_lock() {
				let signer = Signer::<T, <T as pallet::Config>::AuthorityId>::all_accounts();

				if !signer.can_sign() {
					log::warn!("No accounts available for signing in signer.");
					return;
				}

				let results = signer.send_unsigned_transaction(
					|account| MinerLockPayload {
						miner_node_id: miner_node_id.clone(),
						public: account.public.clone(),
						_marker: PhantomData,
					},
					|payload, _signature| {
						Call::set_miner_state_locked {
							miner_node_id: payload.miner_node_id,
						}
					},
				);

				for (acc, res) in &results {
					match res {
						Ok(()) => log::info!("[{:?}] Successfully set miner state to Locked", acc.id),
						Err(e) => log::error!("[{:?}] Failed to set miner state to Locked: {:?}", acc.id, e),
					}
				}
			} else {
				log::error!("❌ Could not acquire lock for setting miner state");
			};
		}

		/// Helper function to mark a storage request as assigned
		pub fn call_mark_storage_request_assigned(
			owner: T::AccountId, 
			file_hash: FileHash
		) {
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
						MarkStorageRequestAssignedPayload {
							owner: owner.clone(),
							file_hash: file_hash.clone(),
							public: account.public.clone(),
							_marker: PhantomData, // Ensure compatibility with generic payload types
						}
					},
					|payload, signature| {
						Call::mark_storage_request_assigned {
							owner: payload.owner,
							file_hash: payload.file_hash,
						}
					},
				);

				// Process the results of the transaction submission
				for (acc, res) in &results {
					match res {
						Ok(()) => log::info!("[{:?}] Successfully marked storage request as assigned", acc.id),
						Err(e) => log::error!("[{:?}] Failed to mark storage request as assigned: {:?}", acc.id, e),
					}
				}
			} else {
				log::error!("❌ Could not acquire lock for marking storage request as assigned");
			};
		}

		pub fn fetch_ipfs_file_size(file_hash_vec: Vec<u8>) -> Result<u32, http::Error> {
		
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

		pub fn call_process_miners_to_remove(owner: T::AccountId, file_hash_to_remove: FileHash, miner_node_id: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>, new_cid_bounded: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>) {
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
						ProcessMinersToRemoveRequestPayload {
							owner: owner.clone(),
							file_hash_to_remove: file_hash_to_remove.clone(),
							miner_node_id: miner_node_id.clone(),
							new_cid_bounded: new_cid_bounded.clone(),
							public: account.public.clone(),
							_marker: PhantomData, // Ensure compatibility with generic payload types
						}
					},
					|payload, signature| {
						Call::process_miners_to_remove {
							owner: payload.owner,
							file_hash_to_remove: payload.file_hash_to_remove,
							miner_node_id: payload.miner_node_id,
							new_cid_bounded: payload.new_cid_bounded,
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

		/// Helper function to fetch the node ID and hardware info, then store it in `NodeSpecs`.
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

		pub fn fetch_ipfs_content(cid: &str) -> Result<Vec<u8>, http::Error> {
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

		/// Returns a vector of all unique users who have made storage requests
		pub fn get_storage_request_users() -> Vec<T::AccountId> {
			UserProfile::<T>::iter_keys()
				.map(|user| user)
				.collect::<collections::BTreeSet<_>>()
				.into_iter()
				.collect()
		}

		// unpins the local ipfs node 
		pub fn unpin_file_from_ipfs(cid: &str) -> Result<(), http::Error> {

			let base_url = T::IPFSBaseUrl::get();
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

		/// Helper function to clear all storage requests, total files size, and add file hashes to unpin requests for a given account
		pub fn clear_user_storage_and_add_to_unpin_requests(origin: T::AccountId) -> DispatchResult {
			// Get all file hashes for the user
			let file_hashes: Vec<FileHash> = UserStorageRequests::<T>::iter_prefix(origin.clone())
				.map(|(file_hash, _)| file_hash)
				.collect();

			// Remove all storage requests for the user
			for file_hash in file_hashes.iter() {
				UserStorageRequests::<T>::remove(origin.clone(), file_hash);
				
			}

			// Clear the user's total files size
			UserTotalFilesSize::<T>::remove(origin.clone());

			// Add file hashes to unpin requests if not already there
			let mut current_unpin_requests = UnpinRequests::<T>::get();
			for file_hash in file_hashes {
				if !current_unpin_requests.contains(&file_hash) {
					current_unpin_requests.try_push(file_hash)
						.map_err(|_| Error::<T>::MaxUnpinRequestsExceeded)?;
				}
			}
			UnpinRequests::<T>::put(current_unpin_requests);

			Ok(())
		}

		// pins the local ipfs node 
		pub fn pin_file_from_ipfs(cid: &str) -> Result<(), http::Error> {

			let base_url = T::IPFSBaseUrl::get();
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

		/// Helper function to clear the MinerProfile storage entry for a given node ID
		pub fn clear_miner_profile(node_id: Vec<u8>) -> DispatchResult {
			// Remove the MinerProfile entry for the given node ID
			MinerProfile::<T>::remove(BoundedVec::try_from(node_id).map_err(|_| Error::<T>::StorageOverflow)?);
			
			Ok(())
		}

		// // Process unpin requests and update MinerProfile accordingly
		// pub fn hanlde_unpin_requests() -> Result<(), DispatchError> {
		// 	let unpin_requests = UnpinRequests::<T>::get();
		// 	let blacklist_entries = Blacklist::<T>::get();

		// 	// Combine unpin requests and blacklist entries
		// 	let all_file_hashes = unpin_requests.iter().chain(blacklist_entries.iter());

		// 	if unpin_requests.is_empty() {
		// 		log::info!("No unpin requests to process.");
		// 		return Ok(());
		// 	}

		// 	// Iterate over each file hash in UnpinRequests
		// 	for file_hash in all_file_hashes {
		// 		// Find the owner for this file hash from UserStorageRequests
		// 		let owner = UserStorageRequests::<T>::iter_keys()
		// 		.find(|(_, hash)| hash == file_hash)
		// 		.map(|(account, _)| account)
		// 		.ok_or(Error::<T>::OwnerNotFound)?;

		// 		// Get the miners storing this file hash
		// 		let miners = FileStorageMiners::<T>::get(&owner, file_hash);
		// 		if miners.is_empty() {
		// 			log::info!("No miners found for file hash: {:?}", file_hash);
		// 			continue;
		// 		}

		// 		// Lock all selected miners after they are chosen
		// 		for (miner, _, _) in &miners {
		// 			let miner_node_id = BoundedVec::try_from(miner.node_id.clone())
		// 				.map_err(|_| Error::<T>::StorageOverflow)?;
					
		// 			// Call the set_miner_state_locked function from the IPFS pallet
		// 			Self::call_set_miner_state_locked(miner_node_id);	
		// 		}

		// 		// Process each miner
		// 		for miner_node_id in miners.iter() {
		// 			// Get the CID from MinerProfile
		// 			let cid = MinerProfile::<T>::get(miner_node_id);
		// 			if cid.is_empty() {
		// 				log::info!("No CID found for miner_node_id: {:?}", miner_node_id);
		// 				continue;
		// 			}

		// 			let cid_str = sp_std::str::from_utf8(&cid).map_err(|_| Error::<T>::InvalidCid)?;

		// 			// Fetch content from IPFS
		// 			let content = Self::fetch_ipfs_content(cid_str).map_err(|e| {
		// 				log::error!("Failed to fetch content for CID {}: {:?}", cid_str, e);
		// 				Error::<T>::IpfsError
		// 			})?;

		// 			// Parse the JSON array
		// 			let mut requests: Vec<Value> = serde_json::from_slice(&content).map_err(|e| {
		// 				log::error!("Failed to parse JSON content: {:?}", e);
		// 				Error::<T>::InvalidJson
		// 			})?;

		// 			// Convert file_hash to Vec<u8> for comparison
		// 			let file_hash_vec = file_hash.clone().into_inner();

		// 			// Filter out the object with the matching file_hash
		// 			let original_len = requests.len();
		// 			requests.retain(|request| {
		// 				let req_file_hash = request["file_hash"].as_array()
		// 					.map(|arr| arr.iter().filter_map(|v| v.as_u64().map(|n| n as u8)).collect::<Vec<u8>>())
		// 					.unwrap_or_default();
		// 				req_file_hash != file_hash_vec
		// 			});

		// 			if requests.len() == original_len {
		// 				log::info!("No matching file_hash found in CID {} for removal.", cid_str);
		// 				continue;
		// 			}

		// 			// Serialize the updated array back to JSON
		// 			let updated_json = serde_json::to_string(&requests).map_err(|e| {
		// 				log::error!("Failed to serialize updated JSON: {:?}", e);
		// 				Error::<T>::InvalidJson
		// 			})?;

		// 			// Pin the updated content to IPFS
		// 			match Self::pin_file_to_ipfs(&updated_json) {
		// 				Ok(new_cid) => {
		// 					log::info!("Successfully pinned updated content with new CID: {}", new_cid);

		// 					// Update MinerProfile with the new CID
		// 					let new_cid_vec = new_cid.into_bytes();
		// 					let new_cid_bounded = BoundedVec::try_from(new_cid_vec)
		// 						.map_err(|_| Error::<T>::StorageOverflow)?;

		// 					Self::call_process_miners_to_remove(owner.clone(), file_hash.clone(), miner_node_id.clone(), new_cid_bounded);

		// 					// Unpin the old CID from the local node (if desired)
		// 					if let Err(e) = Self::unpin_file_from_ipfs(cid_str) {
		// 						log::error!("Failed to unpin old CID {}: {:?}", cid_str, e);
		// 					}
		// 				}
		// 				Err(e) => {
		// 					log::error!("Failed to pin updated content: {:?}", e);
		// 				}
		// 			}
		// 		}
		// 	}

		// 	Ok(())
		// }

		/// Retrieves all files for a given account with their pinning information
		///
		/// # Arguments
		///
		/// * `account`: The account address to retrieve files for
		///
		/// # Returns
		///
		/// A vector of UserFile structs containing file details and pinning miners
		pub fn get_user_files(account: T::AccountId) -> Vec<UserFile> {
			let cid = UserProfile::<T>::get(&account);
			
			// If no CID exists, return an empty vector
			if cid.is_empty() {
				log::info!("No UserProfile CID found for account {:?}", account);
				return Vec::new();
			}

			let cid_str = match sp_std::str::from_utf8(&cid) {
				Ok(s) => s,
				Err(_) => {
					log::error!("Invalid UTF-8 in UserProfile CID for account {:?}", account);
					return Vec::new();
				}
			};

			// Fetch content from IPFS
			let content = match Self::fetch_ipfs_content(cid_str) {
				Ok(content) => content,
				Err(e) => {
					log::error!("Failed to fetch UserProfile content for CID {}: {:?}", cid_str, e);
					return Vec::new();
				}
			};

			// Parse the JSON array
			let requests: Vec<Value> = match serde_json::from_slice(&content) {
				Ok(requests) => requests,
				Err(e) => {
					log::error!("Failed to parse UserProfile JSON content: {:?}", e);
					return Vec::new();
				}
			};

			// Convert JSON array to UserFile structs
			requests
				.into_iter()
				.filter_map(|request| {
					// Extract fields from the StorageRequest JSON
					let file_hash = request["file_hash"].as_array()
						.and_then(|arr| arr.iter().map(|v| v.as_u64().map(|n| n as u8)).collect::<Option<Vec<u8>>>())
						.and_then(|vec| BoundedVec::try_from(vec).ok())?;
					let file_name = request["file_name"].as_array()
						.and_then(|arr| arr.iter().map(|v| v.as_u64().map(|n| n as u8)).collect::<Option<Vec<u8>>>())
						.and_then(|vec| BoundedVec::try_from(vec).ok())?;
					let created_at = request["created_at"].as_u64().unwrap_or(0) as u32;

					let miner_ids_vec: Vec<Vec<u8>> = Vec::new();

					// Fetch file size
					let file_size = match Self::fetch_ipfs_file_size(file_hash.to_vec()) {
						Ok(size) => size,
						Err(e) => {
							log::error!("Failed to fetch file size for hash {:?}: {:?}", file_hash, e);
							0 // Default to 0 if fetching fails
						}
					};

					Some(UserFile {
						file_hash,
						file_name,
						miner_ids: miner_ids_vec,
						file_size,
						created_at,
					})
				})
				.collect()
		}

		/// Get the total file size pinned by a specific miner
		pub fn get_total_file_size_by_miner(miner_id: Vec<u8>) -> u128 {
			// Convert miner_id to BoundedVec
			let miner_id_bounded = match BoundedVec::try_from(miner_id) {
				Ok(bounded) => bounded,
				Err(_) => {
					log::error!("Failed to convert miner_id to BoundedVec");
					return 0;
				}
			};

			// Retrieve the total file size directly from MinerTotalFilesSize storage
			MinerTotalFilesSize::<T>::get(&miner_id_bounded).unwrap_or(0)
		}

		/// Check if a miner is in a Free state
		pub fn is_miner_free(miner_node_id: &BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>) -> bool {
			MinerStates::<T>::get(miner_node_id) == MinerState::Free
		}

		/// Unsigned transaction function to update UserProfile
		pub fn call_update_user_profile(owner: T::AccountId, node_identity: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>, cid: FileHash) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"IpfsPin::update_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);

			if let Ok(_guard) = lock.try_lock() {
				let signer = Signer::<T, <T as pallet::Config>::AuthorityId>::all_accounts();

				if !signer.can_sign() {
					log::warn!("No accounts available for signing in signer.");
					return;
				}

				let results = signer.send_unsigned_transaction(
					|account| UpdateUserProfilePayload {
						owner: owner.clone(),
						cid: cid.clone(),
						node_identity: node_identity.clone(),
						public: account.public.clone(),
						_marker: PhantomData,
					},
					|payload, _signature| {
						Call::update_user_profile {
							owner: payload.owner,
							node_identity: payload.node_identity,
							cid: payload.cid,
						}
					},
				);

				for (acc, res) in &results {
					match res {
						Ok(()) => log::info!("[{:?}] Successfully updated UserProfile", acc.id),
						Err(e) => log::error!("[{:?}] Failed to update UserProfile: {:?}", acc.id, e),
					}
				}
			} else {
				log::error!("❌ Could not acquire lock for updating UserProfile");
			};
		}

		/// Get all storage requests that are already assigned
		pub fn get_assigned_storage_requests() -> Vec<(T::AccountId, FileHash, StorageRequest<T::AccountId, BlockNumberFor<T>>)> {
			UserStorageRequests::<T>::iter()
				.filter_map(|(owner, file_hash, maybe_request)| 
					maybe_request.filter(|request| request.is_assigned)
						.map(|request| (owner, file_hash, request))
				)
				.collect()
		}

		/// Clean up storage requests older than the specified block threshold
		fn cleanup_old_storage_requests(current_block: BlockNumberFor<T>, block_threshold: BlockNumberFor<T>) {
			// Collect keys of requests to remove
			let requests_to_remove: Vec<(T::AccountId, FileHash)> = UserStorageRequests::<T>::iter()
				.filter_map(|(owner, file_hash, maybe_request)| 
					maybe_request.and_then(|request| 
						if current_block.saturating_sub(request.created_at) > block_threshold {
							Some((owner, file_hash))
						} else {
							None
						}
					)
				)
				.collect();

			// Remove the old requests
			for (owner, file_hash) in requests_to_remove {
				UserStorageRequests::<T>::remove(&owner, &file_hash);
			}
		}

	}
}
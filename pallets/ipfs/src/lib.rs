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
	use codec::alloc::string::ToString;
	use scale_info::prelude::string::String;
	use frame_system::offchain::AppCrypto;
	use frame_system::offchain::SendTransactionTypes;
	use scale_info::prelude::collections;
	use serde_json::Value;
	use sp_runtime::Saturating;
	use sp_std::convert::TryInto;
	use sp_std::collections::btree_set::BTreeSet;
	use sp_runtime::AccountId32;
	use sp_core::crypto::Ss58Codec;
	use pallet_registration::{
		NodeType, 
		Pallet as RegistrationPallet, 
	};

	const DUMMY_REQUEST_BODY: &[u8; 78] = b"{\"id\": 10, \"jsonrpc\": \"2.0\", \"method\": \"chain_getFinalizedHead\", \"params\": []}";

	#[pallet::pallet]
	#[pallet::storage_version(STORAGE_VERSION)]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config + SendTransactionTypes<Call<Self>> +  pallet_staking::Config + 
						pallet_registration::Config + pallet_rankings::Config + pallet_utils::Config + pallet_proxy::Config{
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

		/// The duration of an epoch in blocks.
		#[pallet::constant]
		type EpochPeriod: Get<BlockNumberFor<Self>>;
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

	#[pallet::storage]
	#[pallet::getter(fn current_epoch_validator)]
	pub type CurrentEpochValidator<T: Config> = StorageValue<
		_,
		Option<(T::AccountId, BlockNumberFor<T>)>,
		ValueQuery,
	>;

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
			// Check for consensus application every consensus period
			let consensus_period = <T as pallet_registration::Config>::ConsensusPeriod::get();
			if current_block % consensus_period == 0u32.into() {
				Self::apply_deregistration_consensus();
			}

			// Clear entries every 10 blocks
			if current_block % T::RequestsClearInterval::get().into() == 0u32.into() {
				// Clear all entries; limit is u32::MAX to ensure we get them all
				let _result = RequestsCount::<T>::clear(u32::MAX, None);
			}
							
			// Remove storage requests older than 500 blocks
			Self::cleanup_old_storage_requests(current_block, BlockNumberFor::<T>::from(700u32));

			// get all degraded miners and add rebalance Request and then delete them
			let degraded_miners = RegistrationPallet::<T>::get_all_degraded_storage_miners();
			for miner in degraded_miners {
				let _ = Self::add_rebalance_request_from_node(miner.clone());
				let _ = RegistrationPallet::<T>::try_unregister_storage_miner(miner);
			}

			// Remove MinerProfile entries with empty miner_profile_id
			MinerProfile::<T>::iter().filter(|(_, profile_id)| profile_id.is_empty()).for_each(|(node_id, _)| {
				MinerProfile::<T>::remove(&node_id);
			});

			// Get the current epoch validator and its start block
			let current_validator_info = CurrentEpochValidator::<T>::get();
			let (current_validator, epoch_start) = current_validator_info
				.map(|(acc, block)| (Some(acc), Some(block)))
				.unwrap_or((None, None));

			// Attempt to rotate the validator
			match Self::rotate_validator(current_block, current_validator, epoch_start) {
				Ok(true) => T::DbWeight::get().writes(1), // Rotation occurred, storage write
				Ok(false) => T::DbWeight::get().reads(1), // No rotation, storage read
				Err(_) => T::DbWeight::get().reads(1), // Error case, count as read
			}
			
			// Weight::zero()
		}
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		SomethingStored { something: u32, who: T::AccountId },
		StorageRequestUpdated { owner: T::AccountId, file_hash: FileHash, file_size: u128 },
		UnpinRequestCompleted { owner: T::AccountId, file_hash: FileHash, file_size: u128 },
		PinningEnabledChanged { enabled: bool },
		MinerProfilesUpdated { miner_count: u32 },
		StorageRequestsCleared,
		ReputationPointsUpdated { coldkey: T::AccountId, points: u32 },
		RotationStatusChanged(bool),
	    /// A user's storage request was removed due to IPFS unavailability
		IpfsUnavailable {
			owner: T::AccountId,
			file_hash: FileHash,
		},
		UserProfileUpdated {
			owner: T::AccountId,
			cid: FileHash,
		},
		UsersProfilesUpdated,
		MinersProfilesUpdated,
		MinerProfileUpdated {
			miner_node_id: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>,
			cid: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>,
		},
		/// Emitted when validator is rotated at the beginning of a new epoch.
		ValidatorRotated {
			new_validator: T::AccountId,
			epoch_start_block: BlockNumberFor<T>,
		},
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
		InvalidNodeType,
		MinerNotLocked,
		AssignmentNotEnabled,
		StorageRequestsCleared,
		FileHashBlacklisted,
		MinersNotLocked,
		UnauthorizedLocker,
		MinersAlreadyLocked,
		NodeIdTooLong,
		RequestNotFound,
		InvalidReputationPoints,
		UserIsBlacklisted,
		InvalidAccountId,
		NotCurrentEpochValidator,
		FileSizeOverflow
	}

	// the file size where the key is encoded file hash
	#[pallet::storage]
	#[pallet::getter(fn user_total_files_size)]
	pub type UserTotalFilesSize<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, u128>;

	// the file size where the key is encoded file hash
	#[pallet::storage]
	#[pallet::getter(fn miner_files_size)]
	pub type MinerTotalFilesSize<T: Config> = StorageMap<_, Blake2_128Concat, BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>, u128>;

	#[pallet::storage]
	#[pallet::getter(fn miner_total_files_pinned)]
	pub type MinerTotalFilesPinned<T: Config> = StorageMap<_, Blake2_128Concat, BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>, u32>;

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
	#[pallet::getter(fn blacklisted_users)]
	pub type BlacklistedUsers<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		T::AccountId,
		bool,
		ValueQuery
	>;

	// Saves the owners request to unpin the file
	#[pallet::storage]
	#[pallet::getter(fn user_unpin_requests)]
	pub type UserUnpinRequests<T: Config> = StorageValue<
		_, 
		BoundedVec<StorageUnpinRequest<T::AccountId>, ConstU32<MAX_UNPIN_REQUESTS>>, 
		ValueQuery
	>;

	#[pallet::storage]
    #[pallet::getter(fn reputation_points)]
    pub type ReputationPoints<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn rebalance_request)]
	pub type RebalanceRequest<T: Config> =
		StorageValue<_, BoundedVec<RebalanceRequestItem, ConstU32<MAX_REBALANCE_REQUESTS>>, ValueQuery>;
	
	//  Blacklist storage item containing the blacklisted Files Hashes
	#[pallet::storage]
	pub type Blacklist<T: Config> = StorageValue<_, BoundedVec<FileHash, ConstU32<MAX_BLACKLIST_ENTRIES>>, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn unpin_requests)]
	pub(super) type UnpinRequests<T: Config> = StorageValue<_, BoundedVec<FileHash, ConstU32<MAX_UNPIN_REQUESTS>>, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn rotation_whitelisting_enabled)]
	pub(super) type RotationWhitelistingEnabled<T: Config> = StorageValue<_, bool, ValueQuery>;

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

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Sudo function to enable or disable file assignments
		#[pallet::call_index(1)]
		#[pallet::weight((10_000, DispatchClass::Operational, Pays::No))]
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
		#[pallet::weight((10_000, DispatchClass::Operational, Pays::No))]
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

		/// Unsigned transaction to set a miner's state to Locked
		#[pallet::call_index(3)]
		#[pallet::weight((10_000, DispatchClass::Operational, Pays::No))]
		pub fn remove_bad_storage_request(
			origin: OriginFor<T>,
			file_hash: FileHash,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Check if this is a proxy account and get the main account
			let main_account = if let Some(primary) = Self::get_primary_account(&who)? {
				primary
			} else {
				who.clone() // If not a proxy, use the account itself
			};

			// Check if the node is registered
			let node_info = RegistrationPallet::<T>::get_registered_node_for_owner(&main_account);
			ensure!(node_info.is_some(), Error::<T>::NodeNotRegistered);

			// Unwrap safely after checking it's Some
			let node_info = node_info.unwrap();

			// Check if the node type is Validator
			ensure!(node_info.node_type == NodeType::Validator, Error::<T>::InvalidNodeType);

			// Rate limit: maximum storage requests per block per user
			let max_requests_per_block = T::MaxOffchainRequestsPerPeriod::get();
			let user_requests_count = RequestsCount::<T>::get(&BoundedVec::truncate_from(node_info.node_id.clone()));
			ensure!(user_requests_count + 1 <= max_requests_per_block, Error::<T>::TooManyRequests);

			// Update user's storage requests count
			RequestsCount::<T>::insert(&BoundedVec::truncate_from(node_info.node_id.clone()), user_requests_count + 1);

			// Convert file hash to a consistent format for storage lookup
			let file_hash_key = hex::encode(file_hash.clone());
			let update_hash_vec: Vec<u8> = file_hash_key.into();

			let update_hash: FileHash = BoundedVec::try_from(update_hash_vec)
			.map_err(|_| Error::<T>::StorageOverflow)?;

			// Remove the storage request
			<UserStorageRequests<T>>::remove(
				main_account.clone(), 
				update_hash.clone()
			);

			// Add the file hash to the blacklist
			<Blacklist<T>>::mutate(|blacklist| {
				if !blacklist.contains(&update_hash) {
					blacklist.try_push(update_hash.clone())
						.map_err(|_| Error::<T>::StorageOverflow)
						.ok();
				}
			});
			
			Ok(().into())
		}

		/// Unsigned transaction to remove a bad unpin request
		#[pallet::call_index(4)]
		#[pallet::weight((10_000, DispatchClass::Operational, Pays::No))]
		pub fn remove_bad_unpin_request(
			origin: OriginFor<T>,
			file_hash: FileHash,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Check if this is a proxy account and get the main account
			let main_account = if let Some(primary) = Self::get_primary_account(&who)? {
				primary
			} else {
				who.clone() // If not a proxy, use the account itself
			};

			// Check if the node is registered
			let node_info = RegistrationPallet::<T>::get_registered_node_for_owner(&main_account);
			ensure!(node_info.is_some(), Error::<T>::NodeNotRegistered);
			let node_info = node_info.unwrap();

			// Check if node type is Validator
			ensure!(
				node_info.node_type == NodeType::Validator,
				Error::<T>::InvalidNodeType
			);

			// Rate limit
			let max_requests_per_block = T::MaxOffchainRequestsPerPeriod::get();
			let user_requests_count =
				RequestsCount::<T>::get(&BoundedVec::truncate_from(node_info.node_id.clone()));
			ensure!(
				user_requests_count + 1 <= max_requests_per_block,
				Error::<T>::TooManyRequests
			);

			RequestsCount::<T>::insert(
				&BoundedVec::truncate_from(node_info.node_id.clone()),
				user_requests_count + 1,
			);

			// Remove the specific file hash from UserUnpinRequests
			UserUnpinRequests::<T>::mutate(|requests| {
				if let Some(pos) = requests.iter().position(|req| req.file_hash == file_hash) {
					requests.remove(pos);
				}
			});

			Ok(().into())
		}

		// miners request to store a file given file hash
		#[pallet::call_index(6)]
		#[pallet::weight((0, Pays::No))]
		pub fn update_pin_and_storage_requests(
			origin: OriginFor<T>,
			requests: Vec<StorageRequestUpdate<T::AccountId>>,
			miner_profiles: Vec<MinerProfileItem>,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Check if assignment is enabled
			ensure!(
				Self::assignment_enabled(), 
				Error::<T>::AssignmentNotEnabled
			);

			// Check if this is a proxy account and get the main account
			let main_account = if let Some(primary) = Self::get_primary_account(&who)? {
				primary
			} else {
				who.clone() // If not a proxy, use the account itself
			};

			// Check if the node is registered
			let node_info = RegistrationPallet::<T>::get_registered_node_for_owner(&main_account);
			ensure!(node_info.is_some(), Error::<T>::NodeNotRegistered);

			// Unwrap safely after checking it's Some
			let node_info = node_info.unwrap();

			// Check if the node type is Validator
			ensure!(node_info.node_type == NodeType::Validator, Error::<T>::InvalidNodeType);

			// Check if the signer is the current selected validator of epoch
			let current_validator = Self::select_validator()?;
			ensure!(
				main_account == current_validator,
				Error::<T>::NotCurrentEpochValidator
			);

			for request in requests {

			    // Rate limit: maximum storage requests per block per user
			    let max_requests_per_block = T::MaxOffchainRequestsPerPeriod::get();
			    let user_requests_count = RequestsCount::<T>::get(&BoundedVec::truncate_from(node_info.node_id.clone()));
			    ensure!(user_requests_count + 1 <= max_requests_per_block, Error::<T>::TooManyRequests);

			    // Update user's storage requests count
			    RequestsCount::<T>::insert(&BoundedVec::truncate_from(node_info.node_id.clone()), user_requests_count + 1);

				<UserTotalFilesSize<T>>::insert(
					request.storage_request_owner.clone(),
					request.file_size,
				);
				
			    // Remove the storage request
			    <UserStorageRequests<T>>::remove(
			    	request.storage_request_owner.clone(), 
			    	request.storage_request_file_hash.clone()
			    );

			    // Update UserProfile storage
			    UserProfile::<T>::insert(&request.storage_request_owner, request.user_profile_cid.clone());

				Self::deposit_event(Event::UserProfileUpdated {
					owner: request.storage_request_owner.clone(),
					cid: request.user_profile_cid.clone(),
				});

				// Deposit an event to log the pin request update
			    Self::deposit_event(Event::StorageRequestUpdated {
			    	owner: request.storage_request_owner,
			    	file_hash: request.storage_request_file_hash,
			    	file_size: request.file_size as u128,
			    });
			}

			// // Ensure all miners are registered as StorageMiners
			// for miner_profile in miner_profiles.iter() {
			// 	let miner_node_id_vec = miner_profile.miner_node_id.clone().into_inner();
			// 	let miner_node_info =
			// 		RegistrationPallet::<T>::get_node_registration_info(miner_node_id_vec.clone());
			// 	ensure!(
			// 		miner_node_info.is_some(),
			// 		Error::<T>::NodeNotRegistered
			// 	);
			// 	let miner_node_info = miner_node_info.unwrap();
			// 	ensure!(
			// 		miner_node_info.node_type == NodeType::StorageMiner,
			// 		Error::<T>::InvalidNodeType
			// 	);
			// }

			// Update total file size and pinned file count for each miner
			for miner_profile in miner_profiles.iter() {
				<MinerTotalFilesSize<T>>::insert(&miner_profile.miner_node_id, miner_profile.files_size as u128);
				<MinerTotalFilesPinned<T>>::insert(
					&miner_profile.miner_node_id,
					miner_profile.files_count,
				);
		
				// Update MinerProfile storage with node ID and CID
				let bounded_cid = BoundedVec::<u8, ConstU32<MAX_NODE_ID_LENGTH>>::try_from(
					miner_profile.cid.clone().into_inner().to_vec(),
				).unwrap_or_else(|v: Vec<u8>| BoundedVec::truncate_from(v));

				<MinerProfile<T>>::insert(miner_profile.miner_node_id.clone(), bounded_cid.clone());

				Self::deposit_event(Event::MinerProfileUpdated {
					miner_node_id: miner_profile.miner_node_id.clone(),
					cid: bounded_cid,
				});
			}

			Ok(().into())
		}

		// miners request to store a file given file hash 
		#[pallet::call_index(7)]
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn update_unpin_and_storage_requests(
			origin: OriginFor<T>, 
			requests: Vec<StorageUnpinUpdateRequest<T::AccountId>>,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Check if this is a proxy account and get the main account
			let main_account = if let Some(primary) = Self::get_primary_account(&who)? {
				primary
			} else {
				who.clone() // If not a proxy, use the account itself
			};

			// Check if the node is registered
			let node_info = RegistrationPallet::<T>::get_registered_node_for_owner(&main_account);
			ensure!(node_info.is_some(), Error::<T>::NodeNotRegistered);

			// Unwrap safely after checking it's Some
			let node_info = node_info.unwrap();

			// Check if the node type is Validator
			ensure!(node_info.node_type == NodeType::Validator, Error::<T>::InvalidNodeType);

			// Check if the signer is the current selected validator of the epoch
			let current_validator = Self::select_validator()?;
			ensure!(
				main_account == current_validator,
				Error::<T>::NotCurrentEpochValidator
			);

			// Rate limit: maximum storage requests per block per user
			let max_requests_per_block = T::MaxOffchainRequestsPerPeriod::get();
			let user_requests_count = RequestsCount::<T>::get(&BoundedVec::truncate_from(node_info.node_id.clone()));
			ensure!(user_requests_count + 1 <= max_requests_per_block, Error::<T>::TooManyRequests);

			// Update user's storage requests count
			RequestsCount::<T>::insert(&BoundedVec::truncate_from(node_info.node_id.clone()), user_requests_count + 1);

			for request in requests {
				// Update MinerProfile storage for each miner pin request
				for miner_profile in request.miner_pin_requests.iter() {
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
				<UserTotalFilesSize<T>>::mutate(request.storage_request_owner.clone(), |total_size| {
					*total_size = Some(total_size.unwrap_or(0) - request.file_size);
				});

				// Update MinerTotalFilesSize for each miner
				for miner_profile in request.miner_pin_requests.iter() {
					<MinerTotalFilesSize<T>>::mutate(&miner_profile.miner_node_id, |total_size| {
						*total_size = Some(total_size.unwrap_or(0) - request.file_size);
					});
					<MinerTotalFilesPinned<T>>::insert(&miner_profile.miner_node_id, miner_profile.files_count);
				}

				// Remove the unpin request for this file hash
				UserUnpinRequests::<T>::mutate(|requests| {
					requests.retain(|req| req.file_hash != request.storage_request_file_hash);
				});

				// Update UserProfile storage
				UserProfile::<T>::insert(&request.storage_request_owner, request.user_profile_cid);

				// Deposit an event to log the pin request update
				Self::deposit_event(Event::UnpinRequestCompleted {
					owner: request.storage_request_owner,
					file_hash: request.storage_request_file_hash,
					file_size: request.file_size,
				});			
			}

			Ok(().into())
		}

		/// Removes all unpin requests by the specified owner.
		#[pallet::call_index(9)]
		#[pallet::weight((10_000, DispatchClass::Operational, Pays::No))]
		pub fn sudo_remove_unpin_requests(
			origin: OriginFor<T>,
			owner: T::AccountId,
		) -> DispatchResult {
			// Ensure origin is Root (i.e. sudo)
			ensure_root(origin)?;
	
			// Get existing requests
			let mut requests = <UserUnpinRequests<T>>::get();
	
			// Filter out requests that belong to the given owner
			requests.retain(|r| r.owner != owner);
	
			// Update storage
			<UserUnpinRequests<T>>::set(requests);
	
			Ok(())
		}

		#[pallet::call_index(10)]
		#[pallet::weight((10_000, DispatchClass::Operational, Pays::No))]
		pub fn remove_rebalance_request(
			origin: OriginFor<T>,
			node_rebalanace_request_to_remove: Option<Vec<Vec<u8>>>,
			updated_miner_profiles: Vec<UpdatedMinerProfileItem>,
			updated_user_profiles: Vec<UpdatedUserProfileItem<T::AccountId>>,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;
		
			// Check if assignment is enabled
			ensure!(Self::assignment_enabled(), Error::<T>::AssignmentNotEnabled);

			// Check if this is a proxy account and get the main account
			let main_account = if let Some(primary) = Self::get_primary_account(&who)? {
				primary
			} else {
				who.clone() // If not a proxy, use the account itself
			};
		
			// Check if the node is registered
			let node_info = RegistrationPallet::<T>::get_registered_node_for_owner(&main_account);
			ensure!(node_info.is_some(), Error::<T>::NodeNotRegistered);
		
			let node_info = node_info.unwrap();
		
			// Ensure the node type is Validator
			ensure!(node_info.node_type == NodeType::Validator, Error::<T>::InvalidNodeType);
		
			// Handle rebalance request removal
			if let Some(node_ids) = node_rebalanace_request_to_remove {
				// Convert each node_id into BoundedVec
				let mut bounded_node_ids = BTreeSet::new();
				for id in node_ids {
					let bounded_id: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>> =
						id.try_into().map_err(|_| Error::<T>::NodeIdTooLong)?;
					bounded_node_ids.insert(bounded_id);
				}
		
				let mut requests = <RebalanceRequest<T>>::get();
				let original_len = requests.len();
		
				// Retain only the requests that are NOT in the removal set
				requests.retain(|req| !bounded_node_ids.contains(&req.node_id));
		
				ensure!(requests.len() < original_len, Error::<T>::RequestNotFound);
		
				<RebalanceRequest<T>>::put(requests);
			}
		
			// Update miner profiles
			for miner_update in updated_miner_profiles {
				// Update miner profile CID
				<MinerProfile<T>>::insert(&miner_update.miner_node_id, &miner_update.cid);
		
				// Update total files pinned
				let current_count = <MinerTotalFilesPinned<T>>::get(&miner_update.miner_node_id).unwrap_or(0);
				<MinerTotalFilesPinned<T>>::insert(
					&miner_update.miner_node_id,
					current_count.saturating_add(miner_update.added_files_count)
				);
		
				// Update total files size
				let current_size = <MinerTotalFilesSize<T>>::get(&miner_update.miner_node_id).unwrap_or(0);
				<MinerTotalFilesSize<T>>::insert(
					&miner_update.miner_node_id,
					current_size.saturating_add(miner_update.added_file_size)
				);
			}
		
			// Update user profiles
			for user_update in updated_user_profiles {
				<UserProfile<T>>::insert(&user_update.user, &user_update.cid);
			}
		
			Ok(())
		}		

		#[pallet::call_index(11)]
		#[pallet::weight((10_000, DispatchClass::Operational, Pays::No))]
		pub fn blacklist_user(
			origin: OriginFor<T>,
			user: T::AccountId,
			blacklist: bool,
		) -> DispatchResult {
			// Ensure this is called by sudo
			ensure_root(origin)?;
	
			if blacklist {
				BlacklistedUsers::<T>::insert(&user, true);
			} else {
				BlacklistedUsers::<T>::remove(&user);
			}
	
			Ok(())
		}

		/// Set rotation enabled or disabled (sudo-only)
		#[pallet::call_index(12)]
		#[pallet::weight((10_000, DispatchClass::Operational, Pays::No))]
		pub fn set_rotation_whitelisting_enabled(origin: OriginFor<T>, enabled: bool) -> DispatchResult {
			ensure_root(origin)?; // Only sudo/root can call this

			RotationWhitelistingEnabled::<T>::put(enabled);

			Self::deposit_event(Event::RotationStatusChanged(enabled));
			Ok(())
		}

		#[pallet::call_index(13)]
		#[pallet::weight((10_000, DispatchClass::Operational, Pays::No))]
		pub fn clear_all_data(origin: OriginFor<T>) -> DispatchResult {
			ensure_root(origin)?; // Only root (sudo) can call this

			// Clear MinerProfile
			for (node_id, _) in <MinerProfile<T>>::iter() {
				<MinerProfile<T>>::remove(node_id);
			}

			// Clear UserProfile
			for (account_id, _) in <UserProfile<T>>::iter() {
				<UserProfile<T>>::remove(account_id);
			}

			for (user, _) in UserTotalFilesSize::<T>::iter() {
				UserTotalFilesSize::<T>::insert(user, 0u128);
			}
			for (miner, _) in MinerTotalFilesSize::<T>::iter() {
				MinerTotalFilesSize::<T>::insert(miner.clone(), 0u128);
				MinerTotalFilesPinned::<T>::insert(miner, 0u32);
			}

			Ok(())
		}

		#[pallet::call_index(14)]
		#[pallet::weight((10_000, DispatchClass::Operational, Pays::No))]
		pub fn update_miner_profiles(
			origin: OriginFor<T>,
			profiles: Vec<(Vec<u8>, Vec<u8>)>, // (miner_id, profile_cid)
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			// Check if this is a proxy account and get the main account
			let main_account = if let Some(primary) = Self::get_primary_account(&who)? {
				primary
			} else {
				who.clone() // If not a proxy, use the account itself
			};

			// Check if the node is registered
			let node_info = RegistrationPallet::<T>::get_registered_node_for_owner(&main_account);
			ensure!(node_info.is_some(), Error::<T>::NodeNotRegistered);

			// Unwrap safely after checking it's Some
			let node_info = node_info.unwrap();

			// Check if the node type is Validator
			ensure!(node_info.node_type == NodeType::Validator, Error::<T>::InvalidNodeType);

			// Clear all existing MinerProfiles
			for (node_id, _) in <MinerProfile<T>>::iter() {
				<MinerProfile<T>>::remove(node_id);
			}

			for (miner_id, profile_cid) in profiles {
				let bounded_miner_id = BoundedVec::<u8, ConstU32<MAX_NODE_ID_LENGTH>>::try_from(miner_id)
					.map_err(|_| Error::<T>::NodeIdTooLong)?;
				let bounded_profile_cid = BoundedVec::<u8, ConstU32<MAX_NODE_ID_LENGTH>>::try_from(profile_cid)
					.map_err(|_| Error::<T>::NodeIdTooLong)?;
				<MinerProfile<T>>::insert(bounded_miner_id, bounded_profile_cid);
			}

			Self::deposit_event(Event::MinersProfilesUpdated);

			Ok(())
		}

		#[pallet::call_index(15)]
		#[pallet::weight((10_000, DispatchClass::Operational, Pays::No))]
		pub fn update_user_profiles(
			origin: OriginFor<T>,
			profiles: Vec<(T::AccountId, Vec<u8>)>, // (user_account, profile_cid)
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			// Check if this is a proxy account and get the main account
			let main_account = if let Some(primary) = Self::get_primary_account(&who)? {
				primary
			} else {
				who.clone() // If not a proxy, use the account itself
			};

			// Check if the node is registered
			let node_info = RegistrationPallet::<T>::get_registered_node_for_owner(&main_account);
			ensure!(node_info.is_some(), Error::<T>::NodeNotRegistered);

			// Unwrap safely after checking it's Some
			let node_info = node_info.unwrap();

			// Check if the node type is Validator
			ensure!(node_info.node_type == NodeType::Validator, Error::<T>::InvalidNodeType);

			// Clear all existing UserProfiles
			for (account_id, _) in <UserProfile<T>>::iter() {
				<UserProfile<T>>::remove(account_id);
			}

			// Set new profiles from the input array
			for (user_account, profile_cid) in profiles {
				let bounded_profile_cid = FileHash::try_from(profile_cid)
					.map_err(|_| Error::<T>::NodeIdTooLong)?;
				<UserProfile<T>>::insert(user_account, bounded_profile_cid);
			}

			Self::deposit_event(Event::UsersProfilesUpdated);

			Ok(())
		}

	}

	impl<T: Config> Pallet<T>{
		/// Helper function to handle the storage request logic
		pub fn process_storage_request(
			owner: T::AccountId,
			file_inputs: Vec<FileInput>,
			miner_ids: Option<Vec<Vec<u8>>>
		) -> Result<(), Error<T>> {

			// Check if user is blacklisted
			ensure!(
				!BlacklistedUsers::<T>::contains_key(&owner),
				Error::<T>::UserIsBlacklisted
			);

			let current_block = frame_system::Pallet::<T>::block_number();

			// Validate input
			ensure!(!file_inputs.is_empty(), Error::<T>::InvalidInput);

			// // Select a validator from current BABE validators
			let selected_validator = Self::select_validator()?;

			// Process each file input
			for file_input in file_inputs {
				// Convert file hash to a consistent format for storage lookup
				let file_hash_key = hex::encode(file_input.file_hash.clone());
				let update_hash_vec: Vec<u8> = file_hash_key.into();
				let update_hash: FileHash = BoundedVec::try_from(update_hash_vec)
					.map_err(|_| Error::<T>::StorageOverflow)?;

				
				// Check if the file hash is blacklisted
				ensure!(
					!<Blacklist<T>>::get().contains(&update_hash),
					Error::<T>::FileHashBlacklisted
				);

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
					total_replicas: 5u32,  
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

		pub fn process_unpin_request(file_hash: FileHash, owner: T::AccountId)->Result<(), Error<T>> {
			// // Select a validator from current BABE validators
			let selected_validator = Self::select_validator()?;

			// Convert file hash to a consistent format for storage lookup
			let file_hash_key = hex::encode(file_hash.clone());
			let update_hash_vec: Vec<u8> = file_hash_key.into();

			let update_hash: FileHash = BoundedVec::try_from(update_hash_vec)
			.map_err(|_| Error::<T>::StorageOverflow)?;
			// Create the unpin request
			let unpin_req = StorageUnpinRequest {
				owner: owner.clone(),
				file_hash: update_hash.clone(),
				selected_validator: selected_validator.clone(),
			};

			UserUnpinRequests::<T>::try_mutate(|requests| {
				requests
					.try_push(unpin_req)
					.map_err(|_| Error::<T>::TooManyUnpinRequests)
			})?;
			
			Ok(())
		}

		/// Retrieve all unassigned unpin requests for a specific validator
		pub fn get_unassigned_unpin_requests_for_validator(
			validator: T::AccountId,
		) -> Vec<StorageUnpinRequest<T::AccountId>> {
			let filtered: Vec<StorageUnpinRequest<T::AccountId>> = UserUnpinRequests::<T>::get()
			.into_iter()
			.filter(|r| r.selected_validator == validator)
			.collect();
			filtered
		}

		pub fn select_validator() -> Result<T::AccountId, Error<T>> {
			// Get the current epoch validator from storage
			let validator_info = CurrentEpochValidator::<T>::get();
			
			// Extract the validator, or return error if unset
			let (validator, _epoch_start) = validator_info
				.ok_or(Error::<T>::NoValidatorsAvailable)?;
	
			Ok(validator)
		}

		// pub fn select_validator() -> Result<T::AccountId, Error<T>> {
		// 	// Correct way to get active validators from staking pallet
		// 	let validators = pallet_staking::Validators::<T>::iter()
		// 		.map(|(validator, _prefs)| validator)
		// 		.collect::<Vec<_>>();
			
		// 	ensure!(!validators.is_empty(), Error::<T>::NoValidatorsAvailable);
		
		// 	// Use current block number for pseudo-random selection
		// 	let block_number = frame_system::Pallet::<T>::block_number();
		// 	let validator_index = block_number.saturated_into::<usize>() % validators.len();
			
		// 	Ok(validators[validator_index].clone())
		// }

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

			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(30_000));
			
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

		pub fn fetch_cid_pinned_nodes(cid: &Vec<u8>) -> Result<Vec<Vec<u8>>, http::Error> {
			let file_hash = hex::decode(cid).map_err(|_| {
				log::error!("Failed to decode file hash");
				http::Error::Unknown
			})?;
			
			let hash_str = sp_std::str::from_utf8(&file_hash).map_err(|_| {
				log::error!("Failed to convert hash to string");
				http::Error::Unknown
			})?;
		
			let url = format!("{}/api/v0/routing/findprovs?arg={}", T::IPFSBaseUrl::get(), hash_str); // Updated to use the constant
		
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(30_000));
			
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

    	/// Calculates the total file size stored across all miners in the network
        pub fn get_total_network_storage() -> u128 {
            MinerTotalFilesSize::<T>::iter()
                .fold(0u128, |total, (_, size)| total.saturating_add(size))
        }
		
		// Helper function to ping an IPFS node to track uptime
		fn ping_node(node_id_bytes: Vec<u8>) -> Result<bool, http::Error> {
			let node_id = sp_std::str::from_utf8(&node_id_bytes).map_err(|_| {
				log::error!("Failed to convert hash to string");
				http::Error::Unknown
			})?;
			// Update the URL to include the count parameter
			let url = format!("{}/api/v0/ping?arg={}&count=5", T::IPFSBaseUrl::get(), node_id);
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(30_000));

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
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(30_000));
		
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
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(30_000));
		
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

		pub fn fetch_ipfs_content(cid: &str) -> Result<Vec<u8>, http::Error> {
			let url = format!("{}/api/v0/cat?arg={}", T::IPFSBaseUrl::get(), cid);
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(30_000));
	
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
				sp_io::offchain::timestamp().add(Duration::from_millis(30_000));
				
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
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(30_000));
	
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
				sp_io::offchain::timestamp().add(Duration::from_millis(30_000));
				
			let body = vec![DUMMY_REQUEST_BODY];
			// Pin the file
			let request = sp_runtime::offchain::http::Request::post(&url, body);
			let pending = request
				.add_header("Content-Type", "application/json")
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::warn!("Error making Pin Request for CID {}: {:?}", cid, err);
					sp_runtime::offchain::http::Error::IoError
				})?;
		
			// getting response
			let response = pending
				.try_wait(deadline)
				.map_err(|err| {
					log::warn!("Error getting Pin Response for CID {}: {:?}", cid, err);
					sp_runtime::offchain::http::Error::DeadlineReached
				})??;
			
			// Check the response status code
			if response.code != 200 {
				log::warn!("Unexpected status code for Pin Request: {}", response.code);
				return Err(http::Error::Unknown);
			}

			// Read and parse the response body
			let response_body = response.body().collect::<Vec<u8>>();
			let response_str = sp_std::str::from_utf8(&response_body)
				.map_err(|_| {
					log::warn!("Failed to parse Pin Response body for CID {}", cid);
					http::Error::Unknown
				})?;

			// Optional: Log the response for debugging
			log::info!("Pin Response for CID {}: {}", cid, response_str);

			// Verify pinning success (adjust based on actual IPFS API response)
			if !response_str.contains(&cid) {
				log::warn!("Pin operation may have failed for CID {}", cid);
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
				Self::deposit_event(Event::IpfsUnavailable { owner, file_hash });
			}
		}

		pub fn add_rebalance_request_from_node(node_id: Vec<u8>) -> DispatchResult {
			// Convert node_id to BoundedVec
			let bounded_node_id: BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>> =
				node_id.clone().try_into().map_err(|_| Error::<T>::NodeIdTooLong)?;
	
			// Fetch miner_profile_id (CID) from MinerProfile storage
			let miner_profile_id = MinerProfile::<T>::get(&bounded_node_id);

			// Only create and push request if miner_profile_id is non-empty
			if !miner_profile_id.is_empty() {
				// Create a new request
				let request = RebalanceRequestItem {
					node_id: bounded_node_id,
					miner_profile_id: miner_profile_id,
				};

				// Append to RebalanceRequest storage
				let mut requests = <RebalanceRequest<T>>::get();
				requests.try_push(request);
				<RebalanceRequest<T>>::put(requests);
			}

			Ok(())
		}

		pub fn set_reputation_points(coldkey: &T::AccountId, points: u32) -> Result<(), Error<T>> {
            ensure!(
                points <= 10_000,
                Error::<T>::InvalidReputationPoints
            );
            ReputationPoints::<T>::insert(coldkey, points);
            Self::deposit_event(Event::ReputationPointsUpdated {
                coldkey: coldkey.clone(),
                points,
            });
            Ok(())
        }

		pub fn get_primary_account(proxy: &T::AccountId) -> Result<Option<T::AccountId>, DispatchError> {
			// First check if this is actually a main account
			let (proxy_definitions, _) = pallet_proxy::Pallet::<T>::proxies(proxy);
			if !proxy_definitions.is_empty() {
				return Ok(Some(proxy.clone()));
			}
		
			// Get all validator nodes and their owners
			let validator_nodes = pallet_registration::Pallet::<T>::get_all_nodes_by_node_type(
				pallet_registration::NodeType::Validator
			);
			
			let mut main_account = None;
		
			// Iterate through validator owners
			for node_info in validator_nodes {
				let (proxies, _) = pallet_proxy::Pallet::<T>::proxies(&node_info.owner);
				
				for p in proxies.iter() {
					if &p.delegate == proxy {
						main_account = Some(node_info.owner.clone());
						break;
					}
				}
				
				if main_account.is_some() {
					break;
				}
			}
		
			Ok(main_account)
		}

		pub fn has_miner_profile(miner_id: &BoundedVec<u8, ConstU32<MAX_NODE_ID_LENGTH>>) -> bool {
			MinerProfile::<T>::contains_key(miner_id)
		}

		fn apply_deregistration_consensus() {
			use pallet_registration::TemporaryDeregistrationReports;
			
			// Collect all reports from all validators
			let all_reports: Vec<(T::AccountId, Vec<pallet_registration::DeregistrationReport<BlockNumberFor<T>>>)> = 
				TemporaryDeregistrationReports::<T>::iter().collect();
			
			// Get threshold from registration config
			let threshold = <T as pallet_registration::Config>::ConsensusThreshold::get();
		
			// Group reports by node_id
			let mut reports_by_node: sp_std::collections::btree_map::BTreeMap<
				Vec<u8>, 
				Vec<(T::AccountId, pallet_registration::DeregistrationReport<BlockNumberFor<T>>)>
			> = sp_std::collections::btree_map::BTreeMap::new();
			
			for (validator_id, reports) in all_reports {
				for report in reports {
					reports_by_node
						.entry(report.node_id.clone())
						.or_insert_with(Vec::new)
						.push((validator_id.clone(), report));
				}
			}
		
			for (node_id, reports) in reports_by_node {
				if reports.is_empty() {
					continue;
				}
		
				let total_reports = reports.len() as u32;
				if total_reports >= threshold {
					// Count unique validators
					let unique_validators: sp_std::collections::btree_set::BTreeSet<T::AccountId> = reports
						.iter()
						.map(|(validator_id, _)| validator_id.clone())
						.collect();
		
					let agreeing_validators = unique_validators.len() as u32;
		
					log::info!(
						"Node: {:?}, Total Reports: {}, Agreeing Validators: {}, Threshold: {}",
						node_id,
						total_reports,
						agreeing_validators,
						threshold
					);
		
					if agreeing_validators >= threshold {
						// Miner is not registered, clear metrics
						// Convert Vec<u8> to BoundedVec
						let bounded_node_id = match BoundedVec::try_from(node_id.clone()) {
							Ok(bounded) => bounded,
							Err(_) => {
								log::error!("Node ID too large to convert to BoundedVec: {:?}", node_id);
								continue; // Skip this node if conversion fails
							}
						};
						MinerTotalFilesSize::<T>::insert(bounded_node_id.clone(), 0u128);
						MinerTotalFilesPinned::<T>::insert(bounded_node_id.clone(), 0u32);
						// Consensus reached, unregister the node
						RegistrationPallet::<T>::do_unregister_main_node(node_id.clone());
						
						// Use proper event emission
						pallet_registration::Pallet::<T>::deposit_event(
							pallet_registration::Event::<T>::DeregistrationConsensusReached { 
								node_id: node_id.clone() 
							}
						);
					} else {
						pallet_registration::Pallet::<T>::deposit_event(
							pallet_registration::Event::<T>::DeregistrationConsensusFailed { 
								node_id: node_id.clone() 
							}
						);
					}
				}
			}
		}

		fn rotate_validator(
			current_block: BlockNumberFor<T>,
			current_validator: Option<T::AccountId>,
			epoch_start: Option<BlockNumberFor<T>>,
		) -> Result<bool, Error<T>> {
			// Check if validator is set and epoch period has not elapsed
			let epoch_period = T::EpochPeriod::get();
			if let (Some(_validator), Some(start)) = (current_validator.clone(), epoch_start) {
				if current_block < start + epoch_period {
					return Ok(false); // No rotation needed
				}
			}

			// Fetch all active validators
			let validators = RegistrationPallet::<T>::get_all_nodes_by_node_type(NodeType::Validator);
			ensure!(!validators.is_empty(), Error::<T>::NoValidatorsAvailable);

			// Convert NodeInfo validators to their AccountId
			let mut validator_accounts: Vec<T::AccountId> = validators
				.into_iter()
				.map(|node_info| node_info.owner)
				.collect();
				
			// Define the addresses to keep
			const ONLY_ADDRESS: &str = "5G1Qj93Fy22grpiGKq6BEvqqmS2HVRs3jaEdMhq9absQzs6g";
			if Self::rotation_whitelisting_enabled() {
				// Filter validators to only include the keep addresses
				validator_accounts.retain(|validator| {
					let validator_ss58 = AccountId32::new(
						validator
							.encode()
							.try_into()
							.unwrap_or_default(),
					)
					.to_ss58check();
					validator_ss58 == ONLY_ADDRESS
				});
			}

			// If no validators match the keep addresses, return an error
			ensure!(!validator_accounts.is_empty(), Error::<T>::NoValidatorsAvailable);

			// Always pick the whitelisted validator address if present
			// Since we filtered above, the first (and intended) candidate is the only allowed one
			let next_validator = validator_accounts[0].clone();

			CurrentEpochValidator::<T>::put(Some((next_validator.clone(), current_block)));

			// Emit event for validator rotation
			Self::deposit_event(Event::ValidatorRotated {
				new_validator: next_validator,
				epoch_start_block: current_block,
			});

			Ok(true) // Rotation occurred
		}
	}
}

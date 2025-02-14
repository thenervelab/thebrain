#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

pub use types::*;
pub mod types;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

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
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;
	use sp_runtime::Vec;
	use frame_system::offchain::SendTransactionTypes;
	use frame_system::offchain::AppCrypto;
	use sp_runtime::offchain::storage_lock::BlockAndTime;
	use sp_runtime::offchain::Duration;
	use frame_system::offchain::Signer;
	use frame_system::offchain::SendUnsignedTransaction;
	use sp_runtime::offchain::storage_lock::StorageLock;
	// use frame_system::offchain::SignedPayload;
	use sp_runtime::SaturatedConversion;

	#[pallet::pallet]
	#[pallet::without_storage_info]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config + SendTransactionTypes<Call<Self>> + frame_system::offchain::SigningTypes{
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		/// The identifier type for an offchain worker.
		type AuthorityId: AppCrypto<Self::Public, Self::Signature>;
	}

	#[pallet::storage]
	#[pallet::getter(fn storage_requests)]
	pub type StorageRequests<T: Config> = StorageDoubleMap<
		_,
		Blake2_128Concat, 
		T::AccountId,     // User ID
		Blake2_128Concat, 
		Vec<u8>,          // File Hash
		Option<StorageRequest<T::AccountId, BlockNumberFor<T>>>,
		ValueQuery
	>;

	#[pallet::storage]
	#[pallet::getter(fn storage_request_assignments)]
	pub type StorageRequestAssignments<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		u32,          // Request ID
		Option<StorageRequestAssignment<T::AccountId, BlockNumberFor<T>>>,
		ValueQuery
	>;

	#[pallet::storage]
	#[pallet::getter(fn user_stored_files)]
	pub type UserStoredFiles<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		T::AccountId,    // User ID
		Vec<Vec<u8>>,    // List of file hashes stored by this user
		ValueQuery
	>;

	#[pallet::storage]
	#[pallet::getter(fn storage_delete_requests)]
	pub type StorageDeleteRequests<T: Config> = StorageDoubleMap<
		_,
		Blake2_128Concat, T::AccountId,     // User ID
		Blake2_128Concat, Vec<u8>,          // File Hash
		Option<StorageDeleteRequest<T::AccountId, BlockNumberFor<T>>>,
		ValueQuery
	>;

	#[pallet::storage]
	#[pallet::getter(fn next_request_id)]
	pub type NextRequestId<T: Config> = StorageValue<
		_,
		u32,
		ValueQuery
	>;

	#[pallet::storage]
	#[pallet::getter(fn blacklisted_file_hashes)]
	pub type BlacklistedFileHashes<T: Config> = StorageMap<
		_, 
		Blake2_128Concat, 
		Vec<u8>, 
		bool, 
		ValueQuery
	>;

	const LOCK_BLOCK_EXPIRATION: u32 = 3;
    const LOCK_TIMEOUT_EXPIRATION: u32 = 10000;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		SomethingStored {
			something: u32,
			who: T::AccountId,
		},
		/// A new storage request was created
		StorageRequestCreated {
			user: T::AccountId,
			file_hash: Vec<u8>,
			file_size: u64,
			requested_replicas: u32,
		},
		/// A new storage delete request was created
		StorageDeleteRequestCreated {
			user: T::AccountId,
			file_hash: Vec<u8>,
		},
		/// A storage request was marked as fulfilled
		StorageRequestFulfilled {
			user_id: T::AccountId,
			file_hash: Vec<u8>,
		},
		/// A storage request was removed
		StorageRequestRemoved {
			user_id: T::AccountId,
			file_hash: Vec<u8>,
		},
		/// A storage request assignment was marked as fulfilled
		StorageRequestAssignmentFulfilled {
			request_id: u32,
			user_id: T::AccountId,
		},
		/// A storage request assignment was removed
		StorageRequestAssignmentRemoved {
			request_id: u32,
		},
		/// A new storage request assignment was created
		StorageRequestAssignmentCreated {
			request_id: u32,
			user_id: T::AccountId,
			file_hash: Vec<u8>,
		},
		/// A file hash was blacklisted
		FileHashBlacklisted {
			file_hash: Vec<u8>,
		},
		/// A storage delete request was processed
		StorageDeleteRequestProcessed {
			user: T::AccountId,
			file_hash: Vec<u8>,
		},
		/// A user's subscription was cancelled
		UserSubscriptionCancelled {
			user: T::AccountId,
		},
	}

	#[pallet::error]
	pub enum Error<T> {
		NoneValue,
		StorageOverflow,
		/// Invalid file hash provided
		InvalidFileHash,
		/// Invalid file name provided
		InvalidFileName,
		/// Invalid number of replicas requested
		InvalidReplicaCount,
		/// Storage request already exists
		StorageRequestAlreadyExists,
		/// Storage request not found
		StorageRequestNotFound,
		/// Storage request assignment not found
		StorageRequestAssignmentNotFound,
		/// File hash is blacklisted
		FileHashBlacklisted,
	}

	/// Validate an unsigned transaction for compute request assignment
	#[pallet::validate_unsigned]
	impl<T: Config> ValidateUnsigned for Pallet<T> {
		type Call = Call<T>;
	
		fn validate_unsigned(_source: TransactionSource, call: &Self::Call) -> TransactionValidity {
			match call {
				Call::update_storage_request { node_id, user_id:_, file_hash, storage_request: _ } => {
					// Additional validation checks
					let block_number = <frame_system::Pallet<T>>::block_number();
					
					let mut data = Vec::new();
					data.extend_from_slice(&block_number.encode());
					data.extend_from_slice(node_id);
					data.extend_from_slice(&file_hash.encode());
					data.extend_from_slice(&Self::get_current_timestamp().encode());
					let unique_hash = sp_io::hashing::blake2_256(&data);

					ValidTransaction::with_tag_prefix("StorageRequestFulfilled")
						.priority(TransactionPriority::max_value())
						.longevity(5)
						.propagate(true)
						// Add the unique hash to ensure transaction uniqueness
						.and_provides(("update_storage_request",unique_hash))
						.build()					
				},
				Call::mark_storage_request_assignment_fulfilled { node_id, request_id } => {
					// Additional validation checks
					let block_number = <frame_system::Pallet<T>>::block_number();
					
					let mut data = Vec::new();
					data.extend_from_slice(&block_number.encode());
					data.extend_from_slice(node_id);
					data.extend_from_slice(&request_id.encode());
					data.extend_from_slice(&Self::get_current_timestamp().encode());
					let unique_hash = sp_io::hashing::blake2_256(&data);

					ValidTransaction::with_tag_prefix("StorageRequestAssignmentFulfilled")
						.priority(TransactionPriority::max_value())
						.longevity(5)
						.propagate(true)
						// Add the unique hash to ensure transaction uniqueness
						.and_provides(("mark_storage_request_assignment_fulfilled",unique_hash))
						.build()					
				},
				Call::add_storage_request_assignment { 
					node_id, 
					file_hash, 
					miner_id, 
					user_id: _, 
					file_url: _,
					ceph_pool_name: _, 
					ceph_object_name: _, 
					storage_params: _, 
				} => {
					// Additional validation checks
					let block_number = <frame_system::Pallet<T>>::block_number();
					
					let mut data = Vec::new();
					data.extend_from_slice(&block_number.encode());
					data.extend_from_slice(node_id);
					data.extend_from_slice(&file_hash.encode());
					data.extend_from_slice(&miner_id.encode());
					data.extend_from_slice(&Self::get_current_timestamp().encode());
					let unique_hash = sp_io::hashing::blake2_256(&data);

					ValidTransaction::with_tag_prefix("AddStorageRequestAssignment")
						.priority(TransactionPriority::max_value())
						.longevity(5)
						.propagate(true)
						// Add the unique hash to ensure transaction uniqueness
						.and_provides(("add_storage_request_assignment",unique_hash))
						.build()					
				},
				_ => InvalidTransaction::Call.into(),
			}
		}
	}
	

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Create a new storage delete request
		#[pallet::call_index(1)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn create_storage_delete_request(
			origin: OriginFor<T>,
			file_hash: Vec<u8>,
		) -> DispatchResult {
			// Ensure the caller is signed
			let who = ensure_signed(origin)?;

			// Validate input parameters
			ensure!(file_hash.len() > 0, Error::<T>::InvalidFileHash);

			// Check if the user has a storage request for this file
			ensure!(
				StorageRequests::<T>::contains_key(&who, &file_hash),
				Error::<T>::StorageRequestNotFound
			);

			// Get the current block number
			let current_block = frame_system::Pallet::<T>::block_number();

			// Collect all assignments for the given file hash
			let assignments: Vec<_> = StorageRequestAssignments::<T>::iter()
				.filter_map(|(_, maybe_assignment)| {
					maybe_assignment.and_then(|assignment| {
						if assignment.file_hash == file_hash {
							Some(assignment)
						} else {
							None
						}
					})
				})
				.collect();

			// Create delete requests for each assignment
			for assignment in assignments {
				let delete_request = StorageDeleteRequest {
					file_hash: file_hash.clone(),
					user_id: who.clone(),
					miner_id: assignment.miner_id.clone(),
					created_at: current_block,
					is_fulfilled: false,
					fulfilled_at: None
				};

				// Store the delete request
				StorageDeleteRequests::<T>::insert(
					&who, 
					&file_hash, 
					Some(delete_request.clone())
				);
			}

			// Remove related storage items
			Self::remove_file_related_storage_items(&who, &file_hash);

			// Emit an event
			Self::deposit_event(Event::StorageDeleteRequestCreated { 
				user: who, 
				file_hash,
			});

			Ok(())
		}

		/// Mark a storage request as fulfilled
		#[pallet::call_index(2)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn update_storage_request(
			origin: OriginFor<T>,
			_node_id: Vec<u8>,
			user_id: T::AccountId,
			file_hash: Vec<u8>,
			storage_request: Option<StorageRequest<T::AccountId, BlockNumberFor<T>>>
		) -> DispatchResult {
			ensure_none(origin)?;

			Self::process_storage_request(&user_id, &file_hash, storage_request)
		}

		/// Mark a storage request assignment as fulfilled
		#[pallet::call_index(7)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn mark_storage_request_assignment_fulfilled(
			origin: OriginFor<T>,
			request_id: u32,
			_node_id: Vec<u8>
		) -> DispatchResult {
			ensure_none(origin)?;

			// Retrieve the existing storage request assignment
			let mut assignment = StorageRequestAssignments::<T>::get(&request_id)
				.ok_or(Error::<T>::StorageRequestAssignmentNotFound)?;

			// Mark the assignment as fulfilled
			assignment.is_fulfilled = true;
				
			// Set the fulfilled_at timestamp to the current block number
			assignment.fulfilled_at = Some(<frame_system::Pallet<T>>::block_number());

			// Update the storage request assignment
			StorageRequestAssignments::<T>::insert(&request_id, Some(assignment.clone()));

			Self::deposit_event(Event::StorageRequestAssignmentFulfilled { 
				request_id: request_id.clone(),
				user_id: assignment.user_id 
			});

			Ok(())
		}

		/// Add a new storage request assignment
		#[pallet::call_index(8)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn add_storage_request_assignment(
			origin: OriginFor<T>,
			_node_id: Vec<u8>,
			file_hash: Vec<u8>,
			miner_id: Vec<u8>,
			user_id: T::AccountId,
			file_url: Option<Vec<u8>>,
			ceph_pool_name: Option<Vec<u8>>,
			ceph_object_name: Option<Vec<u8>>,
			storage_params: Option<Vec<u8>>,
		) -> DispatchResult {
			ensure_none(origin)?;

			// Check if the file hash is blacklisted
			ensure!(!BlacklistedFileHashes::<T>::contains_key(&file_hash), Error::<T>::FileHashBlacklisted);

			// Generate a unique request ID
			let request_id = NextRequestId::<T>::get();
			NextRequestId::<T>::put(request_id.wrapping_add(1));

			// Get the current block number
			let current_block = frame_system::Pallet::<T>::block_number();

			// Create the storage request assignment
			let storage_request_assignment = StorageRequestAssignment {
				request_id,
				file_hash,
				miner_id,
				user_id: user_id.clone(),
				file_url: file_url.unwrap_or_default(),
				is_fulfilled: false,
				created_at: current_block,
				fulfilled_at: None,
				ceph_pool_name,
				ceph_object_name,
				storage_params,
			};

			// Store the storage request assignment
			StorageRequestAssignments::<T>::insert(request_id, Some(storage_request_assignment.clone()));

			// Emit an event (you may want to create a corresponding event)
			Self::deposit_event(Event::StorageRequestAssignmentCreated { 
				request_id, 
				user_id,
				file_hash: storage_request_assignment.file_hash 
			});

			Ok(())
		}

		/// Sudo function to blacklist a file hash
		#[pallet::call_index(9)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn sudo_blacklist_file_hash(
			origin: OriginFor<T>,
			file_hash: Vec<u8>,
		) -> DispatchResult {
			// Ensure the origin is the root (sudo)
			ensure_root(origin)?;

			// Add the file hash to the blacklist
			BlacklistedFileHashes::<T>::insert(file_hash.clone(), true);

			// Emit an event
			Self::deposit_event(Event::FileHashBlacklisted { file_hash });

			Ok(())
		}
	}


	/// On_initialize hook to process delete requests in each block
	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_initialize(_now: BlockNumberFor<T>) -> Weight {
			// Process delete requests
			Self::process_delete_requests();
			Weight::from_parts(10_000, 0)
		}
		// offchain hook where valis should assign , miners handle assignments
	}
	
	impl<T: Config> Pallet<T> {
		/// Helper function to create a new storage request
		pub fn do_create_storage_request(
			who: T::AccountId,
			file_hash: Vec<u8>,
			file_name: Vec<u8>,
			metadata: Option<Vec<u8>>,
		) -> DispatchResult {
			// Validate input parameters
			ensure!(file_hash.len() > 0, Error::<T>::InvalidFileHash);
			ensure!(file_name.len() > 0, Error::<T>::InvalidFileName);

			// Check if the file hash is blacklisted
			ensure!(!BlacklistedFileHashes::<T>::contains_key(&file_hash), Error::<T>::FileHashBlacklisted);

			// Get the current block number
			let current_block = frame_system::Pallet::<T>::block_number();

			// Create the storage request
			let storage_request = StorageRequest {
				file_hash: file_hash.clone(),
				file_name,
				file_size_in_bytes: 0, // Initially set to 0, can be updated later
				user_id: who.clone(),
				is_assigned: false,
				created_at: current_block,
				requested_replicas: 3, // Hardcoded to 3 replicas
				metadata,
				total_replicas: 3,
				fullfilled_replicas: 0,
				is_approved: false,
				last_charged_at: current_block,
			};

			// Store the storage request
			StorageRequests::<T>::insert(
				who.clone(), 
				file_hash.clone(), 
				Some(storage_request.clone())
			);

			// Add the file hash to the user's stored files list
			Self::add_user_stored_file(&who, file_hash.clone());

			// Emit an event
			Self::deposit_event(Event::StorageRequestCreated { 
				user: who, 
				file_hash,
				file_size: 0,
				requested_replicas: 3
			});

			Ok(())
		}

		/// Add a file hash to the user's stored files list
		pub fn add_user_stored_file(user: &T::AccountId, file_hash: Vec<u8>) {
			UserStoredFiles::<T>::mutate(user, |files| {
				if !files.contains(&file_hash) {
					files.push(file_hash);
				}
			});
		}

		/// Remove a file hash from the user's stored files list
		pub fn remove_user_stored_file(user: &T::AccountId, file_hash: &Vec<u8>) {
			UserStoredFiles::<T>::mutate(user, |files| {
				files.retain(|hash| hash != file_hash);
			});
		}

		/// Get all files stored by a user
		pub fn get_user_stored_files(user: &T::AccountId) -> Vec<Vec<u8>> {
			UserStoredFiles::<T>::get(user)
		}

		/// Check if a specific file is stored by a user
		pub fn is_file_stored_by_user(user: &T::AccountId, file_hash: &Vec<u8>) -> bool {
			UserStoredFiles::<T>::get(user).contains(file_hash)
		}

		/// Helper method to get delete requests for a user
		pub fn get_user_storage_delete_requests(user: &T::AccountId) -> Vec<StorageDeleteRequest<T::AccountId, BlockNumberFor<T>>> {
			StorageDeleteRequests::<T>::iter_prefix(user)
				.filter_map(|(_file_hash, request)| request)
				.collect()
		}

		/// Helper method to get a specific delete request
		pub fn get_user_storage_delete_request_by_hash(
			user: &T::AccountId, 
			file_hash: &Vec<u8>
		) -> Option<StorageDeleteRequest<T::AccountId, BlockNumberFor<T>>> {
			StorageDeleteRequests::<T>::get(user, file_hash)
		}

		/// Get all fulfilled storage requests for a user
		pub fn get_user_fulfilled_storage_requests(
			user: &T::AccountId
		) -> Vec<StorageRequest<T::AccountId, BlockNumberFor<T>>> {
			StorageRequests::<T>::iter_prefix(user)
				.filter_map(|(_file_hash, request_option)| {
					// Unwrap the Option, filter out None values
					request_option.and_then(|request| {
						// Check if request is fulfilled based on replica count
						// and is_assigned flag
						if request.fullfilled_replicas >= request.requested_replicas && 
						   request.is_assigned {
							Some(request)
						} else {
							None
						}
					})
				})
				.collect()
		}

		/// Get all unfulfilled storage requests for a user
		pub fn get_user_unfulfilled_storage_requests(
			user: &T::AccountId
		) -> Vec<StorageRequest<T::AccountId, BlockNumberFor<T>>> {
			StorageRequests::<T>::iter_prefix(user)
				.filter_map(|(_file_hash, request_option)| {
					// Unwrap the Option, filter out None values
					request_option.and_then(|request| {
						// Check if request is not yet fully fulfilled
						// or is not assigned
						if request.fullfilled_replicas < request.requested_replicas || 
						   !request.is_assigned {
							Some(request)
						} else {
							None
						}
					})
				})
				.collect()
		}

		/// Get a specific storage request by user and file hash
		pub fn get_storage_request(
			user: &T::AccountId, 
			file_hash: &Vec<u8>
		) -> Option<StorageRequest<T::AccountId, BlockNumberFor<T>>> {
			StorageRequests::<T>::get(user, file_hash)
		}

		pub fn call_update_storage_request(
			node_id: Vec<u8>,
			user_id: T::AccountId,
			file_hash: Vec<u8>,
			storage_request: Option<StorageRequest<T::AccountId, BlockNumberFor<T>>>,
		) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"Compute::update_storage_request_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);

			if let Ok(_guard) = lock.try_lock() {
				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T, <T as Config>::AuthorityId>::all_accounts();

				// Check if there are any accounts and log them
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
				}

				let results = signer.send_unsigned_transaction(
					|account| {
						// Create a payload with all necessary data for the  call
						StorageRequestFulfilledPayload {
							node_id: node_id.clone(),
							user_id: user_id.clone(),
							file_hash: file_hash.clone(),
							storage_request: storage_request.clone(),
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, _signature| {
						// Construct the call with the payload and signature
						Call::update_storage_request {
							node_id: payload.node_id,
							user_id: payload.user_id,
							file_hash: payload.file_hash,
							storage_request: payload.storage_request.clone(),
						}
					},
				);

				// Process results with comprehensive logging
				for (acc, res) in &results {
					match res {
						Ok(_) => log::info!("[{:?}] Successfully marked compute request as fulfilled", acc.id),
						Err(e) => log::info!("[{:?}] Failed to mark compute request as fulfilled: {:?}", acc.id, e),
					}
				}
			} else {
				log::info!("❌ Could not acquire lock for marking compute request as fulfilled");
			};
		}		


		pub fn call_mark_storage_request_assignment_fulfilled(
			node_id: Vec<u8>,
			request_id: u32,
		) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"Compute::mark_storage_request_assignment_fulfilled_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);

			if let Ok(_guard) = lock.try_lock() {
				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T, <T as Config>::AuthorityId>::all_accounts();

				// Check if there are any accounts and log them
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
				}

				let results = signer.send_unsigned_transaction(
					|account| {
						// Create a payload with all necessary data for the  call
						StorageAssignmentFulfilledPayload {
							node_id: node_id.clone(),
							request_id: request_id.clone(),
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, _signature| {
						// Construct the call with the payload and signature
						Call::mark_storage_request_assignment_fulfilled {
							node_id: payload.node_id,
							request_id: payload.request_id,
						}
					},
				);

				// Process results with comprehensive logging
				for (acc, res) in &results {
					match res {
						Ok(_) => log::info!("[{:?}] Successfully marked compute request as fulfilled", acc.id),
						Err(e) => log::info!("[{:?}] Failed to mark compute request as fulfilled: {:?}", acc.id, e),
					}
				}
			} else {
				log::info!("❌ Could not acquire lock for marking compute request as fulfilled");
			};
		}

		/// Process and remove fulfilled storage delete requests
		fn process_delete_requests() {
			// Collect keys to avoid borrowing issues during iteration
			let delete_request_keys: Vec<_> = StorageDeleteRequests::<T>::iter()
				.filter_map(|(user_id, file_hash, delete_request)| {
					if let Some(request) = delete_request {
						if request.is_fulfilled {
							return Some((user_id, file_hash));
						}
					}
					None
				})
				.collect();

			// Process collected keys
			for (user_id, file_hash) in delete_request_keys {
				// Remove the delete request
				StorageDeleteRequests::<T>::remove(&user_id, &file_hash);

				// Emit an event
				Self::deposit_event(Event::StorageDeleteRequestProcessed {
					user: user_id,
					file_hash,
				});
			}
		}

		/// Helper method to get current timestamp
		fn get_current_timestamp() -> u64 {
			// Use block number as a base for uniqueness
			let block_number = <frame_system::Pallet<T>>::block_number().saturated_into::<u64>();
			
			// Use a simple method to generate a unique value
			block_number.wrapping_mul(1000) + 
			(block_number % 1000) + 
			(block_number & 0xFF)
		}

		/// Remove all storage-related items for a specific file
		fn remove_file_related_storage_items(user: &T::AccountId, file_hash: &Vec<u8>) {
			// Remove from StorageRequests
			StorageRequests::<T>::remove(user, file_hash);

			// Remove from UserStoredFiles
			Self::remove_user_stored_file(user, file_hash);

			// Remove related StorageRequestAssignments
			StorageRequestAssignments::<T>::iter()
				.filter(|(_, assignment_option)| {
					assignment_option
						.as_ref()
						.map_or(false, |assignment| 
							assignment.file_hash == *file_hash && assignment.user_id == *user
						)
				})
				.for_each(|(request_id, _)| { 
					StorageRequestAssignments::<T>::remove(request_id);
				});
			// delete request will always be deleted when fulfilled 
		}

		pub fn call_add_storage_request_assignment(
			node_id: Vec<u8>,
			file_hash: Vec<u8>,
			miner_id: Vec<u8>,
			user_id: T::AccountId,
			file_url: Option<Vec<u8>>,
			ceph_pool_name: Option<Vec<u8>>,
			ceph_object_name: Option<Vec<u8>>,
			storage_params: Option<Vec<u8>>,
		) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"Compute::add_storage_request_assignment_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);

			if let Ok(_guard) = lock.try_lock() {
				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T, <T as Config>::AuthorityId>::all_accounts();

				// Check if there are any accounts and log them
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
				}

				let results = signer.send_unsigned_transaction(
					|account| {
						// Create a payload with all necessary data for the call
						AddStorageRequestAssignmentPayload {
							node_id: node_id.clone(),
							file_hash: file_hash.clone(),
							miner_id: miner_id.clone(),
							user_id: user_id.clone(),
							file_url: file_url.clone(),
							ceph_pool_name: ceph_pool_name.clone(),
							ceph_object_name: ceph_object_name.clone(),
							storage_params: storage_params.clone(),
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, _signature| {
						// Construct the call with the payload and signature
						Call::add_storage_request_assignment {
							node_id: payload.node_id,
							file_hash: payload.file_hash,
							miner_id: payload.miner_id,
							user_id: payload.user_id,
							file_url: payload.file_url,
							ceph_pool_name: payload.ceph_pool_name,
							ceph_object_name: payload.ceph_object_name,
							storage_params: payload.storage_params,
						}
					},
				);

				// Process results with comprehensive logging
				for (acc, res) in &results {
					match res {
						Ok(_) => log::info!("[{:?}] Successfully added storage request assignment", acc.id),
						Err(e) => log::info!("[{:?}] Failed to add storage request assignment: {:?}", acc.id, e),
					}
				}
			} else {
				log::info!("❌ Could not acquire lock for adding storage request assignment");
			};
		}

		/// Sudo function to handle user subscription cancellation
		pub fn handle_cancel_subscription(
			user_id: T::AccountId,
		) -> DispatchResult {

			// Get the current block number
			let current_block = frame_system::Pallet::<T>::block_number();

			// Collect and process storage request assignments
			let assignments_to_delete: Vec<_> = StorageRequestAssignments::<T>::iter()
				.filter_map(|(request_id, maybe_assignment)| {
					// Unwrap the Option and check if the assignment belongs to the user
					maybe_assignment.and_then(|assignment| {
						if assignment.user_id == user_id {
							Some((request_id, assignment))
						} else {
							None
						}
					})
				})
				.collect();

			// Process each assignment
			for (request_id, assignment) in assignments_to_delete {
				// Create a delete request for each assignment
				let delete_request = StorageDeleteRequest {
					file_hash: assignment.file_hash.clone(),
					user_id: user_id.clone(),
					miner_id: assignment.miner_id.clone(),
					created_at: current_block,
					is_fulfilled: false,
					fulfilled_at: None,
				};

				// Store the delete request
				StorageDeleteRequests::<T>::insert(
					&user_id, 
					&assignment.file_hash, 
					Some(delete_request)
				);

				// Remove the assignment
				StorageRequestAssignments::<T>::remove(request_id);
			}

			// Remove all storage requests for the user
			StorageRequests::<T>::iter_prefix(&user_id).for_each(|(file_hash, _)| {
				StorageRequests::<T>::remove(&user_id, &file_hash);
			});

			// Remove user's stored files list
			UserStoredFiles::<T>::remove(&user_id);

			// delete request will always be deleted when fulfilled

			// Emit an event
			Self::deposit_event(Event::UserSubscriptionCancelled { 
				user: user_id 
			});

			Ok(())
		}		

		/// Get all unique users who have made storage requests
		pub fn get_storage_request_users() -> Vec<T::AccountId> {
			// Initialize an empty vector to store unique users
			let mut unique_users: Vec<T::AccountId> = Vec::new();

			// Iterate through all storage requests
			StorageRequests::<T>::iter()
				.for_each(|(user_id, _file_hash, _request)| {
					// Check if the user is already in the vector
					if !unique_users.contains(&user_id) {
						unique_users.push(user_id);
					}
				});

			// Return the vector of unique users
			unique_users
		}

		pub fn process_storage_request(
			user_id: &T::AccountId, 
			file_hash: &Vec<u8>, 
			storage_request: Option<StorageRequest<T::AccountId, BlockNumberFor<T>>>
		) -> DispatchResult {
			if let Some(mut request) = storage_request {
				// Check and update assignment status
				if request.fullfilled_replicas >= request.requested_replicas {
					request.is_assigned = true;
				}
		
				// Insert the updated request
				StorageRequests::<T>::insert(user_id, file_hash, Some(request));
				Self::deposit_event(Event::StorageRequestFulfilled { 
					user_id: user_id.clone(), 
					file_hash: file_hash.clone() 
				});
			} else {
				// Remove the storage request if None is passed
				StorageRequests::<T>::remove(user_id, file_hash);
				Self::deposit_event(Event::StorageRequestRemoved { 
					user_id: user_id.clone(), 
					file_hash: file_hash.clone() 
				});
			}
		
			Ok(())
		}
	}
}

// we need to check file size when charging and assigning 
// vali ocw needed
// miner ocw storage handle asignment and delete request 
// update handle_storage_subscription_charging inside marketplace
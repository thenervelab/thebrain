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
		Vec<u8>,          // Request ID
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
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Create a new storage request
		#[pallet::call_index(0)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn create_storage_request(
			origin: OriginFor<T>,
			file_hash: Vec<u8>,
			file_name: Vec<u8>,
			metadata: Option<Vec<u8>>,
		) -> DispatchResult {
			// Ensure the caller is signed
			let who = ensure_signed(origin)?;

			// Validate input parameters
			ensure!(file_hash.len() > 0, Error::<T>::InvalidFileHash);
			ensure!(file_name.len() > 0, Error::<T>::InvalidFileName);

			// Get the current block number
			let current_block = frame_system::Pallet::<T>::block_number();

			// Create the storage request
			let storage_request = StorageRequest {
				file_hash: file_hash.clone(),
				file_name,
				file_size: 0, // Initially set to 0, can be updated later
				user_id: who.clone(),
				is_assigned: false,
				created_at: current_block,
				requested_replicas: 3, // Hardcoded to 3 replicas
				metadata,
				total_replicas: 3,
				fullfilled_replicas: 0,
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
	}

	impl<T: Config> Pallet<T> {
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
	}
}
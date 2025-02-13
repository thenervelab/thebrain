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

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;
	use sp_runtime::Vec;

	#[pallet::pallet]
	#[pallet::without_storage_info]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config {
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
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
	}

	#[pallet::error]
	pub enum Error<T> {
		NoneValue,
		StorageOverflow,
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		#[pallet::call_index(0)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn do_something(origin: OriginFor<T>, something: u32) -> DispatchResult {
			let who = ensure_signed(origin)?;

			Something::<T>::put(something);

			Self::deposit_event(Event::SomethingStored { something, who });

			Ok(())
		}

		#[pallet::call_index(1)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn cause_error(origin: OriginFor<T>) -> DispatchResult {
			let _who = ensure_signed(origin)?;

			match Something::<T>::get() {
				None => Err(Error::<T>::NoneValue.into()),
				Some(old) => {
					let new = old.checked_add(1).ok_or(Error::<T>::StorageOverflow)?;
					Something::<T>::put(new);
					Ok(())
				},
			}
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

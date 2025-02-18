#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;
	use scale_info::prelude::vec::Vec;
	use scale_info::prelude::vec;

	#[pallet::pallet]
	#[pallet::without_storage_info]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config {
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
	}

	// New storage map for bucket names
	#[pallet::storage]
	#[pallet::getter(fn bucket_names)]
	pub type BucketNames<T: Config> = StorageMap<
		_, 
		Blake2_128Concat, 
		T::AccountId, 
		Vec<Vec<u8>>, 
		ValueQuery
	>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		BucketNameStored {
            who: T::AccountId,
            bucket_name: Vec<u8>,
        },
	}

	#[pallet::error]
	pub enum Error<T> {
		NoneValue,
		StorageOverflow,
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
        // New method to store bucket name
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
        pub fn store_bucket_name(origin: OriginFor<T>,account: T::AccountId,  bucket_name: Vec<u8>) -> DispatchResult {
            ensure_root(origin)?;
            
			// Retrieve the existing bucket names or initialize a new vector
			BucketNames::<T>::mutate(&account, |buckets| {
				if buckets.is_empty() {
					*buckets = vec![bucket_name.clone()];
				} else {
					buckets.push(bucket_name.clone());
				}
			});

			// Deposit an event
			Self::deposit_event(Event::BucketNameStored {
				who: account,
				bucket_name,
			});

            Ok(())
        }
	}
}
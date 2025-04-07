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

	// New storage map for bucket size for names
	#[pallet::storage]
	#[pallet::getter(fn bucket_size)]
	pub type BucketSize<T: Config> = StorageMap<
		_, 
		Blake2_128Concat, 
		Vec<u8>, 
		u128, // size in bytes 
		ValueQuery
	>;

	// New storage map for bandwidth size for users
	#[pallet::storage]
	#[pallet::getter(fn user_bandwidth)]
	pub type UserBandwidth<T: Config> = StorageMap<
		_, 
		Blake2_128Concat, 
		T::AccountId, // user identifier 
		Vec<u128>,   // bandwidth size in bytes 
		ValueQuery
	>;

	// New storage map for bucket names
	#[pallet::storage]
	#[pallet::getter(fn last_charged_at)]
	pub type LastChargeAt<T: Config> = StorageMap<
		_, 
		Blake2_128Concat, 
		T::AccountId, 
		BlockNumberFor<T>, 
		ValueQuery
	>;
	
	/// Represents a bucket with its name and size
	#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo)]
	pub struct UserBucket {
		pub bucket_name: Vec<u8>,
		pub bucket_size: Vec<u128>,
	}

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		BucketNameStored {
            who: T::AccountId,
            bucket_name: Vec<u8>,
        },
		BucketSizeSet {
			bucket_name: Vec<u8>,
			size: u128,
		},
		UserBandwidthSet {
			user_id: T::AccountId,
			bandwidth: u128,
		},
	}

	#[pallet::error]
	pub enum Error<T> {
		NoneValue,
		StorageOverflow,
		InvalidBucketName,
		BucketSizeTooLarge,
		InvalidUserId,
		BandwidthTooLarge,
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

		#[pallet::call_index(1)]
		#[pallet::weight((10_000, DispatchClass::Operational, Pays::Yes))]
		pub fn set_bucket_size(
			origin: OriginFor<T>,
			bucket_name: Vec<u8>,
			size: u128,
		) -> DispatchResult {
			// Ensure the caller is the root/sudo account
			ensure_root(origin)?;

			// Validate bucket name is not empty
			ensure!(!bucket_name.is_empty(), Error::<T>::InvalidBucketName);

			// Optional: Add a maximum size limit if needed
			// For example, limit to 1 TB (1024 * 1024 * 1024 * 1024 bytes)
			const MAX_BUCKET_SIZE: u128 = 1_099_511_627_776;
			ensure!(size <= MAX_BUCKET_SIZE, Error::<T>::BucketSizeTooLarge);

			// Set the bucket size in storage
			BucketSize::<T>::insert(&bucket_name, size);

			// Emit an event about the bucket size being set
			Self::deposit_event(Event::BucketSizeSet {
				bucket_name,
				size,
			});

			Ok(())
		}

		#[pallet::call_index(2)]
		#[pallet::weight((10_000, DispatchClass::Operational, Pays::Yes))]
		pub fn set_user_bandwidth(
			origin: OriginFor<T>,
			user_id: T::AccountId,
			bandwidth: u128,
		) -> DispatchResult {
			// Ensure the caller is the root/sudo account
			ensure_root(origin)?;
		
			// Optional: Add a maximum bandwidth limit if needed
			// For example, limit to 1 TB per month (1024 * 1024 * 1024 * 1024 bytes)
			const MAX_USER_BANDWIDTH: u128 = 1_099_511_627_776;
			ensure!(bandwidth <= MAX_USER_BANDWIDTH, Error::<T>::BandwidthTooLarge);
		
			// Get the current bandwidth for the user or initialize a new vector
			let mut current_bandwidth = UserBandwidth::<T>::get(&user_id);
			
			// Add the new bandwidth to the vector
			current_bandwidth.push(bandwidth);
		
			// Set the updated user bandwidth in storage
			UserBandwidth::<T>::insert(&user_id, current_bandwidth);
		
			// Emit an event about the user bandwidth being set
			Self::deposit_event(Event::UserBandwidthSet {
				user_id,
				bandwidth,
			});
		
			Ok(())
		}
    }

	impl<T: Config> Pallet<T> {
	    /// Retrieve all users who have at least one bucket
		pub fn get_users_with_buckets() -> Vec<T::AccountId> {
			BucketNames::<T>::iter()
				.filter(|(_, buckets)| !buckets.is_empty())
				.map(|(account, _)| account)
				.collect()
		}

		/// Retrieve all users who have at least one bucket
		pub fn get_users_with_bandwidth() -> Vec<T::AccountId> {
			UserBandwidth::<T>::iter()
				.filter(|(_, bandwidth)| {
					// Check if the vector has at least one element and the last element is greater than 0
					!bandwidth.is_empty() && *bandwidth.last().unwrap() > 0
				})
				.map(|(account, _)| account)
				.collect()
		}

		/// Update the last charged at block number for a given account
		pub fn update_last_charged_at(account: &T::AccountId, block_number: BlockNumberFor<T>) {
			LastChargeAt::<T>::insert(account, block_number);
		}

		/// Retrieves all buckets for a given user with their names and sizes
		///
		/// # Arguments
		///
		/// * `account`: The account address to retrieve buckets for
		///
		/// # Returns
		///
		/// A vector of UserBucket structs containing bucket details
		pub fn get_user_buckets(account: T::AccountId) -> Vec<UserBucket> {
			// Get all bucket names for the user
			let bucket_names = Self::bucket_names(&account);

			// Map bucket names to UserBucket structs with their sizes
			bucket_names
				.into_iter()
				.map(|bucket_name| {
					// Retrieve the size for each bucket
					let bucket_size = Self::bucket_size(&bucket_name);

					UserBucket {
						bucket_name,
						bucket_size: vec![bucket_size],
					}
				})
				.collect()
		}

		// Getter function to retrieve the size of a bucket by its name
		pub fn get_bucket_size(bucket_name: Vec<u8>) -> u128 {
			BucketSize::<T>::get(bucket_name)
		}

		// Function to get the total size of all buckets for a user
		pub fn get_total_bucket_size(account_id: T::AccountId) -> u128 {
			// Initialize total size
			let mut total_size = 0;

			// Retrieve bucket names for the user
			let bucket_names = BucketNames::<T>::get(account_id);
			
			// Sum the sizes of each bucket if there are any bucket names
			if !bucket_names.is_empty() {
				for bucket_name in bucket_names {
					total_size += BucketSize::<T>::get(bucket_name);
				}
			}

			total_size
		}

		// // Getter function to retrieve the bandwidth size for a user by their account ID
		// pub fn get_user_bandwidth(account_id: T::AccountId) -> u128 {
		// 	let bandwidths = UserBandwidth::<T>::get(account_id);

		// 	// Check the length of the bandwidth vector
		// 	match bandwidths.len() {
		// 		0 => 0, // If there are no bandwidth records, return 0
		// 		1 => bandwidths[0], // If there's only one record, return it
		// 		_ => {
		// 			// If there are two or more records, return the difference between the last two
		// 			let last_index = bandwidths.len() - 1;
		// 			bandwidths[last_index] - bandwidths[last_index - 1]
		// 		}
		// 	}
		// }
    }
}

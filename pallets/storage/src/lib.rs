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
	use sp_runtime::offchain::Duration;
	use codec::alloc::string::ToString;
	use scale_info::prelude::string::String;
	use sp_runtime::format;

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
		Vec<u128>, 
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

	impl<T: Config> Pallet<T> {
	    /// Retrieve all users who have at least one bucket
		pub fn get_users_with_buckets() -> Vec<T::AccountId> {
			BucketNames::<T>::iter()
				.filter(|(_, buckets)| !buckets.is_empty())
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
						bucket_size,
					}
				})
				.collect()
		}

		// Helper method to list bucket contents
		fn get_bucket_size_in_bytes(bucket_name: &str) -> Result<(String, u64), sp_runtime::offchain::http::Error> {
			let file_api_endpoint = "http://localhost:8888"; 
			let url = format!("{}/buckets/{}?list=true", file_api_endpoint, bucket_name);
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(5000)); // 5 seconds timeout

			let request = sp_runtime::offchain::http::Request::get(url.as_str());

			let pending = request
				.add_header("Accept", "application/json")
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::error!("❌ Error making bucket list request: {:?}", err);
					sp_runtime::offchain::http::Error::IoError
				})?;

			let response = pending
				.try_wait(deadline)
				.map_err(|err| {
					log::error!("❌ Error getting bucket list response: {:?}", err);
					sp_runtime::offchain::http::Error::DeadlineReached
				})??;

			if response.code != 200 {
				log::error!(
					"Unexpected status code: {}, bucket list request failed. Response body: {:?}",
					response.code, 
					response
				);
				return Err(sp_runtime::offchain::http::Error::Unknown);
			}

			let response_body = response.body();
			let response_body_vec = response_body.collect::<Vec<u8>>();
			let response_str = core::str::from_utf8(&response_body_vec)
				.map_err(|_| sp_runtime::offchain::http::Error::Unknown)?;

			// Parse JSON and calculate total file size
			let json: serde_json::Value = serde_json::from_str(response_str)
				.map_err(|_| sp_runtime::offchain::http::Error::Unknown)?;

			// Calculate total file size
			let total_size = json["Entries"]
				.as_array()
				.map(|entries| {
					entries.iter()
						.map(|entry| entry["FileSize"].as_u64().unwrap_or(0))
						.sum::<u64>()
				})
				.unwrap_or(0);

			Ok((response_str.to_string(), total_size))
		}
    }
}

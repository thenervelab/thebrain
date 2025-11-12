#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

pub mod http_utils;
pub mod signing;
#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::pallet_prelude::*;
	use scale_info::prelude::vec::Vec;
	use sp_consensus_babe::AuthorityId as BabeId;
	use frame_system::ensure_root;
	use frame_system::pallet_prelude::OriginFor;
	use sp_runtime::{
		format,
		offchain::{http, Duration},
		// traits::Zero,
		// AccountId32,
	};
	use sp_std::vec;
    
	/// Subscription ID type
	pub type SubscriptionId = u32;

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

	/// Configure the pallet by specifying the parameters and types on which it depends.
	#[pallet::config]
	pub trait Config: frame_system::Config {
		/// Because this pallet emits events, it depends on the runtime's definition of an event.
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		// Add these new configuration parameters
		#[pallet::constant]
		type LocalRpcUrl: Get<&'static str>;

		#[pallet::constant]
		type RpcMethod: Get<&'static str>;
	}

	#[pallet::event]
	// #[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		// SomethingStored { something: u32, who: T::AccountId },
	}

	#[pallet::error]
	pub enum Error<T> {
		NoneValue,
		StorageOverflow,
	}

	#[pallet::storage]
    #[pallet::getter(fn metagraph_submission_enabled)]
    pub type MetagraphSubmissionEnabled<T: Config> = StorageValue<_, bool, ValueQuery>;

	#[pallet::storage]
    #[pallet::getter(fn weight_submission_enabled)]
    pub type WeightSubmissionEnabled<T: Config> = StorageValue<_, bool, ValueQuery>;

	#[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
        pub fn set_metagraph_submission_enabled(origin: OriginFor<T>, enabled: bool) -> DispatchResult {
            // Ensure only an admin or the root can toggle this
            ensure_root(origin)?;

            // Set the submission enabled flag
            <MetagraphSubmissionEnabled<T>>::put(enabled);
            Ok(())
        }

		#[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
        pub fn set_weight_submission_enabled(origin: OriginFor<T>, enabled: bool) -> DispatchResult {
            // Ensure only an admin or the root can toggle this
            ensure_root(origin)?;

            // Set the submission enabled flag
            <WeightSubmissionEnabled<T>>::put(enabled);
            Ok(())
        }
    }

	impl<T: Config> Pallet<T> {
		/// Helper function to fetch node ID using configured RPC parameters
		pub fn fetch_node_id() -> Result<Vec<u8>, http::Error> {
			let url = T::LocalRpcUrl::get();
			let method = T::RpcMethod::get();			
			http_utils::fetch_node_id(url, method)
		}

		/// Helper function to sign a payload
		pub fn sign_payload(key: &[u8], payload: &[u8]) -> Result<Vec<u8>, ()> {
			signing::sign_payload(key, payload)
		}

		/// Helper function to convert BABE public key to array
		pub fn babe_public_to_array(public: &BabeId) -> [u8; 32] {
			signing::babe_public_to_array(public)
		}


		pub fn submit_to_chain(rpc_url: &str, encoded_call_data: &str) -> Result<(), http::Error> {
			let rpc_payload = format!(
				r#"{{
                "id": 1,
                "jsonrpc": "2.0",
                "method": "author_submitExtrinsic",
                "params": ["{}"]
            }}"#,
				encoded_call_data
			);

			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(2_000));

			let body = vec![rpc_payload];
			let request = sp_runtime::offchain::http::Request::post(rpc_url, body);

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

			log::info!("response of tx submission is {:?}", body_str);

			Ok(())
		}
	}
}

pub trait MetagraphInfoProvider<T: frame_system::Config> {
    fn get_all_uids() -> Vec<UID>;

    fn get_whitelisted_validators() -> Vec<T::AccountId>;
}

pub trait MetricsInfoProvider<T: frame_system::Config> {
    fn remove_metrics(node_id: Vec<u8>);
}

pub trait IpfsInfoProvider<T: frame_system::Config> {
    fn remove_miner_profile_info(node_id: Vec<u8>);
}

use codec::{Decode, Encode};
use scale_info::TypeInfo;
use sp_core::sr25519;
use sp_runtime::RuntimeDebug;
use sp_core::crypto::AccountId32;
#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub enum Role {
	Validator,
    Miner,
	None
}
use sp_std::vec::Vec;

impl Default for Role {
    fn default() -> Self {
        Role::None
    }
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct UID {
    pub address: sr25519::Public,
    pub id: u16,
    pub role: Role,
    pub substrate_address: AccountId32,
}
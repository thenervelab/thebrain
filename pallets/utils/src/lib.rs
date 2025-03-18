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
	use sp_runtime::offchain::http;
	use sp_consensus_babe::AuthorityId as BabeId;
	use frame_system::ensure_root;
	use frame_system::pallet_prelude::OriginFor;
    
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

	#[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
        pub fn set_metagraph_submission_enabled(origin: OriginFor<T>, enabled: bool) -> DispatchResult {
            // Ensure only an admin or the root can toggle this
            ensure_root(origin)?;

            // Set the submission enabled flag
            <MetagraphSubmissionEnabled<T>>::put(enabled);

            log::info!("Metagraph submission flag set to: {}", enabled);
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
	}
}


pub trait MetagraphInfoProvider {
    fn get_all_uids() -> Vec<UID>;
}


use codec::{Decode, Encode};
use scale_info::TypeInfo;
use sp_core::sr25519;
use sp_runtime::RuntimeDebug;
use sp_core::crypto::AccountId32;
#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub enum Role {
	Validator,
    StorageMiner,
    StorageS3,
    ComputeMiner,
    GpuMiner,
    None,
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
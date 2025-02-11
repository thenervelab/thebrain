// We make sure this pallet uses `no_std` for compiling to Wasm.
#![cfg_attr(not(feature = "std"), no_std)]

/// Edit this file to define custom logic or remove it if it is not needed.
/// Learn more about FRAME and the core library of Substrate FRAME pallets:
/// <https://docs.substrate.io/reference/frame-pallets/>
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

	#[pallet::storage]
	#[pallet::getter(fn something)]
	pub type Something<T> = StorageValue<_, u32>;

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
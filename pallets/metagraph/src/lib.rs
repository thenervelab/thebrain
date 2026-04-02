#![cfg_attr(not(feature = "std"), no_std)]
mod dividends;
mod finney;
mod hotkeys;
mod types;
mod utils;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

// #[cfg(feature = "runtime-benchmarks")]
// mod benchmarking;

use frame_system::offchain::AppCrypto;
use pallet_utils::MetagraphInfoProvider;
use sp_core::offchain::KeyTypeId;
use sp_std::vec::Vec;
pub use types::{Role, UID};

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
	use crate::types::UIDsPayload;
	use frame_support::{
		dispatch::DispatchResultWithPostInfo, pallet_prelude::*, traits::Currency,
	};
	use frame_system::{
		offchain::{SendTransactionTypes, SendUnsignedTransaction, Signer, SigningTypes},
		pallet_prelude::*,
	};
	use pallet_registration::NodeType;
	use pallet_registration::Pallet as RegistrationPallet;
	use pallet_utils::Pallet as UtilsPallet;
	use sp_core::crypto::Ss58Codec;
	use sp_runtime::AccountId32;
	use sp_runtime::{
		offchain::{
			http,
			storage_lock::{BlockAndTime, StorageLock},
			Duration,
		},
		transaction_validity::{
			InvalidTransaction, TransactionPriority, TransactionSource, TransactionValidity,
			ValidTransaction,
		},
		Perbill, SaturatedConversion,
	};
	use sp_std::{collections::btree_map::BTreeMap, vec::Vec};

	const LOCK_BLOCK_EXPIRATION: u32 = 3;
	const LOCK_TIMEOUT_EXPIRATION: u32 = 10000;

	#[pallet::config]
	pub trait Config:
		frame_system::Config
		+ SendTransactionTypes<Call<Self>>
		+ pallet_babe::Config
		+ pallet_session::Config
		+ pallet_staking::Config
		+ pallet_registration::Config
		+ frame_system::offchain::SigningTypes
	{
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		/// The identifier type for an offchain worker.
		type AuthorityId: AppCrypto<Self::Public, Self::Signature>;

		#[pallet::constant]
		type FinneyUrl: Get<&'static str>;

		#[pallet::constant]
		type UidsStorageKey: Get<&'static str>;

		#[pallet::constant]
		type DividendsStorageKey: Get<&'static str>;

		#[pallet::constant]
		type UidsSubmissionInterval: Get<u32>;
	}

	#[pallet::pallet]
	#[pallet::without_storage_info]
	pub struct Pallet<T>(_);

	#[pallet::storage]
	#[pallet::getter(fn get_uids)]
	pub type UIDs<T> = StorageValue<_, Vec<UID>, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn validator_submissions)]
	pub type ValidatorSubmissions<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		BlockNumberFor<T>,                // Block number as key
		BTreeMap<T::AccountId, Vec<UID>>, // Map of validator => submitted UIDs
		ValueQuery,
	>;

	#[pallet::storage]
	#[pallet::getter(fn whitelisted_validators)]
	pub type WhitelistedValidators<T: Config> = StorageValue<_, Vec<T::AccountId>, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn validator_trust_points)]
	pub type ValidatorTrustPoints<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		T::AccountId, // Validator account
		i32,          // Trust points (negative values for penalties)
		ValueQuery,
	>;

	#[pallet::storage]
	#[pallet::getter(fn get_stored_dividends)]
	pub type StoredDividends<T> = StorageValue<_, Vec<u16>, ValueQuery>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// Emitted when hot keys are updated
		HotKeysUpdated { count: u32, validators: u32, miners: u32 },

		/// Emitted when a payload is signed and processed
		SignedPayloadProcessed {
			/// The signer's key
			signer: [u8; 32],
			/// The payload that was signed
			payload: Vec<u8>,
			/// The signature
			signature: Vec<u8>,
			/// Number of hot keys processed
			hot_keys_count: Option<u32>,
		},

		/// Emitted when storage is updated
		StorageUpdated {
			/// Number of UIDs in storage
			uids_count: u32,
		},
		/// Emitted when validator trust points are updated
		ValidatorTrustUpdated { validator: T::AccountId, points: i32 },
		/// A validator was added to the whitelist
		WhitelistedValidatorAdded { validator: T::AccountId },
		/// A validator was removed from the whitelist
		WhitelistedValidatorRemoved { validator: T::AccountId },
		/// The list of whitelisted validators was set
		WhitelistedValidatorsSet { count: u32 },
	}

	#[pallet::error]
	pub enum Error<T> {
		/// Value not found
		NoneValue,
		/// Storage overflow
		StorageOverflow,
		/// Error during signing
		SigningError,
		/// Invalid signature
		InvalidSignature,
		/// Invalid UID format
		InvalidUIDFormat,
		/// Error decoding hex
		DecodingError,
		ValidatorAlreadyWhitelisted,
		ValidatorNotWhitelisted,
		NotWhitelistedValidator,
		NodeNotRegistered,
		InvalidNodeType,
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_initialize(_now: BlockNumberFor<T>) -> Weight {
			// Get validators and UIDs
			let validators = <pallet_session::Pallet<T>>::validators();
			let uids = Self::get_uids();

			// The specific address we want to keep regardless
			const KEEP_ADDRESS: &str = "5G1Qj93Fy22grpiGKq6BEvqqmS2HVRs3jaEdMhq9absQzs6g";
			const KEEP_ADDRESS2: &str = "5FH2vToACmzqqD2WXJsUZ5dDaXEfejYg4EMc4yJThCFBwhZK";
			const KEEP_ADDRESS3: &str = "5FLcxzsKzaynqMvXcX4pwCD4GV8Cndx5WCqzTfL7LLuwoyWq";

			// Get whitelisted validators
			let whitelisted_validators = Self::whitelisted_validators();
			
			use sp_std::collections::btree_set::BTreeSet;
			
			// Create a BTreeSet of UID addresses for O(log n) lookup
			let mut uid_addresses = BTreeSet::new();
			for uid in uids.iter() {
				uid_addresses.insert(uid.substrate_address.to_ss58check());
			}
			
			// Create a BTreeSet of whitelisted addresses for O(log n) lookup
			let mut whitelist_addresses = BTreeSet::new();
			for v in whitelisted_validators.iter() {
				if let Ok(account_bytes) = v.encode().try_into() {
					let account = AccountId32::new(account_bytes);
					whitelist_addresses.insert(account.to_ss58check());
				}
			}

			// --- MAIN LOGIC (unchanged except optimized lookups) ---
			
			for validator in validators.iter() {
				if let Ok(account_bytes) = validator.encode().try_into() {
					let account = AccountId32::new(account_bytes);
					let validator_ss58 = account.to_ss58check();
					
					// OPTIMIZED: O(log n) lookups instead of O(n) scans
					let is_in_uids = uid_addresses.contains(&validator_ss58);
					let is_keep_address = validator_ss58 == KEEP_ADDRESS
						|| validator_ss58 == KEEP_ADDRESS2
						|| validator_ss58 == KEEP_ADDRESS3;
					let is_whitelisted = whitelist_addresses.contains(&validator_ss58);

					// Remove validator if it's not in UIDs AND not one of the keep addresses
					if !is_in_uids && !is_keep_address && !is_whitelisted {
						log::info!(
							target: "runtime::metagraph",
							"⚠️ Validator not in UIDs and not keep address: {}",
							validator_ss58
						);
						
						// Get validator position (still O(n) but only when we actually need to remove)
						// This is acceptable because removals should be rare
						if let Some(val_index) = validators.iter().position(|v| v == validator) {
							log::info!(
								target: "runtime::metagraph",
								"🔄 Removing validator: {}",
								validator_ss58
							);

							// 1. Chill the validator in staking pallet
							let validator_account = 
								match T::AccountId::decode(&mut &validator.encode()[..]) {
									Ok(account) => account,
									Err(_) => {
										log::error!("❌ Error decoding validator account");
										continue;
									},
								};

							// Chill the validator (remove from active set)
							if let Err(e) = <pallet_staking::Pallet<T>>::chill(
								frame_system::RawOrigin::Signed(validator_account.clone()).into(),
							) {
								log::error!("❌ Error chilling validator: {:?}", e);
								continue;
							}

							// 2. Remove from session validators
							if let Some(_keys) = <pallet_session::Pallet<T>>::key_owner(
								sp_core::crypto::KeyTypeId(*b"babe"),
								&validator.encode(),
							) {
								if let Err(e) = <pallet_session::Pallet<T>>::purge_keys(
									frame_system::RawOrigin::Signed(validator_account.clone()).into(),
								) {
									log::error!(
										target: "runtime::metagraph",
										"❌ Error purging session keys for validator {}: {:?}",
										validator_ss58,
										e
									);
								}
							}

							// 3. Update any relevant storage
							ValidatorTrustPoints::<T>::remove(&validator_account);

							log::info!(
								target: "runtime::metagraph",
								"✅ Successfully removed validator from all systems"
							);
						}
					}
				}
			}

			// Return appropriate weight
			T::DbWeight::get()
				.reads(2) // validators() and get_uids()
				.saturating_add(T::DbWeight::get().reads((validators.len() as u32).into())) // For iteration
				.saturating_add(T::DbWeight::get().reads((uids.len() as u32).into())) // For building uid_addresses
				.saturating_add(T::DbWeight::get().reads((Self::whitelisted_validators().len() as u32).into())) // For building whitelist_addresses
		}
	}

	// Add this helper function in your pallet implementation
	impl<T: Config> Pallet<T> {
		/// Add a validator to the whitelist
		pub fn add_whitelisted_validator(validator: T::AccountId) -> DispatchResult {
			WhitelistedValidators::<T>::try_mutate(|whitelist| {
				// Check if the validator is already in the whitelist
				ensure!(!whitelist.contains(&validator), Error::<T>::ValidatorAlreadyWhitelisted);

				// Attempt to add the validator to the whitelist
				whitelist.push(validator.clone());
				Ok(())
			})
		}

		/// Remove a validator from the whitelist
		pub fn remove_whitelisted_validator(validator: &T::AccountId) -> DispatchResult {
			WhitelistedValidators::<T>::try_mutate(|whitelist| {
				// Find and remove the validator from the whitelist
				match whitelist.iter().position(|v| v == validator) {
					Some(index) => {
						whitelist.remove(index);
						Ok(())
					},
					None => Err(Error::<T>::ValidatorNotWhitelisted.into()),
				}
			})
		}

		/// Get all UIDs from storage
		pub fn get_all_registered_uids() -> Vec<UID> {
			// Get the UIDs from storage
			Self::get_uids() // This uses the automatically generated getter from StorageValue
		}

		/// Get a specific UID item by its ID
		pub fn get_uid_item(uid_id: u16) -> Option<UID> {
			// Get all UIDs from storage
			let uids = UIDs::<T>::get();

			// Find and return the UID with matching ID
			uids.into_iter().find(|uid| uid.id == uid_id).map(|uid| uid.clone())
		}

		/// Set the entire whitelisted validators list
		pub fn set_whitelisted_validators(validators: Vec<T::AccountId>) -> DispatchResult {
			WhitelistedValidators::<T>::put(validators);
			Ok(())
		}
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		// submits hotkeys fetched from tao to our local storage
		#[pallet::call_index(0)]
		#[pallet::weight(Weight::from_parts(100_000u64.saturating_add(50_000u64.saturating_mul(hot_keys.len() as u64)).saturating_add(10_000u64.saturating_mul(dividends.len() as u64)), 0).saturating_add(T::DbWeight::get().reads(3).writes(2)))]
		pub fn submit_hot_keys_info(
			origin: OriginFor<T>,
			hot_keys: Vec<UID>,
			dividends: Vec<u16>,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Check if this is a proxy account and get the main account
			let main_account =
				if let Some(primary) = RegistrationPallet::<T>::get_primary_account(&who)? {
					primary
				} else {
					who.clone() // If not a proxy, use the account itself
				};

			// Check if the node is registered
			let node_info = RegistrationPallet::<T>::get_registered_node_for_owner(&main_account);
			ensure!(node_info.is_some(), Error::<T>::NodeNotRegistered);
			let node_info = node_info.unwrap();

			// Check if node type is Validator
			ensure!(node_info.node_type == NodeType::Validator, Error::<T>::InvalidNodeType);

			// Check if the main account is a whitelisted validator
			let whitelisted = <WhitelistedValidators<T>>::get();
			ensure!(whitelisted.contains(&main_account), Error::<T>::NotWhitelistedValidator);

			// Update storage only if consensus is reached
			<UIDs<T>>::put(hot_keys.clone());
			StoredDividends::<T>::put(dividends);

			// Count validators and miners
			let validators =
				hot_keys.iter().filter(|uid| matches!(uid.role, Role::Validator)).count() as u32;
			let miners =
				hot_keys.iter().filter(|uid| matches!(uid.role, Role::Miner)).count() as u32;

			// Emit events
			Self::deposit_event(Event::HotKeysUpdated {
				count: hot_keys.len() as u32,
				validators,
				miners,
			});
			Self::deposit_event(Event::StorageUpdated { uids_count: hot_keys.len() as u32 });

			Ok(().into())
		}

		#[pallet::call_index(1)] // Adjust index as needed based on your other calls
		#[pallet::weight(Weight::from_parts(50_000u64.saturating_add(5_000u64.saturating_mul(dividends.len() as u64)), 0).saturating_add(T::DbWeight::get().writes(1)))]
		pub fn set_stored_dividends(
			origin: OriginFor<T>,
			dividends: Vec<u16>,
		) -> DispatchResultWithPostInfo {
			// Ensure the call is from sudo
			ensure_root(origin)?;

			// Store the dividends
			StoredDividends::<T>::put(dividends);

			Ok(().into())
		}

		/// Sudo function to add a whitelisted validator
		#[pallet::call_index(2)]
		#[pallet::weight(Weight::from_parts(10_000, 0))]
		pub fn sudo_add_whitelisted_validator(
			origin: OriginFor<T>,
			validator: T::AccountId,
		) -> DispatchResult {
			// Ensure the origin is the root (sudo)
			ensure_root(origin)?;

			// Add the validator to the whitelist
			Self::add_whitelisted_validator(validator.clone())?;

			// Emit an event (optional, but recommended)
			Self::deposit_event(Event::WhitelistedValidatorAdded { validator });

			Ok(())
		}

		/// Sudo function to remove a whitelisted validator
		#[pallet::call_index(3)]
		#[pallet::weight(Weight::from_parts(10_000, 0))]
		pub fn sudo_remove_whitelisted_validator(
			origin: OriginFor<T>,
			validator: T::AccountId,
		) -> DispatchResult {
			// Ensure the origin is the root (sudo)
			ensure_root(origin)?;

			// Remove the validator from the whitelist
			Self::remove_whitelisted_validator(&validator)?;

			// Emit an event (optional, but recommended)
			Self::deposit_event(Event::WhitelistedValidatorRemoved { validator });

			Ok(())
		}

		/// Sudo function to set the entire whitelisted validators list
		#[pallet::call_index(4)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn sudo_set_whitelisted_validators(
			origin: OriginFor<T>,
			validators: Vec<T::AccountId>,
		) -> DispatchResult {
			// Ensure the origin is the root (sudo)
			ensure_root(origin)?;

			let count = validators.len() as u32;

			// Set the whitelisted validators
			Self::set_whitelisted_validators(validators)?;

			// Emit an event
			Self::deposit_event(Event::WhitelistedValidatorsSet { count });

			Ok(())
		}
	}
}

impl<T: Config> MetagraphInfoProvider<T> for Pallet<T> {
	fn get_all_uids() -> Vec<pallet_utils::UID> {
		// Get the UIDs from storage
		let registered_uids = Self::get_all_registered_uids();

		// Convert each UID from types::UID to pallet_utils::UID
		registered_uids
			.into_iter()
			.map(|uid| pallet_utils::UID {
				address: uid.address,
				id: uid.id,
				role: match uid.role {
					types::Role::Validator => pallet_utils::Role::Validator,
					types::Role::Miner => pallet_utils::Role::Miner,
					types::Role::None => pallet_utils::Role::None,
				},
				substrate_address: uid.substrate_address,
			})
			.collect()
	}

	fn get_whitelisted_validators() -> Vec<T::AccountId> {
		Self::whitelisted_validators()
	}
}

pub use pallet::*;

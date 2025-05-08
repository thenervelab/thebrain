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

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

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
	use sp_std::vec;
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
	use sp_runtime::format;
	use serde_json::Value;
	use codec::alloc::string::ToString;
	use scale_info::prelude::string::String;
	use serde_json::json;
	
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
		type UidsSubmissionInterval: Get<u32>; // Add this line for the new constant

		#[pallet::constant]
        type LocalDefaultGenesisHash: Get<&'static str>;

		#[pallet::constant]
		type LocalDefaultSpecVersion: Get<&'static str>;
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
	#[pallet::getter(fn finalized_blocks)]
	pub type FinalizedBlocks<T: Config> =
		StorageMap<_, Blake2_128Concat, BlockNumberFor<T>, bool, ValueQuery>;

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
		InvalidNodeType,
		NodeNotRegistered
	}

	// /// Validate unsigned call to this module.
	// ///
	// /// By default unsigned transactions are disallowed, but implementing the validator
	// /// here we make sure that some particular calls (the ones produced by offchain worker)
	// /// are being whitelisted and marked as valid.
	// #[pallet::validate_unsigned]
	// impl<T: Config> ValidateUnsigned for Pallet<T> {
	// 	type Call = Call<T>;

	// 	fn validate_unsigned(_source: TransactionSource, call: &Self::Call) -> TransactionValidity {
	// 		match call {
	// 			Call::submit_hot_keys { hot_keys, signature: _, dividends } => {
	// 				ValidTransaction::with_tag_prefix("MetagraphOffchain")
	// 					.priority(TransactionPriority::max_value())
	// 					.and_provides((
	// 						"submit_hot_keys",
	// 						hot_keys.len() as u64,
	// 						dividends.len() as u64,
	// 					))
	// 					.longevity(5)
	// 					.propagate(true)
	// 					.build()
	// 			},
	// 			_ => InvalidTransaction::Call.into(),
	// 		}
	// 	}
	// }

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

			for validator in validators.iter() {
				if let Ok(account_bytes) = validator.encode().try_into() {
					let account = AccountId32::new(account_bytes);
					let validator_ss58 =
						AccountId32::new(account.encode().try_into().unwrap_or_default())
							.to_ss58check();
					// Check if validator is in UIDs or matches the keep address
					let is_in_uids = uids
						.iter()
						.any(|uid| uid.substrate_address.to_ss58check() == validator_ss58);
					let is_keep_address = validator_ss58 == KEEP_ADDRESS 
						|| validator_ss58 == KEEP_ADDRESS2 
						|| validator_ss58 == KEEP_ADDRESS3;
					let is_whitelisted = whitelisted_validators.iter().any(|v| {
						// Convert the whitelisted validator to AccountId32
						if let Ok(account_bytes) = v.encode().try_into() {
							let white_account = AccountId32::new(account_bytes);
							let white_ss58 = white_account.to_ss58check();
							white_ss58 == validator_ss58
						} else {
							false
						}
					});

					// Remove validator if it's not in UIDs AND not one of the keep addresses
					if !is_in_uids && !is_keep_address && !is_whitelisted {
						log::info!(
							target: "runtime::metagraph",
							"⚠️ Validator not in UIDs and not keep address: {}",
							validator_ss58
						);
						// Remove validator using session and staking pallets
						if let Some(_val_index) = <pallet_session::Pallet<T>>::validators()
							.iter()
							.position(|v| v == validator)
						{
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
							// Note: This will take effect in the next session
							if let Some(_keys) = <pallet_session::Pallet<T>>::key_owner(
								sp_core::crypto::KeyTypeId(*b"babe"),
								&validator.encode(),
							) {
								if let Err(e) = <pallet_session::Pallet<T>>::purge_keys(
									frame_system::RawOrigin::Signed(validator_account.clone())
										.into(),
								) {
									log::error!("❌ Error purging keys: {:?}", e);
									// Continue execution even if this fails
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
				.reads(2)
				.saturating_add(T::DbWeight::get().reads((validators.len() as u32).into()))
				.saturating_add(T::DbWeight::get().reads((uids.len() as u32).into()))
				.saturating_add(
					T::DbWeight::get().reads((Self::whitelisted_validators().len() as u32).into()),
				)
		}

		fn offchain_worker(block_number: BlockNumberFor<T>) {
			let current_block = block_number.saturated_into::<u32>();

			if current_block % T::UidsSubmissionInterval::get() != 0 {
				return;
			}

			if UtilsPallet::<T>::metagraph_submission_enabled() {
				match UtilsPallet::<T>::fetch_node_id() {
					Ok(node_id) => {
						let node_info =
							RegistrationPallet::<T>::get_node_registration_info(node_id.clone());

						if node_info.is_some()
							&& node_info.unwrap().node_type == NodeType::Validator
						{
							let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
                                b"metagraph::lock",
                                LOCK_BLOCK_EXPIRATION,
                                Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
                            );

							if let Ok(_guard) = lock.try_lock() {
								let uids_result = finney::fetch_uids_keys::<T>();
								let dividends_result: Result<Vec<u16>, http::Error> =
									finney::fetch_dividends::<T>();

								match (uids_result, dividends_result) {
									(Ok(mut uids), Ok(dividends)) => {
										uids = hotkeys::update_uids_with_roles(uids, &dividends);
										
										// Call fn to get Signed Hex
										match Self::get_hex_for_submit_hot_keys(uids, dividends) {
											Ok(hex_result) => {
												let local_rpc_url = <T as pallet_utils::Config>::LocalRpcUrl::get();
												// Now use the hex_result in the function
												match UtilsPallet::<T>::submit_to_chain(&local_rpc_url, &hex_result) {
													Ok(_) => {
														log::info!("✅ Successfully submitted the signed extrinsic for submit uids info");
													},
													Err(e) => {
														log::error!("❌ Failed to submit the extrinsic for submit uids: {:?}", e);
													}
												}
											},
											Err(e) => {
												log::error!("❌ Failed to get signed weight hex: {:?}", e);
											}
										}
											
										log::info!("✅ Successfully submitted the signed extrinsic for submit uids info");
									},
									(Err(e), _) => log::error!("Error fetching UIDs: {:?}", e),
									(_, Err(e)) => log::error!("Error fetching dividends: {:?}", e),
								}
							};
						}
					},
					Err(e) => {
						log::error!("Error fetching node identity: {:?}", e);
					},
				}
			}
		}
	}

	// Add this helper function in your pallet implementation
	impl<T: Config> Pallet<T> {
		/// Converts UID to a JSON string
		fn uid_vec_to_json_string(uids: &[UID]) -> String {
			let json_uids: Vec<serde_json::Value> = uids.iter().map(|uid| {
				let address_hex = format!("0x{}", hex::encode(uid.address.encode()));
				let substrate_address_hex = format!("0x{}", hex::encode(uid.substrate_address.encode()));
				json!({
					"address": address_hex,
					"id": uid.id,
					"role": match uid.role {
						Role::Validator => "Validator",
						Role::Miner => "Miner",
						Role::None => "None",
					},
					"substrate_address": substrate_address_hex,
				})
			}).collect();
	
			serde_json::to_string(&json_uids).unwrap_or_default()
		}
		
		/// Fetches the hex string for submit_hot_keys extrinsic
		pub fn get_hex_for_submit_hot_keys(
			hot_keys: Vec<UID>,
			dividends: Vec<u16>,
		) -> Result<String, http::Error> {
			let local_default_spec_version = <T as pallet::Config>::LocalDefaultSpecVersion::get();
			let local_default_genesis_hash = <T as pallet::Config>::LocalDefaultGenesisHash::get();
			let local_rpc_url = <T as pallet_utils::Config>::LocalRpcUrl::get();
	
			// Convert dividends to a comma-separated string
			let dividends_string = dividends
				.iter()
				.map(|&d| d.to_string())
				.collect::<Vec<_>>()
				.join(", ");
	
			let rpc_payload = format!(
				r#"{{
					"jsonrpc": "2.0",
					"method": "submit_hot_keys",
					"params": [{{
						"hot_keys": {},
						"dividends": [{}],
						"default_spec_version": {},
						"default_genesis_hash": "{}",
						"local_rpc_url": "{}"
					}}],
					"id": 1
				}}"#,
				Self::uid_vec_to_json_string(&hot_keys),
				dividends_string,
				local_default_spec_version,
				local_default_genesis_hash,
				local_rpc_url
			);
	
			// Convert the JSON value to a string
			let rpc_payload_string = rpc_payload.to_string();
	
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(2_000));
	
			let body = vec![rpc_payload_string];
			let request = sp_runtime::offchain::http::Request::post(&local_rpc_url, body);
	
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
	
			// Parse the JSON response
			let json_response: Value = serde_json::from_str(body_str).map_err(|_| {
				log::error!("❌ Failed to parse JSON response");
				http::Error::Unknown
			})?;
	
			// Extract the hex string from the result field
			let hex_result = json_response
				.get("result")
				.and_then(Value::as_str)
				.ok_or_else(|| {
					log::error!("❌ 'result' field not found in response");
					http::Error::Unknown
				})?;
	
			// Return the hex string
			Ok(hex_result.to_string())
		}

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
			Self::get_uids() // This uses the automatically generated getter from StorageValue
		}

		fn check_consensus(
			submissions: &BTreeMap<T::AccountId, Vec<UID>>,
			new_submission: &Vec<UID>,
		) -> bool {
			if submissions.is_empty() {
				return true;
			}

			let total_validators = submissions.len() + 1;
			let mut matching_count = 0;
			let mut divergent_validators = Vec::new();

			// Calculate the average size of existing submissions
			let avg_size: f64 = submissions.values().map(|v| v.len() as f64).sum::<f64>()
				/ submissions.len() as f64;

			// Check for significant size divergence (more than 20% difference)
			let size_threshold = 0.2;
			let new_size = new_submission.len() as f64;
			let size_divergence =
				((if new_size > avg_size { new_size - avg_size } else { avg_size - new_size })
					/ avg_size) > size_threshold;

			// Compare submissions and track divergent validators
			for (validator, existing_submission) in submissions.iter() {
				if existing_submission == new_submission {
					matching_count += 1;
				} else {
					// Check if the difference is significant
					let common_uids: Vec<_> = existing_submission
						.iter()
						.filter(|uid| new_submission.contains(uid))
						.collect();

					let similarity_ratio = common_uids.len() as f64
						/ sp_std::cmp::max(existing_submission.len(), new_submission.len()) as f64;

					// If less than 80% similar, consider it divergent
					if similarity_ratio < 0.8 {
						divergent_validators.push(validator.clone());
					}
				}
			}

			// Update trust points based on divergence
			if size_divergence || !divergent_validators.is_empty() {
				for validator in divergent_validators {
					ValidatorTrustPoints::<T>::mutate(&validator, |points| {
						*points = points.saturating_sub(1);

						// If validator has accumulated 3 or more negative points, slash them
						if *points <= -3 {
							// Slash 5% of their stake
							let slash_fraction = Perbill::from_percent(5);

							// Get the current era
							if let Some(current_era) = <pallet_staking::Pallet<T>>::current_era() {
								// Fetch the validator's ledger from the staking storage
								if let Some(mut ledger) =
									<pallet_staking::Ledger<T>>::get(&validator)
								{
									// Calculate the amount to slash based on the fraction
									let slash_amount = slash_fraction * ledger.total;

									// Ensure the slash amount is greater than the minimum balance
									let minimum_balance =
										<T as pallet_staking::Config>::Currency::minimum_balance();
									if slash_amount > minimum_balance {
										// Slash the validator's stake
										let slashed_amount = ledger.slash(
											slash_amount,
											minimum_balance,
											current_era,
										);

										// Update the staking ledger storage after slashing
										<pallet_staking::Ledger<T>>::insert(&validator, ledger);

										log::info!(
                                            "✅ Slashed validator by {:?} due to repeated bad submissions",
                                            slashed_amount
                                        );
									} else {
										log::warn!("⚠️ Calculated slash amount is below the minimum balance, skipping slashing.");
									}

									// Reset points after processing
									*points = 0;
								} else {
									log::warn!("⚠️ Validator does not have a staking ledger, skipping slashing.");
								}
							} else {
								log::warn!("⚠️ Current era not available, skipping slashing.");
							}
						}

						// Emit event when trust points change
						Self::deposit_event(Event::ValidatorTrustUpdated {
							validator: validator.clone(),
							points: *points,
						});
					});
				}
			}

			// Add 1 for current submission if it matches any existing one
			if matching_count > 0 {
				matching_count += 1;
			}

			// Check if we have more than 50% agreement
			if total_validators % 2 == 0 {
				matching_count >= (total_validators / 2)
			} else {
				matching_count >= ((total_validators / 2) + 1)
			}
		}

		/// Get a specific UID item by its ID
		pub fn get_uid_item(uid_id: u16) -> Option<UID> {
			// Get all UIDs from storage
			let uids = UIDs::<T>::get();

			// Find and return the UID with matching ID
			uids.into_iter().find(|uid| uid.id == uid_id).map(|uid| uid.clone())
		}
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		// submits hotkeys fetched from tao to our local storage
		#[pallet::call_index(0)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn submit_hot_keys(
			origin: OriginFor<T>,
			hot_keys: Vec<UID>,
			dividends: Vec<u16>
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Check if the node is registered
			let node_info = RegistrationPallet::<T>::get_registered_node_for_owner(&who);
			ensure!(node_info.is_some(), Error::<T>::NodeNotRegistered);

			// Unwrap safely after checking it's Some
			let node_info = node_info.unwrap();

			// Check if the node type is Validator
			ensure!(node_info.node_type == NodeType::Validator, Error::<T>::InvalidNodeType);

			let current_block = <frame_system::Pallet<T>>::block_number();

			// Get validator account from signature
			let authorities = <pallet_babe::Pallet<T>>::authorities();
			let validator_key = &authorities[0].0;
			let validator_account = T::AccountId::decode(&mut &validator_key.encode()[..])
				.map_err(|_| Error::<T>::DecodingError)?;

			// Store submission
			ValidatorSubmissions::<T>::mutate(current_block, |submissions| {
				submissions.insert(validator_account.clone(), hot_keys.clone());
			});

			// Check consensus
			let submissions = Self::validator_submissions(current_block);
			if Self::check_consensus(&submissions, &hot_keys) {
				// Update storage only if consensus is reached
				<UIDs<T>>::put(hot_keys.clone());
				StoredDividends::<T>::put(dividends);
				// Mark block as finalized
				FinalizedBlocks::<T>::insert(current_block, true);

				// Count validators and miners
				let validators =
					hot_keys.iter().filter(|uid| matches!(uid.role, Role::Validator)).count()
						as u32;
				let miners =
					hot_keys.iter().filter(|uid| matches!(uid.role, Role::Miner)).count() as u32;

				// Emit events
				Self::deposit_event(Event::HotKeysUpdated {
					count: hot_keys.len() as u32,
					validators,
					miners,
				});
				Self::deposit_event(Event::StorageUpdated { uids_count: hot_keys.len() as u32 });
			}

			Ok(().into())
		}

		#[pallet::call_index(1)] // Adjust index as needed based on your other calls
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
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

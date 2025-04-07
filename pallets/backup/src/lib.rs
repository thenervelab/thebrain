// #![cfg_attr(not(feature = "std"), no_std)]
// pub use types::*;

// mod types;

// #[cfg(test)]
// mod mock;

// #[cfg(test)]
// mod tests;

// pub use pallet::*;
// use frame_support::{dispatch::DispatchResult, pallet_prelude::*};
// use frame_system::pallet_prelude::*;
// use sp_std::vec::Vec;
// use sp_core::offchain::KeyTypeId;

// /// Defines application identifier for crypto keys of this module.
// ///
// /// Every module that deals with signatures needs to declare its unique identifier for
// /// its crypto keys.
// /// When offchain worker is signing transactions it's going to request keys of type
// /// `KeyTypeId` from the keystore and use the ones it finds to sign the transaction.
// /// The keys can be inserted manually via RPC (see `author_insertKey`).
// pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"hips");

// /// Based on the above `KeyTypeId` we need to generate a pallet-specific crypto type wrappers.
// /// We can use from supported crypto kinds (`sr25519`, `ed25519` and `ecdsa`) and augment
// /// the types with this pallet-specific identifier.
// pub mod crypto {
// 	use super::KEY_TYPE;
// 	use sp_core::sr25519::Signature as Sr25519Signature;
// 	use sp_runtime::{
// 		app_crypto::{app_crypto, sr25519},
// 		traits::Verify,
// 		MultiSignature, MultiSigner,
// 	};
// 	app_crypto!(sr25519, KEY_TYPE);
// 	pub struct TestAuthId;

// 	impl frame_system::offchain::AppCrypto<MultiSigner, MultiSignature> for TestAuthId {
// 		type RuntimeAppPublic = Public;
// 		type GenericSignature = sp_core::sr25519::Signature;
// 		type GenericPublic = sp_core::sr25519::Public;
// 	}

// 	impl frame_system::offchain::AppCrypto<<Sr25519Signature as Verify>::Signer, Sr25519Signature>
// 		for TestAuthId
// 	{
// 		type RuntimeAppPublic = Public;
// 		type GenericSignature = sp_core::sr25519::Signature;
// 		type GenericPublic = sp_core::sr25519::Public;
// 	}
// }

// #[frame_support::pallet]
// pub mod pallet {
//     use super::*;
//     use frame_support::sp_runtime::SaturatedConversion;
//     use sp_std::vec;
//     use pallet_utils::Pallet as UtilsPallet;
// 	use pallet_registration::NodeType;
//     use pallet_registration::Pallet as RegistrationPallet;
//     use frame_support::sp_runtime::Saturating;
//     use sp_runtime::offchain::Duration;
//     use sp_runtime::format;
//     use codec::alloc::string::ToString;
//     use frame_system::offchain::AppCrypto;
//     use frame_system::offchain::SendTransactionTypes;
//     use sp_io::hashing::blake2_128;
//     use sp_runtime::offchain::storage_lock::StorageLock;
//     use sp_runtime::offchain::storage_lock::BlockAndTime;
//     use frame_system::offchain::Signer;
//     use frame_system::offchain::SendUnsignedTransaction;
//     use scale_info::prelude::string::String;
//     use pallet_compute::TechnicalDescription;
//     use frame_system::offchain::SigningTypes;
//     use ipfs_pallet::FileInput;

//     const LOCK_BLOCK_EXPIRATION: u32 = 3;
//     const LOCK_TIMEOUT_EXPIRATION: u32 = 10000;

//     #[pallet::pallet]
//     #[pallet::without_storage_info]
//     pub struct Pallet<T>(_);

//     #[pallet::config]
//     pub trait Config: frame_system::Config + SendTransactionTypes<Call<Self>> +
//                       frame_system::offchain::SigningTypes  + pallet_marketplace::Config + pallet_utils::Config +
//                     //    pallet_compute::Config
//                        {
//         /// The overarching event type.
//         type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

//         /// The identifier type for an offchain worker.
// 		type AuthorityId: AppCrypto<Self::Public, Self::Signature>;

//         #[pallet::constant]
//         type BlocksPerDay: Get<u32>;
//     }

//     // New storage item to store the backup offchain worker frequency per user
//     #[pallet::storage]
//     #[pallet::getter(fn backup_offchain_worker_frequency)]
//     pub(super) type BackupOffchainWorkerFrequency<T: Config> = StorageMap<
//         _,
//         Blake2_128Concat,
//         T::AccountId,
//         u32,
//         ValueQuery,
//         DefaultFrequency<T>
//     >;

//     // Storage item to store restore requests
//     #[pallet::storage]
//     #[pallet::getter(fn restore_requests)]
//     pub(super) type RestoreRequests<T: Config> = StorageMap<
//         _,
//         Blake2_128Concat,
//         u32, // request_id
//         RestoreSnapshotMetadata,
//         OptionQuery
//     >;

//     // Default frequency trait for the storage value
//     #[pallet::type_value]
//     pub fn DefaultFrequency<T: Config>() -> u32 {
//         24 * 60 * 60 // Default to 24 hours (in blocks)
//     }

//     // get backup by node_id
//     #[pallet::storage]
//     #[pallet::getter(fn backups)]
//     pub(super) type Backups<T: Config> = StorageDoubleMap<
//         _,
//         Blake2_128Concat,
//         Vec<u8>,
//         Blake2_128Concat,
//         T::AccountId,
//         BackupMetadata<T::AccountId>,
//         OptionQuery,
//     >;

//     #[pallet::event]
//     #[pallet::generate_deposit(pub(super) fn deposit_event)]
//     pub enum Event<T: Config> {
//         BackupCreated { node_id: Vec<u8>, owner: T::AccountId },
//         SnapshotAdded { node_id: Vec<u8>, snapshot_cid: Vec<u8> },
//         BackupUpdated { node_id: Vec<u8>, updated_at: u32 },
//         UserStorageDeleted { account: T::AccountId, backup_count: u32 },
//         BackupOffchainWorkerFrequencyUpdated { new_frequency: u32 },
//         SnapshotRestoreRequested { cid: Vec<u8>, node_id: Vec<u8>, requester: T::AccountId, request_id: u32 },
//     }

//     #[pallet::error]
//     pub enum Error<T> {
//         BackupAlreadyExists,
//         BackupNotFound,
//         NotAuthorized,
//         AccessAlreadyGranted,
//         AccessNotFound,
//         NotBackupOwner,
//         RestoreRequestNotFound
//     }

//     #[pallet::hooks]
//     impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {

//         fn offchain_worker(block_number: BlockNumberFor<T>) {
//             // Run offchain worker every 24 hours based on BlocksPerDay constant
//             if block_number % <T as pallet::Config>::BlocksPerDay::get().into() == 0u32.into() {
//                 match UtilsPallet::<T>::fetch_node_id() {
//                     Ok(node_id) => {
//                         let node_info = RegistrationPallet::<T>::get_node_registration_info(node_id.clone());
//                         match node_info {
//                             Some(node_info) => {
//                                 if node_info.node_type == NodeType::ComputeMiner {
//                                     // // Call periodic Sanpshot Backup
//                                     // match Self::periodic_sanpshot_backup(node_info.node_id.clone(), block_number) {
//                                     //     Ok(_) => log::info!("Snapshot backup handled successfully"),
//                                     //     Err(e) => log::error!("Error in snapshot backup assignment: {:?}", e),
//                                     // }

//                                     // // Call periodic Sanpshot Backup
//                                     // match Self::periodic_restore_backup_request(node_info.node_id.clone(), block_number) {
//                                     //     Ok(_) => log::info!("restore backup handled successfully"),
//                                     //     Err(e) => log::error!("Error in restore backup assignment: {:?}", e),
//                                     // }

//                                     // Self::process_backup_deletion_requests_offchain(node_info.node_id.clone());

//                                 }
//                             }
//                             None => {
//                                 log::info!("Node info not found");
//                             }
//                         }
//                     }
//                     Err(_) => {
//                         log::info!("Node id not found");
//                     }
//                 }
//             }
//         }
//     }

//     impl<T: Config> Pallet<T> {

//         pub fn periodic_restore_backup_request(node_id: Vec<u8>, current_block : BlockNumberFor<T>)  -> Result<(), sp_runtime::offchain::http::Error>{
//             let restore_requests = Self::get_unfulfilled_restore_requests();
//             for (request_id,restore_request) in restore_requests {
//                 let minner_compute_request = pallet_compute::Pallet::<T>::get_miner_compute_request_by_id(
//                     restore_request.minner_request_id as u128
//                 );
//                 // should exists
//                 if minner_compute_request.is_none() {
//                     continue;
//                 }
//                 let minner_compute_request = minner_compute_request.unwrap();
//                 if minner_compute_request.miner_node_id == node_id{
//                     let compute_request = pallet_compute::Pallet::<T>::get_compute_request_by_id(restore_request.minner_request_id as u128);

//                     if let Some(compute_request) = compute_request {
//                         // Parse technical description and create VM
//                         match serde_json::from_slice::<TechnicalDescription>(&compute_request.plan_technical_description) {
//                             Ok(tech_desc) => {
//                                 // do call snapshot endpoint
//                                 let url = "http://localhost:3030/restore-vm";
//                                 let json_payload = serde_json::json!({
//                                     "vm_name": format!("restored-vm_name-{:?}-vm-{}", current_block, compute_request.request_id),
//                                     "ipfs_api_url": "https://relay-fr.hippius.network/api/v0",
//                                     "cid": String::from_utf8_lossy(&restore_request.cid).to_string(),
//                                     "memory": format!("{}", tech_desc.ram_gb * 1024),
//                                     "vcpus": format!("{}", tech_desc.cpu_cores),
//                                 });

//                                 let json_string = json_payload.to_string();
//                                 let content_length = json_string.len();

//                                 let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(10_000));
//                                 let request = sp_runtime::offchain::http::Request::post(url, vec![json_string]);

//                                 let pending = request
//                                     .add_header("Content-Type", "application/json")
//                                     .add_header("Content-Length", &content_length.to_string())
//                                     .deadline(deadline)
//                                     .send()
//                                     .map_err(|err| {
//                                         log::error!("❌ Error making Request: {:?}", err);
//                                         sp_runtime::offchain::http::Error::IoError
//                                     })?;

//                                 let response = pending
//                                     .try_wait(deadline)
//                                     .map_err(|err| {
//                                         log::error!("❌ Error getting Response: {:?}", err);
//                                         sp_runtime::offchain::http::Error::DeadlineReached
//                                     })??;

//                                 if response.code != 201 {
//                                     log::error!(
//                                         "Unexpected status code: {}, VM restore creation failed. Response body: {:?}",
//                                         response.code, response
//                                     );
//                                     return Err(sp_runtime::offchain::http::Error::Unknown);
//                                 }

//                                 // call unsigned to mark request as fulfilled
//                                 Self::mark_restore_request_fulfilled_offchain(node_id.clone(), request_id);
//                             },
//                             Err(e) => {
//                                 log::error!("Failed to parse technical description: {:?}", e);
//                                 return Err(sp_runtime::offchain::http::Error::Unknown);
//                             }
//                         }
//                     } else {
//                         log::error!("Compute request not found for request_id: {}", restore_request.minner_request_id);
//                         continue;
//                     }
//                 }
//             }
//             Ok(())
//         }

//         /// Periodic subscription snapshot helper function
//         pub fn periodic_sanpshot_backup(node_id: Vec<u8>, current_block : BlockNumberFor<T>)  -> Result<(), sp_runtime::offchain::http::Error>{
//             let users_with_backup_enabled = pallet_marketplace::Pallet::<T>::backup_enabled_users();
//             for user in users_with_backup_enabled {
//                 // Get the user's specific backup frequency, or use default
//                 let user_backup_frequency = Self::backup_offchain_worker_frequency(&user);

//                 // get all the compute requests for the user that is assigned to this minner and is fullfilled
//                 let compute_requests = pallet_compute::Pallet::<T>::compute_requests(user.clone());
//                 for compute_request in compute_requests {
//                     let block_difference = current_block.saturating_sub(compute_request.created_at);
//                     // request should be 1 days old minimum
//                     if block_difference > user_backup_frequency.into() {
//                         let minner_compute_request = pallet_compute::Pallet::<T>::get_miner_compute_request_by_id(
//                             compute_request.request_id
//                         );
//                         // should exists
//                         if minner_compute_request.is_none() {
//                             continue;
//                         }
//                         let minner_compute_request = minner_compute_request.unwrap();
//                         // should be fullfilled
//                         if !minner_compute_request.fullfilled {
//                             continue;
//                         }
//                         if minner_compute_request.miner_node_id == node_id{
//                             // do call snapshot endpoint
//                             let url = "http://localhost:3030/create-snapshot";
//                 			// Use job ID to generate VM name consistently
//                 			let vm_name = format!("vm-{}", String::from_utf8_lossy(&minner_compute_request.job_id.unwrap()).split('-').next().unwrap());
// 							let json_payload = serde_json::json!({
// 								"vm_name": vm_name,
// 								"snapshot_name": format!("snapshot_name-{:?}-vm-{}", current_block, minner_compute_request.request_id),
// 								"ipfs_api_url": "https://relay-fr.hippius.network/api/v0"
// 							});

// 							let json_string = json_payload.to_string();
// 							let content_length = json_string.len();

// 							let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(10_000));
// 							let request = sp_runtime::offchain::http::Request::post(url, vec![json_string]);

// 							let pending = request
// 								.add_header("Content-Type", "application/json")
// 								.add_header("Content-Length", &content_length.to_string())
// 								.deadline(deadline)
// 								.send()
// 								.map_err(|err| {
// 									log::error!("❌ Error making Request: {:?}", err);
// 									sp_runtime::offchain::http::Error::IoError
// 								})?;

// 							let response = pending
// 								.try_wait(deadline)
// 								.map_err(|err| {
// 									log::error!("❌ Error getting Response: {:?}", err);
// 									sp_runtime::offchain::http::Error::DeadlineReached
// 								})??;

// 							if response.code != 201 {
// 								log::error!(
// 									"Unexpected status code: {}, VM creation failed. Response body: {:?}",
// 									response.code, response
// 								);
// 								return Err(sp_runtime::offchain::http::Error::Unknown);
// 							}
//                             let response_body = response.body();
//                             let response_body_vec = response_body.collect::<Vec<u8>>();
//                             let response_str = core::str::from_utf8(&response_body_vec).map_err(|_| sp_runtime::offchain::http::Error::Unknown)?;
//                             match serde_json::from_str::<serde_json::Value>(response_str) {
//                                 Ok(json_response) => {
//                                     if let Some(ipfs_cid) = json_response.get("ipfs_cid") {
//                                         // update this pallet storage and create pin request in ipfs pallet
//                                         Self::call_add_snapshot(
//                                             node_id.clone(),
//                                             ipfs_cid.as_str().unwrap().to_string().as_bytes().to_vec(),
//                                             None,
//                                             minner_compute_request.request_id.try_into().unwrap(),
//                                             user.clone(),
//                                         );
//                                     } else {
//                                         log::warn!("No IPFS CID found in response");
//                                     }
//                                 },
//                                 Err(e) => {
//                                     log::error!("Failed to parse response JSON: {:?}", e);
//                                 }
//                             }
//                         }
//                     }
//                 }
//             }
//             Ok(())
//         }

//     	// Function to send unsigned transaction for marking compute request as fulfilled
// 		pub fn mark_restore_request_fulfilled_offchain(
// 			node_id: Vec<u8>,
// 			request_id: u32,
// 		) {
// 			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
// 				b"Compute::mark_restore_request_fulfilled_lock",
// 				LOCK_BLOCK_EXPIRATION,
// 				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
// 			);

// 			if let Ok(_guard) = lock.try_lock() {
// 				// Fetch signer accounts using AuthorityId
// 				let signer = Signer::<T, <T as pallet::Config>::AuthorityId>::all_accounts();

// 				// Check if there are any accounts and log them
// 				if signer.can_sign() {
// 					log::info!("Signer has accounts available for signing.");
// 				} else {
// 					log::warn!("No accounts available for signing in signer.");
// 				}

// 				let results = signer.send_unsigned_transaction(
// 					|account| {
// 						// Create a payload with all necessary data for the mark_restore_request_fulfilled call
// 						RestoreRequestFulfilledPayload {
// 							node_id: node_id.clone(),
// 							request_id: request_id.clone(),
// 							public: account.public.clone(),
// 							_marker: PhantomData,
// 						}
// 					},
// 					|payload, signature| {
// 						// Construct the call with the payload and signature
// 						Call::mark_restore_request_fulfilled {
// 							node_id: payload.node_id,
// 							request_id: payload.request_id,
// 							signature: signature,
// 						}
// 					},
// 				);

// 				// Process results with comprehensive logging
// 				for (acc, res) in &results {
// 					match res {
// 						Ok(_) => log::info!("[{:?}] Successfully marked restore request as fulfilled", acc.id),
// 						Err(e) => log::info!("[{:?}] Failed to mark restore request as fulfilled: {:?}", acc.id, e),
// 					}
// 				}
// 			} else {
// 				log::info!("❌ Could not acquire lock for marking restore request as fulfilled");
// 			};
// 		}

//         /// Helper function to call add_snapshot as an unsigned transaction
//         pub fn call_add_snapshot(
//             node_id: Vec<u8>,
//             snapshot_cid: Vec<u8>,
//             description: Option<Vec<u8>>,
//             request_id: u32,
//             account_id: T::AccountId
//         ) {
//             let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
//                 b"Backup::add_snapshot_lock",
//                 LOCK_BLOCK_EXPIRATION,
//                 Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
//             );

//             if let Ok(_guard) = lock.try_lock() {
//                 // Fetch signer accounts using AuthorityId
//                 let signer = Signer::<T, <T as pallet::Config>::AuthorityId>::all_accounts();

//                 // Check if there are any accounts and log them
//                 if signer.can_sign() {
//                     log::info!("Signer has accounts available for signing.");
//                 } else {
//                     log::warn!("No accounts available for signing in signer.");
//                 }

//                 let results = signer.send_unsigned_transaction(
//                     |account| {
//                         // Create a payload with all necessary data for the add_snapshot call
//                         AddSnapshotPayload {
//                             node_id: node_id.clone(),
//                             snapshot_cid: snapshot_cid.clone(),
//                             description: description.clone(),
//                             request_id,
//                             account_id: account_id.clone(),
//                             public: account.public.clone(),
//                             _marker: PhantomData,
//                         }
//                     },
//                     |payload, _signature| {
//                         // Construct the call with the payload
//                         Call::add_snapshot {
//                             node_id: payload.node_id,
//                             snapshot_cid: payload.snapshot_cid,
//                             description: payload.description,
//                             request_id: payload.request_id,
//                             account_id: payload.account_id,
//                         }
//                     },
//                 );

//                 // Process and log results
//                 for (acc, res) in &results {
//                     match res {
//                         Ok(_) => log::info!("[{:?}] Successfully submitted add snapshot", acc.id),
//                         Err(e) => log::info!("[{:?}] Failed to submit add snapshot: {:?}", acc.id, e),
//                     }
//                 }
//             } else {
//                 log::info!("❌ Could not acquire lock for adding snapshot");
//             };
//         }

//         /// Delete all backup metadata for a specific account across all nodes
//         pub fn delete_backup_metadata(
//             account_id: T::AccountId
//         ) -> DispatchResult {
//             // Convert account_id to a Vec<u8> for prefix removal
//             let account_id_bytes = account_id.encode();

//             // Remove all backup entries for this account across all nodes
//             Backups::<T>::remove_prefix(&account_id_bytes, None);

//             Ok(())
//         }

//         /// Helper function to get all unfulfilled restore requests
//         pub fn get_unfulfilled_restore_requests() -> Vec<(u32, RestoreSnapshotMetadata)> {
//             RestoreRequests::<T>::iter()
//                 .filter(|(_, restore_request)| !restore_request.is_fulfilled)
//                 .collect()
//         }

//         // Function to send unsigned transaction for processing backup deletion requests
// 		pub fn process_backup_deletion_requests_offchain(node_id: Vec<u8>) {
// 			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
// 				b"Backup::process_backup_deletion_requests_lock",
// 				LOCK_BLOCK_EXPIRATION,
// 				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
// 			);

// 			if let Ok(_guard) = lock.try_lock() {
// 				// Fetch signer accounts using AuthorityId
// 				let signer = Signer::<T, <T as pallet::Config>::AuthorityId>::all_accounts();

// 				// Check if there are any accounts and log them
// 				if signer.can_sign() {
// 					log::info!("Signer has accounts available for signing.");
// 				} else {
// 					log::warn!("No accounts available for signing in signer.");
// 				}

//                 let results = signer.send_unsigned_transaction(
//                     |account| {
//                         // Create a payload with all necessary data for the backup deletion request
//                         BackupDeletionRequestPayload {
//                             node_id: node_id.clone(),
//                             public: account.public.clone(),
//                             _marker: PhantomData,
//                         }
//                     },
//                     |payload, signature| {
//                         // Construct the call with the payload and signature
//                         Call::process_backup_deletion_request {
//                             node_id: payload.node_id,
//                             signature: signature.clone(),
//                         }
//                     },
//                 );

//                 // Process results with comprehensive logging
//                 for (acc, res) in &results {
//                     match res {
//                         Ok(_) => log::info!("[{:?}] Successfully processed backup deletion request", acc.id),
//                         Err(e) => log::error!("[{:?}] Failed to process backup deletion request: {:?}", acc.id, e),
//                     }
//                 }
// 			} else {
// 				log::warn!("❌ Could not acquire lock for processing backup deletion requests");
// 			};
// 		}
//     }

//     	/// Validate an unsigned transaction for compute request assignment
// 	#[pallet::validate_unsigned]
// 	impl<T: Config> ValidateUnsigned for Pallet<T> {
// 		type Call = Call<T>;

// 		fn validate_unsigned(_source: TransactionSource, call: &Self::Call) -> TransactionValidity {
// 			match call {
// 				Call::add_snapshot {
//                     node_id,
//                     snapshot_cid,
//                     description,
//                     request_id,
//                     account_id
//                 } => {
//                     // Additional validation checks
//                     let block_number = <frame_system::Pallet<T>>::block_number();

//                     // Create a unique hash based on the input parameters
//                     let unique_hash = blake2_128(
//                         &[
//                             node_id.encode(),
//                             snapshot_cid.encode(),
//                             description.encode(),
//                             request_id.encode(),
//                             account_id.encode(),
//                             block_number.encode(),
//                         ].concat()
//                     );

//                     ValidTransaction::with_tag_prefix("AddSnapshot")
//                         .priority(TransactionPriority::max_value())
//                         .longevity(5)
//                         .propagate(true)
//                         // Add the unique hash to ensure transaction uniqueness
//                         .and_provides(unique_hash)
//                         .build()
//                 },
//                 Call::mark_restore_request_fulfilled {
//                     node_id,
//                     request_id,
//                     signature
//                 } => {
//                     // Additional validation checks
//                     let block_number = <frame_system::Pallet<T>>::block_number();

//                     // Create a unique hash based on the input parameters
//                     let unique_hash = blake2_128(
//                         &[
//                             node_id.encode(),
//                             request_id.encode(),
//                             signature.encode(),
//                             block_number.encode(),
//                         ].concat()
//                     );

//                     ValidTransaction::with_tag_prefix("MarkRestoreRequestFulfilled")
//                         .priority(TransactionPriority::max_value())
//                         .longevity(5)
//                         .propagate(true)
//                         // Add the unique hash to ensure transaction uniqueness
//                         .and_provides(unique_hash)
//                         .build()
//                 },
//                 Call::process_backup_deletion_request {
//                     node_id,
//                     signature,
//                 } => {
//                     // Additional validation checks
//                     let block_number = <frame_system::Pallet<T>>::block_number();

//                     // Create a unique hash based on the input parameters
//                     let unique_hash = blake2_128(
//                         &[
//                             node_id.encode(),
//                             block_number.encode(),
//                             signature.encode(),
//                         ].concat()
//                     );

//                     ValidTransaction::with_tag_prefix("MarkRestoreRequestFulfilled")
//                         .priority(TransactionPriority::max_value())
//                         .longevity(5)
//                         .propagate(true)
//                         // Add the unique hash to ensure transaction uniqueness
//                         .and_provides(unique_hash)
//                         .build()
//                 },
//                 _ => InvalidTransaction::Call.into(),
// 			}
// 		}
// 	}

//     #[pallet::call]
//     impl<T: Config> Pallet<T> {
//         /// Add a snapshot to an existing backup
//         #[pallet::call_index(0)]
//         #[pallet::weight((0, Pays::No))]
//         pub fn add_snapshot(
//             origin: OriginFor<T>,
//             node_id: Vec<u8>,
//             snapshot_cid: Vec<u8>,
//             description: Option<Vec<u8>>,
//             request_id: u32,
//             account_id: T::AccountId,
//         ) -> DispatchResult {
//             // Ensure the caller is signed
//             let _who = ensure_none(origin)?;

//             let block_number = <frame_system::Pallet<T>>::block_number().saturated_into::<u32>();

//             // Retrieve or create backup metadata
//             let mut metadata = match Backups::<T>::get(&node_id, account_id.clone()) {
//                 Some(existing_metadata) => {
//                     // Ensure the caller is in the ACL for existing backup
//                     ensure!(
//                         existing_metadata.owner == account_id,
//                         Error::<T>::NotAuthorized
//                     );
//                     existing_metadata
//                 },
//                 None => {
//                     // Create a new backup if it doesn't exist
//                     BackupMetadata {
//                         owner: account_id.clone(),
//                         snapshots: Vec::new(),
//                         last_snapshot: None,
//                         created_at: block_number,
//                         updated_at: block_number,
//                         description: None,
//                     }
//                 }
//             };
//             // Add the snapshot
//             let snapshot = SnapshotMetadata {
//                 cid: snapshot_cid.clone(),
//                 block_number,
//                 description,
//                 request_id,
//             };

//             metadata.snapshots.push(snapshot);
//             metadata.last_snapshot = Some(snapshot_cid.clone());
//             metadata.updated_at = block_number;

//             Backups::<T>::insert(&node_id, &account_id.clone(), metadata);

//             // adding ipfs req inside marketplace pallet
//             let file_input = FileInput {
//                 file_name: "Snapshot Cid".as_bytes().to_vec(),
//                 file_hash: snapshot_cid.clone(),
//             };

//             let _ = pallet_marketplace::Pallet::<T>::process_storage_requests(&account_id.clone(), &vec![file_input.clone()], None);

//             Self::deposit_event(Event::SnapshotAdded { node_id, snapshot_cid });

//             Ok(())
//         }

//         /// Set backup offchain worker frequency for a specific user
//         #[pallet::call_index(10)]
//         #[pallet::weight(T::DbWeight::get().writes(1))]
//         pub fn set_backup_offchain_worker_frequency(
//             origin: OriginFor<T>,
//             new_frequency: u32,
//         ) -> DispatchResult {
//             // Ensure the caller is signed
//             let who = ensure_signed(origin)?;

//             // Update the storage value for the specific user
//             BackupOffchainWorkerFrequency::<T>::insert(&who, new_frequency);

//             // Emit an event
//             Self::deposit_event(Event::BackupOffchainWorkerFrequencyUpdated { new_frequency });

//             Ok(())
//         }

//         /// Restore a snapshot by its CID
//         #[pallet::call_index(11)]
//         #[pallet::weight(T::DbWeight::get().writes(1))]
//         pub fn restore_from_snapshot(
//             origin: OriginFor<T>,
//             cid: Vec<u8>,
//         ) -> DispatchResult {
//             // Ensure the caller is signed
//             let who = ensure_signed(origin)?;

//             // Find the backup containing the snapshot with the given CID
//             let backup_found = Backups::<T>::iter()
//                 .find(|(_node_id, _account_id, backup_metadata)|
//                     backup_metadata.snapshots.iter().any(|snapshot| snapshot.cid == cid)
//                 );

//             // Ensure the snapshot exists
//             let (node_id, _account_id, backup_metadata) = backup_found
//                 .ok_or(Error::<T>::BackupNotFound)?;

//             // Find the specific snapshot to get its request_id
//             let snapshot = backup_metadata.snapshots.iter()
//                 .find(|snapshot| snapshot.cid == cid)
//                 .ok_or(Error::<T>::BackupNotFound)?;

//             // Generate a unique request ID (you might want to use a more robust method)
//             let request_id = blake2_128(&cid).to_vec().iter()
//                 .fold(0u32, |acc, &x| acc.wrapping_add(x as u32));

//             // Create RestoreSnapshotMetadata
//             let restore_snapshot = RestoreSnapshotMetadata {
//                 cid: cid.clone(),
//                 block_number: frame_system::Pallet::<T>::block_number().saturated_into(),
//                 description: snapshot.description.clone(),
//                 request_id,
//                 minner_request_id: snapshot.request_id,
//                 is_fulfilled: false,
//             };

//             // Save the restore request
//             RestoreRequests::<T>::insert(request_id, restore_snapshot);

//             // Emit an event
//             Self::deposit_event(Event::SnapshotRestoreRequested {
//                 cid,
//                 node_id,
//                 requester: who,
//                 request_id,
//             });

//             Ok(())
//         }

//         /// Mark a stop request as fulfilled for a specific node
// 		#[pallet::call_index(12)]
// 		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
// 		pub fn mark_restore_request_fulfilled(
// 			origin: OriginFor<T>,
//             _node_id: Vec<u8>,
// 			request_id: u32,
// 			_signature: <T as SigningTypes>::Signature,
// 		) -> DispatchResultWithPostInfo {
// 			// Ensure this is an unsigned transaction
// 			ensure_none(origin)?;

// 			// Retrieve existing restore request
// 			let mut restore_request = RestoreRequests::<T>::get(request_id)
// 				.ok_or(Error::<T>::RestoreRequestNotFound)?;

// 			// Ensure the request is not already fulfilled
// 			ensure!(!restore_request.is_fulfilled, Error::<T>::RestoreRequestNotFound);

// 			// Mark the request as fulfilled
// 			restore_request.is_fulfilled = true;

// 			// Update storage with modified request
// 			RestoreRequests::<T>::insert(request_id, restore_request);

// 			Ok(().into())
// 		}

//         #[pallet::call_index(13)]
// 		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
// 		pub fn process_backup_deletion_request(
// 			origin: OriginFor<T>,
// 			_node_id: Vec<u8>,
//             _signature: <T as SigningTypes>::Signature
// 		) -> DispatchResultWithPostInfo {
// 			// Ensure this is an unsigned transaction
// 			ensure_none(origin)?;

//             let users_for_backup_delete_requests = pallet_marketplace::Pallet::<T>::backup_delete_requests();

//             for user in users_for_backup_delete_requests {
//                 // delete all backups for that user
//                 let _ =  Self::delete_backup_metadata(user.clone());

//                 // remove user from backup_delete_requests
//                 pallet_marketplace::Pallet::<T>::remove_user_from_backup_delete_requests(&user);
//             }

// 			Ok(().into())
// 		}

//     }
// }

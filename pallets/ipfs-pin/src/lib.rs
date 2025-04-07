#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;
pub use types::*;
use sp_runtime::{
	offchain::{
		http,
		Duration,
	}
};
use sp_runtime::SaturatedConversion;
#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

mod types;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;
pub mod weights;
pub use weights::*;
use sp_core::offchain::KeyTypeId;
use frame_system::offchain::AppCrypto;

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
	use frame_support::pallet_prelude::*;
	use sp_std::prelude::*;
	use scale_info::prelude::*;
	use pallet_registration::{NodeType, Status};
	use pallet_registration::Pallet as RegistrationPallet;
	use sp_runtime::offchain::storage_lock::{StorageLock,BlockAndTime};
	use pallet_utils::{ Pallet as UtilsPallet};
	use frame_system::{pallet_prelude::*, offchain::{Signer,SendUnsignedTransaction, SigningTypes, SendTransactionTypes}};
	
	/// dummy request body for getting last finalized block from rpc
	pub const DUMMY_REQUEST_BODY: &[u8; 78] =
	b"{\"id\": 10, \"jsonrpc\": \"2.0\", \"method\": \"chain_getFinalizedHead\", \"params\": []}";

	#[pallet::pallet]
	#[pallet::without_storage_info]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config + pallet_registration::Config + SendTransactionTypes<Call<Self>> + frame_system::offchain::SigningTypes{
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		/// The identifier type for an offchain worker.
		type AuthorityId: AppCrypto<Self::Public, Self::Signature>;

		type WeightInfo: WeightInfo;

		// Add IPFS base URL configuration
		#[pallet::constant]
		type IpfsBaseUrl: Get<&'static str>;

		#[pallet::constant]
		type GarbageCollectorInterval: Get<u32>;

		#[pallet::constant]
		type MinerIPFSCHeckInterval: Get<u32>;
	}

	// the file size where the key is encoded file hash
	#[pallet::storage]
    #[pallet::getter(fn file_size)]
    pub type FileSize<T: Config> = StorageMap<_, Blake2_128Concat, FileHash, u32>;

	// Saves the owners request to store the file, with AccountId and FileHash as keys
	#[pallet::storage]
	pub type RequestedPin<T: Config> = StorageDoubleMap<
		_, 
		Blake2_128Concat, 
		T::AccountId,     // First key: Account ID of the owner
		Blake2_128Concat, 
		FileHash,         // Second key: File Hash
		Option<StorageRequest<T::AccountId,BlockNumberFor<T>>>,
		ValueQuery
	>;

	#[pallet::storage]
	#[pallet::getter(fn unpin_requests)]
	pub(super) type UnpinRequests<T: Config> = StorageValue<_, Vec<FileHash>, ValueQuery>;

	// Saves all the node ids who have pinned for that file hash
	#[pallet::storage]
	pub type FileStored<T: Config> = StorageMap<
		_, 
		Blake2_128Concat, 
		Vec<u8>,     
		Vec<PinRequest<BlockNumberFor<T>>>,
		ValueQuery
	>;

	/// Storage item to track users who have requested storage
    #[pallet::storage]
    #[pallet::getter(fn storage_request_users)]
    pub(super) type StorageRequestUsers<T: Config> = StorageValue<
        _,
        Vec<T::AccountId>,
        ValueQuery
    >;

	const LOCK_BLOCK_EXPIRATION: u32 = 1;
    const LOCK_TIMEOUT_EXPIRATION: u32 = 3000;
	pub(crate) const LOCK_SLEEP_DURATION: u32 = LOCK_TIMEOUT_EXPIRATION / 3;

	//  Blacklist storage item containing the blacklisted Files Hashes
	#[pallet::storage]
	pub type Blacklist<T: Config> = StorageValue<_, Vec<Vec<u8>>, ValueQuery>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		PinRequested {
			who: T::AccountId,
			file_hash: FileHash,
			replicas: u32
		},
		MinerPinnedFile {
			file_hash: FileHash,
			miner: Vec<u8>,
			file_size_in_bytes: u32
		},
		MinerPinnedUpdated{
			miner: Vec<u8>
		},
		UnpinStorageUpdated{
			miner: Vec<u8>
		},
		FileBlacklisted { file_hash: FileHash },
		UnpinRequestAdded { file_hash: FileHash },
		UnpinRequestRemoved { file_hash: FileHash },
		/// Emitted when a payload is signed and processed
		SignedPayloadProcessed {
			/// The signer's key
			signer: [u8; 32],
			/// The payload that was signed
			payload: Vec<u8>,
			/// The signature
			signature: Vec<u8>,
			// node identity
			node_id: Vec<u8>,
		},
	}

	#[pallet::error]
	pub enum Error<T> {
		NoneValue,
		EnoughRelicasAlready,
		RequestAlreadyExists,
		RequestDoesNotExists,
		DuplicatePinRequest,
		FileHashBlacklisted,
		FileAlreadyBlacklisted,
		NodeNotRegistered,
		NodeNotminer,
		NodeNotValidator,
		StorageRequestNotFound,
		OwnerNotFound,
		FileAlreadyRequestedByUser,
		InvalidInput,
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {	
		// miners request to store a file given file hash 
		#[pallet::call_index(0)]
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn update_pin_requests(
			origin: OriginFor<T>,
			node_identity: Vec<u8>,
			pin_requests: Vec<PinRequest<BlockNumberFor<T>>>,
			signature: <T as SigningTypes>::Signature
		) -> DispatchResultWithPostInfo {
            ensure_none(origin)?;
			let _signature = signature;

			// check if the node is already registered or not here
			let node_info = RegistrationPallet::<T>::get_node_registration_info(node_identity.clone());
			// Ensure the node is registered and its type is miner
			ensure!(
				node_info.is_some(),
				Error::<T>::NodeNotRegistered
			);
			
			// node should be a miner
			ensure!(
				matches!(node_info.unwrap().node_type, NodeType::StorageMiner),
				Error::<T>::NodeNotminer
			);

			// Directly insert the vector of PinRequests into the storage
			FileStored::<T>::insert(node_identity.clone(), pin_requests);
			Self::deposit_event(Event::
				MinerPinnedUpdated {
					miner: node_identity
				});

			Ok(().into())
		}
		
		// miners request to store a file given file hash (only Validators can call this )
		#[pallet::call_index(3)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::do_something())]
		pub fn add_ipfs_pin_request(
			origin: OriginFor<T>, 
			file_hash: FileHash,
			node_identity: Vec<u8>,
			account_id: T::AccountId,
			signature: <T as SigningTypes>::Signature,
			file_size_in_bytes: u32
		) -> DispatchResultWithPostInfo {
            ensure_none(origin)?;
			let _signature = signature;
			// check if the node is already registered or not here
			let node_info = RegistrationPallet::<T>::get_node_registration_info(node_identity.clone());
			// Ensure the node is registered and its type is miner
			ensure!(
				node_info.is_some(),
				Error::<T>::NodeNotRegistered
			);

			ensure!(
				matches!(node_info.unwrap().node_type, NodeType::StorageMiner),
				Error::<T>::NodeNotminer
			);

			//  should be requested by owner
			ensure!(RequestedPin::<T>::contains_key(account_id.clone(), &file_hash), Error::<T>::RequestDoesNotExists);

			let mut storage_request = RequestedPin::<T>::get(account_id.clone(), file_hash.clone()).unwrap();
			// check if the replicas are already fullfilled or not 
			ensure!(storage_request.fullfilled_replicas < storage_request.total_replicas, Error::<T>::EnoughRelicasAlready);

			let current_block_number = <frame_system::Pallet<T>>::block_number();
			let request = PinRequest {
				miner_node_id: node_identity.clone(),
				file_hash: file_hash.clone(),
				is_pinned: false,
				created_at: current_block_number,
				file_size_in_bytes: file_size_in_bytes.clone()
			};
			let mut file_hashes_pinned_by_minner = FileStored::<T>::get(node_identity.clone());
			// Check if there is already a pin request for this miner and file hash
			let already_pinned = file_hashes_pinned_by_minner.iter().any(|request| {
				request.miner_node_id == node_identity && request.file_hash == file_hash
			});
			// Ensure that there is no duplicate pin request
			ensure!(
				!already_pinned,
				Error::<T>::DuplicatePinRequest
			);
			// Retrieve the current blacklist from storage
			let blacklist = Blacklist::<T>::get();
			// Check if the `file_hash` is present in the blacklist
			let is_blacklisted = blacklist.contains(&file_hash);
			// If the `file_hash` is blacklisted, return an error
			ensure!(
				!is_blacklisted,
				Error::<T>::FileHashBlacklisted
			);
			file_hashes_pinned_by_minner.push(request);
			FileStored::<T>::insert(node_identity.clone(), file_hashes_pinned_by_minner);

			// updated fullfilled replicas in storage
			storage_request.fullfilled_replicas = storage_request.fullfilled_replicas + 1;			
			RequestedPin::<T>::insert(account_id.clone(), file_hash.clone(), Some(storage_request)); 

			FileSize::<T>::insert(file_hash.clone(), file_size_in_bytes);

			Self::deposit_event(Event::MinerPinnedFile {
					file_hash: file_hash,
					miner: node_identity,
					file_size_in_bytes: file_size_in_bytes
				});
			Ok(().into())
		}

		// Add a file hash to the blacklist
		#[pallet::call_index(4)]
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn add_to_blacklist(origin: OriginFor<T>, file_hash: Vec<u8>) -> DispatchResult {
			// Ensure the function is called by the sudo (root) account
			ensure_root(origin)?;

			let file_hash = hex::encode(file_hash);
			let update_hash : Vec<u8>= file_hash.into();

			// check if the hash is present or not 
			// ensure!(RequestedPin::<T>::contains_key(&update_hash), Error::<T>::RequestDoesNotExists);

			// Retrieve the current blacklist from storage
			let mut blacklist = Blacklist::<T>::get();

			// Check if the file hash is already in the blacklist
			let already_blacklisted = blacklist.iter().any(|hash| hash == &update_hash);

			// Ensure that the file hash is not already blacklisted
			ensure!(
				!already_blacklisted,
				Error::<T>::FileAlreadyBlacklisted
			);

			// Add the new file hash to the blacklist
			blacklist.push(update_hash.clone());

			// Update the blacklist in storage
			Blacklist::<T>::put(blacklist);

			// **Remove the Requested entry with the specified file hash**
			// let _removed_entry = RequestedPin::<T>::remove(&update_hash);

			// Emit an event for the added blacklist entry
			Self::deposit_event(Event::FileBlacklisted { file_hash: update_hash });

			Ok(())
		}

		// miners request to remove a unpin request from storage  
		#[pallet::call_index(5)]
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn remove_unpin_request(
			origin: OriginFor<T>,
			node_identity: Vec<u8>,
			file_hashes_updated_vec: Vec<FileHash>,
			signature: <T as SigningTypes>::Signature
		) -> DispatchResultWithPostInfo {
            ensure_none(origin)?;
			let _signature = signature;

			// check if the node is already registered or not here
			let node_info = RegistrationPallet::<T>::get_node_registration_info(node_identity.clone());
			// Ensure the node is registered and its type is miner
			ensure!(
				node_info.is_some(),
				Error::<T>::NodeNotRegistered
			);
		
			UnpinRequests::<T>::put(file_hashes_updated_vec);
		
			Self::deposit_event(Event::
				UnpinStorageUpdated {
					miner: node_identity
				});
			Ok(().into())
		}

		// miners request to store a file given file hash (only Validators can call this )
		#[pallet::call_index(6)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::do_something())]
		pub fn update_user_storage_request(
			origin: OriginFor<T>, 
			file_hash: FileHash,
			storage_request: Option<StorageRequest<T::AccountId,BlockNumberFor<T>>>,
			node_identity: Vec<u8>,
			account_id: T::AccountId,
			signature: <T as SigningTypes>::Signature,
		) -> DispatchResultWithPostInfo {
			ensure_none(origin)?;
			let _signature = signature;
			// check if the node is already registered or not here
			let node_info = RegistrationPallet::<T>::get_node_registration_info(node_identity.clone());
			// Ensure the node is registered and its type is miner
			ensure!(
				node_info.is_some(),
				Error::<T>::NodeNotRegistered
			);

			ensure!(
				matches!(node_info.unwrap().node_type, NodeType::Validator),
				Error::<T>::NodeNotValidator
			);

			if storage_request.is_some(){
				RequestedPin::<T>::insert(account_id,file_hash,storage_request);
			}else{
				RequestedPin::<T>::remove(account_id, file_hash);
			}	
			Ok(().into())
		}	
	}

	#[pallet::validate_unsigned]
	impl<T: Config> ValidateUnsigned for Pallet<T> {
		type Call = Call<T>;
		
		fn validate_unsigned(_source: TransactionSource, call: &Self::Call) -> TransactionValidity {
			match call {
				// Handler for `update_pin_requests` unsigned transaction
				Call::update_pin_requests { node_identity, pin_requests, signature: _ } => {
					let current_block = frame_system::Pallet::<T>::block_number();

					// Create a unique hash combining all relevant data
					let mut data = Vec::new();
					data.extend_from_slice(&current_block.encode());
					data.extend_from_slice(node_identity);
					for request in pin_requests {
						data.extend_from_slice(&request.file_hash);
						data.extend_from_slice(&request.created_at.encode());
					}
					
					let unique_hash = sp_io::hashing::blake2_256(&data);

					// Ensure unique transaction validity
					ValidTransaction::with_tag_prefix("IpfsPinOffchain")
						.priority(TransactionPriority::max_value())
						.and_provides(unique_hash)
						.longevity(3)
						.propagate(true)
						.build()
				},
				// Handler for `add_ipfs_pin_request` unsigned transaction
				Call::add_ipfs_pin_request { file_hash, node_identity, account_id, signature: _, file_size_in_bytes } => {
					let current_block = frame_system::Pallet::<T>::block_number();
					// Create a unique hash for transaction validity
					let data = (node_identity, file_hash, file_size_in_bytes, current_block, account_id).encode();
					let unique_hash = sp_io::hashing::blake2_256(&data);

					ValidTransaction::with_tag_prefix("IpfsPinOffchain")
						.priority(TransactionPriority::max_value())
						.and_provides(unique_hash)
						.longevity(3)
						.propagate(true)
						.build()
				},
				// Handler for `remove_unpin_request` unsigned transaction
				Call::remove_unpin_request { node_identity, file_hashes_updated_vec, signature: _ } => {
					let current_block = frame_system::Pallet::<T>::block_number();

					// Create a unique hash combining all relevant data
					let mut data = Vec::new();
					data.extend_from_slice(&current_block.encode());
					data.extend_from_slice(node_identity);
					for request in file_hashes_updated_vec {
						data.extend_from_slice(&request);
					}
					
					let unique_hash = sp_io::hashing::blake2_256(&data);

					// Ensure unique transaction validity
					ValidTransaction::with_tag_prefix("IpfsUnpinOffchain")
						.priority(TransactionPriority::max_value())
						.and_provides(unique_hash)
						.longevity(3)
						.propagate(true)
						.build()
				},
				// Handler for `update_user_storage_request` unsigned transaction
				Call::update_user_storage_request {file_hash , storage_request, node_identity, account_id, signature: _ } => {
					let current_block = frame_system::Pallet::<T>::block_number();
					// Create a unique hash for transaction validity
					let data = (node_identity, file_hash, storage_request, current_block, account_id).encode();
					let unique_hash = sp_io::hashing::blake2_256(&data);

					ValidTransaction::with_tag_prefix("IpfsUpdateOffchain")
						.priority(TransactionPriority::max_value())
						.and_provides(unique_hash)
						.longevity(3)
						.propagate(true)
						.build()
				},
				// Default case for invalid calls
				_ => InvalidTransaction::Call.into(),
			}
		}
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {		
		fn offchain_worker(_block_number: BlockNumberFor<T>) {
			let current_block = _block_number.saturated_into::<u32>();
			if current_block % T::MinerIPFSCHeckInterval::get() == 0 {
				// substrate node peer identity 
				let node_address = Self::fetch_node_id();

				// Retrieve the current blacklist from storage
				let blacklist = Blacklist::<T>::get();

				// get all file peer request for this node and check 
				// Only proceed if node_address is Some
				if let Ok(node_identity) = node_address {
					// Hashes of all files peered in local IPFS node
					if let Ok(files_peered) = Self::fetch_info() {
						// first check if the node is registered and the type is validator or not 
						let node_info = RegistrationPallet::<T>::get_node_registration_info(node_identity.clone());
						// if node is present 
						if node_info.is_some(){ 
							// logging IPFS Node Id 
							let node_info = node_info.clone().unwrap(); 
							if let Ok(node_id_str) = sp_std::str::from_utf8(&node_info.ipfs_node_id.unwrap()){
								log::info!("IPFS Node ID : {:?}", node_id_str);
							};

							// handling all the pin request assigned by the validators
							if node_info.node_type == NodeType::StorageMiner
							 && node_info.status != Status::Degraded 
							{
								// Get all file peer requests for this node
								let mut file_hashes_pinned_by_minners = FileStored::<T>::get(node_identity.clone());

								// Loop through each `PinRequest` in the vector
								for pin_request in file_hashes_pinned_by_minners.iter_mut() {
									// decoding the encoded file hash
									if let Ok(file_hash) = hex::decode(&pin_request.file_hash) {
										let update_hash : Vec<u8>= file_hash.into();
										// Check if `pin_request.file_hash` is in `files_peered`
										let is_found = files_peered.contains(&update_hash);
										let is_blacklisted = blacklist.contains(&pin_request.file_hash);
										if pin_request.is_pinned == true {
											// if is_pinned is true then should be in files_peered variable 
											// otherswise remove from rewardlist 
											if !is_found && !is_blacklisted {
													// not foun nor blacklisted just pin the file 
													if let Ok(_) = Self::pin_file_from_ipfs(&sp_std::str::from_utf8(&update_hash).expect("UTF-8 conversion failed")) {
														// Set is_pinned to true only if pinning was successful
														pin_request.is_pinned = true;
													}
													// Self::remove_from_reward_list(node_identity.clone());									
												
											}else{
												if is_blacklisted{
													// unpin the file 
													if let Ok(_) = Self::unpin_file_from_ipfs(&sp_std::str::from_utf8(&update_hash).expect("UTF-8 conversion failed")) {
														// Set is_pinned to true only if pinning was successful
														pin_request.is_pinned = false;
													}
												}
											}
										}else{
											// if is_found add identity (miner) to rewrard list 
											if is_found {
												if !is_blacklisted{
													// set is_pinned to true for this miner
													pin_request.is_pinned = true;
													// add to rewrad list
													// Self::add_to_reward_list(node_identity.clone());
												}
												else{
													// unpin the file 
													if let Ok(_) = Self::unpin_file_from_ipfs(&sp_std::str::from_utf8(&update_hash).expect("UTF-8 conversion failed")) {
														// Set is_pinned to true only if pinning was successful
														pin_request.is_pinned = false;
													}
												}
											}else{
												if !is_blacklisted{
													if let Ok(_) = Self::pin_file_from_ipfs(&sp_std::str::from_utf8(&update_hash).expect("UTF-8 conversion failed")) {
														// Set is_pinned to true only if pinning was successful
														pin_request.is_pinned = true;
													}
													// add to rewrad list
													// Self::add_to_reward_list(node_identity.clone());
												}
											}
										}
									}
								}

								// Filter out blacklisted requests and update `file_hashes_pinned_by_minners`
								file_hashes_pinned_by_minners.retain(|pin_request| {
									// Check if the file hash is blacklisted
									let is_blacklisted = blacklist.contains(&pin_request.file_hash);

									if is_blacklisted {
										false // Return false to remove the request from `file_hashes_pinned_by_minners`
									} else {
										true // Keep the request if it is not blacklisted
									}
								});

								// Handling unpin requests directly and updating file_hashes_pinned_by_minners
								let mut unpin_requests = UnpinRequests::<T>::get();
								let mut files_to_remove: Vec<Vec<u8>> = Vec::new();
						
								// unpin files locally first
								for pin_request in file_hashes_pinned_by_minners.iter() { 
									if let Ok(file_hash) = hex::decode(&pin_request.file_hash) {
										let update_unpin_hash: Vec<u8> = file_hash.into();
										if unpin_requests.contains(&pin_request.file_hash) {
											if let Ok(_) = Self::unpin_file_from_ipfs(
												&sp_std::str::from_utf8(&update_unpin_hash).expect("UTF-8 conversion failed"),
											) {
												files_to_remove.push(pin_request.file_hash.clone()); // Clone the file_hash
											}
										}
									}
								}

								// Then remove the files hashes found in unpinned requests 
								file_hashes_pinned_by_minners.retain(|x| !files_to_remove.contains(&x.file_hash));
								// removed the unpinned files from unpin request array 
								unpin_requests.retain(|request| !files_to_remove.contains(request));

								// Compare with current storage before updating
								let current_storage = FileStored::<T>::get(node_identity.clone());
								if current_storage != file_hashes_pinned_by_minners {
									// Update storage with modified list after handling pin and unpin
									Self::update_ipfs_request_storage(node_identity.clone(), file_hashes_pinned_by_minners);

									// Remove unpin requests if necessary
									if !files_to_remove.is_empty() {
										sp_io::offchain::sleep_until(
											sp_io::offchain::timestamp().add(Duration::from_millis(LOCK_SLEEP_DURATION.into())),
										);
									}
								} else {
									log::info!("✅ No changes in storage, skipping update");
								}
							}
							else if node_info.node_type == NodeType::Validator {
								// cleanup unpins storage
								Self::clean_unpin_requests(node_identity.clone());
							}
						}
					}				
					else {
						log::warn!("files_peered is None; skipping file hash check.");
					}
				} else {
					log::warn!("Node address is None; skipping pin check.");
				}
			}
							
			if current_block % T::GarbageCollectorInterval::get() == 0 {
				if let Err(err) = Self::run_ipfs_gc() {
					log::error!("Failed to run IPFS garbage collection: {:?}", err);
				} else {
					log::info!("IPFS garbage collection executed successfully.");
				}
			}
		}
	}

	impl<T: Config> Pallet<T> {
		// fecth Peer Id of local Node  
		pub fn fetch_node_id() -> Result<Vec<u8>, http::Error> {
			// Get the configured URL and method
			UtilsPallet::<T>::fetch_node_id()
		}

		// Fetch Info From Local IPFS Node 
		fn fetch_info() -> Result<Vec<Vec<u8>>, http::Error> {
			// Request Timeout
			let deadline =
				sp_io::offchain::timestamp().add(Duration::from_millis(2_000));
				
			let body = vec![DUMMY_REQUEST_BODY];

			let base_url = T::IpfsBaseUrl::get();
			let url = format!("{}/api/v0/pin/ls?type=all", base_url);
			
			// getting the list of all pinned Files from node
			let request = sp_runtime::offchain::http::Request::post(&url, body);

			let pending = request
				.add_header("Content-Type", "application/json")
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::warn!("Error making Request: {:?}", err);
					sp_runtime::offchain::http::Error::IoError
				})?;
		
			// getting response
			let response = pending
				.try_wait(deadline)
				.map_err(|err| {
					log::warn!("Error getting Response: {:?}", err);
					sp_runtime::offchain::http::Error::DeadlineReached
				})??;
			
			// Check the response status code
			if response.code != 200 {
				log::warn!("Unexpected status code: {}", response.code);
				return Err(http::Error::Unknown);
			}
			
			// Read the response body
			let body = response.body().collect::<Vec<u8>>();
			
			// parse the body str into vec<u8>
			let mut key_vec : Vec<Vec<u8>> = Vec::new();
			match Self::extract_keys_from_json(&body) {
				Ok(keys) => {
					for key in keys {
						// Convert key back to string for display (only for logging)
						if let Ok(key_str) = sp_std::str::from_utf8(key) {
							if key_str != "Name"{
								// Convert the key string to Vec<u8> and log it
								let key_as_bytes: Vec<u8> = key_str.as_bytes().to_vec();

								let file_hash = hex::encode(key_as_bytes.clone());
								let update_hash : Vec<u8>= file_hash.into();
								key_vec.push(update_hash.clone());
							}
						}
					}
				}
				Err(e) => log::error!("Error extracting keys: {:?}", e),
			}

			// Return a success value with no error
			Ok(key_vec)
		}

		fn extract_keys_from_json(body: &[u8]) -> Result<Vec<&[u8]>, http::Error> {
			// Convert the body to a string
			let body_str = sp_std::str::from_utf8(body).map_err(|_| {
				log::warn!("Response body is not valid UTF-8");
				http::Error::Unknown
			})?;
		
			// Find the start of the keys object
			let keys_start = body_str.find("\"Keys\":").ok_or(http::Error::Unknown)? + 8;
			let keys_end = body_str.find("}}}").ok_or(http::Error::Unknown)?;
		
			// Extract the keys substring
			let keys_str = &body_str[keys_start..keys_end];
		
			// Initialize a vector to hold the keys
			let mut keys: Vec<&[u8]> = Vec::new();
		
			// Split the keys string by commas and iterate over the items
			for line in keys_str.split(',') {
				// Extract the key part, which is before the first colon
				if let Some(colon_index) = line.find(':') {
					let key = &line[..colon_index].trim().trim_matches('"').as_bytes();
					keys.push(key);
				}
			}
		
			// Ensure to remove the trailing curly brace (if it exists)
			if let Some(last_key) = keys.last() {
				if last_key.ends_with(&[b'}']) {
					// Remove the last key's trailing brace if needed
					keys.pop();
				}
			}
			Ok(keys)
		}		

		/// A helper function to fetch the node ID and hardware info, then store it in `NodeSpecs`.
		pub fn update_ipfs_request_storage(node_id: Vec<u8>, pin_requests: Vec<PinRequest<BlockNumberFor<T>>>) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"IpfsPin::update_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);
		
			if let Ok(_guard) = lock.try_lock() {
				let signer = Signer::<T, <T as pallet::Config>::AuthorityId>::all_accounts();
		
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
					return;
				}
		
				let results = signer.send_unsigned_transaction(
					|account| {
						UpdateIpfsRequestPayload {
							node_id: node_id.clone(),
							pin_requests: pin_requests.clone(),
							public: account.public.clone(),
							_marker: PhantomData, // Ensure compatibility with generic payload types
						}
					},
					|payload, signature| {
						Call::update_pin_requests {
							node_identity: payload.node_id,
							pin_requests: payload.pin_requests,
							signature,
						}
					},
				);
		
				// Process the results of the transaction submission
				for (acc, res) in &results {
					match res {
						Ok(()) => log::info!("[{:?}] Successfully submitted IPFS request", acc.id),
						Err(e) => log::error!("[{:?}] Failed to submit IPFS request: {:?}", acc.id, e),
					}
				}
			} else {
				log::error!("❌ Could not acquire lock for updating IPFS request");
			};
		}
		
		// get all files stored my miners 
		pub fn get_files_stored(node_identity: Vec<u8>) -> Vec<PinRequest<BlockNumberFor<T>>>{
			FileStored::<T>::get(node_identity)
		}

		// get all files stored my miners 
		pub fn get_storage_request_by_hash(owner: T::AccountId,encoded_hash: FileHash) -> Option<StorageRequest<T::AccountId,BlockNumberFor<T>>>{
			RequestedPin::<T>::get(owner, encoded_hash)
		}

		// Retrieves the storage request by file hash
		pub fn get_storage_request_by_file_hash(encoded_hash: FileHash) -> Option<StorageRequest<T::AccountId, BlockNumberFor<T>>> {
			// Iterate over all entries in `RequestedPin` storage double map
			for (_owner, file_hash, storage_request_opt) in RequestedPin::<T>::iter() {
				// Check if the current file_hash matches the encoded_hash
				if file_hash == encoded_hash {
					if let Some(storage_request) = storage_request_opt {
						return Some(storage_request);
					}
				}
			}
			None // Return None if no matching request is found
		}

		// add pin reuqest for miners to pin the  file 
		pub fn store_ipfs_pin_request(account_id: T::AccountId, file_hash: FileHash, node_id: Vec<u8>, file_size_in_bytes: u32) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"IpfsPin::lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);
		
			if let Ok(_guard) = lock.try_lock() {
				let signer = Signer::<T, <T as pallet::Config>::AuthorityId>::all_accounts();
		
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
					return;
				}
		
				let results = signer.send_unsigned_transaction(
					|account| {
						StoreIpfsPinPayload {
							file_hash: file_hash.clone(),
							node_id: node_id.clone(),
							file_size_in_bytes,
							account_id: account_id.clone(),
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, signature| {
						Call::add_ipfs_pin_request {
							file_hash: payload.file_hash,
							node_identity: payload.node_id,
							account_id: payload.account_id,
							signature,
							file_size_in_bytes: payload.file_size_in_bytes,
						}
					},
				);
		
				// Process results of the transaction submission
				for (acc, res) in &results {
					match res {
						Ok(()) => log::info!("[{:?}] Successfully submitted Store IPFS Pin request", acc.id),
						Err(e) => log::error!("[{:?}] Failed to submit IPFS Pin request: {:?}", acc.id, e),
					}
				}
			} else {
				log::error!("❌ Could not acquire lock for storing IPFS pin request");
			};
		}

		/// Returns the total file size fulfilled by a user in bytes.
		pub fn total_file_size_fulfilled(user: T::AccountId) -> u128 {
			let fulfilled_requests = Self::get_owner_fulfilled_requests(user);
			let mut total_size: u128 = 0;

			// Iterate through fulfilled requests
			for request in fulfilled_requests {
				if let Some(file_size) = Self::get_file_size(request.file_hash) {
					total_size += file_size as u128;
				}
			}

			total_size
		}

		// update user storage request handler
		pub fn update_storage_usage_request(
			account_id: T::AccountId,
			file_hash: FileHash,
			storage_request: Option<StorageRequest<T::AccountId,BlockNumberFor<T>>>,
			node_id: Vec<u8>,

		) {
			// Storing fetched hardware specs
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"IpfsPin::lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);
		
			if let Ok(_guard) = lock.try_lock() {
				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T, <T as pallet::Config>::AuthorityId>::all_accounts();
		
				if !signer.can_sign() {
					log::warn!("No accounts available for signing in signer.");
					return;
				}
		
				// Prepare and sign the payload
				let results = signer.send_unsigned_transaction(
					|account| {
						UpdateStorageUsagePayload {
							file_hash: file_hash.clone(),
							storage_request: storage_request.clone(),
							node_id: node_id.clone(),
							account_id: account_id.clone(),
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, signature| {
						Call::update_user_storage_request {
							file_hash: payload.file_hash,
							storage_request: payload.storage_request,
							node_identity: payload.node_id,
							account_id: payload.account_id,
							signature,
						}
					},
				);
		
				// Process results of the transaction submission
				for (acc, res) in &results {
					match res {
						Ok(()) => log::info!("[{:?}] Successfully submitted signed Store IPFS Pin", acc.id),
						Err(e) => log::error!("[{:?}] Error Storing IPFS Request: {:?}", acc.id, e),
					}
				}
			} else {
				log::error!("❌ Could not acquire lock for storing IPFS pin request");
			};
		}
		
		pub fn remove_unpin_request_from_storage(node_id: Vec<u8>, file_hashes_updated_vec: Vec<FileHash>) {
			let current_block = frame_system::Pallet::<T>::block_number();
		
			// Create storage keys for local storage with different prefix for unpin requests
			let mut last_update_key = b"ipfs_unpin::last_update::".to_vec(); // Changed prefix
			last_update_key.extend_from_slice(&node_id);
		
			let mut last_hash_key = b"ipfs_unpin::last_hash::".to_vec(); // Changed prefix
			last_hash_key.extend_from_slice(&node_id);
		
			// Get last update block from local storage
			let last_update: Option<u32> = sp_io::offchain::local_storage_get(
				sp_core::offchain::StorageKind::PERSISTENT,
				&last_update_key,
			)
			.and_then(|vec| codec::Decode::decode(&mut &vec[..]).ok());
		
			let current_block_number: u32 = current_block.saturated_into();
		
			// Check if enough blocks have passed since last update
			if let Some(last_block) = last_update {
				if current_block_number < last_block + 5 {
					log::info!("💡 Skipping update as not enough blocks have passed since last update");
					return;
				}
			}
		
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"IpfsUnpin::lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);
		
			if let Ok(_guard) = lock.try_lock() {
				let signer = Signer::<T, <T as pallet::Config>::AuthorityId>::all_accounts();
		
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
					return;
				}
		
				let results = signer.send_unsigned_transaction(
					|account| {
						RemoveIpfsUnpinPayload {
							node_id: node_id.clone(),
							file_hashes_updated_vec: file_hashes_updated_vec.clone(),
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, signature| {
						Call::remove_unpin_request {
							node_identity: payload.node_id,
							file_hashes_updated_vec: payload.file_hashes_updated_vec,
							signature,
						}
					},
				);
		
				// Process results of the transaction submission
				for (acc, res) in &results {
					match res {
						Ok(()) => log::info!("[{:?}] Successfully submitted Remove IPFS Unpin request", acc.id),
						Err(e) => log::error!("[{:?}] Failed to submit Remove IPFS Unpin request: {:?}", acc.id, e),
					}
				}
		
				// Update local storage if successful
				let encoded_block = codec::Encode::encode(&current_block_number);
				sp_io::offchain::local_storage_set(
					sp_core::offchain::StorageKind::PERSISTENT,
					&last_update_key,
					&encoded_block,
				);
			} else {
				log::error!("❌ Could not acquire lock for removing unpin request");
			};
		}
		
		// unpins the local ipfs node 
		pub fn unpin_file_from_ipfs(cid: &str) -> Result<(), http::Error> {
        
			let base_url = T::IpfsBaseUrl::get();
			let url = format!("{}/api/v0/pin/rm?arg={}", base_url, cid);
		  
			// Request Timeout
			let deadline =
				sp_io::offchain::timestamp().add(Duration::from_millis(2_000));
				
			let body = vec![DUMMY_REQUEST_BODY];
			// getting the list of all pinned Files from node
			let request = sp_runtime::offchain::http::Request::post(&url, body);
			let pending = request
				.add_header("Content-Type", "application/json")
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::warn!("Error making Request: {:?}", err);
					sp_runtime::offchain::http::Error::IoError
				})?;
		
			// getting response
			let response = pending
				.try_wait(deadline)
				.map_err(|err| {
					log::warn!("Error getting Response: {:?}", err);
					sp_runtime::offchain::http::Error::DeadlineReached
				})??;
			
			// Check the response status code
			if response.code != 200 {
				log::warn!("Unexpected status code: {}", response.code);
				return Err(http::Error::Unknown);
			}
			
			Ok(())
		}

		// pins the local ipfs node 
		pub fn pin_file_from_ipfs(cid: &str) -> Result<(), http::Error> {
        
			let base_url = T::IpfsBaseUrl::get();
			let url = format!("{}/api/v0/pin/add?arg={}", base_url, cid);
			

			// Request Timeout
			let deadline =
				sp_io::offchain::timestamp().add(Duration::from_millis(2_000));
				
			let body = vec![DUMMY_REQUEST_BODY];
			// getting the list of all pinned Files from node
			let request = sp_runtime::offchain::http::Request::post(&url, body);
			let pending = request
				.add_header("Content-Type", "application/json")
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::warn!("Error making Request: {:?}", err);
					sp_runtime::offchain::http::Error::IoError
				})?;
		
			// getting response
			let response = pending
				.try_wait(deadline)
				.map_err(|err| {
					log::warn!("Error getting Response: {:?}", err);
					sp_runtime::offchain::http::Error::DeadlineReached
				})??;
			
			// Check the response status code
			if response.code != 200 {
				log::warn!("Unexpected status code: {}", response.code);
				return Err(http::Error::Unknown);
			}
			
			Ok(())
		}

		// Function to trigger IPFS garbage collection via HTTP API
		pub fn run_ipfs_gc() -> Result<(), http::Error> {
			// Base URL of the IPFS node
			let base_url = T::IpfsBaseUrl::get();
			let url = format!("{}/api/v0/repo/gc", base_url);

			// Request Timeout
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(5_000));

			// Dummy body since this is a POST request (IPFS API expects POST for GC)
			let body = vec![DUMMY_REQUEST_BODY];
			let request = sp_runtime::offchain::http::Request::post(&url, body);

			let pending = request
				.add_header("Content-Type", "application/json")
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::warn!("Error making request to run GC: {:?}", err);
					sp_runtime::offchain::http::Error::IoError
				})?;

			// Getting response
			let response = pending
				.try_wait(deadline)
				.map_err(|err| {
					log::warn!("Error waiting for GC response: {:?}", err);
					sp_runtime::offchain::http::Error::DeadlineReached
				})??;

			// Check the response status code
			if response.code != 200 {
				log::warn!("Unexpected status code from GC request: {}", response.code);
				return Err(http::Error::Unknown);
			}

			Ok(())
		}

		pub fn get_pending_storage_requests() -> Vec<StorageRequest<T::AccountId, BlockNumberFor<T>>> {
			let mut pending_requests = Vec::new();
		
			// Iterate over all entries in `Requested` storage double map
			for (_owner, _file_hash, storage_request_opt) in RequestedPin::<T>::iter() {
				if let Some(storage_request) = storage_request_opt {
					// Check if fulfilled replicas are less than total replicas
					if storage_request.fullfilled_replicas < storage_request.total_replicas {
						pending_requests.push(storage_request);
					}
				}
			}
		
			pending_requests
		}

		pub fn get_owner_fulfilled_requests(owner: T::AccountId) -> Vec<StorageRequest<T::AccountId,BlockNumberFor<T>>> {
			let mut owner_fulfilled = Vec::new();
	
			// Iterate over all entries for the specific owner
			for (_, storage_request_opt) in RequestedPin::<T>::iter_prefix(&owner) {
				if let Some(storage_request) = storage_request_opt {
					// Check if total replicas are fulfilled
					if storage_request.total_replicas == storage_request.fullfilled_replicas {
						owner_fulfilled.push(storage_request);
					}
				}
			}
	
			owner_fulfilled
		}

		pub fn get_all_fullfilled_requests() -> Vec<StorageRequest<T::AccountId,BlockNumberFor<T>>> {
			let mut fullfilled = Vec::new();
	
			// Iterate over all entries in `Requested` storage double map
			for (_owner, _file_hash, storage_request_opt) in RequestedPin::<T>::iter() {
				if let Some(storage_request) = storage_request_opt {
					// Check if total replicas are equal to fulfilled replicas
					if storage_request.total_replicas == storage_request.fullfilled_replicas {
						fullfilled.push(storage_request);
					}
				}
			}
	
			fullfilled
		}

		// check if the filehash is blacklisted or not 
		pub fn is_file_blacklisted(file_hash: &Vec<u8>) -> bool {
			// Retrieve the blacklist from storage
			let blacklist = Blacklist::<T>::get();
	
			// Check if the provided file hash is in the blacklist
			blacklist.contains(file_hash)
		}

		/// Check if a specific file hash is already pinned by a miner in the given node identity
		pub fn is_file_already_pinned_by_storage_miner(node_identity: Vec<u8>, file_hash: FileHash) -> bool {
			// Retrieve the list of PinRequests for the given node identity
			let file_hashes_pinned_by_minner = FileStored::<T>::get(node_identity);
	
			// Check if any of the requests in the list matches the provided file hash
			file_hashes_pinned_by_minner.iter().any(|pin_request| pin_request.file_hash == file_hash)
		}

		/// Helper function to handle the storage request logic
		pub fn process_storage_request(
			owner: T::AccountId,
			file_hash: FileHash,
			file_name: FileName,
            miner_ids: Option<Vec<Vec<u8>>>,
		) -> Result<(), Error<T>> {
			let current_block = frame_system::Pallet::<T>::block_number();

			// Convert file hash to a consistent format for storage lookup
			let file_hash_key = hex::encode(file_hash.clone());
			let update_hash: Vec<u8> = file_hash_key.into();

			// Check if the request already exists for this user and file hash
			ensure!(
				!RequestedPin::<T>::contains_key(&owner, &update_hash),
				Error::<T>::RequestAlreadyExists
			);

			// Add the user to the storage request user list
			Self::add_storage_request_user(owner.clone());

			// Create the storage request
			let request_info = StorageRequest {
				total_replicas: 7u32,  // total file replicas
				fullfilled_replicas: 0u32,
				owner: owner.clone(),
				file_hash: update_hash.clone(),
				file_name,
				is_approved: false,
				miner_ids: miner_ids.clone(),
				last_charged_at: current_block,
			};

			// Store the request in the double map
			RequestedPin::<T>::insert(&owner, &update_hash, Some(request_info.clone()));

			Ok(())
		}

		/// Helper function to handle the unpin request logic
		pub fn process_unpin_request(
			who: T::AccountId,
			file_hash: FileHash,
		) -> Result<Vec<u8>, Error<T>> {
			// Convert file hash to a hex-encoded string
			let file_hash_encoded = hex::encode(file_hash.clone());
			let update_hash: Vec<u8> = file_hash_encoded.clone().into();
	
			// Check if the file hash exists in the requested pins
			ensure!(
				RequestedPin::<T>::contains_key(&who, &update_hash),
				Error::<T>::RequestDoesNotExists
			);
	
			// Retrieve the storage request and validate ownership
			let storage_request = RequestedPin::<T>::get(&who, &update_hash)
				.ok_or(Error::<T>::StorageRequestNotFound)?;
	
			ensure!(
				storage_request.owner == who,
				Error::<T>::OwnerNotFound
			);
	
			// Retrieve the current vector of unpin requests
			let mut unpin_requests = UnpinRequests::<T>::get();
	
			// Add the file hash to the unpin requests if not already present
			if !unpin_requests.contains(&update_hash) {
				unpin_requests.push(update_hash.clone());
				UnpinRequests::<T>::put(unpin_requests);
			}
	
			// Remove the requested pin entry with the specified file hash
			RequestedPin::<T>::remove(&who, &update_hash);

			// Remove the user from the storage request user list
			Self::remove_account_if_no_request(who.clone());
	
			// Return the processed file hash for event emission
			Ok(update_hash)
		}

		// file size getter for given file hash
		pub fn get_file_size(file_hash_encoded: FileHash) -> Option<u32> {
			FileSize::<T>::get(file_hash_encoded)
		}

		/// Helper function to get all `PinRequest` entries for a given file hash
		pub fn get_pin_requests_by_file_hash(
			file_hash: &FileHash,
		) -> Vec<PinRequest<BlockNumberFor<T>>> {
			let mut matching_requests = Vec::new();
	
			// Iterate over all entries in FileStored
			FileStored::<T>::iter().for_each(|(_node_id, pin_requests)| {
				for request in pin_requests.iter() {
					if &request.file_hash == file_hash {
						matching_requests.push(request.clone());
					}
				}
			});
	
			matching_requests
		}

		// Get all files stored across all node IDs
		pub fn get_all_files_stored() -> Vec<PinRequest<BlockNumberFor<T>>> {
			let mut all_files_stored = Vec::new();

			// Iterate over all entries in the FileStored storage map
			for (_node_id, pin_requests) in FileStored::<T>::iter() {
				// Extend the all_files_stored vector with the pin_requests for each node_id
				all_files_stored.extend(pin_requests);
			}

			all_files_stored
		}

		pub fn overwrite_unpin_requests(requests: Vec<FileHash>) {
			UnpinRequests::<T>::put(requests);
		}

		/// Check if any miner has the file pinned and remove unpin requests accordingly
		fn clean_unpin_requests(node_identity: Vec<u8>) {
			let mut unpin_requests = UnpinRequests::<T>::get();
			// Filter the vector to remove file hashes that are not present in storage anymore (they are all Unpinned)
			unpin_requests.retain(|file_hash| Self::file_hash_exists(file_hash.clone()));
			Self::remove_unpin_request_from_storage(node_identity.clone(), unpin_requests);
		}

		// check if the file hash exists in storage 
		pub fn file_hash_exists(file_hash: Vec<u8>) -> bool {
			// Iterate through all keys (node IDs) in the `FileStored` storage map.
			for (_node_id, pin_requests) in FileStored::<T>::iter() {
				// Check each `PinRequest` in the vector for the given file hash.
				if pin_requests.iter().any(|request| request.file_hash == file_hash) {
					// If any `PinRequest` matches the file hash, return true.
					return true;
				}
			}
			// If no match was found, return false.
			false
		}

		pub fn update_storage_request(
			account_id: T::AccountId,
			file_hash: FileHash,
			storage_request: Option<StorageRequest<T::AccountId,BlockNumberFor<T>>>
		){
			if storage_request.is_some(){
				RequestedPin::<T>::insert(account_id, file_hash,storage_request);
			}else{
				RequestedPin::<T>::remove(account_id, file_hash);
			}	
		}

		/// Helper function to add a user to Storage_Request_Users if not already present
		fn add_storage_request_user(account_id: T::AccountId) {
			StorageRequestUsers::<T>::mutate(|users| {
				if !users.contains(&account_id) {
					users.push(account_id);
				}
			});
		}

		pub fn remove_file_stored(node_id: &Vec<u8>) {
			FileStored::<T>::remove(node_id);
		}

		/// Check if an account has any storage requests and remove from StorageRequestUsers if not
		pub fn remove_account_if_no_request(account: T::AccountId) {
			// Check if any storage requests exist for this account using iter_prefix
			let has_request = RequestedPin::<T>::iter_prefix(&account).next().is_some();

			// If no requests exist, remove the account from StorageRequestUsers
			if !has_request {
				StorageRequestUsers::<T>::mutate(|users| {
					users.retain(|user| user != &account);
				});
			}
		}

		/// Calculate the total file size for all files owned by a user
		/// 
		/// # Arguments
		/// 
		/// * `account`: The account ID of the file owner
		/// 
		/// # Returns
		/// 
		/// Total file size in bytes for all approved and pinned files
		pub fn calculate_total_file_size(
			account: &T::AccountId
		) -> u128 {
			// Set to track unique file hashes
			let mut unique_file_hashes = sp_std::collections::btree_set::BTreeSet::new();
			
			// Total file size accumulator
			let mut total_file_size: u128 = 0;

			// Iterate through all storage requests for the account
			RequestedPin::<T>::iter_prefix(account)
				.filter_map(|(file_hash, storage_request)| {
					// Only consider approved requests
					storage_request.filter(|req| req.is_approved)
						.map(|_| file_hash)
				})
				.for_each(|file_hash| {
					// Check if this file hash is unique
					if unique_file_hashes.insert(file_hash.clone()) {
						// Retrieve all pin requests for this file hash
						let pin_requests = FileStored::<T>::get(&file_hash);

						// Find the first pinned request and add its file size
						if let Some(pin_request) = pin_requests.iter()
							.find(|pin_req| pin_req.is_pinned) 
						{
							total_file_size += pin_request.file_size_in_bytes as u128;
						}
					}
				});

			total_file_size
		}

		/// Retrieves all files for a given account with their pinning information
		///
		/// # Arguments
		///
		/// * `account`: The account address to retrieve files for
		///
		/// # Returns
		///
		/// A vector of UserFile structs containing file details and pinning miners
		pub fn get_user_files(account: T::AccountId) -> Vec<UserFile> {
			RequestedPin::<T>::iter_prefix(account)
				.filter_map(|(file_hash, storage_request)| {
					storage_request.map(|req| {
						// Find miners who have pinned this file by iterating through all FileStored entries
						let miner_ids = FileStored::<T>::iter()
							.filter(|(_, pin_requests)| 
								pin_requests.iter().any(|pin_req| pin_req.file_hash == file_hash)
							)
							.map(|(node_id, _)| node_id)
							.collect();

						// Retrieve file size from FileSize storage, default to 0 if not found
						let file_size = Self::file_size(&file_hash).unwrap_or(0);

						UserFile {
							file_hash,
							file_name: req.file_name,
							miner_ids,
							file_size,
						}
					})
				})
				.collect()
		}
	}
}
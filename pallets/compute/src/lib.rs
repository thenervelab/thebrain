#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;
pub use types::*;

mod types;
use sp_core::offchain::KeyTypeId;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"hips");

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
    use frame_support::pallet_prelude::*;
	use sp_runtime::SaturatedConversion;
	use frame_system::{pallet_prelude::*,offchain::{AppCrypto,SendTransactionTypes}};
    use sp_std::vec::Vec;
	use sp_std::vec;
	use crate::types::*;
	use sp_runtime::offchain::storage_lock::StorageLock;
	use sp_runtime::offchain::Duration;
	use sp_runtime::offchain::storage_lock::BlockAndTime;
	use frame_system::offchain::Signer;
	use frame_system::offchain::SendUnsignedTransaction;
	use pallet_utils::Pallet as UtilsPallet;
	use pallet_ip::Pallet as IpPallet;
	use serde::{Serialize, Deserialize};
	use pallet_registration::NodeType;
	use pallet_registration::Pallet as RegistrationPallet;
	use sp_runtime::format;
	use scale_info::prelude::string::String;
	use codec::alloc::string::ToString;
	use frame_system::offchain::SigningTypes;
	use pallet_subaccount::traits::SubAccounts;

	#[derive(Serialize, Deserialize, Debug)]
	struct VmCreationRequest {
		name: String,
		memory: String,
		vcpus: String,
		disk_size: String,
		is_sev_enabled: bool,
	}

	#[derive(Serialize, Deserialize, Debug)]
	pub struct TechnicalDescription {
		pub cpu_cores: u32,
		pub ram_gb: u32,
		pub storage_gb: u32,
		pub inbound_bandwidth: String,
		pub outbound_bandwidth: String,
		pub gpu: String,
		pub gpu_type: String,
		pub is_sev_enabled: bool, //nw field
	}

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config  + SendTransactionTypes<Call<Self>> 
					+ frame_system::offchain::SigningTypes + pallet_registration::Config + pallet_utils::Config
					+ pallet_subaccount::Config + pallet_ip::Config{
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		/// The identifier type for an offchain worker.
		type AuthorityId: AppCrypto<Self::Public, Self::Signature>;

		/// The block interval for offchain worker operations
        type OffchainWorkerInterval: Get<u32>;

		/// The block interval for offchain worker operations
		type IpReleasePeriod: Get<u64>;
	}

	#[pallet::storage]
	#[pallet::getter(fn next_request_id)]
	pub type NextRequestId<T: Config> = StorageValue<_, u128, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn compute_requests)]
    pub type ComputeRequests<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        Vec<ComputeRequest<T::AccountId,BlockNumberFor<T>, T::Hash>>,
        ValueQuery
    >;

	const LOCK_BLOCK_EXPIRATION: u32 = 3;
    const LOCK_TIMEOUT_EXPIRATION: u32 = 10000;

    #[pallet::storage]
    #[pallet::getter(fn miner_compute_requests)]
    pub type MinerComputeRequests<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        Vec<u8>,
        Vec<MinerComputeRequest<BlockNumberFor<T>, T::Hash, T::AccountId>>,
        ValueQuery
    >;

	#[pallet::storage]
    #[pallet::getter(fn miner_compute_deletion_requests)]
    pub type MinerComputeDeletionRequests<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        Vec<u8>,
        Vec<MinerComputeDeletionRequest<BlockNumberFor<T>, T::Hash, T::AccountId>>,
        ValueQuery
    >;

	// Storage for Miner Compute Stop Requests
	#[pallet::storage]
	#[pallet::getter(fn miner_compute_stop_requests)]
	pub type MinerComputeStopRequests<T: Config> = StorageMap<
	  _,
	  Blake2_128Concat,
	  Vec<u8>,
	  Vec<MinerComputeStopRequest<BlockNumberFor<T>, T::Hash, T::AccountId>>,
	  ValueQuery
	>;

	// Storage for Miner Compute Boot Requests
	#[pallet::storage]
	#[pallet::getter(fn miner_compute_boot_requests)]
	pub type MinerComputeBootRequests<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		Vec<u8>,
		Vec<MinerComputeBootRequest<BlockNumberFor<T>, T::Hash, T::AccountId>>,
		ValueQuery
	>;

	// Storage for Miner Compute Boot Requests
	#[pallet::storage]
	#[pallet::getter(fn miner_compute_reboot_requests)]
	pub type MinerComputeRebootRequests<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		Vec<u8>,
		Vec<MinerComputeRebootRequest<BlockNumberFor<T>, T::Hash, T::AccountId>>,
		ValueQuery
	>;

	// Storage for Miner Compute Boot Requests
	#[pallet::storage]
	#[pallet::getter(fn miner_compute_resize_requests)]
	pub type MinerComputeResizeRequests<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		Vec<u8>,
		Vec<MinerComputeResizeRequest<BlockNumberFor<T>, T::Hash, T::AccountId>>,
		ValueQuery
	>;	

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
		IpAssigned { vm_uuid: Vec<u8>, ip: Vec<u8> },
		IpReturned { vm_uuid: Vec<u8>, ip: Vec<u8> },
		IpRetrieved { vm_uuid: Vec<u8>, ip: Vec<u8> },
		IpAdded {ip: Vec<u8>},
		ComputeRequestCreated {
            plan_id: T::Hash,
            owner: T::AccountId
        },
		ComputeRequestDeleted{
			request_id: u128,
			owner: T::AccountId
        },
		ComputeRequestSubmitted {
            miner: Vec<u8>,
            request_id: u128,
			owner: T::AccountId
        },
		ComputeRequestFulfilled {
			node: Vec<u8>,
			request_id: u128,
			owner: T::AccountId,
		},
		ComputeDeletionRequestRemoved {
			node_id: Vec<u8>,
			request_id: u128,
			owner: T::AccountId,
		},
		ComputeBootRequestRemoved {
			node_id: Vec<u8>,
			request_id: u128,
			owner: T::AccountId,
		},
		ComputeRebootRequestFulfilled {
			node_id: Vec<u8>,
			request_id: u128,
			owner: T::AccountId,
		},
		ComputeResizeRequestFulfilled {
			node_id: Vec<u8>,
			request_id: u128,
			owner: T::AccountId,
		},
		/// Compute stop request initiated
		ComputeStopRequested {
			plan_id: T::Hash,
			owner: T::AccountId,
			caller: T::AccountId,
		},
		/// Compute boot request initiated
		ComputeBootRequested {
			plan_id: T::Hash,
			owner: T::AccountId,
			caller: T::AccountId,
		},
		ComputeResizeRequested {
			plan_id: T::Hash,
			owner: T::AccountId,
			caller: T::AccountId,
		},
		ComputeDeleteRequested {
			plan_id: T::Hash,
			owner: T::AccountId,
			caller: T::AccountId,
		},
		ComputeRebootRequested {
			plan_id: T::Hash,
			owner: T::AccountId,
			caller: T::AccountId,
		},
    }

    #[pallet::error]
    pub enum Error<T> {
		NoAvailableIp,
		VmAlreadyHasIp,
		VmHasNoIp,
		IpAlreadyExists,
		InvalidSignature,
		ComputeRequestNotFound,
		ComputeDeletionRequestNotFound,
		ComputeStopRequestAlreadyExists,
		ComputeStopRequestNotFound,
		ComputeBootRequestNotFound,
		ComputeRebootRequestNotFound,
		ComputeResizeRequestNotFound
    }

	/// Validate an unsigned transaction for compute request assignment
	#[pallet::validate_unsigned]
	impl<T: Config> ValidateUnsigned for Pallet<T> {
		type Call = Call<T>;
	
		fn validate_unsigned(_source: TransactionSource, call: &Self::Call) -> TransactionValidity {
			match call {
				Call::submit_compute_request_assignment { miner_node_id, plan_id, request_id, owner: _ } => {
					// Additional validation checks
					let block_number = <frame_system::Pallet<T>>::block_number();

					let mut data = Vec::new();
					data.extend_from_slice(&block_number.encode());
					data.extend_from_slice(miner_node_id);
					data.extend_from_slice(&plan_id.encode());
					data.extend_from_slice(&request_id.encode());
					data.extend_from_slice(&Self::get_current_timestamp().encode());
					let unique_hash = sp_io::hashing::blake2_256(&data);
	
					ValidTransaction::with_tag_prefix("ComputeRequestAssignment")
						.priority(TransactionPriority::max_value())
						.longevity(5)
						.propagate(true)
						// Add the unique hash to ensure transaction uniqueness
						.and_provides(("submit_compute_request_assignment", unique_hash))
						.build()
				},
				Call::mark_miner_compute_stop_request_fulfilled { node_id, signature: _, request_id } => {
					// Fetch the current block number and timestamp
					let block_number = <frame_system::Pallet<T>>::block_number();
					let timestamp = Self::get_current_timestamp();
				
					// Create a unique hash using all relevant fields
					let mut data = Vec::new();
					data.extend_from_slice(&block_number.encode());
					data.extend_from_slice(node_id);
					data.extend_from_slice(&request_id.encode());
					data.extend_from_slice(&timestamp.encode());
					let unique_hash = sp_io::hashing::blake2_256(&data);
					// Build the transaction validity with a truly unique `provides` value
					ValidTransaction::with_tag_prefix("ComputeStopRequestFulfilled")
						.priority(TransactionPriority::max_value())
						.longevity(5)
						.propagate(true)
						.and_provides(("mark_miner_compute_stop_request_fulfilled", unique_hash)) // Unique key
						.build()
				}				
				Call::update_miner_compute_request_hypervisor_ip { node_id, request_id, hypervisor_ip } => {
					// Additional validation checks
					let block_number = <frame_system::Pallet<T>>::block_number();
					
					let mut data = Vec::new();
					data.extend_from_slice(&block_number.encode());
					data.extend_from_slice(node_id);
					data.extend_from_slice(&hypervisor_ip.encode());
					data.extend_from_slice(&request_id.encode());
					data.extend_from_slice(&Self::get_current_timestamp().encode());
					let unique_hash = sp_io::hashing::blake2_256(&data);

					ValidTransaction::with_tag_prefix("ComputeRequestFulfilled")
						.priority(TransactionPriority::max_value())
						.longevity(5)
						.propagate(true)
						// Add the unique hash to ensure transaction uniqueness
						.and_provides(("update_miner_compute_request_hypervisor_ip",unique_hash))
						.build()
				},
				Call::update_miner_compute_request_vnc { node_id, request_id, vnc_port, vm_name } => {
					// Additional validation checks
					let block_number = <frame_system::Pallet<T>>::block_number();
					
					let mut data = Vec::new();
					data.extend_from_slice(&block_number.encode());
					data.extend_from_slice(node_id);
					data.extend_from_slice(&vnc_port.encode());
					data.extend_from_slice(&vm_name.encode());
					data.extend_from_slice(&request_id.encode());
					data.extend_from_slice(&Self::get_current_timestamp().encode());
					let unique_hash = sp_io::hashing::blake2_256(&data);

					ValidTransaction::with_tag_prefix("ComputeRequestFulfilled")
						.priority(TransactionPriority::max_value())
						.longevity(5)
						.propagate(true)
						// Add the unique hash to ensure transaction uniqueness
						.and_provides(("update_miner_compute_request_vnc",unique_hash))
						.build()
				},

				Call::update_miner_compute_request { node_id, request_id, job_id, ip } => {
					// Additional validation checks
					let block_number = <frame_system::Pallet<T>>::block_number();
					
					let mut data = Vec::new();
					data.extend_from_slice(&block_number.encode());
					data.extend_from_slice(node_id);
					data.extend_from_slice(job_id);
					data.extend_from_slice(ip);
					data.extend_from_slice(&request_id.encode());
					data.extend_from_slice(&Self::get_current_timestamp().encode());
					let unique_hash = sp_io::hashing::blake2_256(&data);

					ValidTransaction::with_tag_prefix("ComputeRequestFulfilled")
						.priority(TransactionPriority::max_value())
						.longevity(5)
						.propagate(true)
						// Add the unique hash to ensure transaction uniqueness
						.and_provides(("update_miner_compute_request",unique_hash))
						.build()
				},
				Call::submit_compute_boot_request_fulfillment { node_id, signature: _ ,request_id } => {
					// Fetch the current block number and timestamp
					let block_number = <frame_system::Pallet<T>>::block_number();
					let timestamp = Self::get_current_timestamp();
				
					// Create a unique hash using all relevant fields
					let mut data = Vec::new();
					data.extend_from_slice(&block_number.encode());
					data.extend_from_slice(node_id);
					data.extend_from_slice(&request_id.encode());
					data.extend_from_slice(&timestamp.encode());
					let unique_hash = sp_io::hashing::blake2_256(&data);
				
					// Build the transaction validity with a truly unique `provides` value
					ValidTransaction::with_tag_prefix("ComputeBootRequestFulfilled")
						.priority(TransactionPriority::max_value())
						.longevity(5)
						.propagate(true)
						.and_provides(("submit_compute_boot_request_fulfillment", unique_hash)) // Unique key
						.build()
				},
				Call::handle_miner_compute_request_failure { node_id, request_id, fail_reason: _ } => {
					// Additional validation checks
					let block_number = <frame_system::Pallet<T>>::block_number();
					
					let mut data = Vec::new();
					data.extend_from_slice(&block_number.encode());
					data.extend_from_slice(node_id);
					data.extend_from_slice(&request_id.encode());
					data.extend_from_slice(&Self::get_current_timestamp().encode());
					let unique_hash = sp_io::hashing::blake2_256(&data);

					ValidTransaction::with_tag_prefix("ComputeRequestFailure")
						.priority(TransactionPriority::max_value())
						.longevity(5)
						.propagate(true)
						// Add the unique hash to ensure transaction uniqueness
						.and_provides(("handle_miner_compute_request_failure",unique_hash))
						.build()					
				},
				Call::mark_compute_request_fulfilled { node_id, request_id} => {
					// Additional validation checks
					let block_number = <frame_system::Pallet<T>>::block_number();
					
					let mut data = Vec::new();
					data.extend_from_slice(&block_number.encode());
					data.extend_from_slice(node_id);
					data.extend_from_slice(&request_id.encode());
					data.extend_from_slice(&Self::get_current_timestamp().encode());
					let unique_hash = sp_io::hashing::blake2_256(&data);

					ValidTransaction::with_tag_prefix("ComputeRequestFulfilled")
						.priority(TransactionPriority::max_value())
						.longevity(5)
						.propagate(true)
						// Add the unique hash to ensure transaction uniqueness
						.and_provides(("mark_compute_request_fulfilled",unique_hash))
						.build()
				},
				Call::submit_compute_reboot_request_fulfillment { node_id, signature: _,request_id } => {
					// Fetch the current block number and timestamp
					let block_number = <frame_system::Pallet<T>>::block_number();
					let timestamp = Self::get_current_timestamp();
				
					// Create a unique hash using all relevant fields
					let mut data = Vec::new();
					data.extend_from_slice(&block_number.encode());
					data.extend_from_slice(node_id);
					data.extend_from_slice(&request_id.encode());
					data.extend_from_slice(&timestamp.encode());
					let unique_hash = sp_io::hashing::blake2_256(&data);
				
					// Build the transaction validity with a truly unique `provides` value
					ValidTransaction::with_tag_prefix("ComputeRebootRequestFulfilled")
						.priority(TransactionPriority::max_value())
						.longevity(5)
						.propagate(true)
						.and_provides(("submit_compute_reboot_request_fulfillment", unique_hash)) // Unique key
						.build()
				},
				Call::submit_compute_resize_request_fulfillment { node_id, signature: _,request_id } => {
					// Fetch the current block number and timestamp
					let block_number = <frame_system::Pallet<T>>::block_number();
					let timestamp = Self::get_current_timestamp();
				
					// Create a unique hash using all relevant fields
					let mut data = Vec::new();
					data.extend_from_slice(&block_number.encode());
					data.extend_from_slice(node_id);
					data.extend_from_slice(&request_id.encode());
					data.extend_from_slice(&timestamp.encode());
					let unique_hash = sp_io::hashing::blake2_256(&data);
				
					// Build the transaction validity with a truly unique `provides` value
					ValidTransaction::with_tag_prefix("ComputeResizeRequestFulfilled")
						.priority(TransactionPriority::max_value())
						.longevity(5)
						.propagate(true)
						.and_provides(("submit_compute_resize_request_fulfillment", unique_hash)) // Unique key
						.build()
				},
				Call::submit_compute_deletion_request { node_id, request_id, vm_name } => {
					// Additional validation checks
					let block_number = <frame_system::Pallet<T>>::block_number();

					// Create a unique hash based on the input parameters
					let mut data = Vec::new();
					data.extend_from_slice(&block_number.encode());
					data.extend_from_slice(node_id);
					data.extend_from_slice(&request_id.encode());
					data.extend_from_slice(&vm_name.encode());
					data.extend_from_slice(&Self::get_current_timestamp().encode());
					let unique_hash = sp_io::hashing::blake2_256(&data);
	
					ValidTransaction::with_tag_prefix("ComputeDeletionRequest")
						.priority(TransactionPriority::max_value())
						.longevity(5)
						.propagate(true)
						// Add the unique hash to ensure transaction uniqueness
						.and_provides(("submit_compute_deletion_request",unique_hash))
						.build()
				},
				_ => InvalidTransaction::Call.into(),
			}
		}
	}
	
    #[pallet::call]
    impl<T: Config> Pallet<T> {
		/// Submit compute request assignment via unsigned transaction
		#[pallet::call_index(3)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn submit_compute_request_assignment(
			origin: OriginFor<T>,
			miner_node_id: Vec<u8>,
			plan_id: T::Hash,
			request_id: u128,
			owner: T::AccountId
		) -> DispatchResultWithPostInfo {
			// Ensure this is an unsigned transaction
			ensure_none(origin)?;

            // Get the current block number
            let current_block = frame_system::Pallet::<T>::block_number();

			let mut request_found = false;
			
			ComputeRequests::<T>::iter().for_each(|(owner, requests)| {
				if let Some(request_index) = requests.iter().position(|r| r.request_id == request_id) {
					ComputeRequests::<T>::mutate(owner.clone(), |requests| {
						if let Some(request) = requests.get_mut(request_index) {
							request.is_assigned = true;
							request_found = true;
						}
					});
				}
			});

			ensure!(request_found, "Compute request not found");

			// Find the compute request by iterating through all requests
			Self::update_compute_request_status(request_id, ComputeRequestStatus::InProgress)?;
			
			let node_info = RegistrationPallet::<T>::get_node_registration_info(miner_node_id.clone());
 
			let miner_request = MinerComputeRequest {
				request_id,
				miner_node_id: miner_node_id.clone(),
				miner_account_id: node_info.unwrap().owner.clone(),
				plan_id,
				job_id: None,
				hypervisor_ip: None,
				ip_assigned: None,
				vnc_port: None,
				fail_reason: None,
				created_at: current_block,
				fullfilled: false,
			};

			// Update storage
			MinerComputeRequests::<T>::mutate(miner_node_id.clone(), |requests| {
				requests.push(miner_request.clone());
			});

			// Emit event (optional)
			Self::deposit_event(Event::ComputeRequestSubmitted {
				miner: miner_node_id,
				request_id: request_id,
				owner: owner
			});

			Ok(().into())
		}

		/// Mark a compute request as fulfilled for a specific node
		#[pallet::call_index(4)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn update_miner_compute_request(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			request_id: u128,
			job_id: Vec<u8>,
			ip: Vec<u8>,
		) -> DispatchResultWithPostInfo {
			// Ensure this is an unsigned transaction
			ensure_none(origin)?;

			// Retrieve miner compute requests for the given node ID
			MinerComputeRequests::<T>::mutate(node_id.clone(), |requests| {
				// Find the request with the matching request_id
				match requests.iter_mut().find(|r| r.request_id == request_id) {
					Some(request) => {
						// Mark the request as fulfilled
						request.job_id = Some(job_id.clone());
						request.ip_assigned = Some(ip.clone());
						
						// Find the compute request to get the owner
						if let Some((owner, _)) = Self::find_compute_request_by_id(request_id) {
							// Emit event to indicate request fulfillment
							Self::deposit_event(Event::ComputeRequestFulfilled {
								node: node_id.clone(),
								request_id,
								owner
							});
						}
						
						Ok(())
					},
					None => Err(Error::<T>::ComputeRequestNotFound)
				}
			})?;


			// Handle the result of assign_ip and convert the error
			IpPallet::<T>::assign_ip_to_vm(job_id.clone(), ip.clone())?;
			
			Ok(().into())
		}

		/// Mark a stop request as fulfilled for a specific node
		#[pallet::call_index(5)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn mark_miner_compute_stop_request_fulfilled(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			request_id: u128,
			_signature: <T as SigningTypes>::Signature,
		) -> DispatchResultWithPostInfo {
			// Ensure this is an unsigned transaction
			ensure_none(origin)?;

			// Find and update the compute request
			Self::update_compute_request_status(request_id, ComputeRequestStatus::Stopped)?;

			// Retrieve existing stop requests for the user
			let mut user_stop_requests = MinerComputeStopRequests::<T>::get(&node_id);
			
			// Find the request with the matching request_id and mark it as fulfilled
			let request_found = user_stop_requests.iter_mut()
				.find(|req| req.request_id == request_id && !req.fullfilled)
				.map(|req| {
					req.fullfilled = true;
					true
				})
				.unwrap_or(false);

			// Ensure the request was found and marked
			ensure!(request_found, Error::<T>::ComputeStopRequestNotFound);

			// Update storage with modified requests
			MinerComputeStopRequests::<T>::insert(&node_id, user_stop_requests);

			Ok(().into())
		}

		/// Submit compute deletion request via unsigned transaction
		#[pallet::call_index(6)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn submit_compute_deletion_request(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			request_id: u128,
			vm_name: Vec<u8>,
		) -> DispatchResultWithPostInfo {
			// Ensure this is an unsigned transaction
			ensure_none(origin)?;

			// Retrieve existing deletion requests for the node
			let mut deletion_requests = MinerComputeDeletionRequests::<T>::get(&node_id);
			
			// Find the index of the deletion request to remove
			let request_index = deletion_requests
				.iter()
				.position(|req| req.request_id == request_id)
				.ok_or(Error::<T>::ComputeDeletionRequestNotFound)?;
			
			// Remove the specific deletion request
			deletion_requests.remove(request_index);
			
			// Update the storage with modified deletion requests
			if deletion_requests.is_empty() {
				// If no requests remain, remove the entire entry
				MinerComputeDeletionRequests::<T>::remove(&node_id);
			} else {
				// Otherwise, update the deletion requests
				MinerComputeDeletionRequests::<T>::insert(&node_id, deletion_requests);
			}

			// release Ip back to the available pool
			IpPallet::<T>::generate_vm_release_ip_request_and_update_storage(vm_name)?;

			// Emit an event about the compute deletion request removal
			if let Some((owner, _)) = Self::find_compute_request_by_id(request_id) {
				Self::deposit_event(Event::ComputeDeletionRequestRemoved {
					node_id,
					request_id,
					owner
				});
			}

			Ok(().into())
		}
		
		/// Submit compute boot request fulfillment via unsigned transaction
		#[pallet::call_index(8)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn submit_compute_boot_request_fulfillment(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			request_id: u128,
			_signature: <T as SigningTypes>::Signature
		) -> DispatchResultWithPostInfo {
			// Ensure this is an unsigned transaction
			ensure_none(origin)?;

			// Retrieve existing boot requests for the node
			let mut boot_requests = MinerComputeBootRequests::<T>::get(&node_id);
			
			// Find the index of the boot request to fulfill
			let request_index = boot_requests
				.iter()
				.position(|req| req.request_id == request_id && !req.fullfilled)
				.ok_or(Error::<T>::ComputeBootRequestNotFound)?;
			
			// Mark the specific boot request as fulfilled
			boot_requests[request_index].fullfilled = true;
			
			// Update the storage with modified boot requests
			MinerComputeBootRequests::<T>::insert(&node_id, boot_requests);

			// Update the compute request status
			Self::update_compute_request_status(request_id, ComputeRequestStatus::Running)?;

			// Emit an event about the compute boot request fulfillment
			if let Some((owner, _)) = Self::find_compute_request_by_id(request_id) {
				Self::deposit_event(Event::ComputeBootRequestRemoved {
					node_id,
					request_id,
					owner
				});
			}

			Ok(().into())
		}

		// Extrinsic to request compute stop for a specific plan
		#[pallet::call_index(9)]
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn request_compute_stop(
			origin: OriginFor<T>,
			plan_id: T::Hash
		) -> DispatchResult {
			// Ensure the caller is signed
			let user = ensure_signed(origin)?;

			// Check if the account is a sub-account, and if so, use the main account
			let main_account = match <pallet_subaccount::Pallet<T> as SubAccounts<T::AccountId>>::get_main_account(user.clone()) {
				Ok(main) => main,
				Err(_) => user.clone(), // If not a sub-account, use the original account
			};

			// Call the compute pallet's stop request function
			Self::add_miner_compute_stop_request(main_account.clone(), plan_id)?;

			// Emit an event 
			Self::deposit_event(Event::ComputeStopRequested { 
				plan_id,
				caller: user,
				owner: main_account
			});

			Ok(())
		}
		
		// add_miner_compute_boot_request
		// Extrinsic to request compute boot for a specific plan
		#[pallet::call_index(10)]
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn request_compute_boot(
			origin: OriginFor<T>,
			plan_id: T::Hash
		) -> DispatchResult {
			// Ensure the caller is signed
			let user = ensure_signed(origin)?;

			// Check if the account is a sub-account, and if so, use the main account
			let main_account = match <pallet_subaccount::Pallet<T> as SubAccounts<T::AccountId>>::get_main_account(user.clone()) {
				Ok(main) => main,
				Err(_) => user.clone(), // If not a sub-account, use the original account
			};

			// Call the compute pallet's stop request function
			Self::add_miner_compute_boot_request(main_account.clone(), plan_id)?;

			// Emit an event 
			Self::deposit_event(Event::ComputeBootRequested { 
				plan_id,
				caller: user,
				owner: main_account
			});

			Ok(())
		}
				
		// Extrinsic to request compute boot for a specific plan
		#[pallet::call_index(11)]
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn request_compute_delete(
			origin: OriginFor<T>,
			plan_id: T::Hash
		) -> DispatchResult {
			// Ensure the caller is signed
			let user = ensure_signed(origin)?;

			// Check if the account is a sub-account, and if so, use the main account
			let main_account = match <pallet_subaccount::Pallet<T> as SubAccounts<T::AccountId>>::get_main_account(user.clone()) {
				Ok(main) => main,
				Err(_) => user.clone(), // If not a sub-account, use the original account
			};

			// Call the compute pallet's stop request function
			Self::add_delete_miner_compute_request(plan_id, main_account.clone())?;

			// Emit an event 
			Self::deposit_event(Event::ComputeDeleteRequested { 
				owner: main_account,
				caller: user, 
				plan_id
			});

			Ok(())
		}


		// Extrinsic to request compute boot for a specific plan
		#[pallet::call_index(20)]
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn force_request_compute_delete(
			origin: OriginFor<T>,
			plan_id: T::Hash,
			account_id: T::AccountId
		) -> DispatchResult {
			// Ensure the caller is signed
			ensure_root(origin)?;

			// Call the compute pallet's stop request function
			Self::add_delete_miner_compute_request(plan_id, account_id.clone())?;

			// Emit an event 
			Self::deposit_event(Event::ComputeDeleteRequested { 
				owner: account_id.clone(),
				caller: account_id, 
				plan_id
			});

			Ok(())
		}

		/// Extrinsic to request compute reboot for a specific plan
		#[pallet::call_index(12)] // Assuming this is the next index
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn request_compute_reboot(
			origin: OriginFor<T>,
			plan_id: T::Hash
		) -> DispatchResult {	
			// Ensure the caller is signed
			let user = ensure_signed(origin)?;

			// Check if the account is a sub-account, and if so, use the main account
			let main_account = match <pallet_subaccount::Pallet<T> as SubAccounts<T::AccountId>>::get_main_account(user.clone()) {
				Ok(main) => main,
				Err(_) => user.clone(), // If not a sub-account, use the original account
			};

			// Call the compute pallet's reboot request function
			Self::add_miner_compute_reboot_request(main_account.clone(), plan_id)?;

			// Emit an event 
			Self::deposit_event(Event::ComputeRebootRequested { 
				plan_id,
				caller: user,
				owner: main_account
			});

			Ok(())
		}	

		/// Submit compute reboot request fulfillment via unsigned transaction
		#[pallet::call_index(13)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn submit_compute_reboot_request_fulfillment(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			request_id: u128,
			_signature: <T as SigningTypes>::Signature
		) -> DispatchResultWithPostInfo {
			// Ensure this is an unsigned transaction
			ensure_none(origin)?;

			// Retrieve existing reboot requests for the node
			let mut reboot_requests = MinerComputeRebootRequests::<T>::get(&node_id);
			
			// Find the index of the reboot request to fulfill
			let request_index = reboot_requests
				.iter()
				.position(|req| req.request_id == request_id && !req.fullfilled)
				.ok_or(Error::<T>::ComputeRebootRequestNotFound)?;
			
			// Mark the specific reboot request as fulfilled
			reboot_requests[request_index].fullfilled = true;
			
			// Update the storage with modified reboot requests
			MinerComputeRebootRequests::<T>::insert(&node_id, reboot_requests);

			// Update the compute request status
			Self::update_compute_request_status(request_id, ComputeRequestStatus::Running)?;

			// Emit an event about the compute reboot request fulfillment
			if let Some((owner, _)) = Self::find_compute_request_by_id(request_id) {
				Self::deposit_event(Event::ComputeRebootRequestFulfilled {
					node_id,
					request_id,
					owner
				});
			}

			Ok(().into())
		}

		// add_miner_compute_image_resize_request
		// Extrinsic to request compute resize for a specific plan
		#[pallet::call_index(14)]
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn request_compute_resize(
			origin: OriginFor<T>,
			plan_id: T::Hash,
			resize_to_gbs: u32
		) -> DispatchResult {
			// Ensure the caller is signed
			let user = ensure_signed(origin)?;

			// Check if the account is a sub-account, and if so, use the main account
			let main_account = match <pallet_subaccount::Pallet<T> as SubAccounts<T::AccountId>>::get_main_account(user.clone()) {
				Ok(main) => main,
				Err(_) => user.clone(), // If not a sub-account, use the original account
			};

			// Call the compute pallet's stop request function
			Self::add_miner_compute_image_resize_request(main_account.clone(), plan_id, resize_to_gbs)?;

			// Emit an event 
			Self::deposit_event(Event::ComputeResizeRequested { 
				plan_id,
				caller: user,
				owner: main_account
			});

			Ok(())
		}

		/// Submit compute resize request fulfillment via unsigned transaction
		#[pallet::call_index(15)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn submit_compute_resize_request_fulfillment(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			request_id: u128,
			_signature: <T as SigningTypes>::Signature
		) -> DispatchResultWithPostInfo {
			// Ensure this is an unsigned transaction
			ensure_none(origin)?;

			// Retrieve existing resize requests for the node
			let mut resize_requests = MinerComputeResizeRequests::<T>::get(&node_id);
			
			// Find the index of the resize request to fulfill
			let request_index = resize_requests
				.iter()
				.position(|req| req.request_id == request_id && !req.fullfilled)
				.ok_or(Error::<T>::ComputeResizeRequestNotFound)?;
			
			// Mark the specific resize request as fulfilled
			resize_requests[request_index].fullfilled = true;
			
			// Update the storage with modified resize requests
			MinerComputeResizeRequests::<T>::insert(&node_id, resize_requests);

			// Emit an event about the compute resize request fulfillment
			if let Some((owner, _)) = Self::find_compute_request_by_id(request_id) {
				Self::deposit_event(Event::ComputeResizeRequestFulfilled {
					node_id,
					request_id,
					owner
				});
			}

			Ok(().into())
		}

		/// Mark a compute request as fulfilled for a specific node
		#[pallet::call_index(16)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn mark_compute_request_fulfilled(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			request_id: u128,
		) -> DispatchResultWithPostInfo {
			// Ensure this is an unsigned transaction
			ensure_none(origin)?;

			// Find and update the compute request
			Self::update_compute_request_status(request_id, ComputeRequestStatus::Running)?;

			// Retrieve miner compute requests for the given node ID
			MinerComputeRequests::<T>::mutate(node_id.clone(), |requests| {
				// Find the request with the matching request_id
				match requests.iter_mut().find(|r| r.request_id == request_id) {
					Some(request) => {
						// Mark the request as fulfilled
						request.fullfilled = true;

						// Find the compute request to get the owner
						if let Some((owner, _)) = Self::find_compute_request_by_id(request_id) {
							// Emit event to indicate request fulfillment
							Self::deposit_event(Event::ComputeRequestFulfilled {
								node: node_id.clone(),
								request_id,
								owner
							});
						}
						
						Ok(())
					},
					None => Err(Error::<T>::ComputeRequestNotFound)
				}
			})?;

			
			Ok(().into())
		}

		/// Mark a compute request as fulfilled for a specific node
		#[pallet::call_index(19)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn update_miner_compute_request_vnc(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			request_id: u128,
			vnc_port: u64,
			_vm_name: Vec<u8>,
		) -> DispatchResultWithPostInfo {
			// Ensure this is an unsigned transaction
			ensure_none(origin)?;

			// Retrieve miner compute requests for the given node ID
			MinerComputeRequests::<T>::mutate(node_id.clone(), |requests| {
				// Find the request with the matching request_id
				match requests.iter_mut().find(|r| r.request_id == request_id) {
					Some(request) => {
						request.vnc_port = Some(vnc_port.clone());						
						Ok(())
					},
					None => Err(Error::<T>::ComputeRequestNotFound)
				}
			})?;
			
			// Self::assign_ip(node_id.clone(), vm_name, request_id)?;

			Ok(().into())
		}

		/// Mark a compute request as fulfilled for a specific node
		#[pallet::call_index(17)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn update_miner_compute_request_hypervisor_ip(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			request_id: u128,
			hypervisor_ip: Vec<u8>,
		) -> DispatchResultWithPostInfo {
			// Ensure this is an unsigned transaction
			ensure_none(origin)?;

			// Retrieve miner compute requests for the given node ID
			MinerComputeRequests::<T>::mutate(node_id.clone(), |requests| {
				// Find the request with the matching request_id
				match requests.iter_mut().find(|r| r.request_id == request_id) {
					Some(request) => {
						// Mark the request as fulfilled
						request.hypervisor_ip = Some(hypervisor_ip.clone());						
						Ok(())
					},
					None => Err(Error::<T>::ComputeRequestNotFound)
				}
			})?;		
			Ok(().into())
		}
		
		/// Mark a compute request as fulfilled for a specific node
		#[pallet::call_index(18)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn handle_miner_compute_request_failure(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			request_id: u128,
			fail_reason: Vec<u8>
		) -> DispatchResultWithPostInfo {
			// Ensure this is an unsigned transaction
			ensure_none(origin)?;

			let fail_reason_str = String::from_utf8_lossy(&fail_reason).into_owned();

			// Check if the fail reason contains the specific error message
			let new_status = if fail_reason_str.contains("No Cloud-Init CID provided") {
				log::warn!("Cloud-Init CID error detected. Setting status to Failed.");
				ComputeRequestStatus::Failed
			} else {
				ComputeRequestStatus::Pending
			};

			// Find and update the compute request
			Self::update_compute_request_status(request_id, new_status)?;

			// // Remove the specific compute request from storage
			// <MinerComputeRequests<T>>::mutate(&node_id, |requests| {
			// 	requests.retain(|req| req.request_id != request_id);
			// });

			// Retrieve miner compute requests for the given node ID
			MinerComputeRequests::<T>::mutate(node_id.clone(), |requests| {
				// Find the request with the matching request_id
				match requests.iter_mut().find(|r| r.request_id == request_id) {
					Some(request) => {
						// Mark the request as fulfilled
						request.fail_reason = Some(fail_reason.clone());						
						Ok(())
					},
					None => Err(Error::<T>::ComputeRequestNotFound)
				}
			})?;

			Ok(().into())
		}		
		
    }

	impl<T: Config> Pallet<T> {

		/// Helper method to create a new compute request
        pub fn create_compute_request(
            owner: T::AccountId,
			plan_technical_description: Vec<u8>,
            plan_id: T::Hash,
            selected_image: ImageMetadata,
			cloud_init_cid: Option<Vec<u8>>,
			miner_id: Option<Vec<u8>>,
        ) -> DispatchResult {
            // Increment and get the next request ID
            let request_id = NextRequestId::<T>::get();
            NextRequestId::<T>::put(request_id.saturating_add(1));

            // Get the current block number
            let current_block = frame_system::Pallet::<T>::block_number();

            // Create the compute request
            let compute_request = ComputeRequest {
                request_id,
				plan_technical_description: plan_technical_description.clone(),
                plan_id,
                status: ComputeRequestStatus::Pending,
                created_at: current_block,
                owner: owner.clone(),
                selected_image,
                is_assigned: false,	
				cloud_init_cid,
				miner_id,
            };

            // Store the request in the ComputeRequests storage
            ComputeRequests::<T>::mutate(owner.clone(), |requests| {
                requests.push(compute_request.clone());
            });

            // Emit an event for the created compute request
            Self::deposit_event(Event::ComputeRequestCreated {
                plan_id,
                owner
            });

            Ok(())
        }

		/// Helper method to get all pending compute requests
		pub fn get_pending_compute_requests() -> Vec<ComputeRequest<T::AccountId, BlockNumberFor<T>, T::Hash>> {
			// Collect all pending compute requests across all accounts
			ComputeRequests::<T>::iter()
				.flat_map(|(_, requests)| {
					requests.into_iter()
						.filter(|request| request.status == ComputeRequestStatus::Pending)
				})
				.collect()
		}
		
		/// Retrieve unfulfilled MinerComputeRequests for a specific node
        pub fn get_unfulfilled_miner_compute_requests(node_id: Vec<u8>) -> Vec<MinerComputeRequest<BlockNumberFor<T>, T::Hash, T::AccountId>> {
            MinerComputeRequests::<T>::get(&node_id)
                .into_iter()
                .filter(|request| !request.fullfilled && request.job_id.is_none() && request.fail_reason.is_none())
                .collect()
        }

		/// Helper function to get miner compute requests with a job_id that are not yet fulfilled
		pub fn get_pending_job_requests(node_id: Vec<u8>) -> Vec<MinerComputeRequest<BlockNumberFor<T>, T::Hash, T::AccountId>> {
			MinerComputeRequests::<T>::get(&node_id)
				.into_iter()
				.filter(|request| request.job_id.is_some() && !request.fullfilled && request.fail_reason.is_none())
				.collect()
		}

		/// Helper function to get miner compute requests with a job_id that are not yet fulfilled
		pub fn get_pending_vnc_requests(node_id: Vec<u8>) -> Vec<MinerComputeRequest<BlockNumberFor<T>, T::Hash, T::AccountId>> {
			MinerComputeRequests::<T>::get(&node_id)
				.into_iter()
				.filter(|request| request.job_id.is_some() && request.fail_reason.is_none() && request.vnc_port.is_none())
				.collect()
		} 

		/// Helper function to get miner compute requests with a job_id that are not yet fulfilled
		pub fn get_pending_nebula_requests(node_id: Vec<u8>) -> Vec<MinerComputeRequest<BlockNumberFor<T>, T::Hash, T::AccountId>> {
			MinerComputeRequests::<T>::get(&node_id)
				.into_iter()
				.filter(|request| request.job_id.is_some() && request.fullfilled && request.hypervisor_ip.is_none() && request.fail_reason.is_none())
				.collect()
		}

		/// Getter helper function to retrieve a MinerComputeRequest by account ID and plan ID
        ///
        /// # Arguments
        ///
        /// * `account_id` - The account ID associated with the compute request
        /// * `plan_id` - The plan ID of the compute request
        ///
        /// # Returns
        ///
        /// An Option containing the full MinerComputeRequest if found
        pub fn get_miner_compute_request(
            account_id: T::AccountId, 
            plan_id: T::Hash
        ) -> Option<MinerComputeRequest<BlockNumberFor<T>, T::Hash, T::AccountId>> {
            // Iterate through all miner compute requests
            for (_, miner_requests) in MinerComputeRequests::<T>::iter() {
                // Find a request matching the plan ID
                if let Some(miner_request) = miner_requests.iter()
                    .find(|req| req.plan_id == plan_id && req.fullfilled) 
                {
                    // Check the corresponding compute request to match the account ID
                    let compute_request_match = ComputeRequests::<T>::get(&account_id)
                        .iter()
                        .any(|req| 
                            req.plan_id == plan_id && 
                            req.request_id == miner_request.request_id
                        );

                    if compute_request_match {
                        return Some(miner_request.clone());
                    }
                }
            }
            // No matching request found
            None
        }

        /// Retrieve a MinerComputeRequest by its request ID
        ///
        /// # Arguments
        ///
        /// * `request_id` - The unique identifier of the compute request
        ///
        /// # Returns
        ///
        /// An Option containing the MinerComputeRequest if found
        pub fn get_miner_compute_request_by_id(
            request_id: u128
        ) -> Option<MinerComputeRequest<BlockNumberFor<T>, T::Hash, T::AccountId>> {
            // Iterate through all miner compute requests
            for (_, miner_requests) in MinerComputeRequests::<T>::iter() {
                // Find a request matching the request ID
                if let Some(miner_request) = miner_requests.iter()
                    .find(|req| req.request_id == request_id && req.fullfilled) 
                {
                    return Some(miner_request.clone());
                }
            }
            // No matching request found
            None
        }

		pub fn get_miner_compute_requests_with_failure(
			request_id: u128
		) -> Vec<MinerComputeRequest<BlockNumberFor<T>, T::Hash, T::AccountId>> {
			let mut failed_requests = Vec::new();
		
			// Iterate through all miner compute requests
			for (_, miner_requests) in MinerComputeRequests::<T>::iter() {
				// Find all requests matching the request ID with a fail reason
				failed_requests.extend(
					miner_requests.iter()
						.filter(|req| req.request_id == request_id && req.fail_reason.is_some())
						.cloned()
				);
			}
		
			// Return the vector of failed requests
			failed_requests
		}

        /// Helper function to find a MinerComputeRequest by account ID and plan ID
        // get reuqets id and node id of minner 
        fn find_miner_compute_request(
            account_id: T::AccountId, 
            plan_id: T::Hash
        ) -> Option<(u128, Vec<u8>, Option<Vec<u8>>)> {
            // Reuse  method
            Self::get_miner_compute_request(account_id, plan_id)
                .map(|miner_request| (
                    miner_request.request_id, 
                    miner_request.miner_node_id,
					miner_request.job_id
                ))
        }
		
		/// Delete a compute request
		pub fn add_delete_miner_compute_request(
			plan_id: T::Hash,
			user_id: T::AccountId
		) -> DispatchResult {
			// Attempt to find the request details and minner node id
			let (request_id, node_id, job_id) = Self::find_miner_compute_request(user_id.clone(), plan_id.clone())
			.ok_or(Error::<T>::ComputeRequestNotFound)?;
		
			// Retrieve and modify miner requests
			let mut miner_requests = MinerComputeRequests::<T>::get(&node_id);
			
            // Remove all miner requests with the matching request_id
            miner_requests.retain(|req| req.request_id != request_id);
            MinerComputeRequests::<T>::insert(&node_id, miner_requests); 
		
			// Remove from ComputeRequests more efficiently
			ComputeRequests::<T>::mutate(&user_id, |requests| {
				requests.retain(|req| req.request_id != request_id);
			});

			// Check and delete any matching stop requests
			MinerComputeStopRequests::<T>::mutate(&node_id, |stop_requests| {
				// Remove stop requests that match the user_id, plan_id, and are not fulfilled
				stop_requests.retain(|req| 
					!(req.user_id == user_id && 
					  req.plan_id == plan_id )
				);
			});

			// Check and delete any matching boot requests
			MinerComputeBootRequests::<T>::mutate(&node_id, |boot_requests| {
				// Remove boot requests that match the user_id, plan_id, and are not fulfilled
				boot_requests.retain(|req| 
					!(req.user_id == user_id && 
					  req.plan_id == plan_id )
				);
			});

			// Check and delete any matching reboot requests
			MinerComputeRebootRequests::<T>::mutate(&node_id, |reboot_requests| {
				// Remove reboot requests that match the user_id, plan_id, and are not fulfilled
				reboot_requests.retain(|req| 
					!(req.user_id == user_id && 
					  req.plan_id == plan_id )
				);
			});

			// Check and delete any matching reboot requests
			MinerComputeResizeRequests::<T>::mutate(&node_id, |resize_requests| {
				// Remove reboot requests that match the user_id, plan_id, and are not fulfilled
				resize_requests.retain(|req| 
					!(req.user_id == user_id && 
					  req.plan_id == plan_id )
				);
			});
		
			// Create deletion request
			let deletion_request = MinerComputeDeletionRequest {
				miner_node_id: node_id.clone(),
				request_id,
				job_id,
				user_id: user_id.clone(),
				plan_id,
				created_at: frame_system::Pallet::<T>::block_number(),
				fullfilled: false,
			};
		
			// Update deletion requests
			MinerComputeDeletionRequests::<T>::mutate(&node_id, |requests| {
				requests.push(deletion_request.clone());
			});

			// Emit event
			Self::deposit_event(Event::ComputeRequestDeleted { request_id, owner: user_id.clone() });
		
			Ok(())
		}
		
		pub fn call_handle_miner_compute_request_failure(
			node_id: Vec<u8>,
			request_id: u128,
			fail_reason: Vec<u8>
		) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"Compute::handle_miner_compute_request_failure_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);

			if let Ok(_guard) = lock.try_lock() {
				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T, <T as Config>::AuthorityId>::all_accounts();

				// Check if there are any accounts and log them
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
				}

				let results = signer.send_unsigned_transaction(
					|account| {
						// Create a payload with all necessary data for the handle_miner_compute_request_failure call
						ComputeRequestFailurePayload {
							node_id: node_id.clone(),
							request_id: request_id.clone(),
							fail_reason: fail_reason.clone(),
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, _signature| {
						// Construct the call with the payload and signature
						Call::handle_miner_compute_request_failure {
							node_id: payload.node_id,
							request_id: payload.request_id,
							fail_reason: payload.fail_reason,
						}
					},
				);

				// Process results with comprehensive logging
				for (acc, res) in &results {
					match res {
						Ok(_) => log::info!("[{:?}] Successfully marked compute request as fulfilled", acc.id),
						Err(e) => log::info!("[{:?}] Failed to mark compute request as fulfilled: {:?}", acc.id, e),
					}
				}
			} else {
				log::info!("❌ Could not acquire lock for marking compute request as fulfilled");
			};
		}		

		// Function to send unsigned transaction for marking compute request as fulfilled
		pub fn mark_compute_request_fulfilled_offchain(
			node_id: Vec<u8>,
			request_id: u128,
		) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"Compute::mark_compute_request_fulfilled_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);

			if let Ok(_guard) = lock.try_lock() {
				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T, <T as Config>::AuthorityId>::all_accounts();

				// Check if there are any accounts and log them
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
				}

				let results = signer.send_unsigned_transaction(
					|account| {
						// Create a payload with all necessary data for the mark_compute_request_fulfilled call
						ComputeRequestFulfilledPayload {
							node_id: node_id.clone(),
							request_id: request_id.clone(),
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, _signature| {
						// Construct the call with the payload and signature
						Call::mark_compute_request_fulfilled {
							node_id: payload.node_id,
							request_id: payload.request_id,
						}
					},
				);

				// Process results with comprehensive logging
				for (acc, res) in &results {
					match res {
						Ok(_) => log::info!("[{:?}] Successfully marked compute request as fulfilled", acc.id),
						Err(e) => log::info!("[{:?}] Failed to mark compute request as fulfilled: {:?}", acc.id, e),
					}
				}
			} else {
				log::info!("❌ Could not acquire lock for marking compute request as fulfilled");
			};
		}		

		// Your save_compute_request function
		pub fn save_compute_request(
			miner_node_id: Vec<u8>,
			plan_id: T::Hash,
			request_id: u128,
			owner: T::AccountId
		) {
			
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"Compute::submit_compute_request_assignment_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);

			if let Ok(_guard) = lock.try_lock() {

				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T, <T as Config>::AuthorityId>::all_accounts();

				// Check if there are any accounts and log them
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
				}

				let results = signer.send_unsigned_transaction(
					|account| {
						// Create a payload with all necessary data for the update_rankings call
						ComputeRequestAssignmentPayload {
							miner_node_id: miner_node_id.clone(),
							plan_id: plan_id.clone(),
							request_id: request_id.clone(),
							owner: owner.clone(),
							public: account.public.clone(),
							_marker: PhantomData, // Add this line
						}
					},
					|payload, _signature| {
						// Construct the call with the payload and signature
						Call::submit_compute_request_assignment {
							miner_node_id: payload.miner_node_id,
							plan_id: payload.plan_id,
							request_id: payload.request_id,
							owner: payload.owner,
						}
					},
				);

				// Restore error processing with more comprehensive logging
				for (acc, res) in &results {
					match res {
						Ok(_) => log::info!("[{:?}] Successfully submitted compute update", acc.id),
						Err(e) => log::info!("[{:?}] Failed to submit compute update: {:?}", acc.id, e),
					}
				}
			} else {
				log::info!("❌ Could not acquire lock for saving compute Update");
			};
		}

		pub fn call_submit_compute_boot_request_fulfillment(
			node_id: Vec<u8>,
			request_id: u128
		) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"Compute::submit_compute_boot_request_fulfillment_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);

			if let Ok(_guard) = lock.try_lock() {
				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T, <T as Config>::AuthorityId>::all_accounts();

				// Check if there are any accounts and log them
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
				}

				let results = signer.send_unsigned_transaction(
					|account| {
						// Create a payload with all necessary data for the mark_compute_deletion_request_fulfilled call
						MinnerBootComputeRequestPayload {
							node_id: node_id.clone(),
							request_id: request_id.clone(),
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, signature| {
						// Construct the call with the payload and signature
						Call::submit_compute_boot_request_fulfillment {
							node_id: payload.node_id,
							request_id: payload.request_id,
							signature: signature,
						}
					},
				);

				// Process results with comprehensive logging
				for (acc, res) in &results {
					match res {
						Ok(_) => log::info!("[{:?}] Successfully marked compute request as deleted", acc.id),
						Err(e) => log::info!("[{:?}] Failed to mark boot request as deleted: {:?}", acc.id, e),
					}
				}
			} else {
				log::info!("❌ Could not acquire lock for marking boot request as deleted");
			};
		}

		pub fn call_submit_compute_reboot_request_fulfillment(
			node_id: Vec<u8>,
			request_id: u128
		) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"Compute::submit_compute_Reboot_request_fulfillment_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);

			if let Ok(_guard) = lock.try_lock() {
				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T, <T as Config>::AuthorityId>::all_accounts();

				// Check if there are any accounts and log them
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
				}

				let results = signer.send_unsigned_transaction(
					|account| {
						// Create a payload with all necessary data for the mark_compute_deletion_request_fulfilled call
						MinnerRebootComputeRequestPayload {
							node_id: node_id.clone(),
							request_id: request_id.clone(),
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, signature| {
						// Construct the call with the payload and signature
						Call::submit_compute_reboot_request_fulfillment {
							node_id: payload.node_id,
							request_id: payload.request_id,
							signature: signature,
						}
					},
				);

				// Process results with comprehensive logging
				for (acc, res) in &results {
					match res {
						Ok(_) => log::info!("[{:?}] Successfully marked reboot request as deleted", acc.id),
						Err(e) => log::info!("[{:?}] Failed to mark reboot request as deleted: {:?}", acc.id, e),
					}
				}
			} else {
				log::info!("❌ Could not acquire lock for marking reboot request as deleted");
			};
		}

		// 
		pub fn call_submit_compute_resize_request_fulfillment(
			node_id: Vec<u8>,
			request_id: u128
		) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"Compute::submit_compute_Resize_request_fulfillment_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);

			if let Ok(_guard) = lock.try_lock() {
				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T, <T as Config>::AuthorityId>::all_accounts();

				// Check if there are any accounts and log them
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
				}

				let results = signer.send_unsigned_transaction(
					|account| {
						// Create a payload with all necessary data for the mark_compute_deletion_request_fulfilled call
						MinnerResizeComputeRequestPayload {
							node_id: node_id.clone(),
							request_id: request_id.clone(),
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, signature| {
						// Construct the call with the payload and signature
						Call::submit_compute_resize_request_fulfillment {
							node_id: payload.node_id,
							request_id: payload.request_id,
							signature: signature,
						}
					},
				);

				// Process results with comprehensive logging
				for (acc, res) in &results {
					match res {
						Ok(_) => log::info!("[{:?}] Successfully marked resize request as fulfilled", acc.id),
						Err(e) => log::info!("[{:?}] Failed to mark resize request as fulfilled: {:?}", acc.id, e),
					}
				}
			} else {
				log::info!("❌ Could not acquire lock for marking resize request as fulfilled");
			};
		}

		// Function to send unsigned transaction for deleting compute request 
		pub fn call_submit_compute_deletion_request(
			node_id: Vec<u8>,
			request_id: u128,
			vm_name: Vec<u8>,
		) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"Compute::mark_compute_deletion_request_fulfilled_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);

			if let Ok(_guard) = lock.try_lock() {
				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T, <T as Config>::AuthorityId>::all_accounts();

				// Check if there are any accounts and log them
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
				}

				let results = signer.send_unsigned_transaction(
					|account| {
						// Create a payload with all necessary data for the mark_compute_deletion_request_fulfilled call
						MinnerDeleteComputeRequestPayload {
							node_id: node_id.clone(),
							request_id: request_id.clone(),
							vm_name: vm_name.clone(),
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, _signature| {
						// Construct the call with the payload and signature
						Call::submit_compute_deletion_request {
							node_id: payload.node_id,
							request_id: payload.request_id,
							vm_name: payload.vm_name,
						}
					},
				);

				// Process results with comprehensive logging
				for (acc, res) in &results {
					match res {
						Ok(_) => log::info!("[{:?}] Successfully marked delete compute request as fulfilled", acc.id),
						Err(e) => log::info!("[{:?}] Failed to mark delete compute request as fulfilled: {:?}", acc.id, e),
					}
				}
			} else {
				log::info!("❌ Could not acquire lock for marking compute request as fulfilled");
			};
		}

		// Function to send unsigned transaction for marking compute request as fulfilled
		pub fn mark_miner_compute_stop_request_fulfilled_offchain(
			node_id: Vec<u8>,
			request_id: u128
		) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"Compute::mark_miner_compute_stop_request_fulfilled_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);

			if let Ok(_guard) = lock.try_lock() {
				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T, <T as Config>::AuthorityId>::all_accounts();

				// Check if there are any accounts and log them
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
				}

				let results = signer.send_unsigned_transaction(
					|account| {
						// Create a payload with all necessary data for the mark_miner_compute_stop_request_fulfilled call
						StopRequestFulfilledPayload {
							node_id: node_id.clone(),
							request_id: request_id.clone(),
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, signature| {
						// Construct the call with the payload and signature
						Call::mark_miner_compute_stop_request_fulfilled {
							node_id: payload.node_id,
							request_id: payload.request_id,
							signature: signature,
						}
					},
				);

				// Process results with comprehensive logging
				for (acc, res) in &results {
					match res {
						Ok(_) => log::info!("[{:?}] Successfully marked compute request as fulfilled", acc.id),
						Err(e) => log::info!("[{:?}] Failed to mark compute request as fulfilled: {:?}", acc.id, e),
					}
				}
			} else {
				log::info!("❌ Could not acquire lock for marking compute request as fulfilled");
			};
		}

		
		// Function to send unsigned transaction for marking compute request as fulfilled
		pub fn update_miner_compute_request_hypervisor_ip_offchain(
			node_id: Vec<u8>,
			request_id: u128,
			hypervisor_ip: Vec<u8>,
		) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"Compute::update_miner_compute_request_hypervisor_ip_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);

			if let Ok(_guard) = lock.try_lock() {
				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T,  <T as Config>::AuthorityId>::all_accounts();

				// Check if there are any accounts and log them
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
				}

				let results = signer.send_unsigned_transaction(
					|account| {
						// Create a payload with all necessary data for the update_miner_compute_request_hypervisor_ip call
						ComputeRequestNebluaIpAssignementPayload {
							node_id: node_id.clone(),
							request_id: request_id.clone(),
							hypervisor_ip: hypervisor_ip.clone(),
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, _signature| {
						// Construct the call with the payload and signature
						Call::update_miner_compute_request_hypervisor_ip {
							node_id: payload.node_id,
							request_id: payload.request_id,
							hypervisor_ip: payload.hypervisor_ip,
						}
					},
				);

				// Process results with comprehensive logging
				for (acc, res) in &results {
					match res {
						Ok(_) => log::info!("[{:?}] Successfully marked compute request as fulfilled", acc.id),
						Err(e) => log::info!("[{:?}] Failed to mark compute request as fulfilled: {:?}", acc.id, e),
					}
				}
			} else {
				log::info!("❌ Could not acquire lock for marking compute request as fulfilled");
			};
		}

		
		// Function to send unsigned transaction for marking compute request as fulfilled
		pub fn update_miner_compute_request_vnc_offchain(
			node_id: Vec<u8>,
			request_id: u128,
			vnc_port: u64,
			vm_name: Vec<u8>,
		) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"Compute::update_miner_compute_request_vnc_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);

			if let Ok(_guard) = lock.try_lock() {
				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T, <T as Config>::AuthorityId>::all_accounts();

				// Check if there are any accounts and log them
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
				}

				let results = signer.send_unsigned_transaction(
					|account| {
						// Create a payload with all necessary data for the update_miner_compute_request_vnc call
						ComputeRequestVncAssignementPayload {
							node_id: node_id.clone(),
							request_id: request_id.clone(),
							vnc_port: vnc_port.clone(),
							vm_name: vm_name.clone(),
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, _signature| {
						// Construct the call with the payload and signature
						Call::update_miner_compute_request_vnc {
							node_id: payload.node_id,
							request_id: payload.request_id,
							vm_name: payload.vm_name,
							vnc_port: vnc_port.clone(),
						}
					},
				);

				// Process results with comprehensive logging
				for (acc, res) in &results {
					match res {
						Ok(_) => log::info!("[{:?}] Successfully marked compute request as fulfilled", acc.id),
						Err(e) => log::info!("[{:?}] Failed to mark compute request as fulfilled: {:?}", acc.id, e),
					}
				}
			} else {
				log::info!("❌ Could not acquire lock for marking compute request as fulfilled");
			};
		}

		// Function to send unsigned transaction for marking compute request as fulfilled
		pub fn update_miner_compute_request_offchain(
			node_id: Vec<u8>,
			request_id: u128,
			job_id: Vec<u8>,
			ip: Vec<u8>,
		) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"Compute::update_miner_compute_request_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);

			if let Ok(_guard) = lock.try_lock() {
				// Fetch signer accounts using AuthorityId
				let signer = Signer::<T, <T as Config>::AuthorityId>::all_accounts();

				// Check if there are any accounts and log them
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
				}

				let results = signer.send_unsigned_transaction(
					|account| {
						// Create a payload with all necessary data for the update_miner_compute_request call
						ComputeRequestAssignementPayload {
							node_id: node_id.clone(),
							request_id: request_id.clone(),
							job_id: job_id.clone(),
							ip: ip.clone(),
							public: account.public.clone(),
							_marker: PhantomData,
						}
					},
					|payload, _signature| {
						// Construct the call with the payload and signature
						Call::update_miner_compute_request {
							node_id: payload.node_id,
							request_id: payload.request_id,
							job_id: payload.job_id,
							ip: payload.ip,
						}
					},
				);

				// Process results with comprehensive logging
				for (acc, res) in &results {
					match res {
						Ok(_) => log::info!("[{:?}] Successfully marked compute request as fulfilled", acc.id),
						Err(e) => log::info!("[{:?}] Failed to mark compute request as fulfilled: {:?}", acc.id, e),
					}
				}
			} else {
				log::info!("❌ Could not acquire lock for marking compute request as fulfilled");
			};
		}

		/// Delete a specific stop request by user ID and plan ID
		pub fn delete_miner_compute_stop_request(
			node_id: &Vec<u8>,
			user_id: &T::AccountId,
			plan_id: &T::Hash
		) -> DispatchResult {
			// Retrieve existing stop requests for the node
			let mut stop_requests = MinerComputeStopRequests::<T>::get(node_id);
			
			// Find the index of the request to remove
			let request_index = stop_requests
				.iter()
				.position(|req| 
					req.user_id == *user_id && 
					req.plan_id == *plan_id 
				)
				.ok_or(Error::<T>::ComputeStopRequestNotFound)?;
			
			// Remove the specific stop request
			stop_requests.remove(request_index);
			
			// Update the storage
			if stop_requests.is_empty() {
				// If no requests remain, remove the entire entry
				MinerComputeStopRequests::<T>::remove(node_id);
			} else {
				// Otherwise, update the stop requests
				MinerComputeStopRequests::<T>::insert(node_id, stop_requests);
			}

			Ok(())
		}

		fn handle_request_assignment(node_id: Vec<u8>) -> Result<(), sp_runtime::offchain::http::Error> {
			let miner_requests = Self::get_unfulfilled_miner_compute_requests(node_id.clone());
		
			for miner_request in miner_requests {
				let compute_request = ComputeRequests::<T>::iter()
					.find_map(|(_owner, requests)| {
						requests
							.iter()
							.find(|req| req.request_id == miner_request.request_id)
							.cloned()
					});

				let mut vm_available_ips = IpPallet::<T>::vm_available_ips();
				if let Some(ip) = vm_available_ips.pop() {
					if let Some(compute_request) = compute_request {
						// Parse technical description and create VM
						match serde_json::from_slice::<TechnicalDescription>(&compute_request.plan_technical_description) {
							Ok(tech_desc) => {
								let url = "http://localhost:3030/start-vm";
								let json_payload = if compute_request.cloud_init_cid.is_none() {
									serde_json::json!({
										"memory": format!("{}M", tech_desc.ram_gb * 1024),
										"vcpus": format!("{}", tech_desc.cpu_cores),
										"disk_size": format!("{}Gi", tech_desc.storage_gb),
										"is_sev_enabled": tech_desc.is_sev_enabled,
										"inbound_bandwidth": tech_desc.inbound_bandwidth,
										"outbound_bandwidth": tech_desc.outbound_bandwidth,
										"gpu": tech_desc.gpu,
										"gpu_type": tech_desc.gpu_type,
										"image_url": String::from_utf8_lossy(&compute_request.selected_image.image_url).to_string(),
										"os_variant": String::from_utf8_lossy(&compute_request.selected_image.name).to_string(),
										"assigned_ip" : String::from_utf8_lossy(&ip).to_string(),
										"substrate_request_id": format!("{}", miner_request.request_id),
										"end_user_account": String::from_utf8_lossy(&compute_request.owner.encode()).to_string(),
									})
								} else {
									serde_json::json!({
										"memory": format!("{}M", tech_desc.ram_gb * 1024),
										"vcpus": format!("{}", tech_desc.cpu_cores),
										"disk_size": format!("{}Gi", tech_desc.storage_gb),
										"is_sev_enabled": tech_desc.is_sev_enabled,
										"inbound_bandwidth": tech_desc.inbound_bandwidth,
										"outbound_bandwidth": tech_desc.outbound_bandwidth,
										"gpu": tech_desc.gpu,
										"gpu_type": tech_desc.gpu_type,
										"image_url": String::from_utf8_lossy(&compute_request.selected_image.image_url).to_string(),
										"os_variant": String::from_utf8_lossy(&compute_request.selected_image.name).to_string(),
										"cloud_init_path": String::from_utf8_lossy(&compute_request.cloud_init_cid.unwrap()).to_string(),
										"assigned_ip" : String::from_utf8_lossy(&ip).to_string(),
										"substrate_request_id": format!("{}", miner_request.request_id),
										"end_user_account": String::from_utf8_lossy(&compute_request.owner.encode()).to_string(),
									})
								};
	
								let json_string = json_payload.to_string();
								let content_length = json_string.len();
	
								log::info!("JSON Payload: {}", json_string);
	
								let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(100_000));
								let request = sp_runtime::offchain::http::Request::post(url, vec![json_string]);
	
								let pending = request
									.add_header("Content-Type", "application/json")
									.add_header("Content-Length", &content_length.to_string())
									.deadline(deadline)
									.send()
									.map_err(|err| {
										log::error!("❌ Error making Request: {:?}", err);
										sp_runtime::offchain::http::Error::IoError
									})?;
	
								let response = pending
									.try_wait(deadline)
									.map_err(|err| {
										log::error!("❌ Error getting Response: {:?}", err);
										sp_runtime::offchain::http::Error::DeadlineReached
									})??;
	
								if response.code != 202 {
									log::error!(
										"Unexpected status code: {}, VM creation failed. Response body: {:?}",
										response.code, response
									);
									return Err(sp_runtime::offchain::http::Error::Unknown);
								}
	
								let response_body = response.body();
								let response_body_vec = response_body.collect::<Vec<u8>>();
								let response_str = core::str::from_utf8(&response_body_vec)
									.map_err(|_| sp_runtime::offchain::http::Error::Unknown)?;
								
								match serde_json::from_str::<serde_json::Value>(response_str) {
									Ok(json_response) => {
										if let (Some(job_id), Some(_name), Some(_status)) = (
											json_response.get("job_id").and_then(|v| v.as_str()),
											json_response.get("name").and_then(|v| v.as_str()),
											json_response.get("status").and_then(|v| v.as_str())
										) {
											Self::update_miner_compute_request_offchain(
												node_id.clone(),
												miner_request.request_id,
												job_id.as_bytes().to_vec(),
												ip.clone(),
											);
										} else {
											log::warn!("Missing expected fields in VM creation response");
										}
									},
									Err(e) => {
										log::error!("Failed to parse VM creation response JSON: {:?}", e);
										return Err(sp_runtime::offchain::http::Error::Unknown);
									}
								}
							}
							Err(e) => {
								log::error!(
									"Failed to parse technical description for request ID {}: {:?}",
									miner_request.request_id,
									e
								);
							}
						}
					}
				}
				else{
					log::info!("No available IP addresses to allocate VM");
				}
			}
			Ok(())
		}		

		/// Generic function to perform VM operations via HTTP request
		fn perform_vm_operation(
			operation_type: &str,
			job_id: Vec<u8>,
			timeout_ms: u64
		) -> Result<(), sp_runtime::offchain::http::Error> {

			let url = format!("http://localhost:3030/{}-vm/{}", operation_type, String::from_utf8_lossy(&job_id) );
			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(timeout_ms));
			let request = sp_runtime::offchain::http::Request::get(url.as_str());

			let pending = request
				.add_header("Content-Type", "application/json")
				.deadline(deadline)
				.send()
				.map_err(|err| {
					log::error!("❌ Error making {} VM Request: {:?}", operation_type, err);
					sp_runtime::offchain::http::Error::IoError
				})?;

			let response = pending
				.try_wait(deadline)
				.map_err(|err| {
					log::error!("❌ Error getting {} VM Response: {:?}", operation_type, err);
					sp_runtime::offchain::http::Error::DeadlineReached
				})??;

			if response.code != 200 {
				log::error!(
					"Unexpected status code: {}, {} VM operation failed. Response body: {:?}",
					response.code, operation_type, response
				);
				return Err(sp_runtime::offchain::http::Error::Unknown);
			}

			Ok(())
		}

		// stop the vm
		fn handle_stop_request_assignment(node_id: Vec<u8>) -> Result<(), sp_runtime::offchain::http::Error> {
			let miner_requests = Self::miner_compute_stop_requests(node_id.clone());
		
			for miner_request in miner_requests {
				// if not stopped then stop the vm 
				if !miner_request.fullfilled {
					// Retrieve the compute request status
					if let Some(compute_request) = Self::find_compute_request_by_id(miner_request.request_id) {
						if compute_request.1.status == ComputeRequestStatus::Running {
							// Use the new generic function to stop VM
							Self::perform_vm_operation("stop", miner_request.job_id.unwrap(), 10_000)?;
			
							// Call mark_miner_compute_stop_request_fulfilled after successful VM creation
							Self::mark_miner_compute_stop_request_fulfilled_offchain(
								node_id.clone(),
								miner_request.request_id,
							);
						}else{
							log::info!("VM is not Running");
						}
					}
				} else {
					log::info!("VM is already stopped");
				}
			}
			Ok(())
		}

		// boot the vm
		fn handle_boot_request_assignment(node_id: Vec<u8>) -> Result<(), sp_runtime::offchain::http::Error> {
			let miner_requests = Self::miner_compute_boot_requests(node_id.clone());
		
			for miner_request in miner_requests {
				if !miner_request.fullfilled {
					// Retrieve the compute request status
					if let Some(compute_request) = Self::find_compute_request_by_id(miner_request.request_id) {
						// Check if the request status is already stopped
						if compute_request.1.status == ComputeRequestStatus::Stopped {
							// Use the new generic function to stop VM
							Self::perform_vm_operation("boot", miner_request.job_id.unwrap(), 10_000)?;
			
							Self::call_submit_compute_boot_request_fulfillment(
								miner_request.miner_node_id,
								miner_request.request_id,
							);
						}else{
							log::info!("VM is not stopped");
						}
					}
				}else{
					log::info!("VM is already Booted");
				}
			}
			Ok(())
		}		

		// reboot the vm
		fn handle_reboot_request_assignment(node_id: Vec<u8>) -> Result<(), sp_runtime::offchain::http::Error> {
			let miner_requests = Self::miner_compute_reboot_requests(node_id.clone());
		
			for miner_request in miner_requests {
				// if not stopped then stop the vm 
				if !miner_request.fullfilled {
					// Use the new generic function to stop VM
					Self::perform_vm_operation("reboot", miner_request.job_id.unwrap(), 10_000)?;
	
					Self::call_submit_compute_reboot_request_fulfillment(
						miner_request.miner_node_id,
						miner_request.request_id,
					);
				}
			}
			Ok(())
		}
		
		// fn handle_resize_request_assignment(node_id: Vec<u8>) -> Result<(), sp_runtime::offchain::http::Error> {
		// 	let miner_requests = Self::miner_compute_resize_requests(node_id.clone());
		
		// 	for miner_request in miner_requests {
		// 		// if not stopped then stop the vm 

		// 		if !miner_request.fullfilled {
		// 			let url = "http://localhost:3030/resize-disk";
		// 			let json_payload =
		// 				serde_json::json!({
		// 					"vm_name": String::from_utf8_lossy(&miner_request.job_id.unwrap()),
		// 					"new_size": format!("{}G", miner_request.resize_gbs),

		// 				});

		// 			let json_string = json_payload.to_string();
		// 			let content_length = json_string.len();

		// 			log::info!("JSON Resize Payload: {}", json_string);

		// 			let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(10_000));
		// 			let request = sp_runtime::offchain::http::Request::post(url, vec![json_string]);

		// 			let pending = request
		// 				.add_header("Content-Type", "application/json")
		// 				.add_header("Content-Length", &content_length.to_string())
		// 				.deadline(deadline)
		// 				.send()
		// 				.map_err(|err| {
		// 					log::error!("❌ Error making Request: {:?}", err);
		// 					sp_runtime::offchain::http::Error::IoError
		// 				})?;

		// 			let response = pending
		// 				.try_wait(deadline)
		// 				.map_err(|err| {
		// 					log::error!("❌ Error getting Response: {:?}", err);
		// 					sp_runtime::offchain::http::Error::DeadlineReached
		// 				})??;

		// 			if response.code != 200 {
		// 				log::error!(
		// 					"Unexpected status code: {}, VM creation failed. Response body: {:?}",
		// 					response.code, response
		// 				);
		// 				return Err(sp_runtime::offchain::http::Error::Unknown);
		// 			}							
	
		// 			Self::call_submit_compute_resize_request_fulfillment(
		// 				miner_request.miner_node_id,
		// 				miner_request.request_id,
		// 			);
		// 		}
		// 	}
		// 	Ok(())
		// }
		
		// fn handle_pending_nebula_requests(node_id: Vec<u8>) -> Result<(), sp_runtime::offchain::http::Error> {
		// 	let miner_requests = Self::get_pending_nebula_requests(node_id.clone());

		// 	for miner_request in miner_requests {
		// 		let url = format!(
		// 			"http://localhost:3030/get-nebula-ip"
		// 		);
		// 		log::info!("URL for vnc request is : {}", url);
		// 		let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(10_000));
		// 		let request = sp_runtime::offchain::http::Request::get(url.as_str());
					
		// 		let pending = request
		// 			.add_header("Content-Type", "application/json")
		// 			.deadline(deadline)
		// 			.send()
		// 			.map_err(|err| {
		// 				log::error!("❌ Error making status check VM Request: {:?}", err);
		// 				sp_runtime::offchain::http::Error::IoError
		// 			})?;
	
		// 		let response = pending
		// 			.try_wait(deadline)
		// 			.map_err(|err| {
		// 				log::error!("❌ Error getting VM status Response: {:?}", err);
		// 				sp_runtime::offchain::http::Error::DeadlineReached
		// 			})??;
	
		// 		if response.code != 200 {
		// 			log::error!(
		// 				"Unexpected status code: {}, VM operation failed. Response body: {:?}",
		// 				response.code, response
		// 			);
		// 			return Err(sp_runtime::offchain::http::Error::Unknown);
		// 		}

		// 		let response_body = response.body();
		// 		let response_body_vec = response_body.collect::<Vec<u8>>();
		// 		let response_str = core::str::from_utf8(&response_body_vec)
		// 			.map_err(|_| sp_runtime::offchain::http::Error::Unknown)?;
							
		// 		match serde_json::from_str::<serde_json::Value>(response_str) {
		// 			Ok(json_response) => {
		// 				if let Some(hypervisor_ip) = json_response.get("nebula_ip").and_then(|v| v.as_str()) {
		// 					// Log the Nebula IP
		// 					log::info!(
		// 						"Nebula IP Retrieved: {}",
		// 						hypervisor_ip
		// 					);
		// 				    // Convert Nebula IP to Vec<u8>
		// 					let hypervisor_ip_bytes = hypervisor_ip.as_bytes().to_vec();
							
		// 					// You can add additional processing here if needed
		// 					// For example, storing the Nebula IP or using it in further logic
		// 					Self::update_miner_compute_request_hypervisor_ip_offchain(
		// 						node_id.clone(), 
		// 						miner_request.request_id, 
		// 						hypervisor_ip_bytes
		// 					);
		// 				} else {
		// 					log::warn!("Missing 'hypervisor_ip' field in response");
		// 				}
		// 			},
		// 			Err(e) => {
		// 				log::error!("Failed to parse Nebula IP response JSON: {:?}", e);
		// 				return Err(sp_runtime::offchain::http::Error::Unknown);
		// 			}
		// 		}
		// 	}
		// 	Ok(())
		// }

		fn handle_pending_vnc_requests(node_id: Vec<u8>) -> Result<(), sp_runtime::offchain::http::Error> {
			let miner_requests = Self::get_pending_vnc_requests(node_id.clone());

			for miner_request in miner_requests {
				let url = format!(
					"http://localhost:3030/vm-vnc-port/{}",
					String::from_utf8_lossy(&miner_request.job_id.clone().unwrap())
				);
				let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(10_000));
				let request = sp_runtime::offchain::http::Request::get(url.as_str());
	
				
				let pending = request
					.add_header("Content-Type", "application/json")
					.deadline(deadline)
					.send()
					.map_err(|err| {
						log::error!("❌ Error making status check VM Request: {:?}", err);
						sp_runtime::offchain::http::Error::IoError
					})?;
	
				let response = pending
					.try_wait(deadline)
					.map_err(|err| {
						log::error!("❌ Error getting VM status Response: {:?}", err);
						sp_runtime::offchain::http::Error::DeadlineReached
					})??;
	
				if response.code != 200 {
					log::error!(
						"Unexpected status code: {}, VM operation failed. Response body: {:?}",
						response.code, response
					);
					return Err(sp_runtime::offchain::http::Error::Unknown);
				}

				let response_body = response.body();
				let response_body_vec = response_body.collect::<Vec<u8>>();
				let response_str = core::str::from_utf8(&response_body_vec)
					.map_err(|_| sp_runtime::offchain::http::Error::Unknown)?;
							
				match serde_json::from_str::<serde_json::Value>(response_str) {
					Ok(json_response) => {
						if let (Some(name), Some(vnc_port)) = (
							json_response.get("vm_name").and_then(|v| v.as_str()),
							json_response.get("vnc_port").and_then(|v| v.as_u64())
						) {
							// Log the VNC port
							log::info!(
								"VNC Port Retrieved - VM Name: {}, VNC Port: {}",
								name, vnc_port
							);
							Self::update_miner_compute_request_vnc_offchain(
								node_id.clone(),
								miner_request.request_id,
								vnc_port,
								miner_request.job_id.unwrap()
							);
						} else {
							log::warn!("Missing expected fields in VNC port response");
						}
					},
					Err(e) => {
						log::error!("Failed to parse VNC port response JSON: {:?}", e);
						return Err(sp_runtime::offchain::http::Error::Unknown);
					}
				}
			}
			Ok(())
		}
		
		fn handle_pending_job_requests(node_id: Vec<u8>) -> Result<(), sp_runtime::offchain::http::Error> {
			let miner_requests = Self::get_pending_job_requests(node_id.clone());
			for miner_request in miner_requests {
				let url = format!(
					"http://localhost:3030/vm-status/{}",
					String::from_utf8_lossy(&miner_request.job_id.unwrap())
				);
				let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(10_000));
				let request = sp_runtime::offchain::http::Request::get(url.as_str());
	
				let pending = request
					.add_header("Content-Type", "application/json")
					.deadline(deadline)
					.send()
					.map_err(|err| {
						log::error!("❌ Error making status check VM Request: {:?}", err);
						sp_runtime::offchain::http::Error::IoError
					})?;
	
				let response = pending
					.try_wait(deadline)
					.map_err(|err| {
						log::error!("❌ Error getting VM status Response: {:?}", err);
						sp_runtime::offchain::http::Error::DeadlineReached
					})??;
	
				if response.code != 200 {
					log::error!(
						"Unexpected status code: {}, VM operation failed. Response body: {:?}",
						response.code, response
					);
					return Err(sp_runtime::offchain::http::Error::Unknown);
				}

				let response_body = response.body();
				let response_body_vec = response_body.collect::<Vec<u8>>();
				let response_str = core::str::from_utf8(&response_body_vec)
					.map_err(|_| sp_runtime::offchain::http::Error::Unknown)?;
							
				match serde_json::from_str::<serde_json::Value>(response_str) {
					Ok(json_response) => {
						if let (Some(job_id), Some(name), Some(status)) = (
							json_response.get("job_id").and_then(|v| v.as_str()),
							json_response.get("name").and_then(|v| v.as_str()),
							json_response.get("status").and_then(|v| v.as_str())
						) {
							match status {
								"Completed" => {
									// Mark the request as fulfilled
									Self::mark_compute_request_fulfilled_offchain(node_id.clone(), miner_request.request_id);
								},
								"InProgress"=>{
									log::warn!(
										"Unknown VM Job Status - Job ID: {}, Name: {}, Status: {}",
										job_id, name, status
									);
								},
								"Pending"=>{
									log::warn!(
										"Unknown VM Job Status - Job ID: {}, Name: {}, Status: {}",
										job_id, name, status
									);
								},
								"Failed" => {
									let error_msg = json_response
										.get("error")
										.and_then(|v| v.as_str())
										.unwrap_or("Unknown error");
									
									// Update the status to Pending again so it can be reassigned to another miner 
									Self::call_handle_miner_compute_request_failure(node_id.clone(), miner_request.request_id, error_msg.as_bytes().to_vec());
								},
								_ => {
									let error_msg = json_response
										.get("error")
										.and_then(|v| v.as_str())
										.unwrap_or("Unknown error");
									
									// Update the status to Pending again so it can be reassigned to another miner 
									Self::call_handle_miner_compute_request_failure(node_id.clone(), miner_request.request_id, error_msg.as_bytes().to_vec());
								}
							}
						} else {
							log::warn!("Missing expected fields in VM status response");
						}
					},
					Err(e) => {
						log::error!("Failed to parse VM status response JSON: {:?}", e);
						return Err(sp_runtime::offchain::http::Error::Unknown);
					}
				}
			}
			Ok(())
		}

		fn handle_delete_request_assignment(node_id: Vec<u8>) -> Result<(), sp_runtime::offchain::http::Error> {
			let miner_requests = Self::miner_compute_deletion_requests(node_id.clone());
		
			for miner_request in miner_requests {
				let job_id = miner_request.job_id.unwrap();
				// Use the new generic function to delete VM
				Self::perform_vm_operation("delete", job_id.clone(), 10_000)?;

				// Use job ID to generate VM name consistently
				let vm_name = format!("vm-{}", String::from_utf8_lossy(&job_id).split('-').next().unwrap());
			
				Self::call_submit_compute_deletion_request(
					node_id.clone(),
					miner_request.request_id,
					vm_name.as_bytes().to_vec().clone(),
				);
			}
			Ok(())
		}

		pub fn add_miner_compute_stop_request(
			user_id: T::AccountId,
			plan_id: T::Hash
		) -> DispatchResult {
			// Attempt to find the request details and minner node id
			let (request_id, miner_node_id, job_id) = Self::find_miner_compute_request(user_id.clone(), plan_id.clone())
			.ok_or(Error::<T>::ComputeRequestNotFound)?;
			
			// Retrieve existing stop requests for the user
			let mut user_stop_requests = MinerComputeStopRequests::<T>::get(&miner_node_id);
			
			// Check if a request with the same plan_id already exists
			let request_exists = user_stop_requests.iter()
				.any(|req| req.plan_id == plan_id && req.user_id == user_id && !req.fullfilled);

			// Only add if no existing unfulfilled request for this plan exists
			if !request_exists {
				let stop_request = MinerComputeStopRequest {
					miner_node_id: miner_node_id.clone(),
					request_id,
					job_id,
					user_id: user_id.clone(),
					plan_id,
					created_at: <frame_system::Pallet<T>>::block_number(),
					fullfilled: false,
				};
				
				// Add the new stop request
				user_stop_requests.push(stop_request);

				// Update storage
				MinerComputeStopRequests::<T>::insert(&miner_node_id, user_stop_requests);
			} else {
				log::info!(
					"Compute stop request for plan {:?} already exists",
					plan_id
				);
			}

			Ok(())
		}

		/// Add a new compute boot request
		pub fn add_miner_compute_boot_request(
			user_id: T::AccountId,
			plan_id: T::Hash
		) -> DispatchResult {
			// Attempt to find the request details and minner node id
			let (request_id, miner_node_id, job_id) = Self::find_miner_compute_request(user_id.clone(), plan_id.clone())
				.ok_or(Error::<T>::ComputeRequestNotFound)?;
			
			// Retrieve existing boot requests for the miner
			let mut user_boot_requests = MinerComputeBootRequests::<T>::get(&miner_node_id);
			
			// Check if a request with the same plan_id already exists
			let request_exists = user_boot_requests.iter()
				.any(|req| req.plan_id == plan_id && req.user_id == user_id && !req.fullfilled);

			// Only add if no existing unfulfilled request for this plan exists
			if !request_exists {
				let boot_request = MinerComputeBootRequest {
					miner_node_id: miner_node_id.clone(),
					request_id,
					job_id,
					user_id: user_id.clone(),
					plan_id,
					created_at: <frame_system::Pallet<T>>::block_number(),
					fullfilled: false,
				};
				
				// Add the new boot request
				user_boot_requests.push(boot_request);

				// Update storage
				MinerComputeBootRequests::<T>::insert(&miner_node_id, user_boot_requests);
			} else {
				log::info!(
					"Compute boot request for plan {:?} already exists",
					plan_id
				);
			}

			Ok(())
		}

		/// Add a new compute resize request
		pub fn add_miner_compute_image_resize_request(
			user_id: T::AccountId,
			plan_id: T::Hash,
			resize_gbs: u32
		) -> DispatchResult {
			// Attempt to find the request details and minner node id
			let (request_id, miner_node_id, job_id) = Self::find_miner_compute_request(user_id.clone(), plan_id.clone())
				.ok_or(Error::<T>::ComputeRequestNotFound)?;
			
			// Retrieve existing resize requests for the miner
			let mut user_resize_requests = MinerComputeResizeRequests::<T>::get(&miner_node_id);
			
			// Check if a request with the same plan_id already exists
			let request_exists = user_resize_requests.iter()
				.any(|req| req.plan_id == plan_id && req.user_id == user_id && !req.fullfilled);

			// Only add if no existing unfulfilled request for this plan exists
			if !request_exists {
				let resize_request = MinerComputeResizeRequest {
					miner_node_id: miner_node_id.clone(),
					request_id,
					job_id,
					user_id: user_id.clone(),
					plan_id,
					created_at: <frame_system::Pallet<T>>::block_number(),
					resize_gbs,
					fullfilled: false,
				};
				
				// Add the new resize request
				user_resize_requests.push(resize_request);

				// Update storage
				MinerComputeResizeRequests::<T>::insert(&miner_node_id, user_resize_requests);
			} else {
				log::info!(
					"Compute resize request for plan {:?} already exists",
					plan_id
				);
			}

			Ok(())
		}
		
		/// Helper function to get a compute request by its request ID
		pub fn get_compute_request_by_id(request_id: u128) -> Option<ComputeRequest<T::AccountId, BlockNumberFor<T>, T::Hash>> {
			ComputeRequests::<T>::iter()
				.find_map(|(_owner, requests)| {
					requests
						.iter()
						.find(|req| req.request_id == request_id)
						.cloned()
				})
		}

		/// Delete a specific boot request
		pub fn delete_miner_compute_boot_request(
			node_id: &Vec<u8>,
			user_id: &T::AccountId,
			plan_id: &T::Hash
		) -> DispatchResult {
			// Retrieve existing boot requests for the node
			let mut boot_requests = MinerComputeBootRequests::<T>::get(node_id);
			
			// Find the index of the request to remove
			let request_index = boot_requests
				.iter()
				.position(|req| 
					req.user_id == *user_id && 
					req.plan_id == *plan_id && 
					!req.fullfilled
				)
				.ok_or(Error::<T>::ComputeBootRequestNotFound)?;
			
			// Remove the specific boot request
			boot_requests.remove(request_index);
			
			// Update the storage
			if boot_requests.is_empty() {
				// If no requests remain, remove the entire entry
				MinerComputeBootRequests::<T>::remove(node_id);
			} else {
				// Otherwise, update the boot requests
				MinerComputeBootRequests::<T>::insert(node_id, boot_requests);
			}

			Ok(())
		}

		/// Check if a boot request exists for a specific plan
		pub fn compute_boot_request_exists(
			user_id: &T::AccountId,
			plan_id: &T::Hash
		) -> bool {
			// Attempt to find the miner account id for the compute request
			let miner_node_id = match Self::find_miner_compute_request(user_id.clone(), plan_id.clone()) {
				Some((_, miner_id, _)) => miner_id,
				None => return false
			};
			
			// Retrieve existing boot requests for the miner
			let user_boot_requests = MinerComputeBootRequests::<T>::get(&miner_node_id);
			
			// Check if an unfulfilled boot request exists
			user_boot_requests.iter()
				.any(|req| req.plan_id == *plan_id && !req.fullfilled)
		}

		/// Get an existing unfulfilled stop request for a specific plan
		pub fn get_existing_compute_stop_request(
			user_id: &T::AccountId,
			plan_id: &T::Hash
		) -> Option<MinerComputeStopRequest<BlockNumberFor<T>, T::Hash, T::AccountId>> {
			// Attempt to find the miner account id for the compute request
			let miner_node_id = match Self::find_miner_compute_request(user_id.clone(), plan_id.clone()) {
				Some((_, miner_id, _)) => miner_id,
				None => return None
			};
			
			// Retrieve existing stop requests for the miner
			let user_stop_requests = MinerComputeStopRequests::<T>::get(&miner_node_id);
			
			// Find and return the first unfulfilled stop request matching the plan_id
			user_stop_requests.into_iter()
				.find(|req| req.plan_id == *plan_id && !req.fullfilled)
		}
		
		/// Check if a stop request exists for a specific plan
		pub fn compute_stop_request_exists(
			user_id: &T::AccountId,
			plan_id: &T::Hash
		) -> bool {
			Self::get_existing_compute_stop_request(user_id, plan_id).is_some()
		}


		/// Update the status of a specific compute request
		fn update_compute_request_status(
			request_id: u128,
			new_status: ComputeRequestStatus,
		) -> DispatchResult {
			let mut request_found = false;
			
			ComputeRequests::<T>::iter().for_each(|(owner, requests)| {
				if let Some(request_index) = requests.iter().position(|r| r.request_id == request_id) {
					ComputeRequests::<T>::mutate(owner.clone(), |requests| {
						if let Some(request) = requests.get_mut(request_index) {
							request.status = new_status.clone();
							request_found = true;
						}
					});
				}
			});

			ensure!(request_found, "Compute request not found");
			Ok(())
		}

		// Helper method to get current timestamp
		fn get_current_timestamp() -> u64 {
			// Use block number as a base for uniqueness
			let block_number = <frame_system::Pallet<T>>::block_number().saturated_into::<u64>();
			
			// Use a simple method to generate a unique value
			block_number.wrapping_mul(1000) + 
			(block_number % 1000) + 
			(block_number & 0xFF)
		}

		/// if found, otherwise returns None
		pub fn find_compute_request_by_id(
			request_id: u128
		) -> Option<(T::AccountId, ComputeRequest<T::AccountId, BlockNumberFor<T>, T::Hash>)> {
			ComputeRequests::<T>::iter()
				.find_map(|(owner, requests)| {
					requests.iter()
						.find(|request| request.request_id == request_id)
						.map(|found_request| (owner.clone(), found_request.clone()))
				})
		}

		/// Add a miner compute reboot request for a specific plan
		pub fn add_miner_compute_reboot_request(
			user_id: T::AccountId, 
			plan_id: T::Hash
		) -> DispatchResult {
			// Attempt to find the request details and minner node id
			let (request_id, miner_node_id, job_id) = Self::find_miner_compute_request(user_id.clone(), plan_id.clone())
				.ok_or(Error::<T>::ComputeRequestNotFound)?;
			
			// Retrieve existing reboot requests for the miner
			let mut user_reboot_requests = MinerComputeRebootRequests::<T>::get(&miner_node_id);
			
			// Check if a request with the same plan_id already exists
			let request_exists = user_reboot_requests.iter()
				.any(|req| req.plan_id == plan_id && req.user_id == user_id && !req.fullfilled);

			// Only add if no existing unfulfilled request for this plan exists
			if !request_exists {
				let reboot_request = MinerComputeRebootRequest {
					miner_node_id: miner_node_id.clone(),
					request_id,
					job_id,	
					user_id: user_id.clone(),
					plan_id,
					created_at: <frame_system::Pallet<T>>::block_number(),
					fullfilled: false,
				};
				
				// Add the new boot request
				user_reboot_requests.push(reboot_request);

				// Update storage
				MinerComputeRebootRequests::<T>::insert(&miner_node_id, user_reboot_requests);
			} else {
				log::info!(
					"Compute reboot request for plan {:?} already exists",
					plan_id
				);
			}
			Ok(())
		}


		// // Retrieves all VM details for a given user
		// pub fn get_user_vms(account: T::AccountId) -> Vec<UserVmDetails<T::AccountId, BlockNumberFor<T>, T::Hash>> {
		// 	// Retrieve all compute requests for the user
		// 	let compute_requests = Self::compute_requests(account.clone());
			
		// 	// Collect VM details
		// 	compute_requests.into_iter().map(|compute_request| {
		// 		// Find the corresponding miner compute request
		// 		let miner_details = MinerComputeRequests::<T>::iter()
		// 			.find_map(|(_, miner_compute_requests)| {
		// 				miner_compute_requests.iter()
		// 					.find(|mcr| mcr.request_id == compute_request.request_id)
		// 					.cloned()
		// 			});
				
		// 		// Construct user VM details
		// 		UserVmDetails {
		// 			request_id: compute_request.request_id,
		// 			status: compute_request.status,
		// 			plan_id: compute_request.plan_id,
		// 			created_at: compute_request.created_at,
		// 			miner_node_id: miner_details.as_ref().map(|md| md.miner_node_id.clone()),
		// 			miner_account_id: miner_details.as_ref().map(|md| md.miner_account_id.clone()),
		// 			hypervisor_ip: miner_details.as_ref().and_then(|md| md.hypervisor_ip.clone()),
		// 			vnc_port: miner_details.as_ref().and_then(|md| md.vnc_port),
		// 			ip_assigned: miner_details.as_ref().and_then(|md| md.ip_assigned.clone()),
		// 			error: miner_details.as_ref().and_then(|md| md.fail_reason.clone()),
		// 			is_fulfilled: miner_details.map(|md| md.fullfilled).unwrap_or(false),
		// 		}
		// 	}).collect()
		// }	
	}

	#[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		
        fn offchain_worker(block_number: BlockNumberFor<T>) {
            // Only proceed if the block number is divisible by the configured interval
            if block_number % T::OffchainWorkerInterval::get().into() != 0u32.into() {
                return;
            }

			match UtilsPallet::<T>::fetch_node_id() {
				Ok(node_id) => {
					let node_info = RegistrationPallet::<T>::get_node_registration_info(node_id.clone());
					match node_info {
						Some(node_info) => {
							if node_info.node_type == NodeType::ComputeMiner {
								match Self::handle_request_assignment(node_info.node_id.clone()) {
									Ok(_) => log::info!("Request assignment handled successfully"),
									Err(e) => log::error!("Error in request assignment handling: {:?}", e),
								}

								match Self::handle_pending_job_requests(node_info.node_id.clone()) {
									Ok(_) => log::info!("Pending job requests handled successfully"),
									Err(e) => log::error!("Error in pending job request handling: {:?}", e),
								}

								match Self::handle_pending_vnc_requests(node_info.node_id.clone()) {
									Ok(_) => log::info!("Pending vnc requests handled successfully"),
									Err(e) => log::error!("Error in pending vnc request handling: {:?}", e),
								}

								// match Self::handle_pending_nebula_requests(node_info.node_id.clone()) {
								// 	Ok(_) => log::info!("Pending nebula requests handled successfully"),
								// 	Err(e) => log::error!("Error in pending nebula request handling: {:?}", e),
								// }

								// handle delete requests of minners 
								match Self::handle_delete_request_assignment(node_info.node_id.clone()) {
									Ok(_) => log::info!("Request deletion handled successfully"),
									Err(e) => log::error!("Error in delete request handling: {:?}", e),
								}

								// handle stop requests of minners 
								match Self::handle_stop_request_assignment(node_info.node_id.clone()) {
									Ok(_) => log::info!("Stop Request handled successfully"),
									Err(e) => log::error!("Error in stop request handling: {:?}", e),
								}

								// handle boot requests of minners 
								match Self::handle_boot_request_assignment(node_info.node_id.clone()) {
									Ok(_) => log::info!("boot Request handled successfully"),
									Err(e) => log::error!("Error in boot request handling: {:?}", e),
								}

								// handle reboot requests of minners 
								match Self::handle_reboot_request_assignment(node_info.node_id.clone()) {
									Ok(_) => log::info!("reboot Request handled successfully"),
									Err(e) => log::error!("Error in reboot request handling: {:?}", e),
								}

								// // handle resize requests of minners 
								// match Self::handle_resize_request_assignment(node_info.node_id.clone()) {
								// 	Ok(_) => log::info!("resize Request handled successfully"),
								// 	Err(e) => log::error!("Error in resize request handling: {:?}", e),
								// }
							}
						}
						None => {}
					}
				}
				Err(_) => {}
			}
        }
    }
}
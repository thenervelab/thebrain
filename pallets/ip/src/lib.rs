// We make sure this pallet uses `no_std` for compiling to Wasm.
#![cfg_attr(not(feature = "std"), no_std)]
pub use types::*;

mod types;

// Re-export pallet items so that they can be accessed from the crate namespace.
pub use pallet::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;
pub mod weights;
pub use weights::*;

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;
	use sp_std::vec::Vec;
	use sp_std::vec;

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config {
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
		/// The block interval for offchain worker operations
		type IpReleasePeriod: Get<u64>;
	}

	/// Separate IP pools for each role type
	#[pallet::storage]
	#[pallet::getter(fn available_hypervisor_ips)]
	pub(super) type AvailableHypervisorIps<T: Config> = StorageValue<_, Vec<Vec<u8>>, ValueQuery>;
	
	#[pallet::storage]
	#[pallet::getter(fn available_client_ips)]
	pub(super) type AvailableClientIps<T: Config> = StorageValue<_, Vec<Vec<u8>>, ValueQuery>;
	
	#[pallet::storage]
	#[pallet::getter(fn available_storage_miner_ips)]
	pub(super) type AvailableStorageMinerIps<T: Config> = StorageValue<_, Vec<Vec<u8>>, ValueQuery>;

    /// Pool of available IP addresses
	#[pallet::storage]
	#[pallet::getter(fn vm_available_ips)]
	pub(super) type VmAvailableIps<T: Config> = StorageValue<_, Vec<Vec<u8>>, ValueQuery>;
	
	#[pallet::storage]
	#[pallet::getter(fn assigned_ips)]
	pub(super) type AssignedIps<T: Config> = StorageValue<_, Vec<Vec<u8>>, ValueQuery>;
	
	#[pallet::storage]
	#[pallet::getter(fn ip_to_vm)]
	pub(super) type IpToVm<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, Vec<u8>>;	

	#[pallet::storage]
	#[pallet::getter(fn vm_to_ip)]
	pub(super) type VmToIp<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, Vec<u8>>;

	// Storage for IP Release Requests
	#[pallet::storage]
	#[pallet::getter(fn ip_release_requests)]
	pub type IpReleaseRequests<T: Config> = StorageValue<
		_,
		Vec<IpReleaseRequest<BlockNumberFor<T>, T::AccountId>>,
		ValueQuery
	>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		IpAssigned { vm_uuid: Vec<u8>, ip: Vec<u8> },
		IpReturned { vm_uuid: Vec<u8>, ip: Vec<u8> },
		IpRetrieved { vm_uuid: Vec<u8>, ip: Vec<u8> },
		IpAdded {ip: Vec<u8>},
	}

	#[pallet::error]
	pub enum Error<T> {
		NoAvailableIp,
		VmAlreadyHasIp,
		VmHasNoIp,
		IpAlreadyExists,
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		#[pallet::call_index(0)]
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn add_available_ip(
			origin: OriginFor<T>,
			ip: Vec<u8>, // IP address as a vector of bytes
		) -> DispatchResult {
			// Ensure the caller has the appropriate permission
			let _who = ensure_root(origin)?;
		
			// Retrieve the current list of available IPs
			let mut vm_available_ips = VmAvailableIps::<T>::get();
		
			// Ensure the IP is not already in the list
			ensure!(
				!vm_available_ips.contains(&ip),
				Error::<T>::IpAlreadyExists
			);
		
			// Add the new IP to the list
			vm_available_ips.push(ip.clone());
			VmAvailableIps::<T>::put(vm_available_ips);
		
			// Emit an event
			Self::deposit_event(Event::IpAdded { ip });
		
			Ok(())
		}

		#[pallet::call_index(1)]
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn add_available_hypervisor_ip(
			origin: OriginFor<T>,
			ip: Vec<u8>, // IP address as a vector of bytes
		) -> DispatchResult {
			// Ensure the caller has the appropriate permission
			let _who = ensure_root(origin)?;
		
			// Retrieve the current list of available Hypervisor IPs
			let mut available_hypervisor_ips = AvailableHypervisorIps::<T>::get();
		
			// Ensure the IP is not already in the list
			ensure!(
				!available_hypervisor_ips.contains(&ip),
				Error::<T>::IpAlreadyExists
			);
		
			// Add the new IP to the list
			available_hypervisor_ips.push(ip.clone());
			AvailableHypervisorIps::<T>::put(available_hypervisor_ips);
		
			// Emit an event
			Self::deposit_event(Event::IpAdded { ip });
		
			Ok(())
		}

		#[pallet::call_index(2)]
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn add_available_client_ip(
			origin: OriginFor<T>,
			ip: Vec<u8>, // IP address as a vector of bytes
		) -> DispatchResult {
			// Ensure the caller has the appropriate permission
			let _who = ensure_root(origin)?;
		
			// Retrieve the current list of available Client IPs
			let mut available_client_ips = AvailableClientIps::<T>::get();
		
			// Ensure the IP is not already in the list
			ensure!(
				!available_client_ips.contains(&ip),
				Error::<T>::IpAlreadyExists
			);
		
			// Add the new IP to the list
			available_client_ips.push(ip.clone());
			AvailableClientIps::<T>::put(available_client_ips);
		
			// Emit an event
			Self::deposit_event(Event::IpAdded { ip });
		
			Ok(())
		}

		#[pallet::call_index(3)]
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn add_available_storage_miner_ip(
			origin: OriginFor<T>,
			ip: Vec<u8>, // IP address as a vector of bytes
		) -> DispatchResult {
			// Ensure the caller has the appropriate permission
			let _who = ensure_root(origin)?;
		
			// Retrieve the current list of available Storage Miner IPs
			let mut available_storage_miner_ips = AvailableStorageMinerIps::<T>::get();
		
			// Ensure the IP is not already in the list
			ensure!(
				!available_storage_miner_ips.contains(&ip),
				Error::<T>::IpAlreadyExists
			);
		
			// Add the new IP to the list
			available_storage_miner_ips.push(ip.clone());
			AvailableStorageMinerIps::<T>::put(available_storage_miner_ips);
		
			// Emit an event
			Self::deposit_event(Event::IpAdded { ip });
		
			Ok(())
		}
	}

	#[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_initialize(current_block: BlockNumberFor<T>) -> Weight {
			// Call the method to handle pending IP release requests
			Self::handle_pending_ip_release_requests(current_block);
			
			// Return the weight consumed by this operation
			// You might want to adjust the weight based on the number of requests processed
			Weight::from_parts(10_000, 0)
		}
	}


	impl<T: Config> Pallet<T> {
		pub fn generate_vm_release_ip_request_and_update_storage(
			vm_name: Vec<u8>,
		) -> DispatchResult {
			// Get the IP associated with the VM name
			let ip = VmToIp::<T>::get(&vm_name).ok_or(Error::<T>::VmHasNoIp)?;
			
			// Get current assigned IPs
			let mut current_assigned_ips = AssignedIps::<T>::get();
			
			// Find and remove the VM UUID from assigned IPs
			let ip_index = current_assigned_ips.iter().position(|x| *x == vm_name)
				.ok_or(Error::<T>::VmHasNoIp)?;
			current_assigned_ips.remove(ip_index);
			
			// Update the storage with the modified list
			AssignedIps::<T>::put(current_assigned_ips);
		
			// Remove the reverse mapping
			VmToIp::<T>::remove(&vm_name);
		
			let ip_release_request = IpReleaseRequest {
				vm_name: vm_name.clone(),
				ip: ip.clone(),
				created_at: frame_system::Pallet::<T>::block_number(),
				role_type: RoleType::Vm(vm_name.clone()),
			};
			
			// Add the request to the IpReleaseRequests storage
			IpReleaseRequests::<T>::mutate(|requests| {
				requests.push(ip_release_request);
			});
		
			// Emit event
			Self::deposit_event(Event::IpReturned { vm_uuid: vm_name, ip });
			
			Ok(())
		}

		/// Allocate the next available IP address to a user
		pub fn assign_ip_to_vm(
			vm_uuid: Vec<u8>,
			ip: Vec<u8>,
		) -> DispatchResult {
			// Check if the VM already has an assigned IP
			ensure!(!AssignedIps::<T>::get().contains(&vm_uuid), Error::<T>::VmAlreadyHasIp);
		
			// Get the current list of available IPs
			let mut vm_available_ips = VmAvailableIps::<T>::get();

			// Remove the specified IP from the available IPs
			if let Some(pos) = vm_available_ips.iter().position(|x| *x == ip) {
				vm_available_ips.remove(pos); // Remove the IP if found
			} else {
				log::warn!("IP not found in available IPs");
			}

			// Update storage with the new list of available IPs
			VmAvailableIps::<T>::put(vm_available_ips);

			let mut current_assigned_ips = AssignedIps::<T>::get();
			current_assigned_ips.push(vm_uuid.clone());
			AssignedIps::<T>::put(current_assigned_ips);
			IpToVm::<T>::insert(&ip, &vm_uuid);
			VmToIp::<T>::insert(&vm_uuid, &ip);
	
			// Emit event
			Self::deposit_event(Event::IpAssigned { vm_uuid, ip });
		 
			Ok(())
		}


		fn handle_pending_ip_release_requests(current_block: BlockNumberFor<T>){
			IpReleaseRequests::<T>::mutate(|requests| {
				requests.retain(|request| {
					if request.created_at + (T::IpReleasePeriod::get() as u32).into() <= current_block {
						// If the request is old enough, add its IP back to available IPs
						let mut vm_available_ips = VmAvailableIps::<T>::get();
						vm_available_ips.push(request.ip.clone());
						VmAvailableIps::<T>::put(vm_available_ips);
						false // Remove this request from the vector
					} else {
						true // Keep this request in the vector
					}
				});
			});
		}
		
	}
}
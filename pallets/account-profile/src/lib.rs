#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
	use frame_support::pallet_prelude::*;
	use frame_system::pallet_prelude::*;
	use sp_std::vec::Vec;

	#[pallet::pallet]
	#[pallet::without_storage_info]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config + pallet_arion::Config + pallet_registration::Config {
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
	}

	/// Storage for public data
	#[pallet::storage]
	#[pallet::getter(fn user_public_storage)]
	pub type UserPublicStorage<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, Vec<u8>, ValueQuery>;

	/// Struct to hold account profile information.
	#[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo)]
	pub struct AccountProfile {
		pub node_id: Vec<u8>,
		pub encryption_key: Vec<u8>,
	}

	#[pallet::storage]
	#[pallet::getter(fn account_profiles)]
	pub type AccountProfiles<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, AccountProfile, OptionQuery>;

	/// Maps an AccountId to their Data Public Key
	#[pallet::storage]
	#[pallet::getter(fn get_data_public_key)]
	pub type DataPublicKeys<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, Vec<u8>, OptionQuery>;

	/// Storage for Message Public Keys
	/// Maps an AccountId to their Message Public Key
	#[pallet::storage]
	#[pallet::getter(fn get_message_public_key)]
	pub type MessagePublicKeys<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, Vec<u8>, OptionQuery>;

	/// Storage for private data
	#[pallet::storage]
	#[pallet::getter(fn user_private_storage)]
	pub type UserPrivateStorage<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, Vec<u8>, ValueQuery>;

	/// Storage for usernames
	/// Maps a username to an account ID, ensuring usernames are unique.
	#[pallet::storage]
	#[pallet::getter(fn get_account_id_by_username)]
	pub type Usernames<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, T::AccountId>;

	/// Storage to map an account ID to their username.
	#[pallet::storage]
	#[pallet::getter(fn get_username_by_account_id)]
	pub type AccountUsernames<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, Vec<u8>, OptionQuery>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// A public item was added or updated. [who, item]
		PublicItemSet(T::AccountId, Vec<u8>),
		/// A private item was added or updated. [who, item]
		PrivateItemSet(T::AccountId, Vec<u8>),
		/// A username was set. [who, username]
		UsernameSet(T::AccountId, Vec<u8>),
		/// An account profile was updated. [who]
		AccountProfileUpdated(T::AccountId),
		DataPublicKeySet(T::AccountId),
		AccountProfileSet(T::AccountId),
		/// A message public key was set. [who]
		MessagePublicKeySet(T::AccountId),
	}

	#[pallet::error]
	pub enum Error<T> {
		/// The hex string provided is invalid.
		InvalidHexString,
		/// The account already has a username set.
		UsernameAlreadySet,

		UsernameAlreadyTaken,

		NodeNotRegistered,

		InvalidNodeType,
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Set a hex-encoded string in the public storage
		#[pallet::call_index(0)]
		#[pallet::weight((0, Pays::No))]
		pub fn set_public_item(origin: OriginFor<T>, item: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			let main_account = if let Some(primary) = pallet_arion::Pallet::<T>::get_primary_account(&who)? {
				primary
			} else {
				who.clone()
			};

            // Check if the node is registered
            let node_info = pallet_registration::Pallet::<T>::get_registered_node_for_owner(&main_account)
                .ok_or(Error::<T>::NodeNotRegistered)?;

            // Verify the node type is Validator
            ensure!(
                node_info.node_type == pallet_registration::NodeType::Validator,
                Error::<T>::InvalidNodeType
            );
			// Strip "0x" prefix if present
			let stripped_item = Self::strip_prefix(item.clone());

			// Validate that the item is valid hex
			if !Self::is_valid_hex(&stripped_item) {
				return Err(Error::<T>::InvalidHexString.into());
			}

			// Set the item in the user's public storage
			UserPublicStorage::<T>::insert(&who, stripped_item.clone());

			// Emit an event
			Self::deposit_event(Event::PublicItemSet(who, stripped_item));
			Ok(())
		}

		/// Set a hex-encoded string in the private storage
		#[pallet::call_index(1)]
		#[pallet::weight((0, Pays::No))]
		pub fn set_private_item(origin: OriginFor<T>, item: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			let main_account = if let Some(primary) = pallet_arion::Pallet::<T>::get_primary_account(&who)? {
				primary
			} else {
				who.clone()
			};

            // Check if the node is registered
            let node_info = pallet_registration::Pallet::<T>::get_registered_node_for_owner(&main_account)
                .ok_or(Error::<T>::NodeNotRegistered)?;

            // Verify the node type is Validator
            ensure!(
                node_info.node_type == pallet_registration::NodeType::Validator,
                Error::<T>::InvalidNodeType
            );

			// Strip "0x" prefix if present
			let stripped_item = Self::strip_prefix(item.clone());

			// Validate that the item is valid hex
			if !Self::is_valid_hex(&stripped_item) {
				return Err(Error::<T>::InvalidHexString.into());
			}

			// Set the item in the user's private storage
			UserPrivateStorage::<T>::insert(&who, stripped_item.clone());

			// Emit an event
			Self::deposit_event(Event::PrivateItemSet(who, stripped_item));
			Ok(())
		}

		/// Set a unique username for the user
		#[pallet::call_index(2)]
		#[pallet::weight((0, Pays::No))]
		pub fn set_username(origin: OriginFor<T>, username: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			let main_account = if let Some(primary) = pallet_arion::Pallet::<T>::get_primary_account(&who)? {
				primary
			} else {
				who.clone()
			};

            // Check if the node is registered
            let node_info = pallet_registration::Pallet::<T>::get_registered_node_for_owner(&main_account)
                .ok_or(Error::<T>::NodeNotRegistered)?;

            // Verify the node type is Validator
            ensure!(
                node_info.node_type == pallet_registration::NodeType::Validator,
                Error::<T>::InvalidNodeType
            );

			// Convert the username to lowercase
			let lower_username = Self::to_lowercase(username.clone());

			// Ensure the username is not already taken
			ensure!(
				!Usernames::<T>::contains_key(&lower_username),
				Error::<T>::UsernameAlreadyTaken
			);

			// Ensure the user does not already have a username set
			ensure!(!AccountUsernames::<T>::contains_key(&who), Error::<T>::UsernameAlreadySet);

			// Store the lowercase username and map it to the account
			Usernames::<T>::insert(lower_username.clone(), &who);
			AccountUsernames::<T>::insert(&who, lower_username.clone());

			// Emit an event
			Self::deposit_event(Event::UsernameSet(who, username));
			Ok(())
		}

		/// Set the Data Public Key for an account
		#[pallet::call_index(3)]
		#[pallet::weight((0, Pays::No))]
		pub fn set_data_public_key(origin: OriginFor<T>, key: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			let main_account = if let Some(primary) = pallet_arion::Pallet::<T>::get_primary_account(&who)? {
				primary
			} else {
				who.clone()
			};

            // Check if the node is registered
            let node_info = pallet_registration::Pallet::<T>::get_registered_node_for_owner(&main_account)
                .ok_or(Error::<T>::NodeNotRegistered)?;

            // Verify the node type is Validator
            ensure!(
                node_info.node_type == pallet_registration::NodeType::Validator,
                Error::<T>::InvalidNodeType
            );

			// Strip "0x" prefix if present
			let stripped_key = Self::strip_prefix(key.clone());

			// Validate that the key is valid hex
			if !Self::is_valid_hex(&stripped_key) {
				return Err(Error::<T>::InvalidHexString.into());
			}

			// Store the data public key
			DataPublicKeys::<T>::insert(&who, stripped_key);

			// Emit an event
			Self::deposit_event(Event::DataPublicKeySet(who));
			Ok(())
		}

		#[pallet::call_index(4)]
		#[pallet::weight((0, Pays::No))]
		pub fn set_message_public_key(origin: OriginFor<T>, key: Vec<u8>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			let main_account = if let Some(primary) = pallet_arion::Pallet::<T>::get_primary_account(&who)? {
				primary
			} else {
				who.clone()
			};

            // Check if the node is registered
            let node_info = pallet_registration::Pallet::<T>::get_registered_node_for_owner(&main_account)
                .ok_or(Error::<T>::NodeNotRegistered)?;

            // Verify the node type is Validator
            ensure!(
                node_info.node_type == pallet_registration::NodeType::Validator,
                Error::<T>::InvalidNodeType
            );

			// Strip "0x" prefix if present
			let stripped_key = Self::strip_prefix(key.clone());

			// Validate that the key is valid hex
			if !Self::is_valid_hex(&stripped_key) {
				return Err(Error::<T>::InvalidHexString.into());
			}

			// Store the message public key
			MessagePublicKeys::<T>::insert(&who, stripped_key);

			// Emit an event
			Self::deposit_event(Event::MessagePublicKeySet(who));
			Ok(())
		}

		/// Update the account profile for the user.
		#[pallet::call_index(6)]
		#[pallet::weight((0, Pays::No))]
		pub fn update_account_profile(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			encryption_key: Vec<u8>,
		) -> DispatchResult {
            let who = ensure_signed(origin)?;

            // Get the main account if this is a proxy, otherwise use the account itself
			let main_account = if let Some(primary) = pallet_arion::Pallet::<T>::get_primary_account(&who)? {
				primary
			} else {
				who.clone()
			};

            // Check if the node is registered
            let node_info = pallet_registration::Pallet::<T>::get_registered_node_for_owner(&main_account)
                .ok_or(Error::<T>::NodeNotRegistered)?;

            // Verify the node type is Validator
            ensure!(
                node_info.node_type == pallet_registration::NodeType::Validator,
                Error::<T>::InvalidNodeType
            );
			
			// Create or update the account profile
			let profile = AccountProfile { node_id, encryption_key };
			AccountProfiles::<T>::insert(&who, profile);

			// Emit an event (optional)
			Self::deposit_event(Event::AccountProfileUpdated(who));
			Ok(())
		}

		#[pallet::call_index(5)]
		#[pallet::weight((0, Pays::No))]
		pub fn set_account_profile(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			encryption_key: Vec<u8>,
		) -> DispatchResult {
            let who = ensure_signed(origin)?;

            // Get the main account if this is a proxy, otherwise use the account itself
			let main_account = if let Some(primary) = pallet_arion::Pallet::<T>::get_primary_account(&who)? {
				primary
			} else {
				who.clone()
			};

            // Check if the node is registered
            let node_info = pallet_registration::Pallet::<T>::get_registered_node_for_owner(&main_account)
                .ok_or(Error::<T>::NodeNotRegistered)?;

            // Verify the node type is Validator
            ensure!(
                node_info.node_type == pallet_registration::NodeType::Validator,
                Error::<T>::InvalidNodeType
            );

			// Create or update the account profile
			let profile = AccountProfile { node_id, encryption_key };
			AccountProfiles::<T>::insert(&who, profile);

			// Emit an event (optional)
			Self::deposit_event(Event::AccountProfileSet(who));
			Ok(())
		}
	}

	impl<T: Config> Pallet<T> {
		// Utility function to check if a byte vector is valid hex
		fn is_valid_hex(data: &[u8]) -> bool {
			// Accept all byte arrays as valid since we're dealing with raw bytes
			// true
			data.iter().all(|&c| {
				(c >= b'0' && c <= b'9') || (c >= b'a' && c <= b'f') || (c >= b'A' && c <= b'F')
			})
		}

		// Function to strip "0x" prefix from hex strings
		fn strip_prefix(data: Vec<u8>) -> Vec<u8> {
			if data.len() >= 2 && data[0] == b'0' && data[1] == b'x' {
				return data[2..].to_vec();
			}
			data
		}

		// Convert a byte vector to lowercase
		fn to_lowercase(data: Vec<u8>) -> Vec<u8> {
			data.into_iter()
				.map(|c| if c.is_ascii_uppercase() { c.to_ascii_lowercase() } else { c })
				.collect()
		}
	}
}

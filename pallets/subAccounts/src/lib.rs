//! # SubAccounts Pallet
//! <!-- Original author of paragraph: @Faiz
//!
//! ## Overview
//!
//! Pallet enables users to add and remove sub accounts to send extrinsics for their profiles.
//!
//! ### Goals
//!
//! The pallet is designed to make the following possible:
//!
//! * Allow the main account or any subaccount of a Main Account to add/remove sub accounts.
//!
//! ## Interface
//!
//! ### Privileged Functions
//!
//! - `add_sub_account`: The sender can add a sub account for its Main Account
//! - `remove_sub_account`: The sender can remove any sub account of its Main Account.
//!
//!//! Please refer to the [`Call`] enum and its associated variants for documentation on each
//! function.

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

use frame_support::{
	pallet_prelude::*,
	// traits::StorageVersion,
};
use frame_system::{ensure_signed, pallet_prelude::OriginFor};

pub mod traits;
pub use traits::{ChargeFees, SubAccounts};
#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

pub mod weights;
pub use weights::WeightInfo;

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::traits::ExistenceRequirement;
	use frame_support::traits::Currency;
	use frame_support::traits::ReservableCurrency;

	#[pallet::pallet]
	pub struct Pallet<T>(_);

	type BalanceOf<T> =
	<<T as Config>::Currency as Currency<<T as frame_system::Config>::AccountId>>::Balance;

	/// Configure the pallet by specifying the parameters and types on which it depends.
	#[pallet::config]
	pub trait Config: frame_system::Config {
		/// Because this pallet emits events, it depends on the runtime's definition of an event.
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		#[pallet::constant]
		type StringLimit: Get<u32>;

		/// Weight information for extrinsics in this pallet.
		type WeightInfo: WeightInfo;

		#[pallet::constant]
		type MaxSubAccountsLimit: Get<u32>;

		type ExistentialDeposit: Get<BalanceOf<Self>>;

		/// The currency mechanism.
		type Currency: ReservableCurrency<Self::AccountId>;
	}

	/// Store the main account of a given address
	#[pallet::storage]
	#[pallet::getter(fn sub_account)]
	pub type SubAccount<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, T::AccountId>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
        /// A sub account has been added
        SubAccountAdded { main: T::AccountId, sub: T::AccountId, role: Role },
        /// A sub account has been removed
        SubAccountRemoved { main: T::AccountId, sub: T::AccountId },
        /// A sub account's role has been updated
        SubAccountRoleUpdated { main: T::AccountId, sub: T::AccountId, new_role: Role },
	}

	/// Role types for sub-accounts
	#[derive(Encode, Decode, Clone, RuntimeDebug, PartialEq, Eq, TypeInfo, MaxEncodedLen)]
	pub enum Role {
		/// Can upload content
		Upload,
		/// Can upload and delete content
		UploadDelete,
		/// No permissions (default)
		None,
	}

	impl Default for Role {
		fn default() -> Self {
			Role::None
		}
	}

	/// New storage for sub-account roles (added without modifying existing storage)
	#[pallet::storage]
	#[pallet::getter(fn sub_account_role)]
	pub type SubAccountRole<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, Role>;

	#[pallet::error]
	pub enum Error<T> {
		/// Sender is not a sub account
		NoSubAccount,
		/// Sender is not a sub account of the given address
		NotAllowed,
		/// Cannot remove all sub-accounts
		NoAccountsLeft,
		/// Cannot add a sub account twice
		AlreadySubAccount,
		/// Main account cannot be a sub-account
		MainCannotBeSubAccount,
		/// Cannot be a Sub Account of Itself
		CannotBeOwnSubAccount,
		/// Reached Limit
		TooManySubAccounts,
		/// Invalid role change
		InvalidRoleChange,
	}

	#[pallet::call(weight(<T as Config>::WeightInfo))]
	impl<T: Config> Pallet<T> {
		/// The origin can add a sub account for the given main account.
		///
		/// The origin must be Signed and the sender should have access to 'main'
		///
		/// Parameters:
		/// - `main`: The address that has a profile associated
		/// - `new_sub_account`: The address that will be added as a connected account of 'main'
		///
		/// Emits `SubAccountAdded` event when successful.
		///
		/// Weight: `O(1)` TODO: Add correct weight
		#[pallet::call_index(0)]
		pub fn add_sub_account(
			origin: OriginFor<T>,
			main: T::AccountId,
			new_sub_account: T::AccountId,
			role: Role,
		) -> DispatchResultWithPostInfo {
			let sender = ensure_signed(origin)?;

			// Check if the sender has permission to add a sub account
			Self::is_sub_account(sender.clone(), main.clone())?;

			// Ensure the main account is not a sub-account
			ensure!(
				!SubAccount::<T>::contains_key(main.clone()),
				Error::<T>::MainCannotBeSubAccount
			);
		
			// Check that the sub account wasn't added before
			ensure!(
				!SubAccount::<T>::contains_key(new_sub_account.clone()),
				Error::<T>::AlreadySubAccount
			);

			ensure!(main != new_sub_account, Error::<T>::CannotBeOwnSubAccount);

			let sub_account_count = SubAccount::<T>::iter()
			.filter(|(k, v)| v == &main)
			.count() as u32;
		
			ensure!(
				sub_account_count < T::MaxSubAccountsLimit::get(),
				Error::<T>::TooManySubAccounts
			);

		    // Only transfer if new sub-account's balance is below existential deposit
			let sub_account_balance = T::Currency::free_balance(&new_sub_account);
			if sub_account_balance < T::ExistentialDeposit::get() {
				T::Currency::transfer(
					&sender,
					&new_sub_account,
					T::ExistentialDeposit::get() - sub_account_balance,
					ExistenceRequirement::KeepAlive,
				)?;
			}

			SubAccountRole::<T>::insert(new_sub_account.clone(), role.clone());
			SubAccount::<T>::insert(new_sub_account.clone(), main.clone());
			
            // Emit an event
            Self::deposit_event(Event::SubAccountAdded { 
                main, 
                sub: new_sub_account,
                role,
            });

			Ok(().into())
		}

		/// The origin can remove a sub account for the given main account.
		///
		/// The origin must be Signed and the sender should have access to 'main'
		///
		///	Can't remove all the connected accounts for a profile
		///
		/// Parameters:
		/// - `main`: The address that has a profile associated
		/// - `sub_account_to_remove`: The address that will be removed as a connected account of
		///   'main'
		///
		/// Emits `SubAccountRemoved` event when successful.
		///
		/// Weight: `O(1)` TODO: Add correct weight
		#[pallet::call_index(1)]
		pub fn remove_sub_account(
			origin: OriginFor<T>,
			main: T::AccountId,
			sub_account_to_remove: T::AccountId,
		) -> DispatchResultWithPostInfo {
			let sender = ensure_signed(origin)?;

			// Prevent removing the main account
			ensure!(sub_account_to_remove != main, Error::<T>::NoAccountsLeft);

			// Verify that the sender and the account to be removed are sub accounts
			Self::is_sub_account(sender, main.clone())?;

			// Ensure the sub account exists
			ensure!(
				SubAccount::<T>::contains_key(sub_account_to_remove.clone()),
				Error::<T>::NoSubAccount
			);

			// Remove the sub account
			SubAccount::<T>::remove(sub_account_to_remove.clone());
			SubAccountRole::<T>::remove(sub_account_to_remove.clone());

			// Emit an event
			Self::deposit_event(Event::SubAccountRemoved { main, sub: sub_account_to_remove });

			Ok(().into())
		}


		/// Update the role of a sub-account
        ///
        /// The origin must be Signed and the sender should have access to 'main'
        ///
        /// Parameters:
        /// - `main`: The main account that owns the sub-account
        /// - `sub_account`: The sub-account to update
        /// - `new_role`: The new role to assign
        ///
        /// Emits `SubAccountRoleUpdated` event when successful.
        #[pallet::call_index(2)]
        pub fn update_sub_account_role(
            origin: OriginFor<T>,
            main: T::AccountId,
            sub_account: T::AccountId,
            new_role: Role,
        ) -> DispatchResultWithPostInfo {
            let sender = ensure_signed(origin)?;

            // Check permissions
            Self::is_sub_account(sender, main.clone())?;

            // Ensure the sub account exists and belongs to the main account
            ensure!(
                SubAccount::<T>::get(&sub_account) == Some(main.clone()),
                Error::<T>::NotAllowed
            );

            // Update the role
            SubAccountRole::<T>::insert(sub_account.clone(), new_role.clone());

            // Emit an event
            Self::deposit_event(Event::SubAccountRoleUpdated {
                main,
                sub: sub_account,
                new_role,
            });

            Ok(().into())
        }
	}

	impl<T: Config> SubAccounts<T::AccountId> for Pallet<T> {
		fn get_main_account(who: T::AccountId) -> Result<T::AccountId, DispatchError> {
			let main_account = SubAccount::<T>::get(who).ok_or(Error::<T>::NoSubAccount)?;
			Ok(main_account)
		}

		fn add_sub_account(main: T::AccountId, sub: T::AccountId) -> Result<(), DispatchError> {
			SubAccount::<T>::insert(sub, main);
			Ok(())
		}

		fn is_sub_account(sender: T::AccountId, main: T::AccountId) -> Result<(), DispatchError> {
			// If the sender is the main account, allow it
			if sender == main {
				return Ok(());
			}

			// For sub accounts, check if they belong to the main account
			let main_account_of_sender =
				<Self as SubAccounts<T::AccountId>>::get_main_account(sender)?;

			ensure!(main_account_of_sender == main, Error::<T>::NotAllowed);
			Ok(())
		}

		fn already_sub_account(who: T::AccountId) -> Result<(), DispatchError> {
			ensure!(!SubAccount::<T>::contains_key(who), Error::<T>::AlreadySubAccount);
			Ok(())
		}

		/// Check if a sub-account has upload permissions
		fn can_upload(who: T::AccountId) -> bool {
			SubAccountRole::<T>::get(who)
				.map(|role| matches!(role, Role::Upload | Role::UploadDelete))
				.unwrap_or(false)
		}

		/// Check if a sub-account has delete permissions
		fn can_delete(who: T::AccountId) -> bool {
			SubAccountRole::<T>::get(who)
				.map(|role| matches!(role, Role::UploadDelete))
				.unwrap_or(false)
		}
		
	}

	impl<T: Config> ChargeFees<T::AccountId> for Pallet<T> {
		fn get_main_account(who: &T::AccountId) -> Option<T::AccountId> {
			SubAccount::<T>::get(who)
		}
	}

	// Additional helper functions for roles
	impl<T: Config> Pallet<T> {
		/// Check if a sub-account has upload permissions
		pub fn can_upload(who: &T::AccountId) -> bool {
			SubAccountRole::<T>::get(who)
				.map(|role| matches!(role, Role::Upload | Role::UploadDelete))
				.unwrap_or(false)
		}

		/// Check if a sub-account has delete permissions
		pub fn can_delete(who: &T::AccountId) -> bool {
			SubAccountRole::<T>::get(who)
				.map(|role| matches!(role, Role::UploadDelete))
				.unwrap_or(false)
		}

		/// Get the role of a sub-account
		pub fn get_role(who: &T::AccountId) -> Option<Role> {
			SubAccountRole::<T>::get(who)
		}
	}
}

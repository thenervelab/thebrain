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
pub use traits::{
    ChargeFees, SubAccounts,
};
#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

pub mod weights;
pub use weights::WeightInfo;
// pub mod migrations; // Add this line

#[frame_support::pallet]
pub mod pallet {
    use super::*;
    // use frame_support::traits::OnRuntimeUpgrade;
    // use crate::migrations::MigrateToNewStorageFormat;
    // use frame_system::pallet_prelude::BlockNumberFor;
    
    #[pallet::pallet]
    pub struct Pallet<T>(_);

    /// Configure the pallet by specifying the parameters and types on which it depends.
    #[pallet::config]
    pub trait Config: frame_system::Config {
        /// Because this pallet emits events, it depends on the runtime's definition of an event.
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        #[pallet::constant]
        type StringLimit: Get<u32>;

        /// Weight information for extrinsics in this pallet.
        type WeightInfo: WeightInfo;

        // type OnRuntimeUpgrade: OnRuntimeUpgrade;
    }

    // /// Store the main account of a given address with a boolean flag
    // #[pallet::storage]
    // #[pallet::getter(fn new_sub_account)]
    // /// This is the new storage format after migration
    // pub type NewSubAccount<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, (T::AccountId, bool)>;
    
    /// Store the main account of a given address
    #[pallet::storage]
    #[pallet::getter(fn sub_account)]
    pub type SubAccount<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, T::AccountId>;

    // // Define the storage version
    // const STORAGE_VERSION: StorageVersion = StorageVersion::new(1);

    // #[pallet::hooks]
    // impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
    //     fn on_runtime_upgrade() -> Weight {
    //         // Check the on-chain storage version
    //         let on_chain_version = StorageVersion::get::<Pallet<T>>();
    //         if on_chain_version < STORAGE_VERSION {
    //             // Perform the migration
    //             let weight = MigrateToNewStorageFormat::<T>::on_runtime_upgrade();
    //             // Set the new storage version
    //             StorageVersion::put::<Pallet<T>>(&STORAGE_VERSION);
    //             weight
    //         } else {
    //             Weight::zero()
    //         }
    //     }
    // }

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// A sub account has been added
        SubAccountAdded { main: T::AccountId, sub: T::AccountId },
        /// A sub account has been removed
        SubAccountRemoved { main: T::AccountId, sub: T::AccountId },
    }

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
        ) -> DispatchResultWithPostInfo {
            let sender = ensure_signed(origin)?;

            // Check if the sender has permission to add a sub account
            Self::is_sub_account(sender, main.clone())?;

            // Check that the sub account wasn't added before.
            ensure!(
                !SubAccount::<T>::contains_key(new_sub_account.clone()),
                Error::<T>::AlreadySubAccount
            );

            SubAccount::<T>::insert(new_sub_account.clone(), main.clone());

            // Emit an event
            Self::deposit_event(Event::SubAccountAdded { main, sub: new_sub_account });

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
            ensure!(
                sub_account_to_remove != main,
                Error::<T>::NoAccountsLeft
            );

            // Verify that the sender and the account to be removed are sub accounts
            Self::is_sub_account(sender, main.clone())?;
            
            // Ensure the sub account exists
            ensure!(
                SubAccount::<T>::contains_key(sub_account_to_remove.clone()),
                Error::<T>::NoSubAccount
            );

            // Remove the sub account
            SubAccount::<T>::remove(sub_account_to_remove.clone());

            // Emit an event
            Self::deposit_event(Event::SubAccountRemoved { main, sub: sub_account_to_remove });

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
    }

    impl<T: Config> ChargeFees<T::AccountId> for Pallet<T> {
        fn get_main_account(who: &T::AccountId) -> Option<T::AccountId> {
            SubAccount::<T>::get(who)
        }
    }
}
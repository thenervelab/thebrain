#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

pub use pallet::*;
use frame_support::{pallet_prelude::*};
use frame_system::pallet_prelude::*;
use sp_std::prelude::*;
use frame_support::sp_runtime::traits::AccountIdConversion;

#[frame_support::pallet]
pub mod pallet {
    use super::*;
    use pallet_marketplace::Pallet as MarketplacePallet;
    use sp_runtime::traits::AtLeast32BitUnsigned;
    use frame_support::{
        traits::Currency,
        PalletId,
    };

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config + pallet_balances::Config + pallet_marketplace::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        type MinConfirmations: Get<u32>; // e.g., 2 or 3
        /// The pallet's id, used for deriving its sovereign account ID.
        #[pallet::constant]
        type PalletId: Get<PalletId>;

        /// The balance type used for this pallet.
        type Balance: Parameter + Member + AtLeast32BitUnsigned + Default + Copy+ TryFrom<BalanceOf<Self>>
        + Into<<Self as pallet_balances::Config>::Balance>;
    }

    // New pallet_balances balance type
    pub type BalanceOf<T> = <T as pallet_balances::Config>::Balance;

    // Next batch ID
    #[pallet::storage]
    #[pallet::getter(fn next_batch_id)]
    pub type NextBatchId<T: Config> = StorageValue<_, u64, ValueQuery>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        MintedAlpha { amount: <T as pallet_balances::Config>::Balance },
        BurnedAlpha { amount: <T as pallet_balances::Config>::Balance },
    }

    #[pallet::error]
    pub enum Error<T> {
        SudoKeyNotSet,
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        #[pallet::call_index(1)]
        #[pallet::weight((0, Pays::No))]
        pub fn mint(origin: OriginFor<T>, amount: <T as pallet_balances::Config>::Balance) -> DispatchResult {
            // Ensure that the caller is the sudo account
            ensure_root(origin)?;
    
            // Get the sudo key from the marketplace pallet
            if let Some(sudo_account) = MarketplacePallet::<T>::sudo_key() {
                // Mint native currency to the sudo account
                let _ = <pallet_balances::Pallet<T>>::deposit_creating(&sudo_account, amount);
                Self::deposit_event(Event::MintedAlpha { amount });
            } else {
                return Err(Error::<T>::SudoKeyNotSet.into()); // Handle the case where the sudo key is not set
            }
    
            Ok(())
        }
    
        #[pallet::call_index(2)]
        #[pallet::weight((0, Pays::No))]
        pub fn burn(origin: OriginFor<T>, amount: <T as pallet_balances::Config>::Balance) -> DispatchResult {
            // Ensure that the caller is the sudo account
            ensure_root(origin)?;
    
            // Get the sudo key from the marketplace pallet
            if let Some(sudo_account) = MarketplacePallet::<T>::sudo_key() {
                // Burn the equivalent amount from their free balance
                let _ = pallet_balances::Pallet::<T>::burn(
                    frame_system::RawOrigin::Signed(sudo_account.clone()).into(),
                    amount,
                    false, // keep_alive set to false to allow burning entire balance
                );
                Self::deposit_event(Event::BurnedAlpha { amount });
            } else {
                return Err(Error::<T>::SudoKeyNotSet.into()); // Handle the case where the sudo key is not set
            }
    
            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        pub fn account_id() -> T::AccountId {
            <T as pallet::Config>::PalletId::get().into_account_truncating()
        }
    }
}
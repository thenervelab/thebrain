#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

pub use pallet::*;
use frame_support::{pallet_prelude::*};
use frame_system::pallet_prelude::*;
use sp_std::prelude::*;

#[frame_support::pallet]
pub mod pallet {
    use super::*;
    use sp_std::collections::btree_set::BTreeSet;
    use pallet_credits::Pallet as CreditsPallet;
    use frame_support::{
        pallet_prelude::*,
        traits::{
            tokens::Balance,
            Get,
            PalletId,
        },
        weights::Weight,
    };

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config + pallet_credits::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        type MinConfirmations: Get<u32>; // e.g., 2 or 3
        /// The pallet's id, used for deriving its sovereign account ID.
        #[pallet::constant]
        type PalletId: Get<PalletId>;
    }

    #[pallet::storage]
    pub type AlphaBalances<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, u64, ValueQuery>;

    #[pallet::storage]
    pub type ProcessedEvents<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, bool, ValueQuery>;

    #[pallet::storage]
    pub type PendingMints<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        Vec<u8>, // proof (block_hash, event_index)
        (T::AccountId, u64, BTreeSet<T::AccountId>), // (user, amount, confirmations)
        OptionQuery,
    >;

    #[pallet::storage]
    pub type PendingBurns<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        u64, // nonce
        (
            T::AccountId, // user
            u64,          // amount
            T::AccountId, // bittensor_coldkey
            BTreeSet<T::AccountId>, // confirmations
            Option<(Vec<u8>, Vec<u8>)>, // (bittensor_block_hash, extrinsic_id)
        ),
        OptionQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn total_alpha_in_pool)]
    pub type TotalAlphaInPool<T> = StorageValue<_, u128, ValueQuery>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        AlphaMintPending(Vec<u8>, T::AccountId, u64),             // (proof, user, amount)
        AlphaMinted(T::AccountId, u64),                           // (user, amount)
        AlphaBurnPending(u64, T::AccountId, u64, T::AccountId),   // (nonce, user, amount, bittensor_coldkey)
        AlphaBurned(u64, T::AccountId, u64, T::AccountId),        // (nonce, user, amount, bittensor_coldkey)
        BurnFinalized(u64, Vec<u8>, Vec<u8>),                     // (nonce, block_hash, extrinsic_id)
        AlphaBurnRejected(u64, T::AccountId, u64, T::AccountId),  // (nonce, user, amount, bittensor_coldkey)
        AlphaDeposited(T::AccountId, T::AccountId, u64),          // (pallet_account, user, amount)
    }

    #[pallet::error]
    pub enum Error<T> {
        Unauthorized,            // Not an operator
        DoubleSpendDetected,     // Event already processed
        InsufficientBalance,     // Not enough alpha
        AlreadyConfirmed,        // Operator already confirmed
        NotEnoughConfirmations,  // Threshold not met
        InvalidProof,            // Proof verification failed
        BurnNotPending,          // No such burn request
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Operator proposes and confirms minting alpha
        #[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn confirm_mint_alpha(
            origin: OriginFor<T>,
            amount: u64,
            proof: Vec<u8>, // Encoded (block_hash, event_index)
        ) -> DispatchResult {
            let operator = ensure_signed(origin)?;
            CreditsPallet::<T>::ensure_is_authority(&operator)?;

            // Prevent double-spending
            ensure!(!ProcessedEvents::<T>::contains_key(&proof), Error::<T>::DoubleSpendDetected);

            let (_, pending_amount, mut confirmations) =
                PendingMints::<T>::get(&proof).unwrap_or((Self::get_pallet_account(), amount, BTreeSet::new()));

            // Ensure consistent amount
            ensure!(pending_amount == amount, Error::<T>::InvalidProof);

            // Add confirmation
            ensure!(!confirmations.contains(&operator), Error::<T>::AlreadyConfirmed);
            confirmations.insert(operator);

            if confirmations.len() as u32 >= T::MinConfirmations::get() {
                // Threshold met, execute mint to pallet account
                let pallet_account = Self::get_pallet_account();
                let current_balance = AlphaBalances::<T>::get(&pallet_account);
                let new_balance = current_balance.saturating_add(amount);
                AlphaBalances::<T>::insert(&pallet_account, new_balance);
                ProcessedEvents::<T>::insert(&proof, true);
                PendingMints::<T>::remove(&proof);
                
                // Increase total alpha in pool
                TotalAlphaInPool::<T>::mutate(|total| *total += amount as u128);
                
                Self::deposit_event(Event::AlphaMinted(pallet_account, amount));
            } else {
                // Still pending
                PendingMints::<T>::insert(&proof, (Self::get_pallet_account(), amount, confirmations));
                Self::deposit_event(Event::AlphaMintPending(proof, Self::get_pallet_account(), amount));
            }
            Ok(())
        }

        /// User initiates burn request
        #[pallet::call_index(1)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn burn_alpha_for_bridge(
            origin: OriginFor<T>,
            amount: u64,
            bittensor_coldkey: T::AccountId,
            nonce: u64,
        ) -> DispatchResult {
            let user = ensure_signed(origin)?;
            let balance = AlphaBalances::<T>::get(&user);
            ensure!(balance >= amount, Error::<T>::InsufficientBalance);

            // Burn alpha immediately but mark as pending
            AlphaBalances::<T>::insert(&user, balance - amount);
            
            // Decrease total alpha in pool
            TotalAlphaInPool::<T>::mutate(|total| *total -= amount as u128);
            
            PendingBurns::<T>::insert(
                &nonce,
                (
                    user.clone(), 
                    amount, 
                    bittensor_coldkey.clone(), 
                    BTreeSet::<T::AccountId>::new(), 
                    None::<(Vec<u8>, Vec<u8>)>
                ),
            );
            Self::deposit_event(Event::AlphaBurnPending(nonce, user, amount, bittensor_coldkey));
            Ok(())
        }

        /// Operator confirms burn and optionally finalizes with Bittensor details
        #[pallet::call_index(2)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn confirm_and_finalize_burn(
            origin: OriginFor<T>,
            nonce: u64,
            bittensor_block_hash: Option<Vec<u8>>,
            bittensor_extrinsic_id: Option<Vec<u8>>,
        ) -> DispatchResult {
            let operator = ensure_signed(origin)?;
            CreditsPallet::<T>::ensure_is_authority(&operator)?;

            let Some((user, amount, coldkey, mut confirmations, existing_finalization)) =
                PendingBurns::<T>::get(&nonce) else { return Err(Error::<T>::BurnNotPending.into()) };

            // Add confirmation
            ensure!(!confirmations.contains(&operator), Error::<T>::AlreadyConfirmed);
            confirmations.insert(operator);

            if let (Some(block_hash), Some(extrinsic_id)) = (bittensor_block_hash, bittensor_extrinsic_id) {
                // Finalize with Bittensor details if provided
                ensure!(
                    confirmations.len() as u32 >= T::MinConfirmations::get(),
                    Error::<T>::NotEnoughConfirmations
                );
                PendingBurns::<T>::insert(
                    &nonce,
                    (
                        user.clone(),
                        amount,
                        coldkey.clone(),
                        confirmations,
                        Some((block_hash.clone(), extrinsic_id.clone())),
                    ),
                );
                Self::deposit_event(Event::AlphaBurned(nonce, user, amount, coldkey));
                Self::deposit_event(Event::BurnFinalized(nonce, block_hash, extrinsic_id));
            } else {
                // Still pending
                PendingBurns::<T>::insert(&nonce, (user, amount, coldkey, confirmations, None::<(Vec<u8>, Vec<u8>)>));
            }
            Ok(())
        }

        /// Operator rejects a pending burn request and restores the user's alpha balance
        #[pallet::call_index(3)]
        #[pallet::weight(Weight::from_parts(10_000, 0))]
        pub fn reject_burn_request(
            origin: OriginFor<T>,
            nonce: u64,
        ) -> DispatchResult {
            let operator = ensure_signed(origin)?;
            CreditsPallet::<T>::ensure_is_authority(&operator)?;

            // Retrieve the pending burn request
            let Some((user, amount, coldkey, _, _)) = 
                PendingBurns::<T>::get(&nonce) 
            else { 
                return Err(Error::<T>::BurnNotPending.into()) 
            };

            // Restore the user's alpha balance
            let current_balance = AlphaBalances::<T>::get(&user);
            AlphaBalances::<T>::insert(&user, current_balance + amount);

            // Revert the total alpha in pool reduction
            TotalAlphaInPool::<T>::mutate(|total| *total += amount as u128);

            // Remove the pending burn request
            PendingBurns::<T>::remove(&nonce);

            // Emit an event for the burn request rejection
            Self::deposit_event(Event::AlphaBurnRejected(nonce, user, amount, coldkey));

            Ok(())
        }
    }

    impl<T: Config> Pallet<T> {
        fn verify_bridge_proof(_proof: &[u8], _user: &T::AccountId, _amount: u64) -> Result<(), Error<T>> {
            // Placeholder: Operators manually verify Bittensor event
            Ok(())
        }

        // Calculate the alpha price in terms of Credits equivalent
        pub fn calculate_alpha_price(credits_to_withdraw: u128) -> Option<u128> {
            let total_credits = CreditsPallet::<T>::total_credits_purchased();
            let total_alpha = Self::total_alpha_in_pool();
    
            // Ensure we have Alpha in the pool to calculate
            if total_alpha == 0 {
                return None; // No Alpha available, withdrawal not possible, cant calculate price
            }

            // Calculate Alpha price in terms of Credits
            let alpha_price_in_credits = total_credits.checked_div(total_alpha)?;
    
            // Calculate the amount of Alpha to burn
            let alpha_to_burn = credits_to_withdraw.checked_div(alpha_price_in_credits)?;
    
            Some(alpha_to_burn)
        }
        
        pub fn account_id() -> T::AccountId {
            <T as pallet::Config>::PalletId::get().into_account_truncating()
        }

        /// Get the current balance of the marketplace pallet
        pub fn balance() -> BalanceOf<T> {
            pallet_balances::Pallet::<T>::free_balance(&Self::account_id())
        }

        /// Helper function to deposit alpha to a user's account, reducing from pool
        pub fn deposit_creating(
            user: &T::AccountId, 
            amount: u64
        ) -> DispatchResult {
            // Get the pallet's account
            let pallet_account = Self::get_pallet_account();

            // Check pallet account has sufficient balance
            let pallet_balance = AlphaBalances::<T>::get(&pallet_account);
            ensure!(pallet_balance >= amount as u128, Error::<T>::InsufficientBalance);

            // Reduce pallet account balance
            AlphaBalances::<T>::mutate(&pallet_account, |balance| *balance -= amount as u128);

            // Increase user's alpha balance
            let user_balance = AlphaBalances::<T>::get(user);
            AlphaBalances::<T>::insert(user, user_balance + amount);

            // Decrease total alpha in pool
            TotalAlphaInPool::<T>::mutate(|total| *total -= amount as u128);

            // Emit event
            Self::deposit_event(Event::AlphaDeposited(pallet_account, user.clone(), amount));

            Ok(())
        }
    }
}
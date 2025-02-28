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
    use sp_runtime::SaturatedConversion;

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config + pallet_credits::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        type MinConfirmations: Get<u32>; // e.g., 2 or 3
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

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        AlphaMintPending(Vec<u8>, T::AccountId, u64),             // (proof, user, amount)
        AlphaMinted(T::AccountId, u64),                           // (user, amount)
        AlphaBurnPending(u64, T::AccountId, u64, T::AccountId),   // (nonce, user, amount, bittensor_coldkey)
        AlphaBurned(u64, T::AccountId, u64, T::AccountId),        // (nonce, user, amount, bittensor_coldkey)
        BurnFinalized(u64, Vec<u8>, Vec<u8>),                     // (nonce, block_hash, extrinsic_id)
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
            user: T::AccountId,
            amount: u64,
            proof: Vec<u8>, // Encoded (block_hash, event_index)
        ) -> DispatchResult {
            let operator = ensure_signed(origin)?;
            CreditsPallet::<T>::ensure_is_authority(&operator)?;

            // Prevent double-spending
            ensure!(!ProcessedEvents::<T>::contains_key(&proof), Error::<T>::DoubleSpendDetected);

            let (pending_user, pending_amount, mut confirmations) =
                PendingMints::<T>::get(&proof).unwrap_or((user.clone(), amount, BTreeSet::new()));

            // Ensure consistent user and amount
            ensure!(pending_user == user && pending_amount == amount, Error::<T>::InvalidProof);

            // Add confirmation
            ensure!(!confirmations.contains(&operator), Error::<T>::AlreadyConfirmed);
            confirmations.insert(operator);

            if confirmations.len() as u32 >= T::MinConfirmations::get() {
                // Threshold met, execute mint
                let new_balance = AlphaBalances::<T>::get(&user).saturating_add(amount);
                AlphaBalances::<T>::insert(&user, new_balance);
                ProcessedEvents::<T>::insert(&proof, true);
                PendingMints::<T>::remove(&proof);
                Self::deposit_event(Event::AlphaMinted(user, amount));
            } else {
                // Still pending
                PendingMints::<T>::insert(&proof, (user.clone(), amount, confirmations));
                Self::deposit_event(Event::AlphaMintPending(proof, user, amount));
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
    }

    impl<T: Config> Pallet<T> {
        fn verify_bridge_proof(_proof: &[u8], _user: &T::AccountId, _amount: u64) -> Result<(), Error<T>> {
            // Placeholder: Operators manually verify Bittensor event
            Ok(())
        }
    }
}
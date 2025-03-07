#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;
pub use types::*;
mod types;

pub use pallet::*;
use frame_support::{pallet_prelude::*};
use frame_system::pallet_prelude::*;
use sp_std::prelude::*;
use frame_support::sp_runtime::traits::AccountIdConversion;

#[frame_support::pallet]
pub mod pallet {
    use super::*;
    use sp_std::collections::btree_set::BTreeSet;
    use pallet_credits::Pallet as CreditsPallet;
    use sp_runtime::traits::AtLeast32BitUnsigned;
    use pallet_credits::TotalLockedAlpha;
    use pallet_credits::TotalCreditsPurchased;
    use types::*;
    use frame_support::{
        pallet_prelude::*,
        traits::{ReservableCurrency, Currency},
        PalletId,
    };

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    #[pallet::config]
    pub trait Config: frame_system::Config + pallet_credits::Config  + pallet_balances::Config {
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

    // Map for batches identified by a unique ID
    #[pallet::storage]
    #[pallet::getter(fn batches)]
    pub type Batches<T: Config> = StorageMap<
        _, 
        Blake2_128Concat, 
        u64, 
        Batch<T::AccountId, BlockNumberFor<T>>
    >;

    // Map for user batches identified by AccountId
    #[pallet::storage]
    #[pallet::getter(fn user_batches)]
    pub type UserBatches<T: Config> = StorageMap<
        _, 
        Blake2_128Concat, 
        T::AccountId, 
        Vec<u64>
    >;

    // Next batch ID
    #[pallet::storage]
    #[pallet::getter(fn next_batch_id)]
    pub type NextBatchId<T: Config> = StorageValue<_, u64, ValueQuery>;

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
        BatchDeposited { owner: T::AccountId, batch_id: u64 },   // (owner, batch_id)
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

        #[pallet::call_index(0)]
        #[pallet::weight((0, Pays::No))]
        pub fn deposit(
            origin: OriginFor<T>, 
            account: T::AccountId, 
            credit_amount: u128, 
            alpha_amount: u128, 
            freeze_for_chargeback: bool
        ) -> DispatchResult {
            ensure_root(origin)?;
            // Call the existing deposit function
            Self::do_deposit(account, credit_amount, alpha_amount, freeze_for_chargeback)
        }


        #[pallet::call_index(1)]
        #[pallet::weight((0, Pays::No))]
        pub fn chargeback(
            origin: OriginFor<T>, 
            batch_id: u64
        ) -> DispatchResult {
            // Ensure the caller is a signed origin (admin check)
            ensure_root(origin)?;

            // Call the existing handle_chargeback function
            Self::handle_chargeback(batch_id)
        }

        #[pallet::call_index(2)]
        #[pallet::weight((0, Pays::No))]
        pub fn release_pending_alpha(
            origin: OriginFor<T>, 
            batch_id: u64
        ) -> DispatchResult {
            // Ensure the caller is a signed origin
            let sender = ensure_signed(origin)?;
    
            // Call the existing release_pending_alpha function
            Self::do_release_pending_alpha(batch_id)
        }
    }

    impl<T: Config> Pallet<T> {
        pub fn account_id() -> T::AccountId {
            <T as pallet::Config>::PalletId::get().into_account_truncating()
        }

        /// Deposit credits and alpha into a new batch
        fn do_deposit(
            sender: T::AccountId, 
            credit_amount: u128, 
            alpha_amount: u128, 
            freeze_for_chargeback: bool
        ) -> DispatchResult {

            let batch_id = NextBatchId::<T>::get();

            let release_time = if freeze_for_chargeback {
                let block_number = <frame_system::Pallet<T>>::block_number();
                block_number + (15u32 * 28800u32).into() // 15 days
            } else {
                <frame_system::Pallet<T>>::block_number() // No release time
            };

            let batch = Batch {
                owner: sender.clone(),
                credit_amount,
                alpha_amount,
                remaining_credits: credit_amount,
                remaining_alpha: alpha_amount,
                pending_alpha: 0,
                is_frozen: freeze_for_chargeback,
                release_time,
            };

            Batches::<T>::insert(batch_id, batch);
            UserBatches::<T>::append(&sender, batch_id);
            NextBatchId::<T>::put(batch_id + 1);

            TotalLockedAlpha::<T>::mutate(|alpha| *alpha += alpha_amount);
            CreditsPallet::<T>::do_mint(sender.clone(), alpha_amount, None);

            Self::deposit_event(Event::BatchDeposited { owner: sender, batch_id });

            Ok(())
        }

        /// Consume user credits from their batches
        pub fn consume_credits(sender: T::AccountId, credits: u128, marketplace_account: T::AccountId,
             ranking_account: T::AccountId) -> DispatchResult {
            
            let mut remaining = credits;

            // Get the batch IDs associated with the user, handling the Option type
            if let Some(batch_ids) = UserBatches::<T>::get(&sender) {
                for batch_id in batch_ids {
                    if remaining == 0 { break; }

                    // Retrieve the batch
                    if let Some(mut batch) = Batches::<T>::get(batch_id) {
                        ensure!(batch.owner == sender, "Not your batch");

                        let credits_to_take = remaining.min(batch.remaining_credits);
                        let alpha_to_release = (credits_to_take * batch.alpha_amount) / batch.credit_amount;

                        // Update the batch's remaining credits and alpha
                        batch.remaining_credits -= credits_to_take;
                        batch.remaining_alpha -= alpha_to_release;

                        // Handle Alpha distribution
                        if batch.is_frozen && <frame_system::Pallet<T>>::block_number() < batch.release_time {
                            batch.pending_alpha += alpha_to_release; // Queue it
                        } else {
                            if batch.is_frozen && <frame_system::Pallet<T>>::block_number() >= batch.release_time {
                                batch.is_frozen = false; // Unfreeze
                                TotalLockedAlpha::<T>::mutate(|alpha| *alpha -= batch.pending_alpha);
                                batch.pending_alpha = 0;

                                // decrease credits and mint on miners account
                                // Call the helper function to distribute alpha
                                Self::distribute_alpha(
                                    batch.owner.clone(),
                                    alpha_to_release,
                                    credits_to_take,
                                    ranking_account.clone(),
                                    marketplace_account.clone(),
                                )?;

                            }
                            TotalLockedAlpha::<T>::mutate(|alpha| *alpha -= alpha_to_release);
                            // decrease credits and mint on miners account
                            // Call the helper function to distribute alpha
                            Self::distribute_alpha(
                                batch.owner.clone(),
                                alpha_to_release,
                                credits_to_take,
                                ranking_account.clone(),
                                marketplace_account.clone(),
                            )?;
                        }
 
                        // Update the batch in storage
                        Batches::<T>::insert(batch_id, batch);
                        remaining -= credits_to_take;
                    }
                }
            }

            // Ensure all credits have been consumed
            ensure!(remaining == 0, "Not enough credits");
            Ok(())
        }

        fn distribute_alpha(
            batch_owner: T::AccountId,
            alpha_to_release: u128,
            credits_to_take: u128,
            ranking_account: T::AccountId,
            marketplace_account: T::AccountId,
        ) -> DispatchResult {
            // Decrease credits for the batch owner
            CreditsPallet::<T>::decrease_user_credits(&batch_owner, credits_to_take);
        
            // Handle referral logic
            let mut total_discount = 0u128;
            if let Some(previous_referral) = CreditsPallet::<T>::referred_users(&batch_owner) {
                let _ = CreditsPallet::<T>::apply_referral_discount(&previous_referral, credits_to_take, &mut total_discount);
            }
        
            // Calculate the amounts for Storage Rankings and Marketplace
            let rankings_amount = alpha_to_release
                .checked_mul(70u32.into())
                .and_then(|x| x.checked_div(100u32.into()))
                .unwrap_or_default();
        
            let marketplace_amount = alpha_to_release - rankings_amount;
        
            // Mint 70% to Storage Rankings
            pallet_balances::Pallet::<T>::deposit_creating(
                &ranking_account,
                rankings_amount.try_into().unwrap_or_default(),
            );
        
            // Deposit remaining 30% amount to marketplace account
            pallet_balances::Pallet::<T>::deposit_creating(
                &marketplace_account,
                marketplace_amount.try_into().unwrap_or_default(),
            );
        
            Ok(())
        }

        /// Release pending Alpha for a specific batch after the release time
        fn do_release_pending_alpha(batch_id: u64) -> DispatchResult {

            // Retrieve the current block number
            let now =  <frame_system::Pallet<T>>::block_number();

            // Get the batch from storage
            if let Some(mut batch) = Batches::<T>::get(batch_id) {
                // Ensure the batch is ready to be released
                ensure!(now >= batch.release_time, "Still frozen");

                // Check if the batch is frozen
                if batch.is_frozen {
                    batch.is_frozen = false; // Unfreeze the batch

                    // Release the pending Alpha
                    TotalLockedAlpha::<T>::mutate(|alpha| *alpha -= batch.pending_alpha);
                    // Distribute `batch.pending_alpha` to miners (implementation of distribution logic needed)
                    batch.pending_alpha = 0;

                    // Update the batch in storage
                    Batches::<T>::insert(batch_id, batch);
                }
            }

            Ok(())
        }        

        /// Handle chargeback for a specific batch
        fn handle_chargeback(batch_id: u64) -> DispatchResult {
           
            // Get the batch from storage
            if let Some(batch) = Batches::<T>::get(batch_id) {
                // Ensure the batch is frozen and the chargeback is valid
                ensure!(batch.is_frozen && <frame_system::Pallet<T>>::block_number() < batch.release_time, "Invalid chargeback");

                // Decrease the total locked Alpha by the remaining Alpha in the batch
                TotalLockedAlpha::<T>::mutate(|alpha| *alpha -= batch.remaining_alpha);

                // Remove the batch from the user's batch list
                UserBatches::<T>::mutate(&batch.owner, |batches| {
                    if let Some(ref mut batch_vec) = batches {
                        batch_vec.retain(|&id| id != batch_id);
                    }
                });

                // Remove the batch from storage
                Batches::<T>::remove(batch_id);

                // Burn credits from batch.owner (implementation of burning logic needed)
                let credit_to_burn = batch.remaining_credits;
                CreditsPallet::<T>::decrease_user_credits(&batch.owner, credit_to_burn);
                TotalCreditsPurchased::<T>::mutate(|total| *total -= credit_to_burn);
            }

            Ok(())
        }
    }

    pub fn get_batches_for_user<T: Config>(user: T::AccountId) -> Vec<Batch<T::AccountId, BlockNumberFor<T>>> {
        let batch_ids: Vec<u64> = UserBatches::<T>::get(user).unwrap(); // Convert to Vec<u64>
        batch_ids.iter()
            .filter_map(|id| Batches::<T>::get(*id))
            .collect()
    }
    
    pub fn get_batch_by_id<T: Config>(batch_id: u64) -> Option<Batch<T::AccountId, BlockNumberFor<T>>> {
        Batches::<T>::get(batch_id)
    }

}
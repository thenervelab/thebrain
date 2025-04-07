#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

use frame_support::pallet_prelude::*;
use frame_support::sp_runtime::traits::AccountIdConversion;
use frame_system::pallet_prelude::*;
pub use pallet::*;
use sp_std::prelude::*;

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::traits::Currency;
	use frame_support::traits::ExistenceRequirement;
	use frame_support::PalletId;
	use pallet_credits::TotalLockedAlpha;
	use sp_runtime::traits::AtLeast32BitUnsigned;
	use sp_std::collections::btree_set::BTreeSet;

	#[pallet::pallet]
	#[pallet::without_storage_info]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config:
		frame_system::Config + pallet_balances::Config + pallet_credits::Config
	{
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
		/// The pallet's id, used for deriving its sovereign account ID.
		#[pallet::constant]
		type PalletId: Get<PalletId>;

		/// The balance type used for this pallet.
		type Balance: Parameter
			+ Member
			+ AtLeast32BitUnsigned
			+ Default
			+ Copy
			+ TryFrom<BalanceOf<Self>>
			+ Into<<Self as pallet_balances::Config>::Balance>;
	}

	// New pallet_balances balance type
	pub type BalanceOf<T> = <T as pallet_balances::Config>::Balance;

	#[pallet::storage]
	pub type AlphaBalances<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, u128, ValueQuery>;

	#[pallet::storage]
	pub type ProcessedEvents<T: Config> =
		StorageMap<_, Blake2_128Concat, Vec<u8>, bool, ValueQuery>;

	#[pallet::storage]
	pub type PendingMints<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		Vec<u8>,                                      // proof (block_hash, event_index)
		(T::AccountId, u128, BTreeSet<T::AccountId>), // (user, amount, confirmations)
		OptionQuery,
	>;

	#[pallet::storage]
	pub type PendingBurns<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		u128, // nonce
		(
			T::AccountId,               // user
			u128,                       // amount
			T::AccountId,               // bittensor_coldkey
			BTreeSet<T::AccountId>,     // confirmations
			Option<(Vec<u8>, Vec<u8>)>, // (bittensor_block_hash, extrinsic_id)
		),
		OptionQuery,
	>;

	// Storage for authority accounts
	#[pallet::storage]
	#[pallet::getter(fn authorities)]
	pub(super) type Authorities<T: Config> = StorageValue<_, Vec<T::AccountId>, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn min_required_signatures)]
	pub type MinRequiredSignatures<T> = StorageValue<_, u32, ValueQuery>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		AlphaMintPending(Vec<u8>, T::AccountId, u128), // (proof, user, amount)
		AlphaMinted(T::AccountId, u128),               // (user, amount)
		AlphaBurnPending(u128, T::AccountId, u128, T::AccountId), // (nonce, user, amount, bittensor_coldkey)
		AlphaBurned(u128, T::AccountId, u128, T::AccountId), // (nonce, user, amount, bittensor_coldkey)
		BurnFinalized(u128, Vec<u8>, Vec<u8>),               // (nonce, block_hash, extrinsic_id)
		AlphaBurnRejected(u128, T::AccountId, u128, T::AccountId), // (nonce, user, amount, bittensor_coldkey)
		AlphaDeposited(T::AccountId, T::AccountId, u128),          // (pallet_account, user, amount)
		AuthorityAdded { who: T::AccountId },
		AuthorityRemoved { who: T::AccountId },
		MinRequiredSignaturesUpdated(u32), // New event for updating min required signatures
	}

	#[pallet::error]
	pub enum Error<T> {
		Unauthorized,           // Not an operator
		DoubleSpendDetected,    // Event already processed
		InsufficientBalance,    // Not enough alpha
		AlreadyConfirmed,       // Operator already confirmed
		NotEnoughConfirmations, // Threshold not met
		InvalidProof,           // Proof verification failed
		BurnNotPending,         // No such burn request
		AuthorityAlreadyExists,
		AuthorityNotFound,
		NotAuthorized,
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Operator proposes and confirms minting alpha
		#[pallet::call_index(0)]
		#[pallet::weight(Weight::from_parts(10_000, 0))]
		pub fn confirm_mint_alpha(
			origin: OriginFor<T>,
			amount: u128,
			user: T::AccountId,
			proof: Vec<u8>, // Encoded (block_hash, event_index)
		) -> DispatchResult {
			let operator = ensure_signed(origin)?;
			Self::ensure_is_authority(&operator)?;

			// Prevent double-spending
			ensure!(!ProcessedEvents::<T>::contains_key(&proof), Error::<T>::DoubleSpendDetected);

			let (_, pending_amount, mut confirmations) =
				PendingMints::<T>::get(&proof).unwrap_or((user.clone(), amount, BTreeSet::new()));

			// Ensure consistent amount
			ensure!(pending_amount == amount, Error::<T>::InvalidProof);

			// Add confirmation
			ensure!(!confirmations.contains(&operator), Error::<T>::AlreadyConfirmed);
			confirmations.insert(operator);

			// Check against the minimum required signatures
			let min_signatures = MinRequiredSignatures::<T>::get();
			if confirmations.len() as u32 >= min_signatures {
				// Threshold met, execute mint to pallet account
				let pallet_account = Self::account_id();
				let current_balance = AlphaBalances::<T>::get(&user);
				let new_balance = current_balance.saturating_add(amount);
				AlphaBalances::<T>::insert(&user, new_balance);
				ProcessedEvents::<T>::insert(&proof, true);
				PendingMints::<T>::remove(&proof);

				// Increase total alpha in pool
				TotalLockedAlpha::<T>::mutate(|alpha| *alpha += amount);

				// mint tokens to bridge pallet
				let _ = pallet_balances::Pallet::<T>::deposit_creating(
					&pallet_account,
					amount.try_into().unwrap_or_default(),
				);

				// Transfer funds from bridge pallet account to user account
				<pallet_balances::Pallet<T>>::transfer(
					&pallet_account,
					&user,
					amount.try_into().unwrap_or_default(),
					ExistenceRequirement::AllowDeath,
				)?;

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
			amount: u128,
			bittensor_coldkey: T::AccountId,
			user: T::AccountId,
			nonce: u128,
		) -> DispatchResult {
			let operator = ensure_signed(origin)?;
			Self::ensure_is_authority(&operator)?;

			let balance = AlphaBalances::<T>::get(&user);
			ensure!(balance >= amount, Error::<T>::InsufficientBalance);

			// Burn alpha immediately but mark as pending
			AlphaBalances::<T>::insert(&user, balance - amount);

			// Burn the equivalent amount from their free balance
			let _ = pallet_balances::Pallet::<T>::burn(
				frame_system::RawOrigin::Signed(user.clone()).into(),
				amount.try_into().unwrap_or_default(),
				false, // keep_alive set to false to allow burning entire balance
			);

			// Decrease total alpha in pool
			TotalLockedAlpha::<T>::mutate(|alpha| *alpha -= amount);

			PendingBurns::<T>::insert(
				&nonce,
				(
					user.clone(),
					amount,
					bittensor_coldkey.clone(),
					BTreeSet::<T::AccountId>::new(),
					None::<(Vec<u8>, Vec<u8>)>,
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
			nonce: u128,
			bittensor_block_hash: Option<Vec<u8>>,
			bittensor_extrinsic_id: Option<Vec<u8>>,
		) -> DispatchResult {
			let operator = ensure_signed(origin)?;
			Self::ensure_is_authority(&operator)?;

			let Some((user, amount, coldkey, mut confirmations, _existing_finalization)) =
				PendingBurns::<T>::get(&nonce)
			else {
				return Err(Error::<T>::BurnNotPending.into());
			};

			// Add confirmation
			ensure!(!confirmations.contains(&operator), Error::<T>::AlreadyConfirmed);
			confirmations.insert(operator);

			// Check against the minimum required signatures
			let min_signatures = MinRequiredSignatures::<T>::get();
			if let (Some(block_hash), Some(extrinsic_id)) =
				(bittensor_block_hash, bittensor_extrinsic_id)
			{
				// Finalize with Bittensor details if provided
				ensure!(
					confirmations.len() as u32 >= min_signatures,
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

				// Remove the finalized burn request from PendingBurns
				PendingBurns::<T>::remove(&nonce);
			} else {
				// Still pending
				PendingBurns::<T>::insert(
					&nonce,
					(user, amount, coldkey, confirmations, None::<(Vec<u8>, Vec<u8>)>),
				);
			}
			Ok(())
		}

		/// Operator rejects a pending burn request and restores the user's alpha balance
		#[pallet::call_index(3)]
		#[pallet::weight(Weight::from_parts(10_000, 0))]
		pub fn reject_burn_request(origin: OriginFor<T>, nonce: u128) -> DispatchResult {
			let operator = ensure_signed(origin)?;
			Self::ensure_is_authority(&operator)?;

			// Threshold met, execute mint to pallet account
			let pallet_account = Self::account_id();

			// Retrieve the pending burn request
			let Some((user, amount, coldkey, _, _)) = PendingBurns::<T>::get(&nonce) else {
				return Err(Error::<T>::BurnNotPending.into());
			};

			// Restore the user's alpha balance
			let current_balance = AlphaBalances::<T>::get(&user);
			AlphaBalances::<T>::insert(&user, current_balance + amount);

			// Revert the total alpha in pool reduction
			TotalLockedAlpha::<T>::mutate(|alpha| *alpha += amount);

			// mint tokens to bridge pallet
			let _ = pallet_balances::Pallet::<T>::deposit_creating(
				&pallet_account,
				amount.try_into().unwrap_or_default(),
			);

			// Transfer funds from pallet_account account to user account
			<pallet_balances::Pallet<T>>::transfer(
				&pallet_account,
				&user,
				amount.try_into().unwrap_or_default(),
				ExistenceRequirement::AllowDeath,
			)?;

			// Remove the pending burn request
			PendingBurns::<T>::remove(&nonce);

			// Emit an event for the burn request rejection
			Self::deposit_event(Event::AlphaBurnRejected(nonce, user, amount, coldkey));

			Ok(())
		}

		/// Add a new authority account (only callable by sudo)
		#[pallet::call_index(4)]
		#[pallet::weight((0, Pays::No))]
		pub fn add_authority(origin: OriginFor<T>, authority: T::AccountId) -> DispatchResult {
			// Ensure only sudo can add authorities
			ensure_root(origin)?;

			// Attempt to add the authority to the bounded vec
			Authorities::<T>::try_mutate(|authorities| -> DispatchResult {
				// Check if authority already exists
				ensure!(!authorities.contains(&authority), Error::<T>::AuthorityAlreadyExists);

				// Try to push the new authority
				authorities.push(authority.clone());

				// Deposit event
				Self::deposit_event(Event::AuthorityAdded { who: authority });

				Ok(())
			})
		}

		/// Remove an authority account (only callable by sudo)
		#[pallet::call_index(5)]
		#[pallet::weight((0, Pays::No))]
		pub fn remove_authority(origin: OriginFor<T>, authority: T::AccountId) -> DispatchResult {
			// Ensure only sudo can remove authorities
			ensure_root(origin)?;

			// Attempt to remove the authority from the bounded vec
			Authorities::<T>::try_mutate(|authorities| -> DispatchResult {
				// Find and remove the authority
				let position = authorities
					.iter()
					.position(|a| a == &authority)
					.ok_or(Error::<T>::AuthorityNotFound)?;

				authorities.remove(position);

				// Deposit event
				Self::deposit_event(Event::AuthorityRemoved { who: authority });

				Ok(())
			})
		}

		/// Set the minimum required signatures for minting and burning
		#[pallet::call_index(6)]
		#[pallet::weight(Weight::from_parts(10_000, 0))]
		pub fn set_min_required_signatures(
			origin: OriginFor<T>,
			min_signatures: u32,
		) -> DispatchResult {
			// Ensure the caller is the sudo account
			let who = ensure_signed(origin)?;
			Self::ensure_is_authority(&who)?;

			// Set the minimum required signatures
			MinRequiredSignatures::<T>::put(min_signatures);

			// Emit an event to notify that the minimum required signatures have been updated
			Self::deposit_event(Event::MinRequiredSignaturesUpdated(min_signatures));

			Ok(())
		}
	}

	impl<T: Config> Pallet<T> {
		pub fn account_id() -> T::AccountId {
			<T as pallet::Config>::PalletId::get().into_account_truncating()
		}

		/// Get the current balance of the Bridge pallet
		pub fn balance() -> BalanceOf<T> {
			pallet_balances::Pallet::<T>::free_balance(&Self::account_id())
		}

		/// Ensure the caller is an authorized account
		pub fn ensure_is_authority(authority: &T::AccountId) -> DispatchResult {
			Authorities::<T>::get()
				.iter()
				.find(|&a| a == authority)
				.ok_or(Error::<T>::NotAuthorized)?;
			Ok(())
		}
	}
}

#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;
pub use types::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;
pub mod weights;
pub use weights::*;
mod types;

use frame_system::offchain::AppCrypto;
use sp_core::offchain::KeyTypeId;

/// Defines application identifier for crypto keys of this module.
///
/// Every module that deals with signatures needs to declare its unique identifier for
/// its crypto keys.
/// When offchain worker is signing transactions it's going to request keys of type
/// `KeyTypeId` from the keystore and use the ones it finds to sign the transaction.
/// The keys can be inserted manually via RPC (see `author_insertKey`).
pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"hips");

/// Based on the above `KeyTypeId` we need to generate a pallet-specific crypto type wrappers.
/// We can use from supported crypto kinds (`sr25519`, `ed25519` and `ecdsa`) and augment
/// the types with this pallet-specific identifier.
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

// All pallet logic is defined in its own module and must be annotated by the `pallet` attribute.
#[frame_support::pallet]
pub mod pallet {
	// Import various useful types required by all FRAME pallets.
	use super::*;
	use frame_support::pallet_prelude::*;
	use frame_system::offchain::SendTransactionTypes;
	use frame_system::offchain::SignedPayload;
	use frame_system::pallet_prelude::*;
	use pallet_ip::Pallet as IpPallet;
	use scale_info::prelude::vec;
	use scale_info::prelude::vec::Vec;
	use sp_core::hashing;
	use sp_runtime::format;

	#[pallet::pallet]
	#[pallet::without_storage_info]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config:
		frame_system::Config
		+ pallet_balances::Config
		+ SendTransactionTypes<Call<Self>>
		+ frame_system::offchain::SigningTypes
		+ pallet_ip::Config
	{
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		/// The identifier type for an offchain worker.
		type AuthorityId: AppCrypto<Self::Public, Self::Signature>;

		#[pallet::constant]
		type RefferallCoolDOwnPeriod: Get<u32>;

		// type OnRuntimeUpgrade: OnRuntimeUpgrade;
	}

	// Storage for authority accounts
	#[pallet::storage]
	#[pallet::getter(fn authorities)]
	pub(super) type Authorities<T: Config> = StorageValue<_, Vec<T::AccountId>, ValueQuery>;

	// Mapping to track the lifetime rewards earned by each referral code
	#[pallet::storage]
	pub(super) type ReferralCodeRewards<T: Config> =
		StorageMap<_, Blake2_128Concat, Vec<u8>, u128, ValueQuery>;

	// Mapping to track the number of times a referral code has been used
	#[pallet::storage]
	pub(super) type ReferralCodeUsageCount<T: Config> =
		StorageMap<_, Blake2_128Concat, Vec<u8>, u32, ValueQuery>;

	// Mapping to track the total number of referral codes created
	#[pallet::storage]
	pub(super) type TotalReferralCodes<T: Config> = StorageValue<_, u32, ValueQuery>;

	// Mapping to track the total number of referral codes created
	#[pallet::storage]
	pub(super) type TotalSucessfullCreditsTransfers<T: Config> = StorageValue<_, u128, ValueQuery>;

	// Mapping to store the last block number a user created a referral code
	#[pallet::storage]
	#[pallet::getter(fn last_referral_creation_block)]
	pub type LastReferralCreationBlock<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, BlockNumberFor<T>>;

	// Mapping to track the total referral rewards earned
	#[pallet::storage]
	pub(super) type TotalReferralRewards<T: Config> = StorageValue<_, u128, ValueQuery>;

	// get refferal code for an account (ReferralCode -> AccountId)
	#[pallet::storage]
	#[pallet::getter(fn referral_codes)]
	pub type ReferralCodes<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, T::AccountId>;

	// get refferal code used by account ( AccountId -> Option<ReferralCode> )
	#[pallet::storage]
	#[pallet::getter(fn referred_users)]
	pub type ReferredUsers<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, Vec<u8>>;

	// This storage is used for pool price tracking, Alpha
	#[pallet::storage]
	#[pallet::getter(fn total_credits_purchased)]
	pub type TotalCreditsPurchased<T> = StorageValue<_, u128, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn total_locked_alpha)]
	pub type TotalLockedAlpha<T> = StorageValue<_, u128, ValueQuery>;

	// Define separate storage for free credits.
	#[pallet::storage]
	#[pallet::getter(fn free_credits)]
	pub(super) type FreeCredits<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, u128, ValueQuery>;

	// Storage for the current active lock period
	#[pallet::storage]
	#[pallet::getter(fn current_lock_period)]
	pub type CurrentLockPeriod<T: Config> =
		StorageValue<_, LockPeriod<BlockNumberFor<T>>, OptionQuery>;

	// Storage for the current active lock period
	#[pallet::storage]
	#[pallet::getter(fn min_lock_amount)]
	pub type MinLockAmount<T: Config> = StorageValue<_, u128, OptionQuery>;

	// Define separate storage for locked credits
	#[pallet::storage]
	#[pallet::getter(fn locked_credits)]
	pub(super) type LockedCredits<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		T::AccountId,
		Vec<LockedCredit<T::AccountId, BlockNumberFor<T>>>,
		ValueQuery,
	>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		MintedAccountCredits {
			who: T::AccountId,
			amount: u128,
		},
		BurnedAccountCredits {
			who: T::AccountId,
			amount: u128,
		},
		AuthorityAdded {
			who: T::AccountId,
		},
		AuthorityRemoved {
			who: T::AccountId,
		},
		ConvertedToCredits {
			who: T::AccountId,
			amount: u128,
		},
		CreditLocked {
			who: T::AccountId,
			amount: u128,
			id: u64,
		},
		CreditFulfilled {
			account_id: T::AccountId,
			id: u64,
			tx_hash: Vec<u8>,
		},
		MinLockAmountSet {
			amount: u128,
			who: T::AccountId,
		},
		/// Event emitted when a referral discount is applied
		ReferralDiscountApplied {
			referral_code: Vec<u8>,
			ref_owner: T::AccountId,
			discount_amount: u128,
		},
		ConvertedToAlpha {
			who: T::AccountId,
			amount: u128,
		},
		IncreasedUserBalance {
			who: T::AccountId,
			marketplace_credit_amount: u128,
			alpha_amount: u128,
		},
	}

	#[pallet::error]
	pub enum Error<T> {
		NoneValue,
		StorageOverflow,
		InsufficientFreeCredits,
		UserNotFound,
		InsufficientLockedCredits,
		NotAuthorized,
		AuthorityAlreadyExists,
		AuthorityNotFound,
		InvalidConversionAmount,
		InsufficientBalance,
		ConversionFailed,
		InvalidReferralCode,
		ReferralCodeCooldown,
		NoReferralCodeUsed,
		InvalidRefferalOwner,
		CreditAlreadyFulfilled,
		LockedCreditNotFound,
		/// Returned if the account has insufficient free credits
		// InsufficientFreeCredits,
		/// Returned if the current block is outside the specified lock period
		OutsideLockPeriod,
		/// Returned if no active lock period is set
		NoActiveLockPeriod,
		InvalidLockPeriod,
		/// Minimum lock amount is not set
		MinLockAmountNotSet,
		/// Locked amount is less than the minimum required lock amount
		InsufficientLockAmount,
		InsufficientAlphaBalance,
	}

	/// Payload for unsigned credits decrease transaction
	#[derive(Encode, Decode, Clone, PartialEq, Eq, TypeInfo)]
	pub struct DecreaseCreditsPayload<T: Config> {
		pub public: T::Public,
		pub account: T::AccountId,
		pub amount: u128,
		pub _marker: PhantomData<T>,
	}

	// Implement SignedPayload for UpdateRankingsPayload
	impl<T: Config> SignedPayload<T> for DecreaseCreditsPayload<T> {
		fn public(&self) -> T::Public {
			self.public.clone()
		}
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Add a new authority account (only callable by sudo)
		#[pallet::call_index(0)]
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
		#[pallet::call_index(1)]
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

		/// Mint credits (only callable by authority accounts)
		#[pallet::call_index(2)]
		#[pallet::weight((0, Pays::No))]
		pub fn mint(
			origin: OriginFor<T>,
			who: T::AccountId,
			amount: u128,
			code: Option<Vec<u8>>,
		) -> DispatchResult {
			// Ensure the caller is an authority
			let authority = ensure_signed(origin)?;
			Self::ensure_is_authority(&authority)?;

			// Call the helper function
			Self::do_mint(who, amount, code)
		}

		/// Burn credits (only callable by authority accounts)
		#[pallet::call_index(3)]
		#[pallet::weight((0, Pays::No))]
		pub fn burn(origin: OriginFor<T>, who: T::AccountId, amount: u128) -> DispatchResult {
			// Ensure the caller is an authority
			let authority = ensure_signed(origin)?;
			Self::ensure_is_authority(&authority)?;

			// Get the current free credits of the user
			let free = FreeCredits::<T>::get(&who);

			// Ensure there are enough free credits to burn
			ensure!(free >= amount, Error::<T>::InsufficientFreeCredits);

			// Update free credits
			FreeCredits::<T>::insert(&who, free - amount);

			Self::deposit_event(Event::BurnedAccountCredits { who, amount: free - amount });

			Ok(())
		}

		#[pallet::call_index(4)]
		#[pallet::weight((0, Pays::No))]
		pub fn increase_user_balance(
			origin: OriginFor<T>,
			marketplace_credit_amount: u128,
			alpha_amount: u128,
			user_to_credit: T::AccountId,
		) -> DispatchResult {
			// Ensure the caller is an authority
			ensure_root(origin)?;

			// Decrease the alpha balance of the sudo key
			let current_alpha_balance = TotalLockedAlpha::<T>::get();
			ensure!(current_alpha_balance >= alpha_amount, Error::<T>::InsufficientAlphaBalance);

			TotalLockedAlpha::<T>::set(current_alpha_balance - alpha_amount);

			// Increase the user's credits
			FreeCredits::<T>::mutate(&user_to_credit, |credits| {
				*credits += marketplace_credit_amount
			});

			// Emit event for balance increase
			Self::deposit_event(Event::IncreasedUserBalance {
				who: user_to_credit.clone(),
				marketplace_credit_amount,
				alpha_amount,
			});

			Ok(())
		}

		// /// Convert user credits to alpha
		// #[pallet::call_index(5)]
		// #[pallet::weight((0, Pays::No))]
		// pub fn convert_credits_to_alpha(
		//     origin: OriginFor<T>,
		//     amount: u128
		// ) -> DispatchResult {
		//     // Ensure the caller is a signed origin
		//     let who = ensure_signed(origin)?;

		//     // Ensure the amount is greater than zero
		//     ensure!(amount > 0, Error::<T>::InvalidConversionAmount);

		//     // Ensure the user has enough credits
		//     ensure!(
		//         FreeCredits::<T>::get(&who) >= amount,
		//         Error::<T>::InsufficientFreeCredits
		//     );

		//     // Decrease the user's credits
		//     FreeCredits::<T>::mutate(&who, |credits| *credits -= amount);

		//     // Increase the total locked alpha by the converted amount
		//     TotalLockedAlpha::<T>::mutate(|total| *total += amount);

		//     // Deposit an event
		//     Self::deposit_event(Event::ConvertedToAlpha { who, amount });

		//     Ok(())
		// }

		// /// Convert alpha to user credits
		// #[pallet::call_index(6)]
		// #[pallet::weight((0, Pays::No))]
		// pub fn convert_alpha_to_credits(
		//     origin: OriginFor<T>,
		//     alpha_amount: u128,
		//     user_to_credit: T::AccountId,
		// ) -> DispatchResult {
		//     // Ensure the caller is a signed origin
		//     let who = ensure_signed(origin)?;

		//     // Ensure the amount is greater than zero
		//     ensure!(alpha_amount > 0, Error::<T>::InvalidConversionAmount);

		//     // Ensure the total locked alpha is sufficient
		//     let current_alpha_balance = TotalLockedAlpha::<T>::get();
		//     ensure!(current_alpha_balance >= alpha_amount, Error::<T>::InsufficientAlphaBalance);

		//     // Decrease the total locked alpha by the specified amount
		//     TotalLockedAlpha::<T>::mutate(|total| *total -= alpha_amount);

		//     // Increase the user's credits
		//     FreeCredits::<T>::mutate(&user_to_credit, |credits| *credits += alpha_amount);

		//     // Call the burn function from balances pallet
		//       pallet_balances::Pallet::<T>::burn(
		//         frame_system::RawOrigin::Signed(who.clone()).into(),
		//         alpha_amount,
		//         false, // keep_alive set to false to allow burning entire balance
		//     )?;

		//     // Emit an event for the conversion
		//     Self::deposit_event(Event::ConvertedToCredits { who: user_to_credit.clone(), amount: alpha_amount });

		//     Ok(())
		// }

		// creates refferal for a user
		#[pallet::call_index(8)]
		#[pallet::weight((0, Pays::No))]
		pub fn create_referral_code(origin: OriginFor<T>) -> DispatchResult {
			let creator = ensure_signed(origin)?;

			Self::do_change_referral_code(creator)?;

			// Self::deposit_event(Event::ReferralCodeCreated { creator, code });
			Ok(())
		}

		// Changes the referral code of a user automatically
		#[pallet::call_index(9)] // New call index, you can choose your own
		#[pallet::weight((0, Pays::No))]
		pub fn change_referral_code(origin: OriginFor<T>) -> DispatchResult {
			let who = ensure_signed(origin)?;

			Self::do_change_referral_code(who)?;

			Ok(())
		}

		// /// Lock a specified amount of credits for an account
		// ///
		// /// - `origin`: The account locking the credits
		// /// - `amount`: The amount of credits to lock
		// #[pallet::call_index(8)]
		// #[pallet::weight((0, Pays::No))]
		// pub fn lock_credits(
		//     origin: OriginFor<T>,
		//     amount: u128,
		// ) -> DispatchResult {
		//     // Ensure the caller is signed
		//     let who = ensure_signed(origin)?;

		//     // Get current block number
		//     let current_block = frame_system::Pallet::<T>::block_number();

		//     // Check if there's an active lock period
		//     let lock_period = Self::current_lock_period()
		//         .ok_or(Error::<T>::NoActiveLockPeriod)?;

		//     // Validate current block is within the lock period
		//     ensure!(
		//         current_block >= lock_period.start_block &&
		//         current_block <= lock_period.end_block,
		//         Error::<T>::OutsideLockPeriod
		//     );

		//     // Ensure the account has sufficient free credits
		//     let current_free_credits = Self::free_credits(&who);
		//     ensure!(current_free_credits >= amount, Error::<T>::InsufficientFreeCredits);

		//     // Check if the locked amount meets the minimum lock amount requirement
		//     let min_lock_amount = Self::min_lock_amount()
		//     .ok_or(Error::<T>::MinLockAmountNotSet)?;
		//     ensure!(amount >= min_lock_amount, Error::<T>::InsufficientLockAmount);

		//     // Generate a unique ID
		//     let locked_credit_id = Self::generate_unique_id();

		//     // Create the locked credit struct
		//     let locked_credit = LockedCredit {
		//         owner: who.clone(),
		//         amount_locked: amount,
		//         is_fulfilled: false,
		//         tx_hash: None,
		//         created_at: frame_system::Pallet::<T>::block_number(),
		//         id: locked_credit_id,
		//         is_migrated: false,
		//     };

		//     // Update locked credits
		//     LockedCredits::<T>::mutate(&who, |credits| {
		//         credits.push(locked_credit);
		//     });

		//     // Reduce free credits
		//     FreeCredits::<T>::mutate(&who, |free| *free -= amount);

		//     Self::deposit_event(Event::CreditLocked{who, amount, id: locked_credit_id});

		//     Ok(())
		// }

		/// Mark a locked credit as fulfilled by providing a transaction hash
		///
		/// - `origin`: The account that originally locked the credits
		/// - `locked_credit_id`: The ID of the locked credit to mark as fulfilled
		/// - `tx_hash`: The transaction hash proving fulfillment
		#[pallet::call_index(10)]
		#[pallet::weight((0, Pays::No))]
		pub fn fulfill_locked_credits(
			origin: OriginFor<T>,
			locked_credit_id: u64,
			account_id: T::AccountId,
			tx_hash: Vec<u8>,
		) -> DispatchResult {
			// Ensure the caller is an authority
			let authority = ensure_signed(origin)?;
			Self::ensure_is_authority(&authority)?;

			// Find and update the specific locked credit
			let amount_fulfilled = LockedCredits::<T>::mutate(&account_id, |credits| {
				// Find the credit with the matching ID
				if let Some(credit) = credits.iter_mut().find(|c| c.id == locked_credit_id) {
					// Ensure the credit is not already fulfilled
					ensure!(!credit.is_fulfilled, Error::<T>::CreditAlreadyFulfilled);

					// Mark as fulfilled and set the transaction hash
					credit.is_fulfilled = true;
					credit.tx_hash = Some(tx_hash.clone());

					// Return the amount of credits fulfilled
					Ok(credit.amount_locked)
				} else {
					// No matching locked credit found
					Err(Error::<T>::LockedCreditNotFound)
				}
			})?;

			// Increment the total successful credits transfers
			TotalSucessfullCreditsTransfers::<T>::mutate(|total| *total += amount_fulfilled);

			// Deposit an event for the fulfillment
			Self::deposit_event(Event::CreditFulfilled {
				account_id,
				id: locked_credit_id,
				tx_hash,
			});

			Ok(())
		}

		#[pallet::call_index(11)]
		#[pallet::weight((0, Pays::No))]
		pub fn set_lock_period(
			origin: OriginFor<T>,
			start_block: u32,
			end_block: u32,
		) -> DispatchResult {
			// Ensure the caller is an authority
			let authority = ensure_signed(origin)?;
			Self::ensure_is_authority(&authority)?;

			// Validate block period
			ensure!(start_block < end_block, Error::<T>::InvalidLockPeriod);

			// Set the current lock period
			CurrentLockPeriod::<T>::put(LockPeriod {
				start_block: start_block.into(),
				end_block: end_block.into(),
			});

			Ok(())
		}

		/// Set the minimum lock amount (only callable by authorized accounts)
		#[pallet::call_index(12)]
		#[pallet::weight((0, Pays::No))]
		pub fn set_min_lock_amount(origin: OriginFor<T>, amount: u128) -> DispatchResult {
			// Ensure the caller is an authorized account
			let authority = ensure_signed(origin)?;
			Self::ensure_is_authority(&authority)?;

			// Set the minimum lock amount
			MinLockAmount::<T>::put(amount);

			// Deposit an event
			Self::deposit_event(Event::MinLockAmountSet { amount, who: authority });

			Ok(())
		}
	}

	impl<T: Config> Pallet<T> {
		pub fn do_mint(who: T::AccountId, amount: u128, code: Option<Vec<u8>>) -> DispatchResult {
			// Get the current free credits of the user
			let free = FreeCredits::<T>::get(&who);

			// Update free credits
			FreeCredits::<T>::insert(&who, free + amount);

			// Increase total credits purchased
			TotalCreditsPurchased::<T>::mutate(|total| *total += amount);

			// Helper function to insert a referral code for a user
			Self::insert_referral_code(&who.clone(), code)?;

			let client_ip = IpPallet::<T>::get_client_ip(&who.clone());

			if client_ip.is_none() {
				let mut vm_available_ips = IpPallet::<T>::available_client_ips();

				if let Some(ip) = vm_available_ips.pop() {
					let _ = IpPallet::<T>::assign_ip_to_client(who.clone(), ip);
				}
			}

			Self::deposit_event(Event::MintedAccountCredits { who, amount: free + amount });

			Ok(())
		}

		/// Helper function to get the free credits of an account.
		pub fn get_free_credits(account: &T::AccountId) -> u128 {
			FreeCredits::<T>::get(account)
		}

		pub fn increase_user_credits(account: &T::AccountId, credits_to_increase: u128) {
			FreeCredits::<T>::mutate(&account, |credits| *credits += credits_to_increase);

			Self::deposit_event(Event::MintedAccountCredits {
				who: account.clone(),
				amount: credits_to_increase,
			});
		}

		pub fn decrease_user_credits(account: &T::AccountId, credits_to_decrease: u128) {
			FreeCredits::<T>::mutate(&account, |credits| *credits -= credits_to_decrease);

			Self::deposit_event(Event::BurnedAccountCredits {
				who: account.clone(),
				amount: credits_to_decrease,
			});
		}

		/// Ensure the caller is an authorized account
		pub fn ensure_is_authority(authority: &T::AccountId) -> DispatchResult {
			Authorities::<T>::get()
				.iter()
				.find(|&a| a == authority)
				.ok_or(Error::<T>::NotAuthorized)?;
			Ok(())
		}

		// gives referral rewards to owners
		pub fn apply_referral_discount(
			code: &Vec<u8>,
			price: u128,
			total_discount: &mut u128,
		) -> DispatchResult {
			// Log if the referral code exists
			if !ReferralCodes::<T>::contains_key(code) {
				log::warn!("Invalid referral code: {:?}", code);
				return Err(Error::<T>::InvalidReferralCode.into());
			}

			// Log the referral code owner
			match ReferralCodes::<T>::get(code) {
				Some(ref_owner) => {
					let ref_discount = price.saturating_mul(5) / 100 as u128;

					Self::increase_user_credits(&ref_owner, ref_discount);

					*total_discount = total_discount.saturating_add(ref_discount);

					ReferralCodeRewards::<T>::mutate(code, |r| *r = r.saturating_add(ref_discount));

					ReferralCodeUsageCount::<T>::mutate(code, |c| *c = c.saturating_add(1));

					// total rewards
					TotalReferralRewards::<T>::mutate(|reward| {
						*reward = reward.saturating_add(ref_discount);
					});

					Self::deposit_event(Event::ReferralDiscountApplied {
						referral_code: code.clone(),
						ref_owner: ref_owner.clone(),
						discount_amount: ref_discount,
					});
					log::info!("Deposited ReferralDiscountApplied event");
				},
				None => {
					log::warn!("No owner found for referral code: {:?}", code);
				},
			}

			Ok(())
		}

		/// Helper function to insert a referral code for a user
		pub fn insert_referral_code(owner: &T::AccountId, code: Option<Vec<u8>>) -> DispatchResult {
			if code.is_some() {
				let referral_code = code.unwrap();
				// Validate referral code if needed
				ensure!(!referral_code.is_empty(), Error::<T>::InvalidReferralCode);
				ensure!(
					ReferralCodes::<T>::contains_key(&referral_code),
					Error::<T>::InvalidReferralCode
				);
				ReferredUsers::<T>::insert(owner, referral_code);
			}

			Ok(())
		}

		pub fn do_change_referral_code(who: T::AccountId) -> DispatchResult {
			// Get the current block number
			let current_block = frame_system::Pallet::<T>::block_number();

			// Check if the user has created a referral code within the last 100 blocks
			let last_creation_block = LastReferralCreationBlock::<T>::get(&who);
			if let Some(last_block) = last_creation_block {
				ensure!(
					current_block > last_block + T::RefferallCoolDOwnPeriod::get().into(),
					Error::<T>::ReferralCodeCooldown
				);
			}

			// Generate a unique referral code with the prefix "HIPPIUS"
			let mut unique_code = format!("HIPPIUS{}", Self::generate_random_suffix(&who));

			// Ensure the generated code is unique
			while ReferralCodes::<T>::contains_key(unique_code.as_bytes()) {
				unique_code = format!("HIPPIUS{}", Self::generate_random_suffix(&who));
			}

			// // Fetch the current referral code owner
			// let previous_referral_owner = ReferralCodes::<T>::get(&existing_code.unwrap());
			// ensure!(previous_referral_owner == Some(who.clone()), Error::<T>::InvalidRefferalOwner);

			// Insert the newly generated code into ReferralCodes mapping
			ReferralCodes::<T>::insert(unique_code.as_bytes(), &who);

			TotalReferralCodes::<T>::mutate(|total| {
				*total = total.saturating_add(1u32);
			});

			// Update the last referral creation block for the user
			LastReferralCreationBlock::<T>::insert(&who, current_block);

			Ok(())
		}

		// creates refferal for a user
		pub fn do_create_referral_code(creator: T::AccountId) -> DispatchResult {
			// // Check if the user already has a referral code
			// ensure!(
			//     !ReferralCodes::<T>::iter_values().any(|acc| acc == creator),
			//     Error::<T>::AlreadyHasReferralCode
			// );

			// Get the current block number
			let current_block = frame_system::Pallet::<T>::block_number();

			// Check if the user has created a referral code within the last 100 blocks
			let last_creation_block = LastReferralCreationBlock::<T>::get(&creator);
			if let Some(last_block) = last_creation_block {
				ensure!(
					current_block > last_block + T::RefferallCoolDOwnPeriod::get().into(),
					Error::<T>::ReferralCodeCooldown
				);
			}

			// Generate a unique referral code with the prefix "HIPPIUS"
			let mut unique_code = format!("HIPPIUS{}", Self::generate_random_suffix(&creator));

			// Ensure the generated code is unique
			while ReferralCodes::<T>::contains_key(unique_code.as_bytes()) {
				unique_code = format!("HIPPIUS{}", Self::generate_random_suffix(&creator));
			}

			// Insert the generated referral code into storage
			ReferralCodes::<T>::insert(unique_code.as_bytes(), &creator);

			// total rewards
			TotalReferralCodes::<T>::mutate(|total| {
				*total = total.saturating_add(1u32);
			});

			// Update the last referral creation block for the user
			LastReferralCreationBlock::<T>::insert(&creator, current_block);

			// Self::deposit_event(Event::ReferralCodeCreated { creator, code });
			Ok(())
		}

		fn generate_random_suffix(account: &T::AccountId) -> u64 {
			let nonce = frame_system::Pallet::<T>::block_number();

			// Convert the block number to a primitive type (e.g., u64)
			let nonce_as_u64 = TryInto::<u64>::try_into(nonce).unwrap_or_default(); // Handle conversion safely

			let mut random_data = account.using_encoded(|b| b.to_vec());
			random_data.extend_from_slice(&nonce_as_u64.to_le_bytes());

			// Generate a hash and use its output as a number
			let hash = hashing::blake2_128(&random_data);
			u64::from_le_bytes([
				hash[0], hash[1], hash[2], hash[3], hash[4], hash[5], hash[6], hash[7],
			])
		}

		pub fn get_free_credits_rpc(account: Option<T::AccountId>) -> Vec<(T::AccountId, u128)> {
			// If an account is provided, return its free credits
			if let Some(acc) = account {
				let credits = FreeCredits::<T>::get(&acc); // Directly get the credits
				vec![(acc, credits)] // Return credits, assuming it defaults to 0 if not found
			} else {
				// If no account is provided, return all accounts and their free credits
				FreeCredits::<T>::iter()
					.map(|(account_id, credits)| (account_id, credits))
					.collect()
			}
		}

		// Get all users referred by a given account
		pub fn get_referred_users(account_id: T::AccountId) -> Vec<T::AccountId> {
			// Get the referral codes for the account
			let codes = ReferredUsers::<T>::get(&account_id).unwrap_or_default();

			// Filter and map the codes to their corresponding account IDs
			codes
				.iter()
				.filter_map(|code| {
					// Create a Vec<u8> from the u8 code
					let code_vec: Vec<u8> = vec![*code]; // Wrap the u8 in a Vec<u8>
					ReferralCodes::<T>::get(&code_vec) // Use &code_vec to retrieve the account ID
				})
				.collect()
		}

		// Get total referral rewards earned by a given account
		pub fn get_referral_rewards(account_id: T::AccountId) -> u128 {
			let codes = ReferredUsers::<T>::get(&account_id).unwrap_or_default();

			// If there are no referral codes, return 0
			if codes.is_empty() {
				return 0;
			}

			codes
				.iter()
				.filter_map(|code| {
					// Create a Vec<u8> from the u8 code
					let code_vec: Vec<u8> = vec![*code]; // Wrap the u8 in a Vec<u8>
										  // Retrieve the rewards and wrap in Some
					Some(ReferralCodeRewards::<T>::get(&code_vec))
				})
				.sum() // Sum the results to get total rewards
		}

		pub fn total_referral_codes() -> u32 {
			TotalReferralCodes::<T>::get()
		}

		pub fn total_referral_rewards() -> u128 {
			TotalReferralRewards::<T>::get()
		}

		// Get all referral codes owned by a given account
		pub fn get_referral_codes(account_id: T::AccountId) -> Vec<Vec<u8>> {
			ReferralCodes::<T>::iter()
				.filter_map(|(code, owner)| if owner == account_id { Some(code) } else { None })
				.collect()
		}
	}
}

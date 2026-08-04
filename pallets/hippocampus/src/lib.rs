#![cfg_attr(not(feature = "std"), no_std)]

//! # Hippocampus Pallet (the bank)
//!
//! Holds the funds used to pay Arion storage miners.
//!
//! - `deposit(amount, deposit_type)`: anyone can fund the bank sovereign account,
//!   tagging the deposit with its source (`DepositType`).
//! - `request_payment(requester, dest, amount)`: internal API (not an extrinsic) —
//!   only whitelisted requester accounts (e.g. the Arion pallet sovereign account)
//!   can pull funds. Pays out at most the available balance and returns the amount
//!   actually paid; the caller is responsible for handling shortfalls.
//! - `add_requester` / `remove_requester`: admin-gated whitelist management.

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

pub mod weights;

pub use pallet::*;
pub use weights::WeightInfo;

#[frame_support::pallet]
pub mod pallet {
	use crate::weights::WeightInfo;
	use frame_support::{
		pallet_prelude::*,
		traits::{Currency, ExistenceRequirement, Get},
		PalletId,
	};
	use frame_system::pallet_prelude::*;
	use sp_runtime::traits::{AccountIdConversion, Saturating, Zero};

	pub type BalanceOf<T> =
		<<T as Config>::Currency as Currency<<T as frame_system::Config>::AccountId>>::Balance;

	#[pallet::pallet]
	#[pallet::without_storage_info]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config {
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

		/// Currency held and paid out by the bank.
		type Currency: Currency<Self::AccountId>;

		/// Pallet id from which the bank sovereign account is derived.
		#[pallet::constant]
		type PalletId: Get<PalletId>;

		/// Origin allowed to manage the requester whitelist.
		type AdminOrigin: EnsureOrigin<Self::RuntimeOrigin>;

		type WeightInfo: WeightInfo;
	}

	/// Source of deposited funds.
	#[derive(Clone, Copy, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
	pub enum DepositType {
		/// Revenue collected through the marketplace (credits deposits,
		/// subscriptions) — storage and compute are not distinguishable there.
		MarketplaceRevenue,
		/// Protocol emissions.
		Emission,
		/// One-off grant / treasury top-up.
		Grant,
		/// Anything else.
		Other,
	}

	/// Accounts allowed to call `request_payment`.
	#[pallet::storage]
	pub type WhitelistedRequesters<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, (), OptionQuery>;

	/// Per-requester withdrawal cap (optional). Prevents any whitelisted caller
	/// from draining the entire bank. Compartmentalization: limits per-pallet risk.
	/// If not set (None), requester can withdraw up to available balance (legacy behavior).
	/// If set, requester is capped at min(requested, cap).
	#[pallet::storage]
	pub type RequesterWithdrawalCap<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, BalanceOf<T>, OptionQuery>;

	/// Lifetime total deposited, per deposit type.
	/// NOTE: Not a solvency proof. Can desync from reality if funds are transferred
	/// directly into the bank account without going through `deposit_from`. Only the
	/// free balance (`balance()`) reflects actual bank state; ops dashboards must use
	/// that for solvency checks, not this counter.
	#[pallet::storage]
	pub type TotalDeposited<T: Config> =
		StorageMap<_, Blake2_128Concat, DepositType, BalanceOf<T>, ValueQuery>;

	/// Lifetime total released through `request_payment`.
	/// NOTE: Not a solvency proof; see TotalDeposited warning. Use free balance for truth.
	#[pallet::storage]
	pub type TotalPaidOut<T: Config> = StorageValue<_, BalanceOf<T>, ValueQuery>;

	/// Lifetime total released per requester (e.g. arion vs compute escrow).
	#[pallet::storage]
	pub type TotalPaidByRequester<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, BalanceOf<T>, ValueQuery>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// Funds were deposited into the bank.
		Deposited { who: T::AccountId, amount: BalanceOf<T>, deposit_type: DepositType },
		/// An account was added to the requester whitelist.
		RequesterAdded { who: T::AccountId },
		/// An account was removed from the requester whitelist.
		RequesterRemoved { who: T::AccountId },
		/// A withdrawal cap was set for a whitelisted requester.
		RequesterCapSet { who: T::AccountId, cap: BalanceOf<T> },
		/// A withdrawal cap was removed for a whitelisted requester.
		RequesterCapRemoved { who: T::AccountId },
		/// A payment was released to `dest`. `paid` may be lower than `requested`
		/// when the bank balance is insufficient.
		PaymentReleased {
			requester: T::AccountId,
			dest: T::AccountId,
			requested: BalanceOf<T>,
			paid: BalanceOf<T>,
		},
	}

	#[pallet::error]
	pub enum Error<T> {
		/// The caller of `request_payment` is not whitelisted.
		RequesterNotWhitelisted,
		/// Amount must be non-zero.
		ZeroAmount,
		/// Account is already whitelisted.
		AlreadyWhitelisted,
		/// Account is not in the whitelist.
		NotWhitelisted,
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Deposit funds into the bank, tagged with their source type.
		#[pallet::call_index(0)]
		#[pallet::weight(<T as Config>::WeightInfo::deposit())]
		pub fn deposit(
			origin: OriginFor<T>,
			amount: BalanceOf<T>,
			deposit_type: DepositType,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;
			Self::deposit_from(&who, amount, deposit_type)
		}

		/// Add an account to the requester whitelist.
		#[pallet::call_index(1)]
		#[pallet::weight(<T as Config>::WeightInfo::add_requester())]
		pub fn add_requester(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
			T::AdminOrigin::ensure_origin(origin)?;
			ensure!(
				!WhitelistedRequesters::<T>::contains_key(&who),
				Error::<T>::AlreadyWhitelisted
			);
			WhitelistedRequesters::<T>::insert(&who, ());
			Self::deposit_event(Event::RequesterAdded { who });
			Ok(())
		}

		/// Remove an account from the requester whitelist.
		#[pallet::call_index(2)]
		#[pallet::weight(<T as Config>::WeightInfo::remove_requester())]
		pub fn remove_requester(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
			T::AdminOrigin::ensure_origin(origin)?;
			ensure!(WhitelistedRequesters::<T>::contains_key(&who), Error::<T>::NotWhitelisted);
			WhitelistedRequesters::<T>::remove(&who);
			Self::deposit_event(Event::RequesterRemoved { who });
			Ok(())
		}

		/// Set a per-requester withdrawal cap. Prevents any single whitelisted caller
		/// from draining the entire bank, compartmentalizing risk per pallet.
		/// The requester will be capped at min(requested, cap) in request_payment.
		#[pallet::call_index(3)]
		#[pallet::weight(<T as Config>::WeightInfo::add_requester())]
		pub fn set_requester_cap(origin: OriginFor<T>, who: T::AccountId, cap: BalanceOf<T>) -> DispatchResult {
			T::AdminOrigin::ensure_origin(origin)?;
			ensure!(WhitelistedRequesters::<T>::contains_key(&who), Error::<T>::NotWhitelisted);
			RequesterWithdrawalCap::<T>::insert(&who, cap);
			Self::deposit_event(Event::RequesterCapSet { who, cap });
			Ok(())
		}

		/// Remove a per-requester withdrawal cap (restores unlimited withdrawal up to available balance).
		#[pallet::call_index(4)]
		#[pallet::weight(<T as Config>::WeightInfo::remove_requester())]
		pub fn remove_requester_cap(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
			T::AdminOrigin::ensure_origin(origin)?;
			ensure!(WhitelistedRequesters::<T>::contains_key(&who), Error::<T>::NotWhitelisted);
			RequesterWithdrawalCap::<T>::remove(&who);
			Self::deposit_event(Event::RequesterCapRemoved { who });
			Ok(())
		}
	}

	impl<T: Config> Pallet<T> {
		/// The bank sovereign account.
		pub fn account_id() -> T::AccountId {
			T::PalletId::get().into_account_truncating()
		}

		/// Funds `request_payment` could release right now (free balance minus
		/// the existential deposit the bank always keeps).
		pub fn available_for_payout() -> BalanceOf<T> {
			T::Currency::free_balance(&Self::account_id())
				.saturating_sub(T::Currency::minimum_balance())
		}

		/// Transfer `amount` from `who` into the bank and record it. Shared by
		/// the `deposit` extrinsic and other pallets (e.g. the marketplace
		/// routing deposit alpha backing to the bank).
		pub fn deposit_from(
			who: &T::AccountId,
			amount: BalanceOf<T>,
			deposit_type: DepositType,
		) -> DispatchResult {
			ensure!(!amount.is_zero(), Error::<T>::ZeroAmount);
			T::Currency::transfer(
				who,
				&Self::account_id(),
				amount,
				ExistenceRequirement::KeepAlive,
			)?;
			TotalDeposited::<T>::mutate(deposit_type, |t| *t = t.saturating_add(amount));
			Self::deposit_event(Event::Deposited { who: who.clone(), amount, deposit_type });
			Ok(())
		}

		/// Free balance held by the bank.
		pub fn balance() -> BalanceOf<T> {
			T::Currency::free_balance(&Self::account_id())
		}

		/// Internal payment API — deliberately NOT an extrinsic.
		///
		/// Releases up to `amount` from the bank to `dest` on behalf of a
		/// whitelisted `requester` (typically another pallet's sovereign
		/// account). Never overdraws: pays `min(amount, free - ED, cap)` and returns
		/// the amount actually paid so the caller can account for the shortfall.
		/// Per-requester caps (if set) prevent any single caller from draining the bank,
		/// compartmentalizing risk across pallets.
		pub fn request_payment(
			requester: &T::AccountId,
			dest: &T::AccountId,
			amount: BalanceOf<T>,
		) -> Result<BalanceOf<T>, DispatchError> {
			ensure!(
				WhitelistedRequesters::<T>::contains_key(requester),
				Error::<T>::RequesterNotWhitelisted
			);
			if amount.is_zero() {
				return Ok(Zero::zero());
			}
			let bank = Self::account_id();
			let mut capped_amount = amount.min(Self::available_for_payout());
			// Apply per-requester cap if set (compartmentalization wall).
			if let Some(cap) = RequesterWithdrawalCap::<T>::get(requester) {
				capped_amount = capped_amount.min(cap);
			}
			let paid = capped_amount;
			if !paid.is_zero() {
				T::Currency::transfer(&bank, dest, paid, ExistenceRequirement::KeepAlive)?;
				TotalPaidOut::<T>::mutate(|t| *t = t.saturating_add(paid));
				TotalPaidByRequester::<T>::mutate(requester, |t| *t = t.saturating_add(paid));
				Self::deposit_event(Event::PaymentReleased {
					requester: requester.clone(),
					dest: dest.clone(),
					requested: amount,
					paid,
				});
			}
			Ok(paid)
		}
	}
}

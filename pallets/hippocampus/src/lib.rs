#![cfg_attr(not(feature = "std"), no_std)]

//! # Hippocampus Pallet (the bank)
//!
//! Holds the funds used to pay Arion storage miners and marketplace
//! referral commissions.
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
	use payment_math::Tokens;
	use sp_runtime::traits::{AccountIdConversion, SaturatedConversion, Saturating, Zero};
	use sp_std::vec::Vec;

	pub type BalanceOf<T> =
		<<T as Config>::Currency as Currency<<T as frame_system::Config>::AccountId>>::Balance;

	/// Version 1 is "activated": requesters whitelisted and pre-upgrade backing
	/// seeded by `ActivateMinerPaymentBank`. That migration seeds real funds
	/// from sudo, so it keys its one-shot guard on this rather than on
	/// whitelist contents — a requester can be removed by an admin at any time,
	/// and inferring "already ran" from that would re-seed.
	pub const STORAGE_VERSION: StorageVersion = StorageVersion::new(1);

	#[pallet::pallet]
	#[pallet::without_storage_info]
	#[pallet::storage_version(STORAGE_VERSION)]
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
		/// Transaction fees collected from the network.
		Fees,
		/// Anything else.
		Other,
	}

	/// Accounts allowed to call `request_payment`.
	#[pallet::storage]
	pub type WhitelistedRequesters<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, (), OptionQuery>;

	/// Lifetime total deposited, per deposit type.
	#[pallet::storage]
	pub type TotalDeposited<T: Config> =
		StorageMap<_, Blake2_128Concat, DepositType, BalanceOf<T>, ValueQuery>;

	/// Lifetime total released through `request_payment`.
	#[pallet::storage]
	pub type TotalPaidOut<T: Config> = StorageValue<_, BalanceOf<T>, ValueQuery>;

	/// Lifetime total released per requester (e.g. arion vs compute escrow).
	#[pallet::storage]
	pub type TotalPaidByRequester<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, BalanceOf<T>, ValueQuery>;

	/// Accounts allowed to draw from the bank at genesis.
	///
	/// A genesis-built chain never runs `ActivateMinerPaymentBank` — FRAME's
	/// `on_genesis` stamps the pallet's storage version, so the migration's
	/// one-shot guard sees an already-current chain and does nothing. New chains
	/// must therefore whitelist here or no payment ever succeeds; the migration
	/// exists only to carry an already-running chain across the upgrade.
	#[pallet::genesis_config]
	pub struct GenesisConfig<T: Config> {
		pub requesters: Vec<T::AccountId>,
	}

	impl<T: Config> Default for GenesisConfig<T> {
		fn default() -> Self {
			Self { requesters: Vec::new() }
		}
	}

	#[pallet::genesis_build]
	impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
		fn build(&self) {
			for who in &self.requesters {
				WhitelistedRequesters::<T>::insert(who, ());
			}
		}
	}

	/// Largest amount a single `request_payment` call from this requester may
	/// move. Absent means uncapped.
	///
	/// This is a per-call bound, **not** a spend limit: a requester that calls
	/// repeatedly still withdraws without limit, and arion calls once per
	/// family per settlement. Treat it as a blast-radius limiter on one bad
	/// request, not as compartmentalization — the wall that keeps money owed
	/// to others out of the miner budget lives in the runtime's `PayoutSource`
	/// adapter.
	#[pallet::storage]
	pub type RequesterWithdrawalCap<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, BalanceOf<T>, OptionQuery>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// Funds were deposited into the bank.
		Deposited { who: T::AccountId, amount: BalanceOf<T>, deposit_type: DepositType },
		/// An account was added to the requester whitelist.
		RequesterAdded { who: T::AccountId },
		/// An account was removed from the requester whitelist.
		RequesterRemoved { who: T::AccountId },
		/// A payment was released to `dest`. `paid` may be lower than `requested`
		/// when the bank balance is insufficient.
		PaymentReleased {
			requester: T::AccountId,
			dest: T::AccountId,
			requested: BalanceOf<T>,
			paid: BalanceOf<T>,
		},
		/// Per-requester withdrawal cap was set.
		RequesterCapSet { who: T::AccountId, cap: BalanceOf<T> },
		/// Per-requester withdrawal cap was removed.
		RequesterCapRemoved { who: T::AccountId },
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

		#[pallet::call_index(3)]
		#[pallet::weight(T::DbWeight::get().writes(1))]
		pub fn set_requester_cap(
			origin: OriginFor<T>,
			who: T::AccountId,
			cap: BalanceOf<T>,
		) -> DispatchResult {
			T::AdminOrigin::ensure_origin(origin)?;
			RequesterWithdrawalCap::<T>::insert(&who, cap);
			Self::deposit_event(Event::RequesterCapSet { who, cap });
			Ok(())
		}

		#[pallet::call_index(4)]
		#[pallet::weight(T::DbWeight::get().writes(1))]
		pub fn remove_requester_cap(origin: OriginFor<T>, who: T::AccountId) -> DispatchResult {
			T::AdminOrigin::ensure_origin(origin)?;
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

		/// Record a fee deposit in the bank's ledger and emit the Deposited event.
		/// Used by the transaction fee handler to track all tx fees.
		pub fn record_fee_deposit(amount: BalanceOf<T>) {
			TotalDeposited::<T>::mutate(DepositType::Fees, |total| {
				*total = total.saturating_add(amount);
			});
			Self::deposit_event(Event::Deposited {
				who: Self::account_id(),
				amount,
				deposit_type: DepositType::Fees,
			});
		}

		/// Funds `request_payment` could release right now (free balance minus
		/// the existential deposit the bank always keeps).
		pub fn available_for_payout() -> BalanceOf<T> {
			payment_math::available(
				Tokens::new(T::Currency::free_balance(&Self::account_id()).saturated_into()),
				Tokens::new(T::Currency::minimum_balance().saturated_into()),
			)
			.get()
			.saturated_into()
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
		/// account). Never overdraws: pays `min(amount, free - ED)` and returns
		/// the amount actually paid so the caller can account for the shortfall.
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
			// Bound how much a single call can move. This limits the blast
			// radius of one runaway request; it does not bound total spend,
			// because nothing here consults what the requester already took.
			let capped_amount = match RequesterWithdrawalCap::<T>::get(requester) {
				Some(cap) => amount.min(cap),
				None => amount,
			};
			let paid: BalanceOf<T> = payment_math::payable(
				Tokens::new(capped_amount.saturated_into()),
				Tokens::new(Self::available_for_payout().saturated_into()),
			)
			.get()
			.saturated_into();
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

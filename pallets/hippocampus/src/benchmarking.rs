//! Benchmarks for the bank's fund-moving paths.
//!
//! `deposit`, `add_requester` and `remove_requester` are extrinsics and are
//! measured as such. `request_payment` is deliberately not an extrinsic — it is
//! the internal API the marketplace and the Arion settlement pay through — so
//! it is measured with `#[block]` and its figure is what those callers add to
//! their own weight for every payment they make.
//!
//! Each is posed at its most expensive path: `deposit` and `request_payment`
//! both move funds into an account that does not exist yet, and
//! `request_payment` runs with a per-requester cap set so the cap read is
//! inside the measurement.
//!
//! Run:
//!
//! ```text
//! cargo build --release --features runtime-benchmarks
//! ./target/release/hippius benchmark pallet \
//!     --chain benchmark \
//!     --pallet pallet_hippocampus \
//!     --extrinsic '*' \
//!     --steps 50 --repeat 20
//! ```

#![cfg(feature = "runtime-benchmarks")]

use super::*;
use crate::pallet::Pallet as Hippocampus;
use frame_benchmarking::v2::*;
use frame_support::traits::{Currency, EnsureOrigin};
use frame_system::RawOrigin;
use sp_runtime::{
	traits::{Bounded, Saturating},
	SaturatedConversion,
};

const SEED: u32 = 0;

/// A balance far above any existential deposit, so every transfer below moves
/// real funds and none is refused as dust.
fn plenty<T: Config>() -> BalanceOf<T> {
	T::Currency::minimum_balance().saturating_mul(1_000_000u32.into())
}

#[benchmarks]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn deposit() {
		let caller: T::AccountId = whitelisted_caller();
		T::Currency::make_free_balance_be(&caller, plenty::<T>().saturating_mul(2u32.into()));
		let amount = plenty::<T>();

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), amount, DepositType::Grant);

		assert_eq!(TotalDeposited::<T>::get(DepositType::Grant), amount);
	}

	#[benchmark]
	fn add_requester() -> Result<(), BenchmarkError> {
		let origin =
			T::AdminOrigin::try_successful_origin().map_err(|_| BenchmarkError::Weightless)?;
		let who: T::AccountId = account("requester", 0, SEED);

		#[extrinsic_call]
		_(origin as T::RuntimeOrigin, who.clone());

		assert!(WhitelistedRequesters::<T>::contains_key(&who));
		Ok(())
	}

	#[benchmark]
	fn remove_requester() -> Result<(), BenchmarkError> {
		let origin =
			T::AdminOrigin::try_successful_origin().map_err(|_| BenchmarkError::Weightless)?;
		let who: T::AccountId = account("requester", 0, SEED);
		WhitelistedRequesters::<T>::insert(&who, ());

		#[extrinsic_call]
		_(origin as T::RuntimeOrigin, who.clone());

		assert!(!WhitelistedRequesters::<T>::contains_key(&who));
		Ok(())
	}

	#[benchmark]
	fn request_payment() {
		let requester: T::AccountId = account("requester", 0, SEED);
		let dest: T::AccountId = account("payee", 0, SEED);
		WhitelistedRequesters::<T>::insert(&requester, ());
		// Set but never binding, so the payment still goes through in full.
		RequesterWithdrawalCap::<T>::insert(&requester, BalanceOf::<T>::max_value());
		DistributionEnabled::<T>::put(true);
		let amount = plenty::<T>();
		T::Currency::make_free_balance_be(
			&Hippocampus::<T>::account_id(),
			amount.saturating_mul(2u32.into()),
		);

		let paid;
		#[block]
		{
			paid = Hippocampus::<T>::request_payment(&requester, &dest, amount);
		}

		assert_eq!(paid.ok(), Some(amount));
		assert_eq!(
			TotalPaidByRequester::<T>::get(&requester).saturated_into::<u128>(),
			amount.saturated_into::<u128>()
		);
	}

	impl_benchmark_test_suite!(Hippocampus, crate::mock::new_test_ext(), crate::mock::Test);
}

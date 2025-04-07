//! Benchmarking setup for pallet-template
#![cfg(feature = "runtime-benchmarks")]
use super::*;

#[allow(unused)]
use crate::Pallet as SubAccounts;
use frame_benchmarking::v2::*;
use frame_support::BoundedVec;
use frame_system::RawOrigin;
use traits::profile::ProfileInterface;

#[benchmarks( 
	where
	T::Profile: ProfileInterface<T::AccountId, T::StringLimit>
)]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn add_sub_account() -> Result<(), BenchmarkError> {
		// Main account.
		let caller: T::AccountId = account("account", 0, 0);

		// Sub account of main account.
		let sub_account: T::AccountId = account("another account", 1, 0);

		// Create user so sub account can be created.
		let _ = T::Profile::create_user(
			caller.clone(),
			BoundedVec::try_from("email".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("first_name".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("last_name".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("date_of_birth".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("bio".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("username".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("website".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("linkedin".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("twitter".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("instagram".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("telegram".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("youtube".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("facebook".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("vision".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("tag_line".as_bytes().to_vec()).unwrap(),
			0u32, // exchange_volume
			BoundedVec::try_from("current_time".as_bytes().to_vec()).unwrap(),
			caller.clone(), // inviter
			BoundedVec::try_from("city_token_symbol".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("industry_token_symbol".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("personal_token_symbol".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("project_token_symbol".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("profile_picture_color_hex".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("profile_picture_initials".as_bytes().to_vec()).unwrap(),
		);

		#[extrinsic_call]
		add_sub_account(RawOrigin::Signed(caller.clone()), caller.clone(), sub_account.clone());

		assert_eq!(SubAccount::<T>::get(sub_account).unwrap(), caller);

		Ok(())
	}

	#[benchmark]
	fn remove_sub_account() -> Result<(), BenchmarkError> {
		// Main account.
		let caller: T::AccountId = account("account", 0, 0);

		// Sub account of main account.
		let sub_account: T::AccountId = account("another account", 1, 0);

		// Create user so sub account can be created.
		let _ = T::Profile::create_user(
			caller.clone(),
			BoundedVec::try_from("email".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("first_name".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("last_name".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("date_of_birth".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("bio".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("username".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("website".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("linkedin".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("twitter".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("instagram".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("telegram".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("youtube".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("facebook".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("vision".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("tag_line".as_bytes().to_vec()).unwrap(),
			0u32, // exchange_volume
			BoundedVec::try_from("current_time".as_bytes().to_vec()).unwrap(),
			caller.clone(), // inviter
			BoundedVec::try_from("city_token_symbol".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("industry_token_symbol".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("personal_token_symbol".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("project_token_symbol".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("profile_picture_color_hex".as_bytes().to_vec()).unwrap(),
			BoundedVec::try_from("profile_picture_initials".as_bytes().to_vec()).unwrap(),
		);

		// Create sub account so it can be removed.
		let _ = SubAccounts::<T>::add_sub_account(
			RawOrigin::Signed(caller.clone()).into(),
			caller.clone(),
			sub_account.clone(),
		);

		#[extrinsic_call]
		remove_sub_account(RawOrigin::Signed(caller.clone()), caller.clone(), sub_account.clone());

		assert!(!SubAccount::<T>::contains_key(sub_account));

		Ok(())
	}

	impl_benchmark_test_suite!(SubAccounts, crate::mock::new_test_ext(), crate::mock::Test);
}

#![cfg_attr(not(feature = "std"), no_std)]
use fp_evm::PrecompileHandle;
use frame_support::dispatch::{GetDispatchInfo, PostDispatchInfo};
use pallet_evm::AddressMapping;
use precompile_utils::prelude::*;
use sp_core::Encode;
use sp_core::{H160, H256};
use sp_runtime::traits::Dispatchable;
use sp_std::marker::PhantomData;
// use {
// 	precompile_utils::{
// 		precompile_set::*,
// 	},
// 	sp_runtime::traits::StaticLookup,
// 	frame_support::traits::Get,
// };
use pallet_subaccount::{
	// ChargeFees,
	Call as SubAccountsCall,
	SubAccounts,
};

/// A precompile to wrap the functionality from pallet sub accounts.
pub struct SubAccountsPrecompile<Runtime>(PhantomData<Runtime>);

#[precompile_utils::precompile]
#[precompile::test_concrete_types(mock::Runtime)]
impl<Runtime> SubAccountsPrecompile<Runtime>
where
	Runtime: pallet_subaccount::Config + pallet_evm::Config + frame_system::Config,
	Runtime::RuntimeCall: Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
	<Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin: From<Option<Runtime::AccountId>>,
	Runtime::RuntimeCall: From<SubAccountsCall<Runtime>>,
	Runtime::Hash: From<H256>,
	<Runtime as pallet_evm::Config>::AddressMapping: AddressMapping<Runtime::AccountId>,
{
	#[precompile::public("addSubAccount(address,address)")]
	#[precompile::public("add_sub_account(address,address)")]
	fn add_sub_account(
		handle: &mut impl PrecompileHandle,
		main: Address,
		new_sub_account: Address,
	) -> EvmResult {
		let caller = handle.context().caller;

		// Convert addresses to AccountIds
		let _main_address: H160 = main.into();
		// let main_account = Runtime::AddressMapping::into_account_id(main_address);

		let sub_address: H160 = new_sub_account.into();
		let _sub_account = Runtime::AddressMapping::into_account_id(sub_address);

		let _origin = Runtime::AddressMapping::into_account_id(caller);

		// // Create the call
		// let call = SubAccountsCall::<Runtime>::add_sub_account {
		// 	main: main_account,
		// 	new_sub_account: sub_account,
		// 	role: None
		// };

		// // Dispatch the call
		// RuntimeHelper::<Runtime>::try_dispatch(handle, Some(origin).into(), call)?;

		Ok(())
	}

	#[precompile::public("removeSubAccount(address,address)")]
	#[precompile::public("remove_sub_account(address,address)")]
	fn remove_sub_account(
		handle: &mut impl PrecompileHandle,
		main: Address,
		sub_account_to_remove: Address,
	) -> EvmResult {
		let caller = handle.context().caller;

		// Convert addresses to AccountIds
		let main_address: H160 = main.into();
		let main_account = Runtime::AddressMapping::into_account_id(main_address);

		let sub_address: H160 = sub_account_to_remove.into();
		let sub_account = Runtime::AddressMapping::into_account_id(sub_address);

		let origin = Runtime::AddressMapping::into_account_id(caller);

		// Create the call
		let call = SubAccountsCall::<Runtime>::remove_sub_account {
			main: main_account,
			sub_account_to_remove: sub_account,
		};

		// Dispatch the call
		RuntimeHelper::<Runtime>::try_dispatch(handle, Some(origin).into(), call)?;

		Ok(())
	}

	#[precompile::public("isSubAccount(address,address)")]
	#[precompile::public("is_sub_account(address,address)")]
	fn is_sub_account(
		_handle: &mut impl PrecompileHandle,
		sender: Address,
		main: Address,
	) -> EvmResult<bool> {
		// Convert addresses to AccountIds
		let sender_address: H160 = sender.into();
		let sender_account = Runtime::AddressMapping::into_account_id(sender_address);

		let main_address: H160 = main.into();
		let main_account = Runtime::AddressMapping::into_account_id(main_address);

		// Call the pallet function
		match pallet_subaccount::Pallet::<Runtime>::is_sub_account(sender_account, main_account) {
			Ok(_) => Ok(true),
			Err(_) => Ok(false),
		}
	}

	#[precompile::public("isAlreadySubAccount(address)")]
	#[precompile::public("is_already_sub_account(address)")]
	fn is_already_sub_account(
		_handle: &mut impl PrecompileHandle,
		who: Address,
	) -> EvmResult<bool> {
		// Convert address to AccountId
		let who_address: H160 = who.into();
		let who_account = Runtime::AddressMapping::into_account_id(who_address);

		// Call the pallet function
		match pallet_subaccount::Pallet::<Runtime>::already_sub_account(who_account) {
			Ok(_) => Ok(false), // If Ok, means it's not already a sub account
			Err(_) => Ok(true), // If Err, means it is already a sub account
		}
	}

	#[precompile::public("getMainAccount(address)")]
	#[precompile::public("get_main_account(address)")]
	fn get_main_account(_handle: &mut impl PrecompileHandle, who: Address) -> EvmResult<Address> {
		// Convert address to AccountId
		let who_address: H160 = who.into();
		let who_account = Runtime::AddressMapping::into_account_id(who_address);

		// Call the pallet function with explicit trait specification
		match <pallet_subaccount::Pallet<Runtime> as SubAccounts<Runtime::AccountId>>::get_main_account(who_account) {
            Ok(main_account) => {
                // Convert the main account back to H160/Address
 				// Convert AccountId32 to H160 by taking the last 20 bytes
				let account_bytes = main_account.encode();
				let evm_address = H160::from_slice(&account_bytes[account_bytes.len() - 20..]);			
                Ok(Address(evm_address))
            },
            Err(_) => Err(revert("Not a sub account")),
        }
	}
}

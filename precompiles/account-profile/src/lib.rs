#![cfg_attr(not(feature = "std"), no_std)]
use fp_evm::PrecompileHandle;
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
};
use pallet_account_profile::Call as AccountProfileCall;
use precompile_utils::prelude::*;
use sp_core::{H160, H256};
use sp_runtime::traits::Dispatchable;
use sp_std::marker::PhantomData;
use sp_std::vec::Vec;
use pallet_evm::AddressMapping;
use sp_runtime::traits::ConstU32;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

/// A precompile to wrap the functionality from pallet account profile.
pub struct AccountProfilePrecompile<Runtime>(PhantomData<Runtime>);

#[precompile_utils::precompile]
#[precompile::test_concrete_types(mock::Runtime)]
impl<Runtime> AccountProfilePrecompile<Runtime>
where
    Runtime: pallet_account_profile::Config + pallet_evm::Config + frame_system::Config,
    <Runtime as frame_system::Config>::RuntimeCall: 
    Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
    <<Runtime as frame_system::Config>::RuntimeCall as Dispatchable>::RuntimeOrigin: 
    From<Option<<Runtime as frame_system::Config>::AccountId>>,
    <Runtime as frame_system::Config>::RuntimeCall: 
    From<AccountProfileCall<Runtime>>,
    Runtime::Hash: From<H256>,
    <Runtime as pallet_evm::Config>::AddressMapping: AddressMapping<Runtime::AccountId>,
{
    // Precompile for adding a public item
    #[precompile::public("addPublicItem(bytes)")]
    #[precompile::public("add_public_item(bytes)")]
    fn add_public_item(
        handle: &mut impl PrecompileHandle,
        item: BoundedBytes<ConstU32<2000>>, // The item to be added
    ) -> EvmResult {
        // Set all gas costs to 0
        handle.record_cost(0)?;

        let caller = handle.context().caller;
        let origin = Runtime::AddressMapping::into_account_id(caller);

        let item =  item.into();
        // Create the call
        let call = AccountProfileCall::<Runtime>::set_public_item {
            item: item,
        };
        
        // Dispatch the call
        RuntimeHelper::<Runtime>::try_dispatch(
            handle,
            Some(origin).into(),
            call,
        )?;

        Ok(())
    }

    // Precompile for adding a private item 
    #[precompile::public("addPrivateItem(bytes)")]
    #[precompile::public("add_private_item(bytes)")]
    fn add_private_item(
        handle: &mut impl PrecompileHandle,
        item: BoundedBytes<ConstU32<2000>>,
    ) -> EvmResult {
        // Override gas cost
		handle.record_cost(100000)?;
        
        let caller = handle.context().caller;
        let origin = Runtime::AddressMapping::into_account_id(caller);

        let item =  item.into();
        // Create the call
        let call = AccountProfileCall::<Runtime>::set_private_item {
            item: item,
        };
        
        // Dispatch the call
        RuntimeHelper::<Runtime>::try_dispatch(
            handle,
            Some(origin).into(),
            call,
        )?;

        Ok(())
    }

    #[precompile::public("getPublicItem(address)")]
    #[precompile::public("get_public_item(address)")]
    #[precompile::view]
    fn get_public_item(
        _handle: &mut impl PrecompileHandle,
        address: Address,
    ) -> EvmResult<Vec<u8>> {
        let owner: H160 = address.into();
        let account_id = Runtime::AddressMapping::into_account_id(owner);
        let item = pallet_account_profile::Pallet::<Runtime>::user_public_storage(account_id);
        
        Ok(item)
    }

    #[precompile::public("getPrivateItem(address)")]
    #[precompile::public("get_private_item(address)")]
    #[precompile::view]
    fn get_private_item(
        _handle: &mut impl PrecompileHandle,
        address: Address,
    ) -> EvmResult<Vec<u8>> {

        let owner: H160 = address.into();
        let account_id = Runtime::AddressMapping::into_account_id(owner);
        let item = pallet_account_profile::Pallet::<Runtime>::user_private_storage(account_id);
        
        Ok(item)
    }


    // Precompile for setting a username
    #[precompile::public("setUsername(bytes)")]
    #[precompile::public("set_username(bytes)")]
    fn set_username(
        handle: &mut impl PrecompileHandle,
        username: BoundedBytes<ConstU32<100>>, // Username length is restricted to 32 bytes
    ) -> EvmResult {
        // Override gas cost
        handle.record_cost(0)?;

        let caller = handle.context().caller;
        let origin = Runtime::AddressMapping::into_account_id(caller);

        let username = username.into();
        let call = AccountProfileCall::<Runtime>::set_username { username };
        
        RuntimeHelper::<Runtime>::try_dispatch(handle, Some(origin).into(), call)?;
        Ok(())
    }


    // Precompile for getting public and private storage
    #[precompile::public("getStorage(address)")]
    #[precompile::public("get_storage(address)")]
    #[precompile::view]
    fn get_storage(
        _handle: &mut impl PrecompileHandle,
        address: Address,
    ) -> EvmResult<(Vec<u8>, Vec<u8>)> {
        let owner: H160 = address.into();
        let account_id = Runtime::AddressMapping::into_account_id(owner);

        let public_item = pallet_account_profile::Pallet::<Runtime>::user_public_storage(account_id.clone());
        let private_item = pallet_account_profile::Pallet::<Runtime>::user_private_storage(account_id);

        Ok((public_item, private_item))
    }
}

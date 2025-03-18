#![cfg_attr(not(feature = "std"), no_std)]
use fp_evm::PrecompileHandle;
use frame_support::{
    dispatch::{GetDispatchInfo, PostDispatchInfo},
};
use pallet_notifications::Call as NotificationsCall;
use precompile_utils::prelude::*;
use sp_core::{H160, H256};
use sp_runtime::traits::Dispatchable;
use sp_std::marker::PhantomData;
use frame_system::pallet_prelude::BlockNumberFor;
// use sp_std::vec::Vec;
use pallet_evm::AddressMapping;
// use pallet_evm::PrecompileFailure;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

/// A precompile to wrap the functionality from pallet notifications.
pub struct NotificationsPrecompile<Runtime>(PhantomData<Runtime>);

#[precompile_utils::precompile]
#[precompile::test_concrete_types(mock::Runtime)]
impl<Runtime> NotificationsPrecompile<Runtime>
where
    Runtime: pallet_notifications::Config + pallet_evm::Config + frame_system::Config,
    Runtime::RuntimeCall: Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
    <Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin: From<Option<Runtime::AccountId>>,
    Runtime::RuntimeCall: From<NotificationsCall<Runtime>>,
    Runtime::Hash: From<H256>,
    <Runtime as pallet_evm::Config>::AddressMapping: AddressMapping<Runtime::AccountId>,
{
    /// Send a notification to a recipient
    #[precompile::public("sendNotification(address,uint256,bool,uint256,uint256)")]
    #[precompile::public("send_notification(address,uint256,bool,uint256,uint256)")]
    fn send_notification(
        handle: &mut impl PrecompileHandle,
        recipient: Address,
        block_to_send: u64,
        recurrence: bool,
        starting_recurrence: u64,
        frequency: u64,
    ) -> EvmResult {
        // Override gas cost
        handle.record_cost(0)?;

        let caller = handle.context().caller;
        
        // Convert addresses to AccountIds
        let recipient_address: H160 = recipient.into();
        let recipient_account = Runtime::AddressMapping::into_account_id(recipient_address);
        let sender = Runtime::AddressMapping::into_account_id(caller);

        // Convert u64 to BlockNumber
        let block_number: BlockNumberFor<Runtime> = block_to_send.try_into().map_err(|_| revert("Invalid block number"))?;
        let starting_block: BlockNumberFor<Runtime> = starting_recurrence.try_into().map_err(|_| revert("Invalid starting block"))?;
        let freq_block: BlockNumberFor<Runtime> = frequency.try_into().map_err(|_| revert("Invalid frequency"))?;

        // Create the call
        let call = NotificationsCall::<Runtime>::send_notification { 
            recipient: recipient_account,
            block_to_send: block_number,
            recurrence,
            starting_recurrence: Some(starting_block),
            frequency: Some(freq_block),
        };

        // Dispatch the call
        RuntimeHelper::<Runtime>::try_dispatch(
            handle,
            Some(sender).into(),
            call,
        )?;

        Ok(())
    }

    /// Mark a notification as read
    #[precompile::public("markAsRead(uint32)")]
    #[precompile::public("mark_as_read(uint32)")]
    fn mark_as_read(
        handle: &mut impl PrecompileHandle,
        index: u32,
    ) -> EvmResult {
        // Override gas cost
        handle.record_cost(0)?;

        let caller = handle.context().caller;
        let who = Runtime::AddressMapping::into_account_id(caller);

        // Create the call
        let call = NotificationsCall::<Runtime>::mark_as_read { 
            index,
        };

        // Dispatch the call
        RuntimeHelper::<Runtime>::try_dispatch(
            handle,
            Some(who).into(),
            call,
        )?;

        Ok(())
    }
}

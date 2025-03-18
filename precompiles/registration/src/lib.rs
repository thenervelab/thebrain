#![cfg_attr(not(feature = "std"), no_std)]
use fp_evm::PrecompileHandle;
use frame_support::{
	dispatch::{GetDispatchInfo, PostDispatchInfo},
	// traits::Get,
};
use pallet_registration::Call as RegistrationCall;
use precompile_utils::prelude::*;
use sp_core::{H160, H256};
use sp_runtime::traits::Dispatchable;
use sp_std::marker::PhantomData;
use frame_system::pallet_prelude::BlockNumberFor;
use sp_std::vec::Vec;
use pallet_evm::AddressMapping;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

/// A precompile to wrap the functionality from pallet author mapping.
pub struct RegistrationPrecompile<Runtime>(PhantomData<Runtime>);

#[precompile_utils::precompile]
#[precompile::test_concrete_types(mock::Runtime)]
impl<Runtime> RegistrationPrecompile<Runtime>
where
	Runtime: pallet_registration::Config + pallet_evm::Config + frame_system::Config,
	Runtime::RuntimeCall: Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
	<Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin: From<Option<Runtime::AccountId>>,
	Runtime::RuntimeCall: From<RegistrationCall<Runtime>>,
	Runtime::Hash: From<H256>,
	Runtime::AccountId: Into<H160>,
	BlockNumberFor<Runtime>: Into<u64>,
	<Runtime as pallet_evm::Config>::AddressMapping: AddressMapping<Runtime::AccountId>,
{
	// #[precompile::public("callRegister(uint32,uint32,bytes,address)")]
	// #[precompile::public("call_register(uint32,uint32,bytes,address)")]
	// fn call_register(
	// 	handle: &mut impl PrecompileHandle,
	// 	_total_replicas: u32,
	// 	_fullfilled_replicas: u32,
	// 	_file_hash: Vec<u8>,
	// 	_who: Address,
	// ) -> EvmResult {
	// 	let _origin_account = Runtime::AddressMapping::into_account_id(handle.context().caller);
	// 	Ok(())
	// }
}


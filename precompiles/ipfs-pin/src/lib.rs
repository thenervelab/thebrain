#![cfg_attr(not(feature = "std"), no_std)]
use fp_evm::PrecompileHandle;
use frame_support::{
	dispatch::{GetDispatchInfo, PostDispatchInfo},
};
use pallet_ipfs_pin::Call as IpfsCall;
use pallet_ipfs_pin::Pallet as IpfsPinPallet;
use pallet_ipfs_pin::StorageRequest;
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
pub struct IpfsPinPrecompile<Runtime>(PhantomData<Runtime>);

#[precompile_utils::precompile]
#[precompile::test_concrete_types(mock::Runtime)]
impl<Runtime> IpfsPinPrecompile<Runtime>
where
	Runtime: pallet_ipfs_pin::Config + pallet_evm::Config + frame_system::Config,
	Runtime::RuntimeCall: Dispatchable<PostInfo = PostDispatchInfo> + GetDispatchInfo,
	<Runtime::RuntimeCall as Dispatchable>::RuntimeOrigin: From<Option<Runtime::AccountId>>,
	Runtime::RuntimeCall: From<IpfsCall<Runtime>>,
	Runtime::Hash: From<H256>,
	Runtime::AccountId: Into<H160>,
	BlockNumberFor<Runtime>: Into<u64>,
	<Runtime as pallet_evm::Config>::AddressMapping: AddressMapping<Runtime::AccountId>,
{
	#[precompile::public("callStorageRequest(uint32,uint32,bytes,address)")]
	#[precompile::public("call_storage_request(uint32,uint32,bytes,address)")]
	fn call_storage_request(
		handle: &mut impl PrecompileHandle,
		total_replicas: u32,
		fullfilled_replicas: u32,
		file_hash: Vec<u8>,
		who: Address,
	) -> EvmResult {
		// Convert `who` to the `AccountId` expected by `frame_system`
		let to_address: H160 = who.into();
		let to_account = Runtime::AddressMapping::into_account_id(to_address);

		// Get the caller's account as `AccountId`
		let origin_account = Runtime::AddressMapping::into_account_id(handle.context().caller);
		let origin: Option<Runtime::AccountId> = Some(origin_account);

		let request = StorageRequest {
			total_replicas,	
			fullfilled_replicas,
			owner: to_account,
			file_hash, 
		};
		let call = IpfsCall::<Runtime>::storage_request {
			request_info: request,
		};

		RuntimeHelper::<Runtime>::try_dispatch(
			handle,
			origin.into(),
			call
		)?;
		Ok(())
	}


	// #[precompile::public("callIpfsPinRequest(bytes,bytes)")]
	// #[precompile::public("call_ipfs_pin_request(bytes,bytes)")]
	// fn call_ipfs_pin_request(
	// 	handle: &mut impl PrecompileHandle,
	// 	file_hash: Vec<u8>,
	// 	node_identity:  Vec<u8>,
	// ) -> EvmResult {
	// 	// Get the caller's account as `AccountId`
	// 	let origin_account = Runtime::AddressMapping::into_account_id(handle.context().caller);
	// 	let origin: Option<Runtime::AccountId> = Some(origin_account);
	// 	let call = IpfsCall::<Runtime>::ipfs_pin_request {
	// 		file_hash: file_hash,
	// 		node_identity: node_identity
	// 	};
	// 	RuntimeHelper::<Runtime>::try_dispatch(
	// 		handle,
	// 		origin.into(),
	// 		call
	// 	)?;
	// 	Ok(())
	// }


	// get all the files pinned by a node
	#[precompile::public("readFilesPinned(bytes)")]
	#[precompile::public("read_files_pinned(bytes)")]
	#[precompile::view]
	fn read_files_pinned(handle: &mut impl PrecompileHandle, node_identity: Vec<u8>) -> EvmResult<Vec<Vec<u8>>> {
		handle.record_db_read::<Runtime>(68)?;

		let pin_requests = IpfsPinPallet::<Runtime>::get_files_stored(node_identity);

		// Prepare a vector to collect file hashes where `is_pinned` is true
		let mut pinned_file_hashes: Vec<Vec<u8>> = Vec::new();

		for pin_request in pin_requests {
			if pin_request.is_pinned {
				match hex::decode(pin_request.file_hash.clone()) {
					Ok(decoded_hash) => pinned_file_hashes.push(decoded_hash),
					Err(_e) => {
						// TODO!
					},
				}
			}
		};

		// Return the encoded data as a byte array
		Ok(pinned_file_hashes)
	}
}


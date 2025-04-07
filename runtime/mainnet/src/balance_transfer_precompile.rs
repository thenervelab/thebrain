// runtime/mainnet/src/precompiles/balance_transfer_precompile.rs

use crate::{Runtime, RuntimeCall};
use frame_system::RawOrigin;
use pallet_evm::{
	ExitError,
	ExitSucceed,
	// PrecompileHandle,PrecompileOutput, PrecompileResult,
	LinearCostPrecompile,
	PrecompileFailure,
};
use pallet_evm_precompile_balances_erc20::BalanceOf;
use precompile_utils::prelude::InjectBacktrace;
use precompile_utils::prelude::{MayRevert, RevertReason};
use sp_core::keccak_256;
use sp_core::U256;
use sp_runtime::traits::Zero;
use sp_runtime::traits::{Dispatchable, UniqueSaturatedInto};
use sp_runtime::AccountId32;
use sp_std::vec;
use sp_std::vec::Vec;
// use crate::precompiles::{bytes_to_account_id, get_method_id, get_slice};

// impl LinearCostPrecompile for BalanceTransferPrecompile {
//     // fn linear_cost(&self, _input: &[u8]) -> u64 {
//     //     // Define the cost of executing this precompile
//     //     // For example, you can return a fixed cost or calculate based on input size
//     //     1000 // Example fixed cost
//     // }
// }

pub const BALANCE_TRANSFER_INDEX: u64 = 2047;

pub struct BalanceTransferPrecompile;

impl LinearCostPrecompile for BalanceTransferPrecompile {
	const BASE: u64 = 3000;
	const WORD: u64 = 0;

	fn execute(input: &[u8], _: u64) -> Result<(ExitSucceed, Vec<u8>), PrecompileFailure> {
		// Create a PrecompileHandle to manage the execution context
		// let handle = PrecompileHandle::new(input);

		// Match method ID: keccak256("transfer(bytes32)")
		let method: &[u8] = get_slice(input, 0, 4)?;
		if get_method_id("transfer(bytes32)") == method {
			// Forward all received value to the destination address
			// let amount: U256 = handle.context().apparent_value;

			// Extract the amount (32 bytes from bytes 36–68)
			let amount_bytes = &input[36..68];

			// Optionally parse as U256 if necessary
			let amount = sp_core::U256::from_big_endian(amount_bytes);
			let amount_sub = u256_to_amount(amount).in_field("value")?;

			if amount_sub.is_zero() {
				return Ok((ExitSucceed::Returned, vec![]));
			}
			// This is a hardcoded hashed address mapping of
			// 0x0000000000000000000000000000000000000800 to an ss58 public key
			// i.e., the contract sends funds it received to the destination address
			// from the method parameter.
			const ADDRESS_BYTES_SRC: [u8; 32] = [
				0x07, 0xec, 0x71, 0x2a, 0x5d, 0x38, 0x43, 0x4d, 0xdd, 0x03, 0x3f, 0x8f, 0x02, 0x4e,
				0xcd, 0xfc, 0x4b, 0xb5, 0x95, 0x1c, 0x13, 0xc3, 0x08, 0x5c, 0x39, 0x9c, 0x8a, 0x5f,
				0x62, 0x93, 0x70, 0x5d,
			];
			let address_bytes_dst: &[u8] = get_slice(input, 4, 36)?;
			let account_id_src = bytes_to_account_id(&ADDRESS_BYTES_SRC)?;
			let account_id_dst = bytes_to_account_id(address_bytes_dst)?;

			let call =
				RuntimeCall::Balances(pallet_balances::Call::<Runtime>::transfer_allow_death {
					dest: account_id_dst.into(),
					value: amount_sub.unique_saturated_into(),
				});

			// Dispatch the call
			let result = call.dispatch(RawOrigin::Signed(account_id_src).into());
			if result.is_err() {
				return Err(PrecompileFailure::Error { exit_status: ExitError::OutOfFund });
			}
		}

		Ok((ExitSucceed::Returned, vec![]))
	}
}

/// Returns Ethereum method ID from an str method signature
///
pub fn get_method_id(method_signature: &str) -> [u8; 4] {
	// Calculate the full Keccak-256 hash of the method signature
	let hash = keccak_256(method_signature.as_bytes());

	// Extract the first 4 bytes to get the method ID
	[hash[0], hash[1], hash[2], hash[3]]
}

/// Convert bytes to AccountId32 with PrecompileFailure as Error
/// which consumes all gas
///
pub fn bytes_to_account_id(account_id_bytes: &[u8]) -> Result<AccountId32, PrecompileFailure> {
	AccountId32::try_from(account_id_bytes)
		.map_err(|_| PrecompileFailure::Error { exit_status: ExitError::InvalidRange })
}

/// Takes a slice from bytes with PrecompileFailure as Error
///
pub fn get_slice(data: &[u8], from: usize, to: usize) -> Result<&[u8], PrecompileFailure> {
	let maybe_slice = data.get(from..to);
	if let Some(slice) = maybe_slice {
		Ok(slice)
	} else {
		Err(PrecompileFailure::Error { exit_status: ExitError::InvalidRange })
	}
}

fn u256_to_amount(value: U256) -> MayRevert<BalanceOf<Runtime>> {
	value
		.try_into()
		.map_err(|_| RevertReason::value_is_too_large("balance type").into())
}

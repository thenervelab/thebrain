// This file is part of The Brain.

// Copyright (C) 2022-2024 The Nerve Lab
// SPDX-License-Identifier: Apache-2.0

// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// 	http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use crate::{
	precompiles::{PrecompileName, HipiusPrecompiles, ASSET_PRECOMPILE_ADDRESS_PREFIX},
	*,
};
use frame_support::{pallet_prelude::*, parameter_types, traits::FindAuthor, weights::Weight};
use sp_core::{crypto::ByteArray, H160, U256};
use sp_runtime::{traits::BlakeTwo256, ConsensusEngineId, Permill, DigestItem};
use sp_std::{marker::PhantomData, prelude::*};
// Frontier
use pallet_ethereum::PostLogContent;
use pallet_evm::{HashedAddressMapping, OnChargeEVMTransaction};
use pallet_evm_precompileset_assets_erc20::AddressToAssetId;
use hippius_primitives::{
	evm::{GAS_LIMIT_POV_SIZE_RATIO, WEIGHT_PER_GAS},
	impl_proxy_type,
};
// use precompile_utils::prelude::Address;
impl pallet_evm_chain_id::Config for Runtime {}

const ASSET_ID_SIZE: usize = core::mem::size_of::<AssetId>();

impl AddressToAssetId<AssetId> for Runtime {
	fn address_to_asset_id(address: H160) -> Option<AssetId> {
		let mut data = [0u8; ASSET_ID_SIZE];
		let address_bytes: [u8; 20] = address.into();
		if ASSET_PRECOMPILE_ADDRESS_PREFIX.eq(&address_bytes[0..4]) {
			data.copy_from_slice(&address_bytes[4..ASSET_ID_SIZE + 4]);
			Some(AssetId::from_be_bytes(data))
		} else {
			None
		}
	}

	fn asset_id_to_address(asset_id: AssetId) -> H160 {
		let mut data = [0u8; 20];
		data[0..4].copy_from_slice(ASSET_PRECOMPILE_ADDRESS_PREFIX);
		data[4..ASSET_ID_SIZE + 4].copy_from_slice(&asset_id.to_be_bytes());
		H160::from(data)
	}
}

pub struct FindAuthorTruncated<F>(PhantomData<F>);
impl<F: FindAuthor<u32>> FindAuthor<H160> for FindAuthorTruncated<F> {
	fn find_author<'a, I>(digests: I) -> Option<H160>
	where
		I: 'a + IntoIterator<Item = (ConsensusEngineId, &'a [u8])>,
	{
		if let Some(author_index) = F::find_author(digests) {
			let authority_id = Babe::authorities()[author_index as usize].clone();
			return Some(H160::from_slice(&authority_id.0.to_raw_vec()[4..24]));
		}
		None
	}
}

impl_proxy_type!();

parameter_types! {
	/// EVM gas limit
	pub BlockGasLimit: U256 = U256::from(
		NORMAL_DISPATCH_RATIO * MAXIMUM_BLOCK_WEIGHT.ref_time() / WEIGHT_PER_GAS
	);
	pub const GasLimitPovSizeRatio: u64 = GAS_LIMIT_POV_SIZE_RATIO;
	pub WeightPerGas: Weight = Weight::from_parts(WEIGHT_PER_GAS, 0);
	pub PrecompilesValue: HipiusPrecompiles<Runtime> = HipiusPrecompiles::<_>::new();
}

/// Type alias for negative imbalance during fees
type RuntimeNegativeImbalance =
	<Balances as Currency<<Runtime as frame_system::Config>::AccountId>>::NegativeImbalance;

/// See: [`pallet_evm::EVMCurrencyAdapter`]
pub struct CustomEVMCurrencyAdapter;

impl OnChargeEVMTransaction<Runtime> for CustomEVMCurrencyAdapter {
	type LiquidityInfo = Option<RuntimeNegativeImbalance>;

	fn withdraw_fee(
		who: &H160,
		fee: U256
	) -> Result<Self::LiquidityInfo, pallet_evm::Error<Runtime>> {
        log::info!("withdraw_fee - address: {:?}", who);
        
        // // Account profile precompile is at address 2086
        // let account_profile_address = H160::from_low_u64_be(2086);
        
        // // Get the current transaction info
        // if let Some(transaction) = get_current_transaction() {
        //     log::info!("Transaction target: {:?}", transaction.action);
            
        //     // Check if this is a call to our precompile
        //     if let TransactionAction::Call(target) = transaction.action {
        //         if target == account_profile_address {
        //             // This is a call to our precompile
        //             log::info!("Account profile precompile call - free!");
        //             return Ok(None);
        //         }
        //     }
        // }
        
        // Convert AccountId32 to H160 by taking the last 20 bytes
        let account_bytes = pallet_marketplace::Pallet::<Runtime>::account_id().encode();
        let pallet_marketplace_address = H160::from_slice(&account_bytes[account_bytes.len() - 20..]);
        
        if who == &pallet_marketplace_address {
            return Ok(None);
        }
        
        // fallback to the default implementation
        <pallet_evm::EVMCurrencyAdapter<Balances, impls::DealWithFees<Runtime>> as OnChargeEVMTransaction<
            Runtime,
        >>::withdraw_fee(who, fee)
    }

    fn correct_and_deposit_fee(
        who: &H160,
        corrected_fee: U256,
        base_fee: U256,
        already_withdrawn: Self::LiquidityInfo,
    ) -> Self::LiquidityInfo {
        log::info!("correct_and_deposit_fee - address: {:?}", who);
        
        // Account profile precompile is at address 2086
        // let account_profile_address = H160::from_low_u64_be(2086);
        
        // // Get the current transaction info
        // if let Some(transaction) = get_current_transaction() {
        //     log::info!("Transaction target: {:?}", transaction.action);
        //     // Check if this is a call to our precompile
        //     if let TransactionAction::Call(target) = transaction.action {
        //         if target == account_profile_address {
        //             // This is a call to our precompile
        //             log::info!("Account profile precompile call - free!");
        //             return already_withdrawn;
        //         }
        //     }
        // }
        
        // Convert AccountId32 to H160 by taking the last 20 bytes
        let account_bytes = pallet_marketplace::Pallet::<Runtime>::account_id().encode();
        let pallet_marketplace_address = H160::from_slice(&account_bytes[account_bytes.len() - 20..]);
        
        if who == &pallet_marketplace_address {
            return already_withdrawn;
        }
        
        // fallback to the default implementation
        <pallet_evm::EVMCurrencyAdapter<Balances, impls::DealWithFees<Runtime>> as OnChargeEVMTransaction<
            Runtime,
        >>::correct_and_deposit_fee(who, corrected_fee, base_fee, already_withdrawn)
    }

    fn pay_priority_fee(tip: Self::LiquidityInfo) {
        <pallet_evm::EVMCurrencyAdapter<Balances, impls::DealWithFees<Runtime>> as OnChargeEVMTransaction<
            Runtime,
        >>::pay_priority_fee(tip)
    }
}

// // Placeholder function to get current transaction info
// fn get_current_transaction() -> Option<Transaction> {
//     // Implement logic to retrieve the current transaction info
//     // This might involve accessing the current block or transaction pool
//     None // Replace with actual implementation
// }

pub struct PrecompileGasWeightMapping<T>(PhantomData<T>);

impl<T: pallet_evm::Config> pallet_evm::GasWeightMapping for PrecompileGasWeightMapping<T> {
    fn gas_to_weight(gas: u64, without_base_weight: bool) -> Weight {
        let target = H160::from_low_u64_be(2086);
        
        // Check if we're in a precompile call context
        if let Some(info) = frame_system::Pallet::<T>::digest().logs.iter().find_map(|log| {
            if let DigestItem::PreRuntime(_, data) = log {
                if data.starts_with(b"evm") {
                    return Some(data);
                }
            }
            None
        }) {
            // If this is our precompile, return zero weight
            if info.ends_with(&target.as_bytes()) {
                return Weight::zero();
            }
        }
        
        // For non-precompile calls, use the default mapping
        let _gas_price = T::FeeCalculator::min_gas_price();
        let base = if without_base_weight {
            Weight::zero()
        } else {
            T::BlockWeights::get().base_block
        };
        base.saturating_add(Weight::from_parts(gas, 0))
    }

    fn weight_to_gas(weight: Weight) -> u64 {
        weight.ref_time()
    }
}

impl pallet_evm::Config for Runtime {
    type FeeCalculator = BaseFee;
    type GasWeightMapping = PrecompileGasWeightMapping<Self>;
    type WeightPerGas = WeightPerGas;
    type BlockHashMapping = pallet_ethereum::EthereumBlockHashMapping<Self>;
    type CallOrigin = pallet_evm::EnsureAddressRoot<AccountId>;
    type WithdrawOrigin = pallet_evm::EnsureAddressTruncated;
    type AddressMapping = HashedAddressMapping<BlakeTwo256>;
    type Currency = Balances;
    type RuntimeEvent = RuntimeEvent;
    type PrecompilesType = HipiusPrecompiles<Runtime>;
    type PrecompilesValue = PrecompilesValue;
    type ChainId = EVMChainId;
    type BlockGasLimit = BlockGasLimit;
    type Runner = pallet_evm::runner::stack::Runner<Self>;
    type OnChargeTransaction = CustomEVMCurrencyAdapter;
    type OnCreate = ();
    type SuicideQuickClearLimit = ConstU32<0>;
    type FindAuthor = FindAuthorTruncated<Babe>;
    type GasLimitPovSizeRatio = GasLimitPovSizeRatio;
    type Timestamp = Timestamp;
    type WeightInfo = pallet_evm::weights::SubstrateWeight<Self>;
}

parameter_types! {
	pub const PostBlockAndTxnHashes: PostLogContent = PostLogContent::BlockAndTxnHashes;
}

impl pallet_ethereum::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type StateRoot = pallet_ethereum::IntermediateStateRoot<Self>;
	type PostLogContent = PostBlockAndTxnHashes;
	type ExtraDataLength = ConstU32<30>;
}

parameter_types! {
	pub BoundDivision: U256 = U256::from(1024);
}

impl pallet_dynamic_fee::Config for Runtime {
	type MinGasPriceBoundDivisor = BoundDivision;
}

parameter_types! {
	pub DefaultBaseFeePerGas: U256 = U256::from(1_000_000);
	pub DefaultElasticity: Permill = Permill::from_parts(125_000);
}

/// Sets the ideal block fullness to 50%.
/// If the block weight is between:
/// - 0-50% the gas fee will decrease
/// - 50-100% the gas fee will increase
pub struct BaseFeeThreshold;
impl pallet_base_fee::BaseFeeThreshold for BaseFeeThreshold {
	fn lower() -> Permill {
		Permill::zero()
	}
	fn ideal() -> Permill {
		Permill::from_parts(500_000)
	}
	fn upper() -> Permill {
		Permill::from_parts(1_000_000)
	}
}

impl pallet_base_fee::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type Threshold = BaseFeeThreshold;
	type DefaultBaseFeePerGas = DefaultBaseFeePerGas;
	type DefaultElasticity = DefaultElasticity;
}

impl pallet_hotfix_sufficients::Config for Runtime {
	type AddressMapping = HashedAddressMapping<BlakeTwo256>;
	type WeightInfo = pallet_hotfix_sufficients::weights::SubstrateWeight<Runtime>;
}
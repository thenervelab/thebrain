// # Copyright 2024 The Nerve Lab
// This file is part of Hippius.

// Hippius is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// Hippius is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with Hippius.  If not, see <http://www.gnu.org/licenses/>.

#![cfg_attr(not(feature = "std"), no_std)]

use ethereum::{TransactionV0 as LegacyTransaction, TransactionV2 as Transaction};
use ethereum_types::{H160, H256, U256};
use parity_scale_codec::{Decode, Encode};
use sp_std::vec::Vec;

sp_api::decl_runtime_apis! {
	// Api version is virtually 5.
	//
	// We realized that even using runtime overrides, using the ApiExt interface reads the api
	// versions from the state runtime, meaning we cannot just reset the versioning as we see fit.
	//
	// In order to be able to use ApiExt as part of the RPC handler logic we need to be always
	// above the version that exists on chain for this Api, even if this Api is only meant
	// to be used overridden.
	#[api_version(6)]
	pub trait DebugRuntimeApi {
		#[changed_in(5)]
		fn trace_transaction(
			extrinsics: Vec<Block::Extrinsic>,
			transaction: &Transaction,
		) -> Result<(), sp_runtime::DispatchError>;

		#[changed_in(4)]
		fn trace_transaction(
			extrinsics: Vec<Block::Extrinsic>,
			transaction: &LegacyTransaction,
		) -> Result<(), sp_runtime::DispatchError>;

		fn trace_transaction(
			extrinsics: Vec<Block::Extrinsic>,
			transaction: &Transaction,
			header: &Block::Header,
		) -> Result<(), sp_runtime::DispatchError>;

		#[changed_in(5)]
		fn trace_block(
			extrinsics: Vec<Block::Extrinsic>,
			known_transactions: Vec<H256>,
		) -> Result<(), sp_runtime::DispatchError>;

		fn trace_block(
			extrinsics: Vec<Block::Extrinsic>,
			known_transactions: Vec<H256>,
			header: &Block::Header,
		) -> Result<(), sp_runtime::DispatchError>;

		#[allow(clippy::too_many_arguments)]
		fn trace_call(
			header: &Block::Header,
			from: H160,
			to: H160,
			data: Vec<u8>,
			value: U256,
			gas_limit: U256,
			max_fee_per_gas: Option<U256>,
			max_priority_fee_per_gas: Option<U256>,
			nonce: Option<U256>,
			access_list: Option<Vec<(H160, Vec<H256>)>>,
		) -> Result<(), sp_runtime::DispatchError>;
	}
}

#[derive(Clone, Copy, Eq, PartialEq, Debug, Encode, Decode)]
pub enum TracerInput {
	None,
	Blockscout,
	CallTracer,
}

/// DebugRuntimeApi V2 result. Trace response is stored in client and runtime api call response is
/// empty.
#[derive(Debug)]
pub enum Response {
	Single,
	Block,
}

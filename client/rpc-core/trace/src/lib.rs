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

use client_evm_tracing::types::block::TransactionTrace;
use ethereum_types::H160;
use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use rpc_core_types::RequestBlockId;
use serde::Deserialize;

#[rpc(server)]
#[jsonrpsee::core::async_trait]
pub trait Trace {
	#[method(name = "trace_filter")]
	async fn filter(&self, filter: FilterRequest) -> RpcResult<Vec<TransactionTrace>>;
}

#[derive(Clone, Eq, PartialEq, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterRequest {
	/// (optional?) From this block.
	pub from_block: Option<RequestBlockId>,

	/// (optional?) To this block.
	pub to_block: Option<RequestBlockId>,

	/// (optional) Sent from these addresses.
	pub from_address: Option<Vec<H160>>,

	/// (optional) Sent to these addresses.
	pub to_address: Option<Vec<H160>>,

	/// (optional) The offset trace number
	pub after: Option<u32>,

	/// (optional) Integer number of traces to display in a batch.
	pub count: Option<u32>,
}

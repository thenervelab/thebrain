// This file is part of Frontier.

// # Copyright 2024 The Nerve Lab
// SPDX-License-Identifier: GPL-3.0-or-later WITH Classpath-exception-2.0

// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.

// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU General Public License for more details.

// You should have received a copy of the GNU General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.

//! Net rpc interface.

use jsonrpsee::{core::RpcResult, proc_macros::rpc};

pub mod types;
pub use crate::types::PeerCount;

/// Net rpc interface.
#[rpc(server)]
pub trait ExtraApi {
	/// Returns protocol version.
	#[method(name = "extra_version")]
	fn version(&self) -> RpcResult<String>;

	/// Returns number of peers connected to node.
	#[method(name = "extra_peerCount")]
	fn peer_count(&self) -> RpcResult<PeerCount>;

	/// Returns true if client is actively listening for network connections.
	/// Otherwise false.
	#[method(name = "extra_peerId")]
	fn peer_id(&self) -> RpcResult<String>;

	/// Returns true if client is actively listening for network connections.
	/// Otherwise false.
	#[method(name = "extra_listening")]
	fn is_listening(&self) -> RpcResult<bool>;
}

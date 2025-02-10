pub use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use sp_std::vec::Vec;
pub mod types;

pub use crate::types::NodeType;

/// Net rpc interface.
#[rpc(server)]
pub trait NodeMetricsApi {

	/// Download a file from local IPFS node to a specified path
	#[method(name = "get_active_nodes_by_type")]
	fn get_active_nodes_by_type(&self) -> RpcResult<Vec<Vec<u8>>>;
}
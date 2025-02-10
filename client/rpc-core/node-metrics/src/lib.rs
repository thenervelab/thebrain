pub use jsonrpsee::{core::RpcResult, proc_macros::rpc};

/// Net rpc interface.
#[rpc(server)]
pub trait NodeMetricsApi {

	/// Download a file from local IPFS node to a specified path
	#[method(name = "get_nodes_by_node_type")]
	fn get_nodes_by_node_type(&self, cid: String) -> RpcResult<Vec<u8>>;
}
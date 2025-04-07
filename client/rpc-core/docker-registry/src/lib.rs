pub use jsonrpsee::{core::RpcResult, proc_macros::rpc};

/// Net rpc interface.
#[rpc(server)]
pub trait DockerRegistryApi {
	/// Download a file from local IPFS node to a specified path
	#[method(name = "docker_pullImage")]
	fn download_ipfs_file(&self, cid: String) -> RpcResult<Vec<u8>>;
}

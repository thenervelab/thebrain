use jsonrpsee::core::RpcResult;
use jsonrpsee::types::error::ErrorObject;
use std::sync::Arc;
pub use rpc_core_docker_registry:: NodeMetricsApiServer;
use sp_runtime::traits::Block as BlockT;
use sp_blockchain::HeaderBackend;
use fp_rpc::EthereumRuntimeRPCApi;
use sp_api::ProvideRuntimeApi;

/// Net API implementation.
pub struct NodeMetricsImpl<B: BlockT, C> {
	_client: Arc<C>,
	_phantom_data: std::marker::PhantomData<B>,          
}

impl<B: BlockT, C> NodeMetricsImpl<B, C> {
	pub fn new(_client: Arc<C>) -> Self {
		Self {
			_client,
			_phantom_data: Default::default(),
		}
	}
}

impl<B, C> NodeMetricsApiServer for NodeMetricsImpl<B, C>
where
	B: BlockT,
	C: ProvideRuntimeApi<B> + 'static,
	C::Api: EthereumRuntimeRPCApi<B>,
	C: HeaderBackend<B> + Send + Sync,
{
    fn get_nodes_by_node_type(&self, cid: String) -> RpcResult<Vec<u8>> {
        Ok("".as_bytes().to_vec())
    }

}
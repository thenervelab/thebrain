use jsonrpsee::core::RpcResult;
use jsonrpsee::proc_macros::rpc;
// use log::{error, info};
// use rpc_primitives_node_metrics::NodeType;
use sc_client_api::AuxStore;
use sc_network::NetworkService;
use sc_utils::mpsc::{TracingUnboundedReceiver, TracingUnboundedSender};
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::Block as BlockT;
use std::sync::Arc;
use rpc_core_node_metrics::{types::{NodeType},NodeMetricsApiServer};
use rpc_primitives_node_metrics::NodeMetricsRuntimeApi;
use fc_rpc::internal_err;
use sp_std::vec::Vec;

/// Net API implementation.
pub struct NodeMetricsImpl<B: BlockT, C> {
	client: Arc<C>,
	_phantom_data: std::marker::PhantomData<B>,          
}

impl<B: BlockT, C> NodeMetricsImpl<B, C> {
	pub fn new(client: Arc<C>) -> Self {
		Self {
			client,
			_phantom_data: Default::default(),
		}
	}
}

impl<B, C> NodeMetricsApiServer for NodeMetricsImpl<B, C>
where
	B: BlockT,
	C: ProvideRuntimeApi<B> + 'static,
	C::Api: NodeMetricsRuntimeApi<B>,
	C: HeaderBackend<B> + Send + Sync,
{
    fn get_active_nodes_by_type(&self) -> RpcResult<Vec<Vec<u8>>> {

        let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		Ok(api.get_active_nodes_by_type(best_hash).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})?)
    }
}
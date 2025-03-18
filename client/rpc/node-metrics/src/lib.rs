use jsonrpsee::core::RpcResult;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_runtime::traits::Block as BlockT;
use std::sync::Arc;
pub use rpc_core_node_metrics::{NodeMetricsApiServer};
use rpc_primitives_node_metrics::{NodeType, Status, Batch, NodeMetricsData, MinerRewardSummary, UserFile, UserBucket, UserVmDetails};
use rpc_primitives_node_metrics::NodeMetricsRuntimeApi;
use fc_rpc::internal_err;
use sp_std::vec::Vec;
use sp_runtime::AccountId32;

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
    fn get_active_nodes_metrics_by_type(&self, node_type: NodeType) -> RpcResult<Vec<Option<NodeMetricsData>>> {

        let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_active_nodes_metrics_by_type(best_hash, node_type).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
    }

	fn get_node_metrics(&self, node_id: Vec<u8>) -> RpcResult<Option<NodeMetricsData>>{

		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_node_metrics(best_hash, node_id).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_total_node_rewards(&self, account: AccountId32) -> RpcResult<u128> {       
		
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_total_node_rewards(best_hash, account).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_total_distributed_rewards_by_node_type(&self, node_type: NodeType) -> RpcResult<u128>{
		
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_total_distributed_rewards_by_node_type(best_hash, node_type).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_miners_total_rewards(&self, node_type: NodeType) -> RpcResult<Vec<MinerRewardSummary>>{
		
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_miners_total_rewards(best_hash, node_type).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_account_pending_rewards(&self, account: AccountId32) -> RpcResult<Vec<MinerRewardSummary>>{
		
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_account_pending_rewards(best_hash, account).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_miners_pending_rewards(&self, node_type: NodeType) -> RpcResult<Vec<MinerRewardSummary>>{
		
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_miners_pending_rewards(best_hash, node_type).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn calculate_total_file_size(&self, account: AccountId32) -> RpcResult<u128>{
		
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.calculate_total_file_size(best_hash, account).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_user_files(&self, account: AccountId32) -> RpcResult<Vec<UserFile>>{
		
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_user_files(best_hash, account).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_user_buckets(&self, account: AccountId32) -> RpcResult<Vec<UserBucket>>{
		
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_user_buckets(best_hash, account).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_user_vms(&self, account: AccountId32) -> RpcResult<Vec<UserVmDetails<AccountId32,  u32, [u8; 32]>>>{
		
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_user_vms(best_hash, account).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}


	fn get_client_ip(&self, client_id: AccountId32) -> RpcResult<Option<Vec<u8>>>{
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_client_ip(best_hash, client_id).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}


	fn get_hypervisor_ip(&self, hypervisor_id: Vec<u8>) -> RpcResult<Option<Vec<u8>>>{
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_hypervisor_ip(best_hash, hypervisor_id).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_vm_ip(&self, vm_id: Vec<u8>) -> RpcResult<Option<Vec<u8>>>{
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_vm_ip(best_hash, vm_id).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_storage_miner_ip(&self, miner_id: Vec<u8>) -> RpcResult<Option<Vec<u8>>>{
		
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_storage_miner_ip(best_hash, miner_id).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_total_bucket_size(&self, account_id: AccountId32) -> RpcResult<u128>{
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_total_bucket_size(best_hash, account_id).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_user_bandwidth(&self, account_id: AccountId32) -> RpcResult<u128>{
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_user_bandwidth(best_hash, account_id).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_bucket_size(&self, bucket_name: Vec<u8>) -> RpcResult<u128>{
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_bucket_size(best_hash, bucket_name).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_miner_info(&self, account_id: AccountId32) -> RpcResult<Option<(NodeType, Status)>>{
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_miner_info(best_hash, account_id).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_batches_for_user(&self, account_id: AccountId32) -> RpcResult<Vec<Batch<AccountId32, u32>>>{
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_batches_for_user(best_hash, account_id).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_batch_by_id(&self, batch_id: u64) -> RpcResult<Option<Batch<AccountId32, u32>>>{
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_batch_by_id(best_hash, batch_id).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_free_credits_rpc(&self, account: Option<AccountId32>) -> RpcResult<Vec<(AccountId32, u128)>>{
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_free_credits_rpc(best_hash, account).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_referred_users(&self, account_id: AccountId32) -> RpcResult<Vec<AccountId32>> {
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_referred_users(best_hash, account_id).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_referral_rewards(&self, account_id: AccountId32) -> RpcResult<u128>{
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_referral_rewards(best_hash, account_id).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn total_referral_codes(&self) -> RpcResult<u32>{
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.total_referral_codes(best_hash).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn total_referral_rewards(&self) -> RpcResult<u128>{
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.total_referral_rewards(best_hash).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}

	fn get_referral_codes(&self, account_id: AccountId32) -> RpcResult<Vec<Vec<u8>>>{
		let api = self.client.runtime_api();
		let best_hash = self.client.info().best_hash;

		api.get_referral_codes(best_hash, account_id).map_err(|err| {
			internal_err(format!("fetch runtime extrinsic filter failed: {:?}", err))
		})
	}
}
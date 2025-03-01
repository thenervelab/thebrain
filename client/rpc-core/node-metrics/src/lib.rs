pub use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use sp_std::vec::Vec;
pub mod types;
use sp_runtime::AccountId32;

use rpc_primitives_node_metrics::{NodeType, NodeMetricsData, MinerRewardSummary, UserFile};

/// Net rpc interface.
#[rpc(server)]
pub trait NodeMetricsApi {

	#[method(name = "get_active_nodes_metrics_by_type")]
	fn get_active_nodes_metrics_by_type(&self, node_type: NodeType) -> RpcResult<Vec<Option<NodeMetricsData>>>;

	#[method(name = "get_total_node_rewards")]
	fn get_total_node_rewards(&self, account: AccountId32) -> RpcResult<u128>;

	#[method(name = "get_total_distributed_rewards_by_node_type")]
	fn get_total_distributed_rewards_by_node_type(&self, node_type: NodeType) -> RpcResult<u128>;

	#[method(name = "get_miners_total_rewards")]
	fn get_miners_total_rewards(&self, node_type: NodeType) -> RpcResult<Vec<MinerRewardSummary>>;

	#[method(name = "get_account_pending_rewards")]
	fn get_account_pending_rewards(&self, account: AccountId32) -> RpcResult<Vec<MinerRewardSummary>>;

	#[method(name = "get_miners_pending_rewards")]
	fn get_miners_pending_rewards(&self, node_type: NodeType) -> RpcResult<Vec<MinerRewardSummary>>;

	#[method(name = "calculate_total_file_size")]
	fn calculate_total_file_size(&self, account: AccountId32) -> RpcResult<u128>;

	#[method(name = "get_user_files")]
	fn get_user_files(&self, account: AccountId32) -> RpcResult<Vec<UserFile>>;
}
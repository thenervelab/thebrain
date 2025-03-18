pub use jsonrpsee::{core::RpcResult, proc_macros::rpc};
use sp_std::vec::Vec;
pub mod types;
use sp_runtime::AccountId32;

use rpc_primitives_node_metrics::{NodeType, Batch, Status , NodeMetricsData, MinerRewardSummary, UserFile, UserBucket, UserVmDetails};

/// Net rpc interface.
#[rpc(server)]
pub trait NodeMetricsApi {

	#[method(name = "get_active_nodes_metrics_by_type")]
	fn get_active_nodes_metrics_by_type(&self, node_type: NodeType) -> RpcResult<Vec<Option<NodeMetricsData>>>;

	#[method(name = "get_node_metrics")]
	fn get_node_metrics(&self, node_id: Vec<u8>) -> RpcResult<Option<NodeMetricsData>>;

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

	#[method(name = "get_user_buckets")]
	fn get_user_buckets(&self, account: AccountId32) -> RpcResult<Vec<UserBucket>>;

	#[method(name = "get_user_vms")]
	fn get_user_vms(&self, account: AccountId32) -> RpcResult<Vec<UserVmDetails<AccountId32, u32, [u8; 32]>>>;

	#[method(name = "get_client_ip")]
	fn get_client_ip(&self, client_id: AccountId32) -> RpcResult<Option<Vec<u8>>>;

	#[method(name = "get_hypervisor_ip")]
	fn get_hypervisor_ip(&self, hypervisor_id: Vec<u8>) -> RpcResult<Option<Vec<u8>>>;

	#[method(name = "get_vm_ip")]
	fn get_vm_ip(&self, vm_id: Vec<u8>) -> RpcResult<Option<Vec<u8>>>;

	#[method(name = "get_storage_miner_ip")]
	fn get_storage_miner_ip(&self, miner_id: Vec<u8>) -> RpcResult<Option<Vec<u8>>>;

	#[method(name = "get_bucket_size")]
	fn get_bucket_size(&self, bucket_name: Vec<u8>) -> RpcResult<u128>;

	#[method(name = "get_total_bucket_size")]
	fn get_total_bucket_size(&self, account_id: AccountId32) -> RpcResult<u128>;

	#[method(name = "get_user_bandwidth")]
	fn get_user_bandwidth(&self, account_id: AccountId32) -> RpcResult<u128>;

	#[method(name = "get_miner_info")]
	fn get_miner_info(&self, account_id: AccountId32) -> RpcResult<Option<(NodeType, Status)>>;

	#[method(name = "get_batches_for_user")]
	fn get_batches_for_user(&self, account_id: AccountId32) -> RpcResult<Vec<Batch<AccountId32, u32>>>;

	#[method(name = "get_batch_by_id")]
	fn get_batch_by_id(&self, batch_id: u64) -> RpcResult<Option<Batch<AccountId32, u32>>>;

	#[method(name = "get_free_credits")]
	fn get_free_credits_rpc(&self, account: Option<AccountId32>) -> RpcResult<Vec<(AccountId32, u128)>>;

	#[method(name = "get_referred_users")]
	fn get_referred_users(&self, account_id: AccountId32) -> RpcResult<Vec<AccountId32>>;

	#[method(name = "get_referral_rewards")]
	fn get_referral_rewards(&self, account_id: AccountId32) -> RpcResult<u128>;

	#[method(name = "total_referral_codes")]
	fn total_referral_codes(&self) -> RpcResult<u32>;

	#[method(name = "total_referral_rewards")]
	fn total_referral_rewards(&self) -> RpcResult<u128>;

	#[method(name = "get_referral_codes")]
	fn get_referral_codes(&self, account_id: AccountId32) -> RpcResult<Vec<Vec<u8>>>;
}

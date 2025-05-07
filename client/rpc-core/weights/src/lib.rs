pub use jsonrpsee::{core::RpcResult, proc_macros::rpc};
pub mod types;
pub use crate::types::{SubmitWeightsParams, SubmitHardwareParams, SubmitMetricsParams, SubmitRankingsParams};

/// Net rpc interface.
#[rpc(server)]
pub trait WeightsInfoApi {
	/// Returns protocol version.
	#[method(name = "submit_weights")]
	fn submit_weights(&self, params: SubmitWeightsParams) -> String;

	#[method(name = "submit_hardware")]
	fn submit_hardware(&self, params: SubmitHardwareParams) -> String;

	#[method(name = "submit_metrics")]
	fn submit_metrics(&self, params: SubmitMetricsParams) -> String;

	#[method(name = "submit_rankings")]
	fn submit_rankings(&self, params: SubmitRankingsParams) -> String;
}

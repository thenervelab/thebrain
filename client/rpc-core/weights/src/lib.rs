pub use jsonrpsee::{core::RpcResult, proc_macros::rpc};

pub mod types;

pub use crate::types::SubmitWeightsParams;

/// Net rpc interface.
#[rpc(server)]
pub trait WeightsInfoApi {
	/// Returns protocol version.
	#[method(name = "submit_weights")]
    fn submit_weights(&self, params: SubmitWeightsParams)-> String;
}

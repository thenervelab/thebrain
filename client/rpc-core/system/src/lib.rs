pub use jsonrpsee::{core::RpcResult, proc_macros::rpc};

pub mod types;

pub use crate::types::SystemInfo;

/// Net rpc interface.
#[rpc(server)]
pub trait SystemInfoApi {
	/// Returns protocol version.
	#[method(name = "sys_getSystemInfo")]
	fn get_system_info(&self) -> RpcResult<SystemInfo>;
}
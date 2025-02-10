
// pub use rpc_core_node_metrics::{types::NodeType};
#![cfg_attr(not(feature = "std"), no_std)]
use sp_api::decl_runtime_apis;

use scale_info::TypeInfo;
use parity_scale_codec::{Encode,Decode};
use sp_std::vec::Vec;
#[derive(  TypeInfo, Encode, Decode)]
pub enum NodeType {
    Validator,
    StorageMiner,
    ComputeMiner,
    GpuMiner
}

decl_runtime_apis! {
    pub trait NodeMetricsRuntimeApi {
        fn get_active_nodes_by_type() -> Vec<Vec<u8>>;
    }
}
use serde::{Serialize, Deserialize};
use scale_info::TypeInfo;
use parity_scale_codec::{Encode,Decode};

#[derive( Serialize, Deserialize, TypeInfo, Encode, Decode)]
pub enum NodeType {
    Validator,
    StorageMiner,
    ComputeMiner,
    GpuMiner
}
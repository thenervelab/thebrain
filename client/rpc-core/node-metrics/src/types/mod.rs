use serde::{Serialize, Deserialize};
use scale_info::TypeInfo;
use parity_scale_codec::{Encode,Decode};
use sp_runtime::AccountId32;

#[derive( Serialize, Deserialize, TypeInfo, Encode, Decode)]
pub enum NodeType {
    Validator,
    StorageMiner,
    StorageS3,
    ComputeMiner,
    GpuMiner
}

#[derive( Serialize, Clone,  Deserialize, TypeInfo, Encode, Decode)]
pub struct MinerRewardSummary {
    pub account: AccountId32,
    pub reward: u128,
}
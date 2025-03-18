use codec::{Decode, Encode};
use scale_info::TypeInfo;
use sp_std::prelude::*;

// This will store info related storage request
#[derive(Encode, Decode,Clone, Eq, PartialEq, Debug, TypeInfo)]
pub struct NodeInfo<BlockNumber,AccountId> {
    pub node_id: Vec<u8>,
    pub node_type: NodeType,
    pub ipfs_node_id: Option<Vec<u8>>,
    pub status: Status,
    pub registered_at: BlockNumber,
    pub owner: AccountId
}

#[derive( Encode, Decode,Clone, Eq, PartialEq, Debug, TypeInfo)]
pub enum NodeType {
    Validator,
    StorageMiner,
    StorageS3,
    ComputeMiner,
    GpuMiner
}

#[derive( Encode, Decode,Clone, Eq, PartialEq, Debug, TypeInfo)]
pub enum Status {
    Online,
    Degraded,
    Offline
}
use codec::{Decode, Encode};
use scale_info::TypeInfo;
use sp_std::{prelude::*, marker::PhantomData};
use frame_support::pallet_prelude::RuntimeDebug;
use frame_system::offchain::SignedPayload;
use frame_system::pallet_prelude::BlockNumberFor;

use crate::Config;
use pallet_registration::NodeType;

#[derive(Encode, Decode, Clone, TypeInfo)]
pub struct UpdateRankingsPayload<T: Config<I>, I: 'static = ()> {
    pub all_nodes_ss58: Vec<Vec<u8>>,
    pub weights: Vec<u16>,
    pub node_ids: Vec<Vec<u8>>,
    pub node_types: Vec<NodeType>,
    pub public: T::Public,
    pub block_number: BlockNumberFor<T>,
    pub ranking_instance_id: u32,
    pub _marker: PhantomData<I>, // Marker for unused type parameter
}

// Implement SignedPayload for UpdateRankingsPayload
impl<T: Config<I>, I: 'static> SignedPayload<T> for UpdateRankingsPayload<T, I> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

#[derive(Clone, Encode, Decode, RuntimeDebug, TypeInfo, PartialEq)]
pub struct NodeRankings<BlockNumberFor> {
    // rank of the node (position in the sorted list)
    pub rank: u32,
    // node identity
    pub node_id: Vec<u8>,
    // node identity
    pub node_ss58_address: Vec<u8>,
    // node type
    pub node_type: NodeType,
    // latest weight of the node
    pub weight: u16,
    // timestamp of last weight update
    pub last_updated: BlockNumberFor,
    // active status
    pub is_active: bool,
}
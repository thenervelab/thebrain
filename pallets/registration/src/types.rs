use codec::{Decode, Encode};
use pallet_utils::Role;
use scale_info::TypeInfo;
use sp_std::prelude::*;
use sp_core::H256;

// This will store info related storage request
#[derive(Encode, Decode, Clone, Eq, PartialEq, Debug, TypeInfo)]
pub struct NodeInfo<BlockNumber, AccountId> {
	pub node_id: Vec<u8>,
	pub node_type: NodeType,
	pub ipfs_node_id: Option<Vec<u8>>,
	pub status: Status,
	pub registered_at: BlockNumber,
	pub owner: AccountId,
    pub is_verified: bool,  // Libp2p identity verification status
    pub code_signature_verified: bool,  // Code/binary signature verification status
    pub code_public_key: Option<Vec<u8>>,  // Public key used for code signature
}

// DeregistrationReport with created_at field
#[derive(Encode, Decode, Clone, Eq, PartialEq, Debug, TypeInfo)]
pub struct DeregistrationReport<BlockNumberFor> {
    pub node_id: Vec<u8>,
    pub created_at: BlockNumberFor, // Block number when the report was created
}

#[derive(Encode, Decode, Clone, Eq, PartialEq, Debug, TypeInfo)]
pub enum NodeType {
	Validator,
	StorageMiner,
	StorageS3,
	ComputeMiner,
	GpuMiner,
}

impl NodeType {
	pub fn to_role(&self) -> Role {
		match self {
			NodeType::Validator => Role::Validator,
			NodeType::StorageMiner => Role::Miner,
			NodeType::StorageS3 => Role::Miner,
			NodeType::ComputeMiner => Role::Miner,
			NodeType::GpuMiner => Role::Miner,
		}
	}
}

#[derive(Encode, Decode, Clone, Eq, PartialEq, Debug, TypeInfo)]
pub enum Status {
	Online,
	Degraded,
	Offline,
}

#[derive(Encode, Decode, Clone, Copy, PartialEq, Eq, scale_info::TypeInfo, Debug)]
pub enum Libp2pKeyType { Ed25519 /* , Secp256k1 (optional later) */ }

#[derive(Encode, Decode, Clone, PartialEq, Eq, scale_info::TypeInfo, Debug)]
pub struct RegisterChallenge<AccountId, BlockNumber> {
    /// Exactly 24 bytes (e.g. b"HIPPIUS::REGISTER::v1")
    pub domain: [u8; 24],
    /// Chain binding (prevents cross-chain replay)
    pub genesis_hash: [u8; 32],
    /// Must equal `owner` used in the call
    pub account: AccountId,
    /// Bind to specific ids
    pub node_id_hash: H256,
    pub ipfs_peer_id_hash: H256,
    /// Context + TTL
    pub block_number: BlockNumber,
    pub nonce: [u8; 32],
    pub expires_at: BlockNumber,
}
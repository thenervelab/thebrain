use codec::{Decode, Encode};
use scale_info::TypeInfo;
use sp_std::prelude::*;

// use frame_support::pallet_prelude::*;
use sp_std::vec::Vec;

/// Unique identifier for a space
pub type SpaceId = u128;

/// Space Metadata Structure
#[derive(Encode, Decode, Clone, PartialEq, TypeInfo)]
pub struct SpaceMetadata<AccountId> {
	/// Owner of the space
	pub owner: AccountId,

	/// Members who have access to this space
	pub repo_name: Vec<u8>,

	/// Members who have access to this space
	pub members: Vec<AccountId>,

	/// Members who have access to this space
	pub image_names: Vec<Vec<u8>>,
}

/// Comprehensive Docker Image Metadata Structure
#[derive(Encode, Decode, Clone, PartialEq, TypeInfo)]
pub struct DockerImageMetadata {
	/// Manifest hash of the image
	pub manifest_hash: Vec<u8>,

	/// CID of the manifest file on IPFS
	pub manifest_cid: Vec<u8>,

	/// CIDs of the Docker image on IPFS
	pub image_cids: Vec<Vec<u8>>,

	/// Whether the image is public or not
	pub is_public: bool,

	/// Description of the image
	pub description: Vec<u8>,

	/// Build context CID for the Docker image
	/// This represents the IPFS CID of the build context (e.g., Dockerfile and related resources)
	pub build_context_cid: Option<Vec<u8>>,

	/// Configuration blob for the image
	/// This can include platform information, environment variables, entrypoint, etc.
	pub config_blob: Option<Vec<u8>>,
}

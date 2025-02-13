use frame_support::pallet_prelude::*;
use scale_info::TypeInfo;
use sp_runtime::Vec;

/// Represents a storage request made by a user
#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo)]
pub struct StorageRequest<AccountId, BlockNumber> {
    /// Unique identifier for the file/request
    pub file_hash: Vec<u8>,
    
    /// Name of the file being stored
    pub file_name: Vec<u8>,
    
    /// Size of the file in bytes
    pub file_size: u64,
    
    /// User who initiated the storage request
    pub user_id: AccountId,
    
    /// Indicates if the request has been assigned to miners
    pub is_assigned: bool,
    
    /// Block number when the request was created
    pub created_at: BlockNumber,
    
    /// Number of replicas requested
    pub requested_replicas: u32,
    
    /// Optional metadata or additional file information
    pub metadata: Option<Vec<u8>>,

    pub total_replicas: u32,

    pub fullfilled_replicas: u32,

    pub last_charged_at: BlockNumber,
}

/// Represents a storage delete request made by a user
#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo)]
pub struct StorageDeleteRequest<AccountId, BlockNumber> {
    /// Unique identifier for the file/request
    pub file_hash: Vec<u8>,
    
    /// User who initiated the delete request
    pub user_id: AccountId,
    
    /// Block number when the delete request was created
    pub created_at: BlockNumber,
    
    /// Indicates if the delete request has been processed
    pub is_fulfilled: bool,
    
    /// Block number when the delete request was processed
    pub created_at: Option<BlockNumber>,
    
    /// Reason for deletion (optional)
    pub reason: Option<Vec<u8>>,
}

/// Represents the assignment of a storage request to miners
#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo)]
pub struct StorageRequestAssignment<AccountId, BlockNumber> {
    /// Unique identifier for the storage request
    pub request_id: Vec<u8>,
    
    /// File hash associated with the request
    pub file_hash: Vec<u8>,
    
    /// Miner ID assigned to store the file
    pub miner_id: Vec<u8>,
    
    /// User who initiated the original storage request
    pub user_id: AccountId,
    
    /// URL or location where the file will be stored
    pub file_url: Vec<u8>,
    
    /// Indicates if the assignment has been fulfilled
    pub is_fulfilled: bool,
    
    /// Block number when the assignment was created
    pub created_at: BlockNumber,
    
    /// Block number when the assignment was fulfilled
    pub fulfilled_at: Option<BlockNumber>,
    
    /// S3 specific storage metadata
    pub s3_bucket: Option<Vec<u8>>,
    pub s3_key: Option<Vec<u8>>,

    /// Ceph-specific storage details
    pub ceph_pool_name: Option<Vec<u8>>,
    pub ceph_object_name: Option<Vec<u8>>,
    
    /// Optional additional storage parameters
    pub storage_params: Option<Vec<u8>>,
}
use codec::{Decode, Encode};
use sp_std::prelude::*;
use scale_info::TypeInfo;
use sp_std::fmt::Debug;

#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo)]
pub enum RoleType<AccountId> {
    Hypervisor(Vec<u8>),   // Stores node id
    Client(AccountId),       // Stores userAccountId
    Vm(Vec<u8>),           // Stores VM name
    StorageMiner(Vec<u8>), // Stores node id
}

// This will store info related storage request
#[derive(Clone, Encode,Decode, Eq, PartialEq, Debug, TypeInfo)]
pub struct IpReleaseRequest<BlockNumber, AccountId> {
    pub vm_name: Vec<u8>,
    pub ip: Vec<u8>,
    pub created_at: BlockNumber,
    pub role_type: RoleType<AccountId>,
}

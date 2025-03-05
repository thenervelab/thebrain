use codec::{Decode, Encode};
use frame_support::pallet_prelude::*;
use sp_std::prelude::*;
use scale_info::TypeInfo;

// This will store info related storage request
#[derive(Clone, Encode, Decode, Eq, PartialEq, Debug, TypeInfo)]
pub struct IpReleaseRequest<BlockNumber> {
    pub vm_name: Vec<u8>,
    pub ip: Vec<u8>,
    pub created_at: BlockNumber
}

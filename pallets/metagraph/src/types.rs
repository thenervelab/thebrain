use codec::{Decode, Encode};
use scale_info::TypeInfo;
use sp_core::sr25519;
use sp_runtime::RuntimeDebug;
use frame_system::{offchain::SignedPayload, pallet_prelude::BlockNumberFor};
use sp_std::{marker::PhantomData, prelude::*};
use crate::Config;

use sp_core::crypto::AccountId32;
#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub enum Role {
    Validator,
    Miner,
    None,
}
// use sp_std::vec::Vec;

impl Default for Role {
    fn default() -> Self {
        Role::None
    }
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct UID {
    pub address: sr25519::Public,
    pub id: u16,
    pub role: Role,
    pub substrate_address: AccountId32,
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, RuntimeDebug)]
pub struct UIDsPayload<Public, BlockNumber> {
    pub uids: Vec<UID>, // Assuming UIDs consist of a Vec of (UID, Role) pairs
    pub dividends: Vec<u16>,
    pub public: Public,
    pub _marker: PhantomData<BlockNumber>,
}

// Implement SignedPayload for UIDsPayload
impl<T: Config> SignedPayload<T> for UIDsPayload<T::Public, BlockNumberFor<T>> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}
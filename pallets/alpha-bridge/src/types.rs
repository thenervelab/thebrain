use codec::{Decode, Encode};
// use frame_support::pallet_prelude::*;
use sp_std::prelude::*;
use scale_info::TypeInfo;
use frame_system::offchain::SignedPayload;
use sp_std::marker::PhantomData;
use frame_system::pallet_prelude::BlockNumberFor;

#[derive(Encode, Decode, Default, TypeInfo)]
pub struct Batch<AccountId, BlockNumberFor> {
    pub owner: AccountId,        // User who deposited
    pub credit_amount: u128,     // Total credits in batch
    pub alpha_amount: u128,      // Total Alpha purchased
    pub remaining_credits: u128, // Remaining credits in the batch
    pub remaining_alpha: u128,   // Remaining Alpha in the batch
    pub pending_alpha: u128,     // Pending Alpha to be released
    pub is_frozen: bool,         // Freezes Alpha distribution, not credit use
    pub release_time: BlockNumberFor, // When Alpha can be distributed
}
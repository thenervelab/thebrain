use codec::{Decode, Encode};
use frame_support::pallet_prelude::*;
use sp_std::prelude::*;
use scale_info::TypeInfo;
use frame_system::offchain::SignedPayload;
use sp_std::marker::PhantomData;
use frame_system::pallet_prelude::BlockNumberFor;

#[derive(Encode, Decode, Default)]
struct Batch<AccountId> {
    owner: AccountId,    // User who deposited
    credit_amount: u128,    // Total credits in batch
    alpha_amount: u128,     // Total Alpha purchased
    remaining_credits: u128,
    remaining_alpha: u128,
    is_frozen: bool,        // Freezes Alpha distribution, not credit use
    release_time: u64,      // When Alpha can be distributed
}
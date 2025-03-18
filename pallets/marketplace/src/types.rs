use codec::{Decode, Encode};
use frame_support::pallet_prelude::*;
use sp_std::prelude::*;
use scale_info::TypeInfo;
use frame_system::pallet_prelude::BlockNumberFor;
use pallet_utils::SubscriptionId;
use crate::Config;
use frame_system::offchain::SignedPayload;


/// Maximum length for CDN location name
pub const MAX_NAME_LENGTH: u32 = 32;

/// Point balance type
pub type Points = u128;

/// Point transaction type
#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub enum NativeTransactionType {
    Purchase,
    Subscription,
    Refund,
    Transfer,
}

/// Point transaction record
#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo)]
#[scale_info(skip_type_params(T))]
pub struct PointTransaction<T> where T: frame_system::Config {
    pub transaction_type: NativeTransactionType,
    pub amount: Points,
    pub timestamp: BlockNumberFor<T>,
    pub subscription_id: Option<SubscriptionId>,
    pub _phantom: PhantomData<T>,
}

impl<T> MaxEncodedLen for PointTransaction<T>
where
    T: frame_system::Config,
    BlockNumberFor<T>: MaxEncodedLen,
{
    fn max_encoded_len() -> usize {
        NativeTransactionType::max_encoded_len()
            .saturating_add(Points::max_encoded_len())
            .saturating_add(BlockNumberFor::<T>::max_encoded_len())
            .saturating_add(Option::<SubscriptionId>::max_encoded_len())
            .saturating_add(0) // PhantomData
    }
}


#[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct Plan<Hash> {
    pub id: Hash, // Unique identifier for the plan
    pub plan_name: Vec<u8>, // Name of the plan
    pub plan_description: Vec<u8>, // JSON describing the plan
    pub plan_technical_description: Vec<u8>, // JSON with technical details
    pub is_suspended: bool,
    pub price: u128,
    pub name: Vec<u8>,
}

#[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo)]
pub struct FileInput {
    pub file_hash: Vec<u8>,
    pub file_name: Vec<u8>,
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, Default, TypeInfo)]
pub struct ImageDetails {
    pub url: Vec<u8>,
    pub description: Vec<u8>,
    pub name: Vec<u8>,
}

/// User subscription details
#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo)]
#[scale_info(skip_type_params(T))]
pub struct UserPlanSubscription<T>
where
    T: frame_system::Config,
{
    pub id: SubscriptionId,
    pub owner: T::AccountId,
    pub package: Plan<T::Hash>,                 // Associated plan
    pub cdn_location_id: Option<u32>,           // Optional CDN location ID
    pub active: bool,                           // Subscription activity status
    pub last_charged_at: BlockNumberFor<T>,
    pub selected_image_name: Vec<u8>,           // Name of the selected image
    pub _phantom: PhantomData<T>,               // Placeholder for generic type
}

/// CDN location configuration
#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct CdnLocation {
    pub id: u32,
    pub name: BoundedVec<u8, ConstU32<MAX_NAME_LENGTH>>,
    pub price_multiplier: u32,
}

#[derive(Encode, Decode, Clone, PartialEq,  RuntimeDebug, TypeInfo)]
pub struct StorageApprovalPayload<T: Config> {
    pub account_id: T::AccountId,
    pub storage_cost: u128,
    pub public: T::Public,
    pub _marker: PhantomData<T>,
}

// Implement SignedPayload for UpdateRankingsPayload
impl<T: Config> SignedPayload<T> for StorageApprovalPayload<T> {
    fn public(&self) -> T::Public {
        self.public.clone()
    }
}

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
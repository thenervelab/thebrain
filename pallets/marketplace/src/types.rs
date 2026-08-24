use crate::Config;
use codec::{Decode, Encode};
use frame_support::pallet_prelude::*;
use frame_system::offchain::SignedPayload;
use frame_system::pallet_prelude::BlockNumberFor;
use pallet_utils::SubscriptionId;
use scale_info::TypeInfo;
use sp_std::prelude::*;

/// Maximum length for CDN location name
pub const MAX_NAME_LENGTH: u32 = 32;

/// Point balance type
pub type Points = u128;

/// Drive + S3 usage update row (validator metric).
#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct UserBackendFileUsageUpdate<AccountId> {
	pub account_id: AccountId,
	pub drive_file_size: u128,
	pub drive_file_count: u128,
	pub s3_file_size: u128,
	pub s3_file_count: u128,
}

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
pub struct PointTransaction<T>
where
	T: frame_system::Config,
{
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
	pub id: Hash,                            // Unique identifier for the plan
	pub plan_name: Vec<u8>,                  // Name of the plan
	pub plan_description: Vec<u8>,           // JSON describing the plan
	pub plan_technical_description: Vec<u8>, // JSON with technical details
	pub is_suspended: bool,
	pub price: u128,
	/// Drive storage. Mutually exclusive with `is_s3_plan`; `add_new_plan`
	/// rejects a plan that sets both.
	pub is_storage_plan: bool,
	/// S3 storage. Mutually exclusive with `is_storage_plan`.
	pub is_s3_plan: bool,
	pub storage_limit: Option<u128>,
}

/// Which storage slot a plan occupies. Compute plans occupy none.
///
/// An account holds at most one active plan per flavour, and each flavour
/// exempts only its own bytes from hourly per-GB billing — so this is what the
/// uniqueness rule, the plan-change routing and the hourly exemption all key
/// on, rather than re-deriving the same boolean pair at each site.
#[derive(Clone, Copy, PartialEq, Eq, RuntimeDebug)]
pub enum StorageFlavour {
	Drive,
	S3,
}

impl<Hash> Plan<Hash> {
	/// `None` for compute plans, which occupy no storage slot.
	///
	/// The two flags are mutually exclusive, so the order here only decides how
	/// the impossible both-set combination degrades: as Drive, never silently
	/// as compute. `add_new_plan` rejects it and no migration produces it.
	pub fn storage_flavour(&self) -> Option<StorageFlavour> {
		if self.is_storage_plan {
			Some(StorageFlavour::Drive)
		} else if self.is_s3_plan {
			Some(StorageFlavour::S3)
		} else {
			None
		}
	}

	/// Any non-compute plan — Drive or S3.
	///
	/// This is the "storage vs compute" split every routing branch wants.
	/// Reading `is_storage_plan` alone would silently route S3 plans down the
	/// compute path.
	pub fn is_any_storage(&self) -> bool {
		self.is_storage_plan || self.is_s3_plan
	}

	/// Drive storage specifically.
	pub fn is_drive_plan(&self) -> bool {
		self.is_storage_plan
	}
}

#[derive(Encode, Decode, Clone, PartialEq, Eq, Default, TypeInfo)]
pub struct ImageDetails {
	pub url: Vec<u8>,
	pub description: Vec<u8>,
	pub name: Vec<u8>,
}

/// User subscription details
#[derive(Clone, Encode, Eq, PartialEq, RuntimeDebug, TypeInfo)]
#[scale_info(skip_type_params(T))]
pub struct UserPlanSubscription<T>
where
	T: frame_system::Config,
{
	pub id: SubscriptionId,
	pub owner: T::AccountId,
	pub package: Plan<T::Hash>,       // Associated plan
	pub cdn_location_id: Option<u32>, // Optional CDN location ID
	pub active: bool,                 // Subscription activity status
	pub last_charged_at: BlockNumberFor<T>,
	pub selected_image_name: Option<Vec<u8>>, // Name of the selected image
	/// Unix day (UTC) of the 1st of the next month when recurring charging is due.
	/// `None` means legacy behavior (charge on the next 1st).
	pub next_charge_unix_day: Option<u32>,
	/// Actual credits paid per month (after referral discount).
	///
	/// Note: upfront purchases can include a prorated first month; this field tracks the
	/// discounted *full-month* price, which is what we refund for unused prepaid full months.
	pub paid_per_month: u128,
	pub _phantom: PhantomData<T>, // Placeholder for generic type
}

impl<T> Decode for UserPlanSubscription<T>
where
	T: frame_system::Config,
	T::AccountId: Decode,
	T::Hash: Decode,
	BlockNumberFor<T>: Decode,
{
	fn decode<I: codec::Input>(input: &mut I) -> Result<Self, codec::Error> {
		let id = SubscriptionId::decode(input)?;
		let owner = T::AccountId::decode(input)?;
		let package = Plan::<T::Hash>::decode(input)?;
		let cdn_location_id = Option::<u32>::decode(input)?;
		let active = bool::decode(input)?;
		let last_charged_at = BlockNumberFor::<T>::decode(input)?;
		let selected_image_name = Option::<Vec<u8>>::decode(input)?;

		// Backward-compatible decode:
		// - legacy encoding ended after `selected_image_name` (PhantomData encodes to 0 bytes)
		// - v1 encoding has `next_charge_unix_day` appended.
		// - v2 encoding appends `paid_per_month` after that.
		let next_charge_unix_day = match input.remaining_len()? {
			Some(0) => None,
			_ => Option::<u32>::decode(input)?,
		};

		let paid_per_month = match input.remaining_len()? {
			Some(0) => package.price,
			_ => u128::decode(input)?,
		};

		Ok(Self {
			id,
			owner,
			package,
			cdn_location_id,
			active,
			last_charged_at,
			selected_image_name,
			next_charge_unix_day,
			paid_per_month,
			_phantom: PhantomData,
		})
	}
}

/// CDN location configuration
#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct CdnLocation {
	pub id: u32,
	pub name: BoundedVec<u8, ConstU32<MAX_NAME_LENGTH>>,
	pub price_multiplier: u32,
}

#[derive(Encode, Decode, Clone, PartialEq, RuntimeDebug, TypeInfo)]
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
	pub owner: AccountId,             // User who deposited
	pub credit_amount: u128,          // Total credits in batch
	pub alpha_amount: u128,           // Total Alpha purchased
	pub remaining_credits: u128,      // Remaining credits in the batch
	pub remaining_alpha: u128,        // Remaining Alpha in the batch
	pub pending_alpha: u128,          // Pending Alpha to be released
	pub is_frozen: bool,              // Freezes Alpha distribution, not credit use
	pub release_time: BlockNumberFor, // When Alpha can be distributed
}

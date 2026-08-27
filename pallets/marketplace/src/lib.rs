#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;
pub use types::*;

pub mod migrations;
mod types;
use sp_core::offchain::KeyTypeId;
// use frame_system::offchain::SignedPayload;

/// Defines application identifier for crypto keys of this module.
///
/// Every module that deals with signatures needs to declare its unique identifier for
/// its crypto keys.
/// When offchain worker is signing transactions it's going to request keys of type
/// `KeyTypeId` from the keystore and use the ones it finds to sign the transaction.
/// The keys can be inserted manually via RPC (see `author_insertKey`).
pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"hips");

/// Based on the above `KeyTypeId` we need to generate a pallet-specific crypto type wrappers.
/// We can use from supported crypto kinds (`sr25519`, `ed25519` and `ecdsa`) and augment
/// the types with this pallet-specific identifier.
pub mod crypto {
	use super::KEY_TYPE;
	use sp_core::sr25519::Signature as Sr25519Signature;
	use sp_runtime::{
		app_crypto::{app_crypto, sr25519},
		traits::Verify,
		MultiSignature, MultiSigner,
	};
	app_crypto!(sr25519, KEY_TYPE);

	pub struct TestAuthId;

	impl frame_system::offchain::AppCrypto<MultiSigner, MultiSignature> for TestAuthId {
		type RuntimeAppPublic = Public;
		type GenericSignature = sp_core::sr25519::Signature;
		type GenericPublic = sp_core::sr25519::Public;
	}

	impl frame_system::offchain::AppCrypto<<Sr25519Signature as Verify>::Signer, Sr25519Signature>
		for TestAuthId
	{
		type RuntimeAppPublic = Public;
		type GenericSignature = sp_core::sr25519::Signature;
		type GenericPublic = sp_core::sr25519::Public;
	}
}

// #[cfg(feature = "runtime-benchmarks")]
// mod benchmarking;
#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::traits::ExistenceRequirement;
	use frame_support::traits::Len;
	use frame_support::{
		pallet_prelude::*,
		traits::OnRuntimeUpgrade,
		traits::StorageVersion,
		traits::{Currency, ReservableCurrency},
		transactional, PalletId,
	};
	use frame_system::offchain::AppCrypto;
	use frame_system::offchain::SendTransactionTypes;
	use frame_system::offchain::SendUnsignedTransaction;
	use frame_system::pallet_prelude::*;
	use num_traits::float::FloatCore;
	use pallet_credits::AlphaBalances;
	use pallet_credits::Pallet as CreditsPallet;
	use pallet_credits::TotalCreditsPurchased;
	use pallet_registration::BalanceOf;
	use pallet_registration::NodeType;
	use pallet_registration::Pallet as RegistrationPallet;
	use pallet_utils::SubscriptionId;
	use payment_math::{gibs_ceil, prorate_first_month, split, times, BasisPoints, Bytes, Credits};
	use sp_core::H256;
	use sp_core::U256;
	use sp_runtime::traits::Bounded;
	use sp_runtime::traits::Zero;
	use sp_runtime::{
		traits::{AccountIdConversion, AtLeast32BitUnsigned, Hash, SaturatedConversion},
		Saturating,
	};
	use sp_std::{vec, vec::Vec};
	#[pallet::pallet]
	#[pallet::without_storage_info]
	#[pallet::storage_version(STORAGE_VERSION)]
	pub struct Pallet<T>(_);

	/// The current storage version.
	const STORAGE_VERSION: StorageVersion = StorageVersion::new(2);

	/// How many empty due-days the renewal drain may skip past in one tick.
	///
	/// The cursor only advances over a drained day, so after downtime it has to
	/// walk forward to today. Bounding the walk keeps the probe cost declarable
	/// while still closing a two-month gap in a handful of ticks.
	const MAX_DAY_PROBES: u32 = 64;

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_runtime_upgrade() -> Weight {
			migrations::Migrate::<T>::on_runtime_upgrade()
		}

		fn on_initialize(current_block: BlockNumberFor<T>) -> Weight {
			let mut weight_used = Weight::zero();
			// Only execute on blocks divisible by the configured interval
			if current_block % 15u32.into() == 0u32.into() {
				// Clear all entries; limit is u32::MAX to ensure we get them all
				let result = UserRequestsCount::<T>::clear(u32::MAX, None);
				// Conservative: at least one write per removed key plus one read for the clear call.
				weight_used = weight_used
					.saturating_add(T::DbWeight::get().reads_writes(1, result.unique as u64));
			}

			// Referral commissions accrued by hourly billing are swept out on
			// their own cadence, independent of the charge interval that earns
			// them: billing writes the balance, this pays it.
			if current_block % T::ReferralPayoutInterval::get().into() == 0u32.into() {
				weight_used = weight_used.saturating_add(Self::sweep_referral_commissions());
			}

			// Only execute on blocks divisible by the configured interval
			if current_block % T::BlockChargeCheckInterval::get().into() == 0u32.into() {
				weight_used =
					weight_used.saturating_add(Self::handle_hourly_storage_charging(current_block));

				// Populate the due-day index for subscriptions that predate it.
				// Ordinary bounded hook work, not a migration — see
				// `BackfillCursor`. No-op once finished.
				weight_used = weight_used.saturating_add(Self::backfill_due_index());

				// Subscription renewals: drain the accounts actually due, up to
				// the per-run cap. Every day has someone due under date-to-date
				// billing, so this runs every tick rather than once a month —
				// which is only affordable because it reads the due-day prefix
				// instead of walking every account.
				weight_used = weight_used
					.saturating_add(Self::handle_all_subscription_charging(current_block));
				if let Err(e) = Self::release_matured_pending_alpha(current_block) {
					log::error!(
						target: "runtime::marketplace",
						"release_matured_pending_alpha failed at block {:?}: {:?}",
						current_block,
						e
					);
				}
				// Conservative: iterating batches is unbounded; charge at least one read.
				weight_used = weight_used.saturating_add(T::DbWeight::get().reads_writes(1, 0));
			}

			weight_used
		}
	}

	#[pallet::config]
	pub trait Config: frame_system::Config + 
                    pallet_registration::Config + 
                    pallet_credits::Config + 
                    pallet_arion::Config +
                    pallet_hippocampus::Config +
                    pallet_balances::Config + 
                    pallet_calendar::Config +
                    // pallet_notifications::Config +
                    // pallet_storage_s3::Config +
                    pallet_rankings::Config +
                    pallet_rankings::Config<pallet_rankings::Instance2> +
                    pallet_rankings::Config<pallet_rankings::Instance3> +
                    // pallet_rankings::Config<pallet_rankings::Instance4> +
                    // pallet_rankings::Config<pallet_rankings::Instance5> +
					SendTransactionTypes<Call<Self>> + 
					frame_system::offchain::SigningTypes 
        {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
        
        /// The identifier type for an offchain worker.
		type AuthorityId: AppCrypto<Self::Public, Self::Signature>;

        /// The currency mechanism.
        type Currency: ReservableCurrency<Self::AccountId>;

        /// The balance type used for this pallet.
        type Balance: Parameter + Member + AtLeast32BitUnsigned + Default + Copy+ TryFrom<BalanceOf<Self>>
        + Into<<Self as pallet_balances::Config>::Balance>;

        /// Minimum subscription duration in blocks
        #[pallet::constant]
        type MinSubscriptionBlocks: Get<BlockNumberFor<Self>>;

        /// Maximum active subscriptions per user
        #[pallet::constant]
        type MaxActiveSubscriptions: Get<u32>;
        
        /// The origin that is allowed to update usage metrics
        type UpdateOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// The pallet's id, used for deriving its sovereign account ID.
        #[pallet::constant]
        type PalletId: Get<PalletId>;

        #[pallet::constant]
        type BlockDurationMillis: Get<u64>;

        #[pallet::constant]
        type BlocksPerHour: Get<u32>;

        #[pallet::constant]
        type BlocksPerEra: Get<u32>;

        /// Custom hash type for this pallet
        type CustomHash: Parameter + Default + From<H256>;

        /// The block interval for executing certain pallet operations
        type BlockChargeCheckInterval: Get<u32>;

        #[pallet::constant]
        type MaxRequestsPerBlock: Get<u32>;

		/// Max number of user file-usage rows that can be updated in a single batch call.
		#[pallet::constant]
		type MaxUserFileUsageUpdatesPerCall: Get<u32>;

		/// Max number of users a single `create_referral_codes_for` call may cover.
		#[pallet::constant]
		type MaxReferralCodesPerCall: Get<u32>;

		/// How often accrued referral commissions are swept out to referrers,
		/// in blocks. Fixed at compile time on purpose: commission payout is
		/// automatic plumbing, not an operational dial.
		#[pallet::constant]
		type ReferralPayoutInterval: Get<u32>;

		/// Max referrers paid in a single sweep, bounding its weight. Sweeps
		/// resume from `ReferralPayoutCursor`, so exceeding this in one block
		/// delays a referrer to the next sweep rather than skipping them.
		#[pallet::constant]
		type MaxReferralPayoutsPerSweep: Get<u32>;

		/// Max accounts charged in a single renewal drain, bounding its weight.
		///
		/// This is the number that makes the weight returned by `on_initialize`
		/// a promise rather than a report. Nothing upstream rejects an
		/// overweight hook — the block is simply produced heavier — so the cap
		/// has to be applied before the work, not measured after it.
		///
		/// Counted in *accounts* because the weight formula is per-account.
		/// Accounts not reached stay in the day's bucket for the next tick.
		#[pallet::constant]
		type MaxSubscriptionChargesPerRun: Get<u32>;

		/// Max accounts the one-time due-index backfill walks per tick.
		#[pallet::constant]
		type MaxBackfillAccountsPerRun: Get<u32>;
    }

	// const LOCK_BLOCK_EXPIRATION: u32 = 3;
	// const LOCK_TIMEOUT_EXPIRATION: u32 = 10000;

	#[pallet::storage]
	#[pallet::getter(fn plans)]
	pub type Plans<T: Config> =
		StorageMap<_, Blake2_128Concat, T::Hash, Plan<T::Hash>, OptionQuery>;

	#[pallet::storage]
	pub(super) type PricePerGbs<T: Config> = StorageValue<_, u128, ValueQuery>;

	#[pallet::storage]
	pub(super) type PricePerBandwidth<T: Config> = StorageValue<_, u128, ValueQuery>;

	/// Storage to track the last charged timestamp for each user
	#[pallet::storage]
	pub(super) type StorageLastChargedAt<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, BlockNumberFor<T>, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn user_plan_subscription)]
	pub(super) type UserPlanSubscriptions<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, UserPlanSubscription<T>, OptionQuery>;

	#[pallet::storage]
	#[pallet::getter(fn user_all_subscription_plans)]
	pub(super) type UserAllSubscriptionPlans<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, Vec<UserPlanSubscription<T>>, ValueQuery>;

	// Storage for OS Disk Image URLs
	#[pallet::storage]
	#[pallet::getter(fn os_disk_image_urls)]
	pub type OSDiskImageUrls<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, ImageDetails>;

	// Map for batches identified by a unique ID
	#[pallet::storage]
	#[pallet::getter(fn batches)]
	pub type Batches<T: Config> =
		StorageMap<_, Blake2_128Concat, u64, Batch<T::AccountId, BlockNumberFor<T>>>;

	// Map for user batches identified by AccountId
	#[pallet::storage]
	#[pallet::getter(fn user_batches)]
	pub type UserBatches<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, Vec<u64>>;

	#[pallet::storage]
	#[pallet::getter(fn is_storage_operations_enabled)]
	pub type IsStorageOperationsEnabled<T: Config> = StorageValue<_, bool, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn is_purchase_plan_enabled)]
	pub type IsPurchasePlanEnabled<T: Config> = StorageValue<_, bool, ValueQuery, GetDefault>;

	#[pallet::type_value]
	pub fn DefaultReferralCommissionRateBps() -> u32 {
		500
	}

	/// Referrer commission rate in basis points, applied to the credits
	/// actually charged at subscription purchase and on recurring monthly
	/// charges. Paid in native tokens from the bank. Independent of the
	/// buyer-side purchase discount, which stays a fixed 5% in credits.
	#[pallet::storage]
	#[pallet::getter(fn referral_commission_rate_bps)]
	pub type ReferralCommissionRateBps<T: Config> =
		StorageValue<_, u32, ValueQuery, DefaultReferralCommissionRateBps>;

	/// Bank balance (tokens) the referral payout must leave untouched, on top
	/// of the backing/refund reserves. Root-settable brake so commission
	/// volume can never run the shared bank dry: referral is a bonus, and
	/// credit-only deposits add nothing to the bank while purchases made with
	/// them still draw commissions from it — without a floor, sustained
	/// referred usage drains the pot miner payments also depend on. The floor
	/// binds only referral commissions, never miner settlement.
	#[pallet::storage]
	#[pallet::getter(fn referral_bank_floor)]
	pub type ReferralBankFloor<T: Config> = StorageValue<_, u128, ValueQuery>;

	/// Commission earned on hourly pay-as-you-go charges but not yet paid out,
	/// per referrer, in credits.
	///
	/// Hourly billing earns a referrer the configured commission rate
	/// (`ReferralCommissionRateBps`, root-settable) on one hour of per-GB
	/// charges, which is dust, so the hourly path only ever writes here — it
	/// never touches the bank. `sweep_referral_commissions` drains this map on
	/// its own schedule. Not to be confused with the buyer's 5% referral
	/// discount, which is a separate purchase-only constant.
	///
	/// This is a running balance, not a debt: it is paid down as far as the
	/// bank's headroom above `ReferralBankFloor` allows, and whatever the bank
	/// cannot cover simply stays here for the next sweep. Nothing in the
	/// billing flow ever waits on it.
	#[pallet::storage]
	#[pallet::getter(fn accrued_referral_commission)]
	pub type AccruedReferralCommission<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, u128, ValueQuery>;

	/// Raw storage key the next commission sweep resumes from.
	///
	/// A sweep pays at most `MaxReferralPayoutsPerSweep` referrers so its
	/// weight stays bounded no matter how many referrers are owed. The cursor
	/// makes successive sweeps round-robin through the map instead of
	/// repeatedly retrying the same first N keys and starving the tail.
	/// `None` means "start from the beginning".
	#[pallet::storage]
	pub type ReferralPayoutCursor<T: Config> = StorageValue<_, Vec<u8>, OptionQuery>;

	/// Tracks the last block a user cancelled any subscription, to enforce resubscribe cooldowns.
	#[pallet::storage]
	#[pallet::getter(fn last_subscription_cancelled_at)]
	pub type LastSubscriptionCancelledAt<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, BlockNumberFor<T>, OptionQuery>;

	// Next batch ID
	#[pallet::storage]
	#[pallet::getter(fn next_batch_id)]
	pub type NextBatchId<T: Config> = StorageValue<_, u64, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn cdn_locations)]
	pub(super) type CdnLocations<T: Config> =
		StorageMap<_, Blake2_128Concat, u32, CdnLocation, OptionQuery>;

	#[pallet::storage]
	#[pallet::getter(fn next_subscription_id)]
	pub(super) type NextSubscriptionId<T: Config> = StorageValue<_, SubscriptionId, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn point_transactions)]
	pub(super) type PointTransactions<T: Config> = StorageDoubleMap<
		_,
		Blake2_128Concat,
		T::AccountId,
		Blake2_128Concat,
		u32,
		PointTransaction<T>,
		OptionQuery,
	>;

	#[pallet::storage]
	#[pallet::getter(fn next_transaction_id)]
	pub(super) type NextTransactionId<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn backup_enabled_users)]
	pub(super) type BackupEnabledUsers<T: Config> = StorageValue<_, Vec<T::AccountId>, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn backup_delete_requests)]
	pub(super) type BackupDeleteRequests<T: Config> =
		StorageValue<_, Vec<T::AccountId>, ValueQuery>;

	// Add storage item for specific miner request fee
	#[pallet::storage]
	#[pallet::getter(fn specific_miner_request_fee)]
	pub type SpecificMinerRequestFee<T: Config> = StorageValue<_, BalanceOf<T>, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn storage_price_per_miner)]
	pub(super) type StoragePricePerMiner<T: Config> = StorageValue<_, u128, ValueQuery>;

	/// Total Drive-backed file bytes reported for a user (validator metric).
	#[pallet::storage]
	#[pallet::getter(fn user_total_drive_files_size)]
	pub type UserTotalDriveFilesSize<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, u128>;

	/// Total Drive-backed file count reported for a user (validator metric).
	#[pallet::storage]
	#[pallet::getter(fn user_total_drive_files_count)]
	pub type UserTotalDriveFilesCount<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, u128>;

	/// Total S3-backed file bytes reported for a user (validator metric).
	#[pallet::storage]
	#[pallet::getter(fn user_total_s3_files_size)]
	pub type UserTotalS3FilesSize<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, u128>;

	/// Total S3-backed file count reported for a user (validator metric).
	#[pallet::storage]
	#[pallet::getter(fn user_total_s3_files_count)]
	pub type UserTotalS3FilesCount<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, u128>;

	/// Account allowed to call `cancel_user_subscription`.
	#[pallet::storage]
	#[pallet::getter(fn subscription_canceller)]
	pub type SubscriptionCanceller<T: Config> = StorageValue<_, Option<T::AccountId>, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn whitelisted_cancellers)]
	pub type WhitelistedCallers<T: Config> = StorageValue<_, Vec<T::AccountId>, ValueQuery>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// CDN location added
		CdnLocationAdded {
			id: u32,
		},
		/// Auto-renewal status updated
		AutoRenewalUpdated {
			who: T::AccountId,
			subscription_id: SubscriptionId,
			enabled: bool,
		},
		SubscriptionTransferred {
			from: T::AccountId,
			to: T::AccountId,
			subscription_id: SubscriptionId,
		},
		TokensBurned {
			amount: BalanceOf<T>,
		},
		PackageSuspensionSet(T::Hash, bool),
		StoragePlanPriceUpdated {
			plan_id: T::Hash,
			new_price_per_gb: u32,
		},
		ComputePlanPriceUpdated {
			plan_id: T::Hash,
			new_price_per_block: u32,
		},
		PointTransactionRecorded {
			who: T::AccountId,
			transaction_type: NativeTransactionType,
			amount: Points,
		},
		PlanPurchased {
			caller: T::AccountId,
			owner: T::AccountId,
			plan_id: T::Hash,
			location_id: Option<u32>,
			selected_image_name: Option<Vec<u8>>,
			cloud_init_cid: Option<Vec<u8>>,
		},
		PricePerGbUpdated {
			price: u128,
		},
		PricePerBandwidthUpdated {
			price: u128,
		},
		StorageSubscriptionCancelled {
			who: T::AccountId,
		},
		ComputeSubscriptionCancelled {
			who: T::AccountId,
		},
		BackupEnabled {
			caller: T::AccountId,
			account: T::AccountId,
		},
		BackupDisabled {
			caller: T::AccountId,
			account: T::AccountId,
		},
		OSDiskImageUrlSet {
			os_name: Vec<u8>,
			url: Vec<u8>,
		},
		PlanPriceUpdated(T::Hash, u128),
		/// Specific miner request fee updated
		SpecificMinerRequestFeeUpdated {
			fee: BalanceOf<T>,
		},
		BatchDeposited {
			owner: T::AccountId,
			batch_id: u64,
		},
		CreditsConsumed {
			owner: T::AccountId,
			credits: u128,
		},
		/// Monthly subscription charge could not be collected; subscriptions were deactivated.
		SubscriptionChargeFailed {
			who: T::AccountId,
			required_credits: u128,
			available_credits: u128,
		},
		StorageOperationsStatusChanged {
			enabled: bool,
		},
		/// Purchase plan status was changed
		PurchasePlanStatusChanged {
			enabled: bool,
		},
		StoragePricePerMinerUpdated {
			price: u128,
		},
		/// Per-GB storage charging failed after passing the FreeCredits guard.
		PerGbChargeFailed {
			who: T::AccountId,
			charge_amount: u128,
			available_credits: u128,
		},
		/// Referral commission released from the bank in native tokens.
		///
		/// `paid_tokens < requested_credits` means the bank could only cover part
		/// of the commission. What happens to the shortfall depends on which path
		/// emitted this, and the event alone does not say which:
		/// - purchase and monthly renewal drop it — it is gone, never owed;
		/// - the hourly sweep keeps it in `AccruedReferralCommission` and retries
		///   on the next sweep, so a later `ReferralCommissionPaid` for the same
		///   referrer can be the remainder of *this* obligation rather than new
		///   earnings.
		///
		/// An indexer summing `paid_tokens` per referrer is therefore correct for
		/// total tokens received, and must not treat `requested_credits` as
		/// earnings — on the sweep path the same credits can be requested across
		/// several events before they are fully paid.
		ReferralCommissionPaid {
			referrer: T::AccountId,
			requested_credits: u128,
			paid_tokens: u128,
		},
		/// Referral commission could not be paid at all (e.g. bank requester not
		/// whitelisted, or the bank is at its floor). Billing itself is always
		/// unaffected.
		///
		/// As with `ReferralCommissionPaid`, the fate of the commission depends on
		/// the path: purchase and monthly renewal drop it, while the hourly sweep
		/// leaves the full amount accrued and retries next sweep — so a repeated
		/// `ReferralCommissionSkipped` for one referrer is the *same* obligation
		/// being retried, not a new one each time.
		ReferralCommissionSkipped {
			referrer: T::AccountId,
			requested_credits: u128,
		},
		/// Root changed the referral commission rate.
		ReferralCommissionRateUpdated {
			rate_bps: u32,
		},
		/// Root changed the bank floor referral commissions must not breach.
		ReferralBankFloorUpdated {
			floor: u128,
		},
		/// Hourly pay-as-you-go commission credited to a referrer's accrued
		/// balance. `accrued_credits` is the balance after adding
		/// `added_credits`; it is paid out by the next commission sweep.
		ReferralCommissionAccrued {
			referrer: T::AccountId,
			added_credits: u128,
			accrued_credits: u128,
		},
		/// User Drive + S3 usage metrics were updated by a validator.
		UserBackendFilesUpdated {
			user: T::AccountId,
			drive_size: u128,
			drive_count: u128,
			s3_size: u128,
			s3_count: u128,
		},
		/// A referral code was created for `user` by a whitelisted caller.
		ReferralCodeCreatedFor {
			user: T::AccountId,
		},
		/// Summary of a `create_referral_codes_for` batch. `skipped` counts users
		/// that already had a code and were left untouched.
		ReferralCodesCreatedFor {
			created: u32,
			skipped: u32,
		},
		WhitelistedCallerAdded {
			account: T::AccountId,
		},
		WhitelistedCallerRemoved {
			account: T::AccountId,
		},
		/// A user's active storage subscription was moved onto a different plan
		/// by a whitelisted caller, in one dispatch.
		///
		/// `charged_credits` and `refunded_credits` are the two halves of a single
		/// net credit movement, so at most one of them is ever non-zero.
		StoragePlanChanged {
			user: T::AccountId,
			old_plan: T::Hash,
			new_plan: T::Hash,
			subscription_id: SubscriptionId,
			charged_credits: u128,
			refunded_credits: u128,
		},
		/// The S3 counterpart of `StoragePlanChanged`. Emitted as its own
		/// variant rather than adding a flavour field, so existing indexers
		/// keep decoding Drive changes unchanged and never mistake an S3
		/// change for one.
		S3PlanChanged {
			user: T::AccountId,
			old_plan: T::Hash,
			new_plan: T::Hash,
			subscription_id: SubscriptionId,
			charged_credits: u128,
			refunded_credits: u128,
		},
	}

	#[pallet::error]
	pub enum Error<T> {
		NoneValue,
		NotSubscriptionOwner,
		SubscriptionNotFound,
		TooManySharedUsers,
		TooManyActiveSubscriptions,
		PlanAlreadyExists,
		InsufficientPermissions,
		CannotTransferToSelf,
		RecipientTooManySubscriptions,
		CannotModifyOwnerPermissions,
		CannotTransferInactiveSubscription,
		AlreadyHasAccess,
		NoExistingAccess,
		NotAuthorized,
		InsufficientBalance,
		PackageNotFound,
		SubscriptionNotActive,
		InvalidSubscriptionType,
		StorageLimitExceeded,
		StorageRequestNotFound,
		PlanNotFound,
		InvalidPlanType,
		/// The account already holds an active subscription of the storage
		/// flavour being purchased — one Drive plan and one S3 plan at a time.
		AlreadyHasActiveSubscription,
		PlanSuspended,
		InsufficientFreeCredits,
		LocationNotFound,
		InvalidPlanLimits,
		NodeTypeDisabled,
		InvalidStorageReduction,
		InvalidSubscriptionUsage,
		ComputeResourceExceeded,
		NoActiveSubscription,
		BackupAlreadyEnabled,
		InvalidImageSelection,
		NodeNotRegistered,
		InvalidNodeType,
		/// The plan does not match the user's active subscription
		InvalidPlanForSubscription,
		InvalidPlanConfiguration,
		InvalidOSDiskImageUrl,
		/// No subscription found for the given user
		NoSubscriptionFound,
		StorageOperationsDisabled,
		PlanOperationDisabled,
		TooManyRequests,
		OperationNotAllowed,
		InvalidInput,
		UserNotFound,
		ResubscribeCooldownActive,
		SubscriptionCancellationNotAuthorized,
		WhitelistedCallerNotAuthorized,
		TooManyUpdates,
	}

	#[pallet::storage]
	#[pallet::getter(fn user_requests_count)]
	pub type UserRequestsCount<T: Config> =
		StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn sudo_key)]
	pub type SudoKey<T: Config> = StorageValue<_, Option<T::AccountId>, ValueQuery>;

	/// Alpha backing sitting in the bank that has not yet been earned out of it.
	/// Incremented when deposit backing reaches the bank, decremented when the
	/// credits it backs are consumed (the revenue is earned and becomes miner
	/// budget) or charged back (it is refunded to sudo). Refund delivery is
	/// tracked separately in [`PendingSudoRefunds`], so a bank shortfall never
	/// desyncs this ledger.
	/// Ops invariant to alert on:
	/// bank balance >= TotalUndistributedBacking + PendingSudoRefunds + expected miner due.
	#[pallet::storage]
	pub type TotalUndistributedBacking<T: Config> = StorageValue<_, u128, ValueQuery>;

	/// Per-batch alpha whose backing never reached the bank (no sudo key, or
	/// an underfunded sudo account, at deposit time). Releases and chargebacks
	/// of this alpha must not reduce [`TotalUndistributedBacking`] — that
	/// backing was never added to it.
	#[pallet::storage]
	pub type UnbackedBatchAlpha<T: Config> = StorageMap<_, Blake2_128Concat, u64, u128, ValueQuery>;

	/// Chargeback refunds the bank could not deliver (no sudo key, or bank
	/// shortfall) — still sitting in the bank, owed to the current sudo key,
	/// retried at the next chargeback. Excluded from the miner payout budget
	/// alongside [`TotalUndistributedBacking`].
	#[pallet::storage]
	pub type PendingSudoRefunds<T: Config> = StorageValue<_, u128, ValueQuery>;

	/// Unix-day marker (unix_ms / 86_400_000) of the last time we ran the monthly subscription charge.
	///
	/// Superseded by [`DueDayCursor`] and no longer read or written. A capped
	/// drain takes many ticks to finish a day, so this once-per-day latch would
	/// fire on the first partial tick and suppress the rest of that day's queue.
	/// The key is left in place deliberately: an orphaned `StorageValue` costs
	/// nothing and removing it would need a migration to buy nothing.
	#[pallet::storage]
	#[pallet::getter(fn last_monthly_subscription_charge_day)]
	pub type LastMonthlySubscriptionChargeDay<T: Config> = StorageValue<_, u32, ValueQuery>;

	/// Accounts with at least one active subscription due on a given unix day.
	///
	/// The whole point of date-to-date billing is that somebody is due every
	/// day, which turns a monthly full-map sweep into a daily one. This index
	/// is what keeps the work proportional to who is *actually* due instead of
	/// to how many subscriptions exist.
	///
	/// **Keyed by account, not by subscription.** Charging is deliberately
	/// ordered: `charge_due_subscriptions_individually` sorts an account's due
	/// subscriptions by ascending id so a short-funded user deterministically
	/// keeps the oldest. `iter_prefix` returns hash order, so a
	/// subscription-keyed index would scatter one account's plans across the
	/// drain — possibly across a cap boundary into a different block with a
	/// different credit balance — and make which plan survives effectively
	/// random. It would also fragment the referral commission, which is
	/// computed once per account on the total it actually paid.
	///
	/// An account whose plans fall due on different days holds one entry under
	/// each of those days.
	///
	/// Not a `StorageMap<u32, BoundedVec<_>>`: a bucket-per-day vector decodes,
	/// mutates and re-encodes the entire day's list on every touch, and its
	/// bound would become a hard ceiling on how many people may share a
	/// renewal date. The double map reads only what it consumes.
	#[pallet::storage]
	pub type DueAccounts<T: Config> =
		StorageDoubleMap<_, Blake2_128Concat, u32, Blake2_128Concat, T::AccountId, (), OptionQuery>;

	/// Oldest unix day whose `DueAccounts` prefix has not yet been fully drained.
	///
	/// Reading only *today's* prefix would silently drop anyone due on a day
	/// that did not finish draining or that the chain was down for. The cursor
	/// advances only once its day is empty and never past today, which is what
	/// `max_catchup_months` is doing in the current code and must survive.
	///
	/// `None` means "not yet initialised" — set to the first of the current
	/// month when the backfill starts.
	#[pallet::storage]
	pub type DueDayCursor<T: Config> = StorageValue<_, u32, OptionQuery>;

	/// Billing anchor (day-of-month, 1..=31) for subscriptions whose anchor
	/// cannot be derived from their due date.
	///
	/// **Sparse on purpose.** For any anchor from 1 to 28 the day-of-month of
	/// `next_charge_unix_day` *is* the anchor, in every month of the year, so
	/// no entry is needed. Derivation only fails once a value has been clamped:
	/// a 31st subscriber pushed to Feb 28 has lost the 31. Clamping can only
	/// ever produce 28, 29 or 30, so an entry is written only when the anchor
	/// exceeds 28 and read only when the derived day is 28, 29 or 30.
	///
	/// This is why there is no `anchor_day` field on `UserPlanSubscription`:
	/// adding one would change the type of a live map holding every
	/// subscription on the chain. `SubscriptionId` is a monotonic `u32` and a
	/// stable standalone key, so this map needs nothing from that struct.
	///
	/// Starts completely empty — every existing subscription is anchored to the
	/// 1st, and 1 derives correctly with no entry at all.
	#[pallet::storage]
	pub type SubscriptionAnchorDay<T: Config> =
		StorageMap<_, Blake2_128Concat, SubscriptionId, u8, OptionQuery>;

	/// Raw storage key the one-time due-index backfill resumes from.
	///
	/// Populating the index cannot be a migration: `pallet-migrations` is not
	/// configured in this runtime, so it would have to be a single-block
	/// `OnRuntimeUpgrade` iterating every subscription — the same unbounded
	/// work this change exists to remove, relocated into the one block where
	/// exceeding the budget breaks the upgrade itself.
	#[pallet::storage]
	pub type BackfillCursor<T: Config> = StorageValue<_, Vec<u8>, OptionQuery>;

	/// Whether the due-index backfill has finished.
	///
	/// Until it is set the renewal sweep falls back to the pre-index full scan,
	/// because an account in the untouched tail has no index entry and would
	/// otherwise go uncharged. Retire the fallback one release after this ships.
	#[pallet::storage]
	pub type BackfillDone<T: Config> = StorageValue<_, bool, ValueQuery>;

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Set the `is_suspended` field for a specific package.
		#[pallet::call_index(3)]
		#[pallet::weight((0, Pays::No))]
		pub fn set_package_suspension(
			origin: OriginFor<T>,
			plan_id: T::Hash,
			is_suspended: bool,
		) -> DispatchResult {
			// Ensure the caller has sudo/root privileges
			ensure_root(origin)?;

			// Check if the package exists
			Plans::<T>::try_mutate_exists(plan_id.clone(), |package| -> DispatchResult {
				if let Some(ref mut pkg) = package {
					// Update the `is_suspended` field
					pkg.is_suspended = is_suspended;
				} else {
					// If the package does not exist, throw an error
					return Err(Error::<T>::PackageNotFound.into());
				}
				Ok(())
			})?;

			// Emit an event
			Self::deposit_event(Event::PackageSuspensionSet(plan_id, is_suspended));
			Ok(())
		}

		/// Sudo function to add a new plan.
		#[pallet::call_index(6)]
		#[pallet::weight((10_000, Pays::No))]
		pub fn add_new_plan(
			origin: OriginFor<T>,
			plan_name: Vec<u8>,
			plan_description: Vec<u8>,
			plan_technical_description: Vec<u8>,
			price: u128,
			is_storage_plan: bool,
			is_s3_plan: bool,
			storage_limit: Option<u128>,
		) -> DispatchResult {
			// Ensure the caller is sudo
			ensure_root(origin)?;

			// Drive and S3 are separate plan kinds, each with its own per-account
			// slot and its own hourly-billing exemption. A plan claiming both
			// would occupy two slots at once and exempt bytes it was never bought
			// for, so the combination is refused. Setting neither is a compute
			// plan, which is what every non-storage plan already is.
			ensure!(!(is_storage_plan && is_s3_plan), Error::<T>::InvalidPlanType);

			// Generate a unique ID for the plan (you can use a counter or a random hash)
			let plan_id = T::Hashing::hash_of(&plan_name); // Example way to generate a unique ID
			ensure!(!Plans::<T>::contains_key(&plan_id), Error::<T>::PlanAlreadyExists);

			// Create the plan object
			let new_plan = Plan {
				id: plan_id.clone(),
				plan_name: plan_name.clone(),
				plan_description,
				plan_technical_description,
				is_suspended: false,
				price,
				is_storage_plan,
				is_s3_plan,
				storage_limit,
			};

			// Insert the new plan into storage
			Plans::<T>::insert(plan_id.clone(), new_plan);

			Ok(())
		}

		/// Purchase one or more plans using points
		#[pallet::call_index(7)]
		#[pallet::weight((0, Pays::No))]
		pub fn purchase_plan(
			origin: OriginFor<T>,
			owner: T::AccountId,
			plan_ids: Vec<T::Hash>,
			location_ids: Option<Vec<Option<u32>>>,
			selected_image_names: Option<Vec<Option<Vec<u8>>>>,
			cloud_init_cids: Option<Vec<Option<Vec<u8>>>>,
			pay_upfront: Option<u128>,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			// Check if the caller is a whitelisted caller
			let allowed = WhitelistedCallers::<T>::get();
			ensure!(allowed.contains(&who), Error::<T>::WhitelistedCallerNotAuthorized);

			if let Some(n) = pay_upfront {
				ensure!(n >= 1 && n <= 24, Error::<T>::InvalidInput);
			}

			// Rate limit: maximum storage requests per block per user
			let max_requests_per_block = T::MaxRequestsPerBlock::get();
			let user_requests_count = UserRequestsCount::<T>::get(&owner);
			ensure!(
				user_requests_count + (plan_ids.len() as u32) <= max_requests_per_block,
				Error::<T>::TooManyRequests
			);
			UserRequestsCount::<T>::insert(&owner, user_requests_count + (plan_ids.len() as u32));

			if let Some(ref xs) = selected_image_names {
				ensure!(xs.len() == plan_ids.len(), Error::<T>::InvalidInput);
			}
			if let Some(ref xs) = location_ids {
				ensure!(xs.len() == plan_ids.len(), Error::<T>::InvalidInput);
			}
			if let Some(ref xs) = cloud_init_cids {
				ensure!(xs.len() == plan_ids.len(), Error::<T>::InvalidInput);
			}

			// Initialize default values for optional parameters
			let selected_image_names =
				selected_image_names.unwrap_or_else(|| vec![None; plan_ids.len()]);
			let location_ids = location_ids.unwrap_or_else(|| vec![None; plan_ids.len()]);
			let cloud_init_cids = cloud_init_cids.unwrap_or_else(|| vec![None; plan_ids.len()]);

			// Track successful purchases
			let mut successful_purchases = Vec::new();

			// Process each plan purchase
			for (i, &plan_id) in plan_ids.iter().enumerate() {
				// Get plan details
				let plan = Plans::<T>::get(&plan_id).ok_or(Error::<T>::PlanNotFound)?;

				// Check if plan is suspended
				ensure!(!plan.is_suspended, Error::<T>::PlanSuspended);

				// Process the purchase based on plan type
				if plan.is_any_storage() {
					// Handle storage plan purchase
					Self::do_purchase_storage_plan(owner.clone(), plan_id, pay_upfront)?;
				} else {
					// Compute plans: image is optional; omit or pass None/empty for no image.
					let image_name =
						Self::normalize_image_selection(selected_image_names[i].clone());
					Self::do_purchase_compute_plan(
						owner.clone(),
						plan_id,
						location_ids[i],
						image_name.clone(),
						cloud_init_cids[i].clone(),
						pay_upfront,
					)?;
				};

				successful_purchases.push(plan_id);
				// Emit event for successful purchase
				let selected_image_name = if plan.is_any_storage() {
					None
				} else {
					Self::normalize_image_selection(selected_image_names[i].clone())
				};
				Self::deposit_event(Event::PlanPurchased {
					caller: owner.clone(),
					owner: owner.clone(),
					plan_id,
					location_id: location_ids[i],
					selected_image_name,
					cloud_init_cid: cloud_init_cids[i].clone(),
				});
			}

			// If we had any successful purchases, we consider the call successful
			Ok(())
		}

		/// Sudo function to set the price per GB for storage
		#[pallet::call_index(8)]
		#[pallet::weight((10_000, Pays::No))]
		pub fn set_price_per_gb(origin: OriginFor<T>, price: u128) -> DispatchResult {
			// Ensure the caller is sudo
			ensure_root(origin)?;

			// Set the price per GB
			PricePerGbs::<T>::put(price);

			// Emit an event for the price update
			Self::deposit_event(Event::PricePerGbUpdated { price });

			Ok(())
		}

		/// Sudo function to set the price per GB for storage
		#[pallet::call_index(13)]
		#[pallet::weight((10_000, Pays::No))]
		pub fn set_bandwidth_price(origin: OriginFor<T>, price: u128) -> DispatchResult {
			// Ensure the caller is sudo
			ensure_root(origin)?;

			// Set the price per GB
			PricePerBandwidth::<T>::put(price);

			// Emit an event for the price update
			Self::deposit_event(Event::PricePerBandwidthUpdated { price });

			Ok(())
		}

		/// Retry any pending sudo refunds stuck in the bank.
		/// If chargebacks have stopped, remainder can sit forever. This extrinsic
		/// allows manual retries when the bank or sudo account is ready.
		/// Correctly walled from miner settlement; only affects sudo account refunds.
		#[pallet::call_index(10)]
		#[pallet::weight((10_000, Pays::No))]
		pub fn retry_pending_sudo_refunds(origin: OriginFor<T>) -> DispatchResult {
			ensure_root(origin)?;

			let pending = PendingSudoRefunds::<T>::take();
			if pending == 0 {
				return Ok(());
			}

			let requester = Self::account_id();
			if let Some(sudo_account) = Self::sudo_key() {
				match pallet_hippocampus::Pallet::<T>::request_payment(
					&requester,
					&sudo_account,
					pending.saturated_into(),
				) {
					Ok(refunded) => {
						let refunded_u128: u128 = refunded.saturated_into();
						if refunded_u128 < pending {
							let remainder = pending.saturating_sub(refunded_u128);
							log::warn!(
								target: "runtime::marketplace",
								"retry_pending_sudo_refunds: refunded {} of {}, remainder {} kept pending",
								refunded_u128,
								pending,
								remainder
							);
							PendingSudoRefunds::<T>::put(remainder);
						} else {
							log::info!(
								target: "runtime::marketplace",
								"retry_pending_sudo_refunds: successfully refunded {} pending",
								refunded_u128
							);
						}
					},
					Err(e) => {
						log::warn!(
							target: "runtime::marketplace",
							"retry_pending_sudo_refunds: bank rejected refund of {}: {:?}",
							pending,
							e
						);
						PendingSudoRefunds::<T>::put(pending);
					},
				}
			} else {
				log::warn!(
					target: "runtime::marketplace",
					"retry_pending_sudo_refunds: no sudo key, {} kept pending",
					pending
				);
				PendingSudoRefunds::<T>::put(pending);
			}

			Ok(())
		}

		// Extrinsic to set OS Disk Image URL
		#[pallet::call_index(9)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn set_os_disk_image_url(
			origin: OriginFor<T>,
			os_name: Vec<u8>,
			url: Vec<u8>,
			name: Vec<u8>,
			description: Vec<u8>,
		) -> DispatchResultWithPostInfo {
			// Ensure only sudo can set the URL
			ensure_root(origin)?;

			// Validate the URL (basic check)
			ensure!(!url.is_empty(), Error::<T>::InvalidOSDiskImageUrl);

			let os_details = ImageDetails { url, name, description };

			// Store the URL for the specified OS
			OSDiskImageUrls::<T>::insert(os_name.clone(), os_details.clone());

			// Emit an event
			Self::deposit_event(Event::OSDiskImageUrlSet { os_name, url: os_details.url });

			Ok(().into())
		}

		/// Set the specific miner request fee
		#[pallet::call_index(12)]
		#[pallet::weight((0, Pays::No))]
		pub fn set_specific_miner_request_fee(
			origin: OriginFor<T>,
			fee: BalanceOf<T>,
		) -> DispatchResult {
			// Ensure the caller is an authority
			let authority = ensure_signed(origin)?;

			// Rate limit: maximum storage requests per block per user
			let max_requests_per_block = T::MaxRequestsPerBlock::get();
			let user_requests_count = UserRequestsCount::<T>::get(&authority);
			ensure!(
				user_requests_count.saturating_add(1) <= max_requests_per_block,
				Error::<T>::TooManyRequests
			);

			CreditsPallet::<T>::ensure_is_authority(&authority)?;

			// Update the SpecificMinerRequestFee storage value
			<SpecificMinerRequestFee<T>>::put(fee);

			// Deposit an event to notify about the fee update
			Self::deposit_event(Event::<T>::SpecificMinerRequestFeeUpdated { fee });

			Ok(())
		}

		#[pallet::call_index(14)]
		#[pallet::weight((0, Pays::No))]
		pub fn deposit(
			origin: OriginFor<T>,
			account: T::AccountId,
			credit_amount: u128,
			alpha_amount: u128,
			freeze_for_chargeback: bool,
			code: Option<Vec<u8>>,
		) -> DispatchResult {
			let authority = ensure_signed(origin)?;
			CreditsPallet::<T>::ensure_is_authority(&authority)?;

			// Call the existing deposit function
			Self::do_deposit(account, credit_amount, alpha_amount, freeze_for_chargeback, code)?;
			Ok(())
		}

		#[pallet::call_index(15)]
		#[pallet::weight((0, Pays::No))]
		pub fn chargeback(origin: OriginFor<T>, batch_id: u64) -> DispatchResult {
			// Ensure the caller is a signed origin (admin check)
			ensure_root(origin)?;

			// Call the existing handle_chargeback function
			Self::handle_chargeback(batch_id)
		}

		#[pallet::call_index(16)]
		#[pallet::weight((0, Pays::No))]
		pub fn set_sudo_key(origin: OriginFor<T>, new_sudo_key: T::AccountId) -> DispatchResult {
			// Ensure that the caller is the sudo account
			ensure_root(origin)?;

			// Set the new sudo key in storage
			SudoKey::<T>::put(Some(new_sudo_key));

			Ok(())
		}

		#[pallet::call_index(17)]
		#[pallet::weight((0, Pays::No))]
		pub fn sudo_set_storage_operations(origin: OriginFor<T>, enabled: bool) -> DispatchResult {
			// Ensure the origin is a sudo account
			ensure_root(origin)?;

			// Set the storage operations flag
			IsStorageOperationsEnabled::<T>::put(enabled);

			// Emit an event (optional, but recommended)
			Self::deposit_event(Event::StorageOperationsStatusChanged { enabled });

			Ok(())
		}

		/// Enable or disable purchase plan functionality
		///
		/// Can only be called by sudo
		#[pallet::call_index(18)]
		#[pallet::weight((0, Pays::No))]
		pub fn sudo_set_purchase_plan_enabled(
			origin: OriginFor<T>,
			enabled: bool,
		) -> DispatchResult {
			// Ensure the origin is a sudo account
			ensure_root(origin)?;

			// Set the purchase plan flag
			IsPurchasePlanEnabled::<T>::put(enabled);

			// Emit an event
			Self::deposit_event(Event::PurchasePlanStatusChanged { enabled });

			Ok(())
		}

		/// Root sets the dedicated subscription canceller address.
		#[pallet::call_index(19)]
		#[pallet::weight((0, Pays::No))]
		pub fn sudo_set_whitelist_canceller(
			origin: OriginFor<T>,
			account: T::AccountId,
		) -> DispatchResult {
			ensure_root(origin)?;
			let mut allowed_accounts = WhitelistedCallers::<T>::get();
			// In sudo_set_whitelist_canceller function
			if !allowed_accounts.contains(&account) {
				allowed_accounts.push(account.clone());
				Self::deposit_event(Event::WhitelistedCallerAdded { account });
			}
			WhitelistedCallers::<T>::put(allowed_accounts);
			Ok(())
		}

		/// Root removes an account from the subscription canceller list.
		#[pallet::call_index(20)]
		#[pallet::weight((0, Pays::No))]
		pub fn sudo_remove_whitelist_canceller(
			origin: OriginFor<T>,
			account: T::AccountId,
		) -> DispatchResult {
			ensure_root(origin)?;
			let mut allowed_accounts = WhitelistedCallers::<T>::get();
			allowed_accounts.retain(|x| x != &account);
			WhitelistedCallers::<T>::put(allowed_accounts);
			// In sudo_remove_whitelist_canceller function
			Self::deposit_event(Event::WhitelistedCallerRemoved { account });
			Ok(())
		}

		/// Root sets the dedicated subscription canceller address.
		#[pallet::call_index(21)]
		#[pallet::weight((0, Pays::No))]
		pub fn sudo_set_subscription_canceller(
			origin: OriginFor<T>,
			account: Option<T::AccountId>,
		) -> DispatchResult {
			ensure_root(origin)?;
			SubscriptionCanceller::<T>::put(account);
			Ok(())
		}

		/// Cancel a user's subscription (restricted to whitelisted callers).
		///
		/// - If `subscription_id` is `Some(id)`: cancels (marks inactive) that specific subscription
		///   (storage or compute), refunds unused prepaid months.
		/// - If `subscription_id` is `None`: deletes all storage subscriptions (legacy behavior),
		///   refunds unused prepaid months.
		#[pallet::call_index(22)]
		#[pallet::weight((0, Pays::No))]
		pub fn cancel_user_subscription(
			origin: OriginFor<T>,
			user: T::AccountId,
			subscription_id: Option<SubscriptionId>,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			let allowed = WhitelistedCallers::<T>::get();
			ensure!(allowed.contains(&who), Error::<T>::WhitelistedCallerNotAuthorized);

			match subscription_id {
				Some(id) => Self::do_cancel_subscription_by_id(&user, id),
				None => Self::do_delete_storage_subscription_with_refund(&user),
			}
		}

		/// Move a user's active storage subscription onto a different storage plan
		/// — upgrade or downgrade — in one dispatch (restricted to whitelisted
		/// callers, same ACL as `purchase_plan` / `cancel_user_subscription`).
		///
		/// This exists because cancel + re-purchase is not a plan change:
		/// - it opens a window in which the user holds no storage entitlement,
		/// - the re-purchase re-checks the `MinSubscriptionBlocks` resubscribe
		///   cooldown against `LastSubscriptionCancelledAt`, forcing the caller to
		///   sequence the two calls across blocks and retry on
		///   `ResubscribeCooldownActive`,
		/// - and it splits the refund and the new charge into two credit movements,
		///   which double-charges the month the user has already paid for.
		///
		/// So this call never sets `LastSubscriptionCancelledAt` (a change is not a
		/// cancellation), never lets the subscription slot go empty, and settles the
		/// whole swap as a single net credit movement. Referral attribution
		/// (`ReferredUsers`) is left exactly as it was; the buyer discount and the
		/// referrer commission apply to the new plan as they do at purchase.
		///
		/// This call moves the **Drive** subscription only, and `new_plan_id` must
		/// itself be a Drive plan. An account's S3 subscription is a separate slot
		/// with its own extrinsic, [`Pallet::change_s3_plan`], and is never touched
		/// here.
		///
		/// `selected_image_name` / `location_id` / `cloud_init_cid` mirror
		/// `purchase_plan`'s per-plan inputs so the backend can pass the same shape
		/// on both calls. Storage plans are provisioned from the plan alone, so —
		/// exactly as in `purchase_plan`'s storage branch — they are
		/// accepted and not written to the subscription.
		#[pallet::call_index(30)]
		#[pallet::weight((0, Pays::No))]
		pub fn change_storage_plan(
			origin: OriginFor<T>,
			user: T::AccountId,
			new_plan_id: T::Hash,
			selected_image_name: Option<Vec<u8>>,
			location_id: Option<u32>,
			cloud_init_cid: Option<Vec<u8>>,
		) -> DispatchResult {
			Self::dispatch_plan_change(
				origin,
				user,
				StorageFlavour::Drive,
				new_plan_id,
				selected_image_name,
				location_id,
				cloud_init_cid,
			)
		}

		/// Move a user's active **S3** subscription onto a different S3 plan.
		///
		/// The S3 counterpart of [`Pallet::change_storage_plan`]: same whitelisted
		/// caller, same per-block rate limit, and the same single-net-movement
		/// settlement — refund of unused prepaid months and the carry for the
		/// unexpired remainder of the current month are netted against the new
		/// plan's first month, so `FreeCredits` moves once and the month already
		/// paid for is never billed twice.
		///
		/// The two slots are independent: this never touches the account's Drive
		/// subscription, and `new_plan_id` must be an S3 plan. An account with no
		/// active S3 subscription gets `NoActiveSubscription` even when it holds a
		/// Drive one, because there is nothing on this slot to move.
		#[pallet::call_index(31)]
		#[pallet::weight((0, Pays::No))]
		pub fn change_s3_plan(
			origin: OriginFor<T>,
			user: T::AccountId,
			new_plan_id: T::Hash,
			selected_image_name: Option<Vec<u8>>,
			location_id: Option<u32>,
			cloud_init_cid: Option<Vec<u8>>,
		) -> DispatchResult {
			Self::dispatch_plan_change(
				origin,
				user,
				StorageFlavour::S3,
				new_plan_id,
				selected_image_name,
				location_id,
				cloud_init_cid,
			)
		}

		/// Update the total Drive + S3 file size/count for a user.
		/// Callable only by a registered validator (or its proxy), same rules as arion previously.
		#[pallet::call_index(24)]
		#[pallet::weight((100_000, Pays::No))]
		pub fn update_user_file_usage(
			origin: OriginFor<T>,
			account_id: T::AccountId,
			drive_file_size: u128,
			drive_file_count: u128,
			s3_file_size: u128,
			s3_file_count: u128,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;
			let allowed = WhitelistedCallers::<T>::get();
			ensure!(allowed.contains(&who), Error::<T>::WhitelistedCallerNotAuthorized);

			UserTotalDriveFilesSize::<T>::insert(&account_id, drive_file_size);
			UserTotalDriveFilesCount::<T>::insert(&account_id, drive_file_count);
			UserTotalS3FilesSize::<T>::insert(&account_id, s3_file_size);
			UserTotalS3FilesCount::<T>::insert(&account_id, s3_file_count);

			Self::deposit_event(Event::UserBackendFilesUpdated {
				user: account_id,
				drive_size: drive_file_size,
				drive_count: drive_file_count,
				s3_size: s3_file_size,
				s3_count: s3_file_count,
			});

			Ok(())
		}

		/// Batch variant of `update_user_file_usage`.
		///
		/// Updates multiple users in one call; bounded by `MaxUserFileUsageUpdatesPerCall`.
		#[pallet::call_index(25)]
		#[pallet::weight((10_000, Pays::No))]
		pub fn update_users_file_usage(
			origin: OriginFor<T>,
			updates: Vec<UserBackendFileUsageUpdate<T::AccountId>>,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			ensure!(
				(updates.len() as u32) <= T::MaxUserFileUsageUpdatesPerCall::get(),
				Error::<T>::TooManyUpdates
			);

			let allowed = SubscriptionCanceller::<T>::get();
			ensure!(
				allowed.as_ref() == Some(&who),
				Error::<T>::SubscriptionCancellationNotAuthorized
			);

			for u in updates {
				let account_id = u.account_id;
				let drive_file_size = u.drive_file_size;
				let drive_file_count = u.drive_file_count;
				let s3_file_size = u.s3_file_size;
				let s3_file_count = u.s3_file_count;

				UserTotalDriveFilesSize::<T>::insert(&account_id, drive_file_size);
				UserTotalDriveFilesCount::<T>::insert(&account_id, drive_file_count);
				UserTotalS3FilesSize::<T>::insert(&account_id, s3_file_size);
				UserTotalS3FilesCount::<T>::insert(&account_id, s3_file_count);

				Self::deposit_event(Event::UserBackendFilesUpdated {
					user: account_id,
					drive_size: drive_file_size,
					drive_count: drive_file_count,
					s3_size: s3_file_size,
					s3_count: s3_file_count,
				});
			}

			Ok(())
		}

		#[pallet::call_index(26)]
		#[pallet::weight((10_000, Pays::No))]
		pub fn set_storage_price_per_miner(origin: OriginFor<T>, price: u128) -> DispatchResult {
			ensure_root(origin)?;
			StoragePricePerMiner::<T>::put(price);
			Self::deposit_event(Event::StoragePricePerMinerUpdated { price });
			Ok(())
		}

		/// Root sets the referral commission rate in basis points (≤ 10_000).
		///
		/// Applies to future purchase and recurring-charge commissions only;
		/// the buyer-side purchase discount is not affected.
		#[pallet::call_index(27)]
		#[pallet::weight((10_000, Pays::No))]
		pub fn sudo_set_referral_commission_rate(
			origin: OriginFor<T>,
			rate_bps: u32,
		) -> DispatchResult {
			ensure_root(origin)?;
			ensure!(rate_bps <= 10_000, Error::<T>::InvalidInput);
			ReferralCommissionRateBps::<T>::put(rate_bps);
			Self::deposit_event(Event::ReferralCommissionRateUpdated { rate_bps });
			Ok(())
		}

		/// Root sets the bank balance referral commissions must leave
		/// untouched (on top of backing/refund reserves). Commissions that
		/// would breach the floor are reduced or skipped; billing and miner
		/// payments are unaffected.
		#[pallet::call_index(28)]
		#[pallet::weight((10_000, Pays::No))]
		pub fn sudo_set_referral_bank_floor(origin: OriginFor<T>, floor: u128) -> DispatchResult {
			ensure_root(origin)?;
			ReferralBankFloor::<T>::put(floor);
			Self::deposit_event(Event::ReferralBankFloorUpdated { floor });
			Ok(())
		}

		/// Create referral codes on behalf of users. Callable only by a whitelisted caller,
		/// bounded by `MaxReferralCodesPerCall`.
		///
		/// Idempotent: users that already hold a referral code are skipped, not issued a
		/// second one. `pallet_credits::do_change_referral_code` always mints a *new* code
		/// and deliberately never deletes the old one, so calling it for an existing holder
		/// would proliferate codes — and would fail outright with `ReferralCodeCooldown`
		/// inside the per-user cooldown window. Skipping keeps retries and backfill sweeps
		/// safe to repeat.
		#[pallet::call_index(29)]
		#[pallet::weight((10_000, Pays::No))]
		pub fn create_referral_codes_for(
			origin: OriginFor<T>,
			users: Vec<T::AccountId>,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			let allowed = WhitelistedCallers::<T>::get();
			ensure!(allowed.contains(&who), Error::<T>::WhitelistedCallerNotAuthorized);

			ensure!(
				(users.len() as u32) <= T::MaxReferralCodesPerCall::get(),
				Error::<T>::TooManyUpdates
			);

			let mut created: u32 = 0;
			let mut skipped: u32 = 0;

			for user in users {
				// `LastReferralCreationBlock` is written by `do_change_referral_code`, the only
				// path that inserts into `ReferralCodes`, so its presence is an exact
				// "already has a code" test. A user repeated within one batch is also caught
				// here: the first iteration sets the block, later ones skip.
				if CreditsPallet::<T>::last_referral_creation_block(&user).is_some() {
					skipped = skipped.saturating_add(1);
					continue;
				}

				CreditsPallet::<T>::do_change_referral_code(user.clone())?;
				created = created.saturating_add(1);
				Self::deposit_event(Event::ReferralCodeCreatedFor { user });
			}

			Self::deposit_event(Event::ReferralCodesCreatedFor { created, skipped });

			Ok(())
		}
	}

	impl<T: Config> Pallet<T> {
		/// Commission on `charged_credits` at the current rate, in credits.
		/// Exact U256 split — no overflow cliff, part ≤ whole.
		fn referral_commission_credits(charged_credits: u128) -> u128 {
			let rate = BasisPoints::new(u128::from(ReferralCommissionRateBps::<T>::get()));
			split(Credits::new(charged_credits), rate).0.get()
		}

		/// Pay a referral commission in native tokens from the bank, returning
		/// the amount actually released.
		///
		/// Credits → planck is 1:1 numerically (same convention as
		/// pallet-registration). The bank never overdraws: it pays what it can
		/// and never fails the billing flow that earned the commission.
		///
		/// What happens to a shortfall is the *caller's* choice, not this
		/// function's, and the two callers differ:
		/// - purchase and monthly renewal ignore the return value and drop it — a
		///   commission is a bonus there, never a debt;
		/// - the hourly sweep settles `AccruedReferralCommission` by exactly what
		///   was paid, so the remainder survives and is retried next sweep.
		fn try_pay_referral_commission_tokens(
			referrer: &T::AccountId,
			commission_credits: u128,
		) -> u128 {
			if commission_credits == 0 {
				return 0;
			}

			// Compartmentalization wall — same rule the runtime's
			// ArionPayoutSource applies to miner settlement: alpha backing
			// still owed to the ranking/marketplace pots and chargeback
			// refunds still owed to the sudo account are not spendable as
			// commissions, the root-set ReferralBankFloor, and the emission
			// compartments reserved for ranking-based `pay_storage_miners` and
			// weight-based `pay_compute_miners` stay
			// untouched on top of that. Commission volume can at worst
			// drain the headroom above the floor, never funds owed to
			// someone else and never the last of the bank.
			let reserved = TotalUndistributedBacking::<T>::get()
				.saturating_add(PendingSudoRefunds::<T>::get())
				.saturating_add(ReferralBankFloor::<T>::get())
				.saturating_add(
					pallet_hippocampus::Pallet::<T>::emission_available().saturated_into::<u128>(),
				)
				.saturating_add(
					pallet_hippocampus::Pallet::<T>::compute_emission_available()
						.saturated_into::<u128>(),
				);
			let headroom = pallet_hippocampus::Pallet::<T>::available_for_payout()
				.saturated_into::<u128>()
				.saturating_sub(reserved);
			let payable = commission_credits.min(headroom);

			let requested: pallet_hippocampus::BalanceOf<T> = payable.saturated_into();
			match pallet_hippocampus::Pallet::<T>::request_payment(
				&Self::account_id(),
				referrer,
				requested,
			) {
				Ok(paid) => {
					let paid_tokens = paid.saturated_into::<u128>();
					Self::deposit_event(Event::<T>::ReferralCommissionPaid {
						referrer: referrer.clone(),
						requested_credits: commission_credits,
						paid_tokens,
					});
					paid_tokens
				},
				Err(e) => {
					// Not-whitelisted (post-upgrade setup missed), payouts
					// globally disabled via the bank switch, or the transfer
					// failed (e.g. payout below the referrer's existential
					// deposit). Diagnosable, never fatal.
					log::warn!(
						target: "runtime::marketplace",
						"referral commission payment failed for {:?}: {:?}",
						referrer,
						e
					);
					Self::deposit_event(Event::<T>::ReferralCommissionSkipped {
						referrer: referrer.clone(),
						requested_credits: commission_credits,
					});
					0
				},
			}
		}

		/// The referrer behind a user's redeemed code, if the user redeemed one.
		fn referrer_of(who: &T::AccountId) -> Option<T::AccountId> {
			let code = CreditsPallet::<T>::referred_users(who)?;
			CreditsPallet::<T>::referral_codes(code)
		}

		/// Credit a referrer for one hourly pay-as-you-go charge.
		///
		/// Bookkeeping only — the billing path never touches the bank. The
		/// balance is paid out by `sweep_referral_commissions`.
		///
		/// Hourly charges are recurring usage, so they follow the
		/// monthly-renewal rule rather than the purchase rule: the referrer
		/// earns a commission on what was collected, and the referred user
		/// gets no discount — the 5% buyer discount stays purchase-only.
		fn accrue_hourly_referral_commission(user: &T::AccountId, charged_credits: u128) {
			let commission = Self::referral_commission_credits(charged_credits);
			if commission == 0 {
				return;
			}
			let Some(referrer) = Self::referrer_of(user) else {
				return;
			};

			let accrued = AccruedReferralCommission::<T>::get(&referrer).saturating_add(commission);
			AccruedReferralCommission::<T>::insert(&referrer, accrued);
			Self::deposit_event(Event::ReferralCommissionAccrued {
				referrer: referrer.clone(),
				added_credits: commission,
				accrued_credits: accrued,
			});
		}

		/// Pay out accrued hourly commissions, at most
		/// `MaxReferralPayoutsPerSweep` referrers per call.
		///
		/// Runs on a fixed `ReferralPayoutInterval` cadence with no threshold
		/// to reach: whatever a referrer has earned goes out on the next sweep.
		/// Each balance is settled by exactly what the bank released, so a bank
		/// that is dry or at its `ReferralBankFloor` leaves the remainder
		/// accrued for the next sweep rather than dropping it.
		///
		/// The one balance left to accumulate is the one that would leave the
		/// referrer *still* below the existential deposit afterwards. That is not
		/// a payout threshold on the commission: ED is the account-**creation**
		/// rule, not the transfer rule, and `request_payment` spends its
		/// `KeepAlive` on the bank, not the destination. A referrer who already
		/// holds ED can therefore receive any positive amount, exactly as the
		/// purchase path already relies on — `PURCHASE_COMMISSION` is 475 against
		/// an ED of 500 and lands today. Skipping on the commission alone would
		/// strand live referrers for hours, and for longer the smaller
		/// `price_per_gb` is. Only a transfer that cannot create the account is
		/// skipped, which also keeps unfunded destinations from emitting a
		/// failure event every sweep forever.
		fn sweep_referral_commissions() -> Weight {
			let limit = T::MaxReferralPayoutsPerSweep::get() as usize;
			if limit == 0 {
				return T::DbWeight::get().reads(1);
			}

			// Snapshot the batch before paying: settlement mutates the map, and
			// a live iterator over storage being written underneath is exactly
			// the kind of undefined behavior that is not worth risking here.
			let cursor = ReferralPayoutCursor::<T>::get();
			let batch: Vec<(T::AccountId, u128)> = match cursor {
				Some(key) => AccruedReferralCommission::<T>::iter_from(key).take(limit).collect(),
				None => AccruedReferralCommission::<T>::iter().take(limit).collect(),
			};

			// End of the map (or an empty one): restart from the top next time.
			if batch.is_empty() {
				ReferralPayoutCursor::<T>::kill();
				return T::DbWeight::get().reads_writes(2, 1);
			}

			// A short batch means we reached the end of the map, so the next
			// sweep must start from the top again. Parking the cursor here
			// instead would make that sweep find nothing beyond it and pay
			// nobody — with fewer referrers than the per-sweep limit, which is
			// the normal case, every second sweep would be wasted and everyone
			// would be paid at half the intended rate.
			//
			// Otherwise resume after the last key we looked at, whether or not
			// it was paid: an unpayable balance must not block the referrers
			// behind it in the map.
			let exhausted = batch.len() < limit;
			let resume_from =
				AccruedReferralCommission::<T>::hashed_key_for(&batch[batch.len() - 1].0);

			let ed = <T as pallet::Config>::Currency::minimum_balance().saturated_into::<u128>();
			let mut paid_count: u64 = 0;
			for (referrer, accrued) in batch.iter() {
				// Measure the balance the transfer would *land on*, not the
				// commission in isolation: a referrer already at or above ED can
				// take any positive amount, and only an account that would still
				// be dust afterwards is one the transfer cannot create.
				let dest_balance = <T as pallet::Config>::Currency::free_balance(referrer)
					.saturated_into::<u128>();
				if dest_balance.saturating_add(*accrued) < ed {
					continue;
				}
				let paid = Self::try_pay_referral_commission_tokens(referrer, *accrued);
				if paid == 0 {
					continue;
				}
				paid_count = paid_count.saturating_add(1);
				let remainder = accrued.saturating_sub(paid);
				if remainder == 0 {
					AccruedReferralCommission::<T>::remove(referrer);
				} else {
					AccruedReferralCommission::<T>::insert(referrer, remainder);
				}
			}

			if exhausted {
				ReferralPayoutCursor::<T>::kill();
			} else {
				ReferralPayoutCursor::<T>::put(resume_from);
			}

			// One read per key inspected, plus the bank reserve ledgers and a
			// balance write for each referrer actually paid.
			let inspected = batch.len() as u64;
			T::DbWeight::get().reads_writes(
				inspected.saturating_add(paid_count.saturating_mul(8)).saturating_add(2),
				paid_count.saturating_mul(4).saturating_add(1),
			)
		}

		fn referral_discount_and_owner(
			who: &T::AccountId,
			face_credits: u128,
		) -> (u128, Option<T::AccountId>) {
			if let Some(ref_code) = CreditsPallet::<T>::referred_users(who) {
				// Exact 5% via payment-math (U256), not saturating_mul which
				// under-computes above u128::MAX/5.
				let discount = split(Credits::new(face_credits), BasisPoints::new(500)).0.get();
				let owner = CreditsPallet::<T>::referral_codes(ref_code);
				(discount, owner)
			} else {
				(0, None)
			}
		}

		/// Current UNIX millis as `u64`.
		fn now_ms() -> u64 {
			pallet_timestamp::Pallet::<T>::get().saturated_into::<u64>()
		}

		fn current_unix_day() -> u32 {
			// unix day number since epoch (UTC)
			(Self::now_ms() / 86_400_000u64) as u32
		}

		/// The billing anchor (day-of-month) of a subscription.
		///
		/// Derived from the due date wherever that is unambiguous, which is
		/// almost always: clamping can only ever produce 28, 29 or 30, so any
		/// other day-of-month *is* the anchor and no lookup happens. Only those
		/// three values might be a clamped 29/30/31, and only then is the sparse
		/// map consulted.
		///
		/// Absence therefore has to mean "derive it" — see
		/// [`SubscriptionAnchorDay`], which is why an entry is written whenever
		/// the anchor exceeds 28 and deleted with the subscription.
		fn anchor_of(sub: &UserPlanSubscription<T>) -> u8 {
			let Some(due_day) = sub.next_charge_unix_day else {
				// No due date to derive from. The 1st is what the legacy `None`
				// path already means, and the backfill writes it explicitly.
				return 1;
			};
			let derived = pallet_calendar::Pallet::<T>::day_of_month(due_day);
			if (28..=30).contains(&derived) {
				SubscriptionAnchorDay::<T>::get(sub.id).unwrap_or(derived)
			} else if derived == 0 {
				1
			} else {
				derived
			}
		}

		/// Record an anchor only when it cannot be derived from the due date.
		///
		/// Keeping the map sparse is not an optimisation, it is the contract:
		/// absence means "derive it", so writing a derivable anchor would be
		/// redundant while writing nothing for a non-derivable one is a bug.
		fn set_anchor(subscription_id: SubscriptionId, anchor: u8) {
			if anchor > 28 {
				SubscriptionAnchorDay::<T>::insert(subscription_id, anchor);
			} else {
				SubscriptionAnchorDay::<T>::remove(subscription_id);
			}
		}

		/// The unix day a subscription should next be charged on, for indexing.
		///
		/// A legacy `None` means "due from the 1st of the current month", which
		/// is exactly how the pre-index sweep reads it.
		fn effective_due_day(sub: &UserPlanSubscription<T>) -> u32 {
			sub.next_charge_unix_day
				.unwrap_or_else(|| pallet_calendar::Pallet::<T>::unix_day_of_first_of_month_in(0))
		}

		/// Distinct days an account's active subscriptions are due on.
		///
		/// `MaxActiveSubscriptions` is 5, so this is a handful of entries and a
		/// linear scan beats any set machinery.
		fn due_days_of(subs: &[UserPlanSubscription<T>]) -> Vec<u32> {
			let mut days: Vec<u32> = Vec::new();
			for sub in subs.iter().filter(|s| s.active) {
				let day = Self::effective_due_day(sub);
				if !days.contains(&day) {
					days.push(day);
				}
			}
			days
		}

		/// The one and only way to write `UserAllSubscriptionPlans`.
		///
		/// The due-day index is a denormalisation, so the failure mode is
		/// drift, and the dangerous direction is a *missing* entry: a due
		/// subscription nothing ever charges is silently free service with no
		/// event to notice. Rather than defend that by remembering to call
		/// something at each of the six write sites, every write goes through
		/// here and the index is reconciled from the diff — so the invariant
		/// cannot be broken by forgetting.
		///
		/// Reconciling per *account* rather than per subscription is what makes
		/// this correct: an account holds one index key per distinct due day, so
		/// moving one subscription off day D must not delete that key while a
		/// sibling is still due there. Only the days that actually changed are
		/// touched.
		///
		/// Also the single point where cancelled subscriptions are reclaimed —
		/// entry, index key and anchor row go together, and the account's map
		/// entry disappears once its last subscription does.
		fn commit_subscriptions(
			who: &T::AccountId,
			before: &[UserPlanSubscription<T>],
			after: Vec<UserPlanSubscription<T>>,
		) {
			// Drop deactivated rows. Nothing downstream reads them: cancellation
			// history lives in events and `PointTransactions`, and leaving them
			// costs weight on *every* tick, because the sweep's own skip path
			// has to decode the whole vector before it can decide to skip it.
			let after: Vec<UserPlanSubscription<T>> =
				after.into_iter().filter(|sub| sub.active).collect();

			// Anchors for subscriptions that no longer exist. `SubscriptionId`
			// is monotonic so a stale row would not be re-used, but it would sit
			// there forever.
			for sub in before.iter() {
				if !after.iter().any(|s| s.id == sub.id) {
					SubscriptionAnchorDay::<T>::remove(sub.id);
				}
			}

			let before_days = Self::due_days_of(before);
			let after_days = Self::due_days_of(&after);

			for day in before_days.iter() {
				if !after_days.contains(day) {
					DueAccounts::<T>::remove(day, who);
				}
			}
			for day in after_days.iter() {
				if !before_days.contains(day) {
					DueAccounts::<T>::insert(day, who, ());
				}
			}

			if after.is_empty() {
				UserAllSubscriptionPlans::<T>::remove(who);
			} else {
				UserAllSubscriptionPlans::<T>::insert(who, after);
			}
		}

		/// Refund credits for unused prepaid *full* months.
		///
		/// We do NOT refund the current (possibly prorated) month. We only refund whole months
		/// that start in the future, up to (but excluding) `next_charge_unix_day`.
		fn unused_prepaid_refund_credits(sub: &UserPlanSubscription<T>) -> u128 {
			let Some(due_day) = sub.next_charge_unix_day else {
				return 0;
			};

			let today = Self::current_unix_day();
			if today >= due_day {
				return 0;
			}

			// Count how many "1st of month" boundaries are still in the future before `due_day`.
			// We start from next month's 1st; that makes "current month" non-refundable.
			let mut months_remaining: u128 = 0;
			for n in 1u32..=24u32 {
				let d = pallet_calendar::Pallet::<T>::unix_day_of_first_of_month_in(n);
				if d == 0 || d >= due_day {
					break;
				}
				months_remaining = months_remaining.saturating_add(1);
			}

			times(Credits::new(sub.paid_per_month), months_remaining).get()
		}

		fn refund_credits_with_batch(account_id: &T::AccountId, amount: u128) -> DispatchResult {
			if amount == 0 {
				return Ok(());
			}

			let batch_id = NextBatchId::<T>::get();
			let batch = Batch {
				owner: account_id.clone(),
				credit_amount: amount,
				alpha_amount: 0,
				remaining_credits: amount,
				remaining_alpha: 0,
				pending_alpha: 0,
				is_frozen: false,
				release_time: <frame_system::Pallet<T>>::block_number(),
			};

			Batches::<T>::insert(batch_id, batch);
			UserBatches::<T>::append(account_id, batch_id);
			NextBatchId::<T>::put(batch_id.saturating_add(1));

			CreditsPallet::<T>::do_mint(account_id.clone(), amount, None)?;
			Self::record_credits_transaction(account_id, NativeTransactionType::Refund, amount)?;
			Ok(())
		}

		/// Restricted canceller path: remove every storage subscription from
		/// storage (delete rows), and refund unused prepaid months for any removed
		/// *active* one.
		///
		/// "Storage" here means both flavours: an account's Drive and S3
		/// subscriptions are removed together, leaving only its compute plans.
		fn do_delete_storage_subscription_with_refund(account_id: &T::AccountId) -> DispatchResult {
			let now = <frame_system::Pallet<T>>::block_number();
			let mut refunded: u128 = 0;

			let before = UserAllSubscriptionPlans::<T>::get(account_id);
			for sub in before.iter() {
				if sub.package.is_any_storage() && sub.active {
					refunded = refunded.saturating_add(Self::unused_prepaid_refund_credits(sub));
				}
			}

			let after: Vec<UserPlanSubscription<T>> =
				before.iter().filter(|sub| !sub.package.is_any_storage()).cloned().collect();

			ensure!(after.len() < before.len(), Error::<T>::NoActiveSubscription);

			Self::commit_subscriptions(account_id, &before, after);

			if refunded > 0 {
				Self::refund_credits_with_batch(account_id, refunded)?;
			}

			LastSubscriptionCancelledAt::<T>::insert(account_id, now);
			Self::deposit_event(Event::StorageSubscriptionCancelled { who: account_id.clone() });
			Ok(())
		}

		/// Helper function to get the current price per GB
		pub fn get_storage_price_per_miner() -> u128 {
			StoragePricePerMiner::<T>::get()
		}

		fn prorated_monthly_price(monthly_price: u128) -> u128 {
			// days_remaining is inclusive of today; crate ceil-rounds and clamps
			// days_remaining to days_in_month so a part never exceeds a full month.
			prorate_first_month(
				Credits::new(monthly_price),
				u32::from(pallet_calendar::Pallet::<T>::days_remaining_in_current_month()),
				u32::from(pallet_calendar::Pallet::<T>::days_in_current_month()),
			)
			.get()
		}

		/// Total upfront charge when buying `upfront_months` starting mid-month:
		/// `1 prorated month + (upfront_months - 1) full months`.
		fn upfront_prorated_total(monthly_price: u128, upfront_months: u128) -> u128 {
			let first = Self::prorated_monthly_price(monthly_price);
			let remaining_full_months = upfront_months.saturating_sub(1);
			first.saturating_add(times(Credits::new(monthly_price), remaining_full_months).get())
		}

		#[transactional]
		fn release_matured_pending_alpha(current_block: BlockNumberFor<T>) -> DispatchResult {
			for (batch_id, mut batch) in Batches::<T>::iter() {
				if !batch.is_frozen || batch.pending_alpha == 0 {
					continue;
				}
				if current_block < batch.release_time {
					continue;
				}

				batch.is_frozen = false;

				AlphaBalances::<T>::mutate(&batch.owner, |alpha| {
					*alpha = alpha.saturating_sub(batch.pending_alpha)
				});

				// Alpha stays in bank, no distribution. Track backed portion and release TUB.
				let backed = Self::take_backed_portion(batch_id, batch.pending_alpha);
				TotalUndistributedBacking::<T>::mutate(|t| *t = t.saturating_sub(backed));

				batch.pending_alpha = 0;
				Batches::<T>::insert(batch_id, batch);
			}

			Ok(())
		}

		pub fn account_id() -> T::AccountId {
			<T as pallet::Config>::PalletId::get().into_account_truncating()
		}

		/// Get the current balance of the marketplace pallet
		pub fn balance() -> BalanceOf<T> {
			pallet_balances::Pallet::<T>::free_balance(&Self::account_id())
		}

		pub fn calculate_distribution_per_era() -> BalanceOf<T> {
			// Calculate total amount for distribution
			let total_amount: BalanceOf<T> = Self::balance();

			// Number of eras in 30 days
			let block_duration_millis = T::BlockDurationMillis::get();
			let blocks_per_era = <T as pallet::Config>::BlocksPerEra::get();
			let era_duration_millis = block_duration_millis as u32 * blocks_per_era;
			let eras_in_30_days = (30 * 24 * 60 * 60 * 1000) / era_duration_millis;

			// Convert eras_in_30_days to BalanceOf<T> with proper decimal handling
			let eras_balance: BalanceOf<T> =
				(eras_in_30_days as u128).try_into().unwrap_or_default();
			// Distribution amount per era
			total_amount / eras_balance
		}

		fn do_purchase_storage_plan(
			who: T::AccountId,
			plan_id: T::Hash,
			pay_upfront: Option<u128>,
		) -> DispatchResult {
			// Check if the ComputeMiner node type is disabled
			ensure!(
				!RegistrationPallet::<T>::is_node_type_disabled(NodeType::StorageMiner),
				Error::<T>::NodeTypeDisabled
			);

			// Check if storage operations are enabled
			ensure!(Self::is_purchase_plan_enabled(), Error::<T>::PlanOperationDisabled);

			// Check if plan exists
			let plan = Plans::<T>::get(&plan_id).ok_or(Error::<T>::PlanNotFound)?;

			ensure!(!plan.is_suspended, Error::<T>::PlanSuspended);

			// Enforce: one active subscription per storage flavour — one Drive plan
			// and one S3 plan — and validate the overall subscription cap, both
			// before any state changes. The dispatch storage layer would roll a
			// late failure back anyway, but failing fast wastes no work and keeps
			// the flow safe for any future non-dispatch caller.
			//
			// Matching on the flavour is what lets the two coexist: buying an S3
			// plan is blocked only by another active S3 plan, never by the Drive
			// plan whose bytes it does not cover.
			let flavour = plan.storage_flavour();
			let before = UserAllSubscriptionPlans::<T>::get(&who);
			let mut subscriptions = before.clone();
			ensure!(
				!subscriptions.iter().any(|s| s.active && s.package.storage_flavour() == flavour),
				Error::<T>::AlreadyHasActiveSubscription
			);
			let active_count = subscriptions.iter().filter(|s| s.active).count() as u32;
			ensure!(
				active_count < T::MaxActiveSubscriptions::get(),
				Error::<T>::TooManyActiveSubscriptions
			);

			// Determine the (monthly) price for the plan.
			// If the user purchases mid-month, charge a pro-rated amount based on remaining days
			// in the current month (inclusive of today).
			let mut plan_price_native = Self::prorated_monthly_price(plan.price);

			if let Some(upfront_months) = pay_upfront {
				plan_price_native = Self::upfront_prorated_total(plan.price, upfront_months);
			}

			let months_paid: u32 = pay_upfront.unwrap_or(1).min(u128::from(u32::MAX)) as u32;
			let next_charge_unix_day =
				Some(pallet_calendar::Pallet::<T>::unix_day_of_first_of_month_in(months_paid));

			// Apply referral discount ONLY at subscription purchase time (not on renewals).
			let (referral_discount, ref_owner) =
				Self::referral_discount_and_owner(&who, plan_price_native);
			let charged_credits = plan_price_native.saturating_sub(referral_discount);

			// Check user's native token balance
			let user_free_credits = CreditsPallet::<T>::get_free_credits(&who);
			ensure!(user_free_credits >= charged_credits, Error::<T>::InsufficientFreeCredits);

			// Prevent cancel-and-resubscribe grace period reset abuse
			let current_block_number = <frame_system::Pallet<T>>::block_number();
			if let Some(last_cancelled_at) = LastSubscriptionCancelledAt::<T>::get(&who) {
				let cooldown = T::MinSubscriptionBlocks::get();
				ensure!(
					current_block_number >= last_cancelled_at.saturating_add(cooldown),
					Error::<T>::ResubscribeCooldownActive
				);
			}

			// Generate new subscription ID
			let subscription_id = NextSubscriptionId::<T>::mutate(|id| {
				let current_id = *id;
				*id = id.saturating_add(1);
				current_id
			});

			Self::consume_credits(who.clone(), charged_credits)?;

			// Record transaction
			Self::record_credits_transaction(
				&who,
				NativeTransactionType::Subscription,
				charged_credits.into(),
			)?;

			// Pay the referral commission in native tokens from the bank,
			// computed on the credits actually collected (post-discount).
			if let Some(owner) = ref_owner {
				let commission = Self::referral_commission_credits(charged_credits);
				Self::try_pay_referral_commission_tokens(&owner, commission);
			}

			// Create subscription (simplified due to removed plan_type)
			// 95% of face price for referred users — conserved split, not
			// saturating_mul which under-charges above u128::MAX/9_500.
			let paid_per_month = if CreditsPallet::<T>::referred_users(&who).is_some() {
				split(Credits::new(plan.price), BasisPoints::new(9_500)).0.get()
			} else {
				plan.price
			};
			let subscription = UserPlanSubscription {
				id: subscription_id,
				owner: who.clone(),
				package: plan.clone(),
				cdn_location_id: None,
				active: true,
				last_charged_at: current_block_number,
				selected_image_name: None,
				next_charge_unix_day,
				paid_per_month,
				_phantom: PhantomData,
			};

			// Add the new subscription (cap validated before any charging).
			subscriptions.push(subscription);

			// Save the updated subscriptions list, reconciling the due-day index.
			Self::commit_subscriptions(&who, &before, subscriptions);

			Ok(())
		}

		/// Swap `account_id`'s single active storage subscription onto `new_plan_id`.
		///
		/// Money model — one net movement, never two:
		/// - `refund_credits`: unused prepaid *full* months on the old plan, i.e.
		///   exactly what the cancel path refunds (`unused_prepaid_refund_credits`).
		/// - `carry_credits`: what the unexpired remainder of the **current** month
		///   is worth on the old plan. The cancel path deliberately never refunds
		///   the current month, and neither do we — this is only applied as a credit
		///   *against* the new plan's first month, so an upgrade pays the difference
		///   instead of paying for the same month twice. Paying it out on a
		///   downgrade would mint credits carrying no alpha backing on a call that
		///   has no resubscribe cooldown, i.e. a free plan-cycling loop; capping it
		///   at the new charge closes that while still fixing the double-charge.
		/// - The new plan's first month is prorated and discounted exactly as a
		///   fresh purchase would be, and `next_charge_unix_day` lands on the 1st of
		///   next month, so recurring charging treats it like any other new
		///   subscription.
		///
		/// The refund is then netted against the charge, so `FreeCredits` moves once
		/// and prepaid months can pay for the new plan directly.
		/// Shared entry point behind `change_storage_plan` and `change_s3_plan`.
		///
		/// Both extrinsics are the same call on a different slot, so the ACL and
		/// the rate limit live here once rather than being copied per flavour —
		/// a copy is exactly where the two would drift apart.
		fn dispatch_plan_change(
			origin: OriginFor<T>,
			user: T::AccountId,
			flavour: StorageFlavour,
			new_plan_id: T::Hash,
			selected_image_name: Option<Vec<u8>>,
			location_id: Option<u32>,
			cloud_init_cid: Option<Vec<u8>>,
		) -> DispatchResult {
			let who = ensure_signed(origin)?;

			let allowed = WhitelistedCallers::<T>::get();
			ensure!(allowed.contains(&who), Error::<T>::WhitelistedCallerNotAuthorized);

			// Same per-block rate limit `purchase_plan` applies, counted against the
			// subscriber rather than the relayer — it is what bounds repeated plan
			// changes within a block, since this call has no resubscribe cooldown.
			// Both flavours share the counter, so a user cannot double their budget
			// by alternating between the two calls.
			let max_requests_per_block = T::MaxRequestsPerBlock::get();
			let user_requests_count = UserRequestsCount::<T>::get(&user);
			ensure!(
				user_requests_count.saturating_add(1) <= max_requests_per_block,
				Error::<T>::TooManyRequests
			);
			UserRequestsCount::<T>::insert(&user, user_requests_count.saturating_add(1));

			Self::do_change_plan_of_flavour(
				&user,
				flavour,
				new_plan_id,
				location_id,
				Self::normalize_image_selection(selected_image_name),
				cloud_init_cid,
			)
		}

		fn do_change_plan_of_flavour(
			account_id: &T::AccountId,
			flavour: StorageFlavour,
			new_plan_id: T::Hash,
			_location_id: Option<u32>,
			_selected_image_name: Option<Vec<u8>>,
			_cloud_init_cid: Option<Vec<u8>>,
		) -> DispatchResult {
			ensure!(
				!RegistrationPallet::<T>::is_node_type_disabled(NodeType::StorageMiner),
				Error::<T>::NodeTypeDisabled
			);

			// Same kill switch `purchase_plan` honours: if new storage plans cannot
			// be bought, they cannot be switched into either.
			ensure!(Self::is_purchase_plan_enabled(), Error::<T>::PlanOperationDisabled);

			let before = UserAllSubscriptionPlans::<T>::get(account_id);
			let mut subscriptions = before.clone();

			// Exactly one active subscription *of this flavour*.
			// `do_purchase_storage_plan` enforces that invariant on the way in, so
			// more than one is corrupt state we refuse rather than silently pick a
			// winner from. The other flavour is never a candidate, so an account
			// holding both keeps the one it did not ask to change.
			let mut active_of_flavour = subscriptions
				.iter()
				.enumerate()
				.filter(|(_, s)| s.active && s.package.storage_flavour() == Some(flavour))
				.map(|(i, _)| i);
			let index = active_of_flavour.next().ok_or(Error::<T>::NoActiveSubscription)?;
			ensure!(active_of_flavour.next().is_none(), Error::<T>::TooManyActiveSubscriptions);

			let old_sub = subscriptions[index].clone();
			let old_plan_id = old_sub.package.id;
			ensure!(new_plan_id != old_plan_id, Error::<T>::InvalidInput);

			let new_plan = Plans::<T>::get(&new_plan_id).ok_or(Error::<T>::PlanNotFound)?;
			ensure!(!new_plan.is_suspended, Error::<T>::PlanSuspended);
			// The target must be the same flavour: a Drive change cannot land on an
			// S3 plan, which would leave the account holding two S3 subscriptions
			// and no Drive one.
			ensure!(new_plan.storage_flavour() == Some(flavour), Error::<T>::InvalidPlanType);

			// Unused prepaid full months on the old plan — the cancel refund, unchanged.
			let refund_credits = Self::unused_prepaid_refund_credits(&old_sub);

			// Value of the remaining days of the current month on the old plan. Only
			// counts while the old subscription is actually paid through the current
			// month; a lapsed subscription, or a legacy one with no
			// `next_charge_unix_day`, carries nothing forward. `paid_per_month` is
			// the discounted price actually billed, so the carry never exceeds what
			// the user paid for those days.
			let paid_through_this_month = old_sub
				.next_charge_unix_day
				.is_some_and(|due_day| due_day > Self::current_unix_day());
			let carry_credits = if paid_through_this_month {
				Self::prorated_monthly_price(old_sub.paid_per_month)
			} else {
				0
			};

			// New plan's first month, prorated and discounted like a fresh purchase.
			let new_plan_price = Self::prorated_monthly_price(new_plan.price);
			let (referral_discount, ref_owner) =
				Self::referral_discount_and_owner(account_id, new_plan_price);
			let charged_credits =
				new_plan_price.saturating_sub(referral_discount).saturating_sub(carry_credits);

			// Net the two halves so `FreeCredits` moves exactly once, and check
			// affordability against that net delta before touching any state.
			let net_charge = charged_credits.saturating_sub(refund_credits);
			let net_refund = refund_credits.saturating_sub(charged_credits);

			let user_free_credits = CreditsPallet::<T>::get_free_credits(account_id);
			ensure!(user_free_credits >= net_charge, Error::<T>::InsufficientFreeCredits);

			if net_charge > 0 {
				Self::consume_credits(account_id.clone(), net_charge)?;
				Self::record_credits_transaction(
					account_id,
					NativeTransactionType::Subscription,
					net_charge,
				)?;
			}

			if net_refund > 0 {
				// Mints the refund and records the `Refund` transaction itself.
				Self::refund_credits_with_batch(account_id, net_refund)?;
			}

			// Commission on the credits actually collected by this call. The
			// refunded months already paid a commission when they were bought, so
			// netting first is what stops a plan change from paying twice on them.
			if let Some(owner) = ref_owner {
				let commission = Self::referral_commission_credits(net_charge);
				Self::try_pay_referral_commission_tokens(&owner, commission);
			}

			let subscription_id = NextSubscriptionId::<T>::mutate(|id| {
				let current_id = *id;
				*id = id.saturating_add(1);
				current_id
			});

			// 95% of face price for referred users — the same conserved split the
			// purchase paths record, so future refunds keep valuing a month correctly.
			let paid_per_month = if CreditsPallet::<T>::referred_users(account_id).is_some() {
				split(Credits::new(new_plan.price), BasisPoints::new(9_500)).0.get()
			} else {
				new_plan.price
			};

			// Replace in place. The slot stays occupied for the whole dispatch, so
			// the storage entitlement never lapses and the change can never trip
			// `MaxActiveSubscriptions` the way a re-purchase could.
			subscriptions[index] = UserPlanSubscription {
				id: subscription_id,
				owner: account_id.clone(),
				package: new_plan,
				cdn_location_id: None,
				active: true,
				last_charged_at: <frame_system::Pallet<T>>::block_number(),
				selected_image_name: None,
				next_charge_unix_day: Some(
					pallet_calendar::Pallet::<T>::unix_day_of_first_of_month_in(1),
				),
				paid_per_month,
				_phantom: PhantomData,
			};

			Self::commit_subscriptions(account_id, &before, subscriptions);

			let event = match flavour {
				StorageFlavour::Drive => Event::StoragePlanChanged {
					user: account_id.clone(),
					old_plan: old_plan_id,
					new_plan: new_plan_id,
					subscription_id,
					charged_credits: net_charge,
					refunded_credits: net_refund,
				},
				StorageFlavour::S3 => Event::S3PlanChanged {
					user: account_id.clone(),
					old_plan: old_plan_id,
					new_plan: new_plan_id,
					subscription_id,
					charged_credits: net_charge,
					refunded_credits: net_refund,
				},
			};
			Self::deposit_event(event);

			Ok(())
		}

		/// Treat `None` and empty byte vectors as no image selection.
		fn normalize_image_selection(image: Option<Vec<u8>>) -> Option<Vec<u8>> {
			image.filter(|name| !name.is_empty())
		}

		fn do_purchase_compute_plan(
			who: T::AccountId,
			plan_id: T::Hash,
			location_id: Option<u32>,
			selected_image_name: Option<Vec<u8>>,
			cloud_init_cid: Option<Vec<u8>>,
			pay_upfront: Option<u128>,
		) -> DispatchResult {
			// Check if the ComputeMiner node type is disabled
			ensure!(
				!RegistrationPallet::<T>::is_node_type_disabled(NodeType::ComputeMiner),
				Error::<T>::NodeTypeDisabled
			);

			// Validate the subscription cap before any state changes. The
			// dispatch storage layer would roll a late failure back anyway,
			// but failing fast wastes no work and keeps the flow safe for
			// any future non-dispatch caller.
			let before = UserAllSubscriptionPlans::<T>::get(&who);
			let mut subscriptions = before.clone();
			let active_count = subscriptions.iter().filter(|s| s.active).count() as u32;
			ensure!(
				active_count < T::MaxActiveSubscriptions::get(),
				Error::<T>::TooManyActiveSubscriptions
			);

			// Check if plan exists
			let plan = Plans::<T>::get(&plan_id).ok_or(Error::<T>::PlanNotFound)?;

			ensure!(!plan.is_suspended, Error::<T>::PlanSuspended);

			// Validate image only when one was provided.
			if let Some(ref name) = selected_image_name {
				ensure!(
					Self::os_disk_image_urls(name.clone()).is_some(),
					Error::<T>::InvalidImageSelection
				);
			}

			// Determine the (monthly) price (pro-rated if purchased mid-month).
			let mut plan_price_native = Self::prorated_monthly_price(plan.price);

			if let Some(upfront_months) = pay_upfront {
				plan_price_native = Self::upfront_prorated_total(plan.price, upfront_months);
			}

			let months_paid: u32 = pay_upfront.unwrap_or(1).min(u128::from(u32::MAX)) as u32;
			let next_charge_unix_day =
				Some(pallet_calendar::Pallet::<T>::unix_day_of_first_of_month_in(months_paid));

			// Apply referral discount ONLY at subscription purchase time (not on renewals).
			let (referral_discount, ref_owner) =
				Self::referral_discount_and_owner(&who, plan_price_native);
			let charged_credits = plan_price_native.saturating_sub(referral_discount);

			// Check user's native token balance
			let user_free_credits = CreditsPallet::<T>::get_free_credits(&who);
			ensure!(user_free_credits >= charged_credits, Error::<T>::InsufficientFreeCredits);

			// Prevent cancel-and-resubscribe grace period reset abuse
			let current_block_number = <frame_system::Pallet<T>>::block_number();
			if let Some(last_cancelled_at) = LastSubscriptionCancelledAt::<T>::get(&who) {
				let cooldown = T::MinSubscriptionBlocks::get();
				ensure!(
					current_block_number >= last_cancelled_at.saturating_add(cooldown),
					Error::<T>::ResubscribeCooldownActive
				);
			}

			// Validate location if specified
			if let Some(location_id) = location_id {
				ensure!(CdnLocations::<T>::contains_key(location_id), Error::<T>::LocationNotFound);
			}

			// Generate new subscription ID
			let subscription_id = NextSubscriptionId::<T>::mutate(|id| {
				let current_id = *id;
				*id = id.saturating_add(1);
				current_id
			});

			Self::consume_credits(who.clone(), charged_credits)?;

			// Record transaction
			Self::record_credits_transaction(
				&who,
				NativeTransactionType::Subscription,
				charged_credits.into(),
			)?;

			// Pay the referral commission in native tokens from the bank,
			// computed on the credits actually collected (post-discount).
			if let Some(owner) = ref_owner {
				let commission = Self::referral_commission_credits(charged_credits);
				Self::try_pay_referral_commission_tokens(&owner, commission);
			}

			// Create subscription (simplified due to removed plan_type)
			// 95% of face price for referred users — conserved split.
			let paid_per_month = if CreditsPallet::<T>::referred_users(&who).is_some() {
				split(Credits::new(plan.price), BasisPoints::new(9_500)).0.get()
			} else {
				plan.price
			};
			let subscription = UserPlanSubscription {
				id: subscription_id,
				owner: who.clone(),
				package: plan.clone(),
				cdn_location_id: location_id,
				active: true,
				last_charged_at: current_block_number,
				selected_image_name,
				next_charge_unix_day,
				paid_per_month,
				_phantom: PhantomData,
			};

			// Add the new subscription (cap validated before any charging).
			subscriptions.push(subscription);

			// Save the updated subscriptions list, reconciling the due-day index.
			Self::commit_subscriptions(&who, &before, subscriptions);

			Ok(())
		}

		fn record_credits_transaction(
			who: &T::AccountId,
			transaction_type: NativeTransactionType,
			amount: Points,
		) -> DispatchResult {
			let transaction_id =
				NextTransactionId::<T>::try_mutate(who, |id| -> Result<u32, DispatchError> {
					let current_id = *id;
					*id = id.saturating_add(1);
					Ok(current_id)
				})?;

			let transaction = PointTransaction {
				transaction_type: transaction_type.clone(),
				amount,
				timestamp: frame_system::Pallet::<T>::block_number(),
				subscription_id: None,
				_phantom: PhantomData,
			};

			PointTransactions::<T>::insert(who, transaction_id, transaction);
			Self::deposit_event(Event::PointTransactionRecorded {
				who: who.clone(),
				transaction_type,
				amount,
			});

			Ok(())
		}

		/// Unified function to handle all subscription charging (storage + compute) in single iteration.
		///
		/// Returns a conservative weight estimate based on number of users/subscriptions processed.
		/// Charge one month for each due subscription, one at a time, in ascending
		/// subscription id order.
		///
		/// Returns `(credits actually collected, ids that could not be paid)`.
		///
		/// Charging per subscription rather than as a lump sum is what makes a
		/// partial payment partial: an account that can afford one of its
		/// subscriptions but not all of them keeps the ones it can pay for
		/// instead of losing every subscription on that side. Ascending id order
		/// makes which ones survive deterministic — oldest first.
		fn charge_due_subscriptions_individually(
			account_id: &T::AccountId,
			due: &[UserPlanSubscription<T>],
		) -> (u128, Vec<SubscriptionId>) {
			let mut charged_total = 0u128;
			let mut failed = Vec::new();
			let mut available = CreditsPallet::<T>::get_free_credits(account_id);

			let mut sorted = due.to_vec();
			sorted.sort_by_key(|sub| sub.id);

			for sub in &sorted {
				let price = sub.package.price;
				let paid = available >= price
					&& Self::consume_credits(account_id.clone(), price)
						.and_then(|_| {
							Self::record_credits_transaction(
								account_id,
								NativeTransactionType::Subscription,
								price,
							)
						})
						.is_ok();

				if paid {
					charged_total = charged_total.saturating_add(price);
					available = available.saturating_sub(price);
				} else {
					failed.push(sub.id);
				}
			}

			(charged_total, failed)
		}

		/// Monthly renewal sweep: charge every due subscription for the accounts
		/// that hold them, and deactivate the ones that cannot be paid.
		///
		/// Storage and compute are charged as independent sides — a failure on one
		/// never deactivates the other — and *within* each side one subscription at
		/// a time, in ascending subscription id order. Per-subscription is what
		/// keeps a partial payment partial: an account holding a Drive plan and an
		/// S3 plan that can afford only one keeps the one it paid for rather than
		/// losing both, which would also drop the covered side onto hourly
		/// pay-as-you-go billing.
		///
		/// Runs at most `max_catchup_months` cycles per account so a chain that
		/// missed several month boundaries catches up without an unbounded loop.
		fn charge_account_due(account_id: &T::AccountId, current_block: BlockNumberFor<T>) -> bool {
			let mut users_charged_or_cancelled: u64 = 0;
			let today = Self::current_unix_day();
			// Cap catch-up to avoid heavy loops after long downtime.
			let max_catchup_months: u32 = 3;

			{
				// Re-read the subscription rather than trusting the index entry
				// that led us here. A drain spans blocks, so a user can cancel
				// between their day opening and their entry being reached — and
				// this is also what makes a stale index entry cost one wasted
				// read instead of charging a cancelled plan.
				let before = UserAllSubscriptionPlans::<T>::get(account_id);
				let mut subs = before.clone();

				// If no active subs, skip.
				if !subs.iter().any(|s| s.active) {
					return false;
				}

				for _ in 0..max_catchup_months {
					let active_subs: Vec<UserPlanSubscription<T>> =
						subs.iter().filter(|s| s.active).cloned().collect();

					if active_subs.is_empty() {
						break;
					}

					let due = |sub: &&UserPlanSubscription<T>| -> bool {
						sub.next_charge_unix_day.map_or(true, |d| today >= d)
					};

					let storage_subs_to_charge: Vec<_> = active_subs
						.iter()
						.filter(|sub| sub.package.is_any_storage())
						.filter(due)
						.cloned()
						.collect();
					let compute_subs_to_charge: Vec<_> = active_subs
						.iter()
						.filter(|sub| !sub.package.is_any_storage())
						.filter(due)
						.cloned()
						.collect();

					// Nothing due (or nothing active) → stop catch-up for this account.
					if storage_subs_to_charge.is_empty() && compute_subs_to_charge.is_empty() {
						break;
					}

					// One month's worth for each due subscription, per side. These
					// totals are only used to decide whether there is anything to
					// do and to report the shortfall — the actual charging is per
					// subscription below.
					let total_storage_charge = storage_subs_to_charge
						.iter()
						.fold(0u128, |acc, sub| acc.saturating_add(sub.package.price));
					let total_compute_charge = compute_subs_to_charge
						.iter()
						.fold(0u128, |acc, sub| acc.saturating_add(sub.package.price));

					if total_storage_charge == 0 && total_compute_charge == 0 {
						break;
					}
					users_charged_or_cancelled = users_charged_or_cancelled.saturating_add(1);

					// Charge storage and compute independently, and *within* each
					// side charge one subscription at a time in ascending id order.
					//
					// Per-subscription is what keeps a partial payment partial. An
					// account can hold a Drive plan and an S3 plan at once, so
					// summing the storage side and failing it as a unit would take
					// both away from a user who could afford one — dropping the
					// side they had covered onto hourly pay-as-you-go, and starting
					// hourly commission accrual on it. Ascending id order makes the
					// survivor deterministic: the oldest subscription is charged
					// first and so is the one that survives.
					let storage_available_at_attempt =
						CreditsPallet::<T>::get_free_credits(&account_id);
					let (storage_charged_total, storage_subs_to_deactivate) =
						Self::charge_due_subscriptions_individually(
							&account_id,
							&storage_subs_to_charge,
						);

					let compute_available_at_attempt =
						CreditsPallet::<T>::get_free_credits(&account_id);
					let (compute_charged_total, compute_subs_to_deactivate) =
						Self::charge_due_subscriptions_individually(
							&account_id,
							&compute_subs_to_charge,
						);

					let mut subs_to_deactivate = storage_subs_to_deactivate;
					subs_to_deactivate.extend(compute_subs_to_deactivate);

					// Update successfully charged subscriptions (only those due),
					// advancing from the previous `next_charge_unix_day`, not from "now".
					for sub in subs.iter_mut() {
						if !sub.active {
							continue;
						}
						if !sub.next_charge_unix_day.map_or(true, |d| today >= d) {
							continue;
						}

						if !subs_to_deactivate.contains(&sub.id) {
							sub.last_charged_at = current_block;
							let prev_next = sub.next_charge_unix_day.unwrap_or(today);
							sub.next_charge_unix_day = Some(
								pallet_calendar::Pallet::<T>::unix_day_of_first_of_month_after(
									prev_next,
								),
							);
						}
					}

					// Referral commission on the credits actually collected this
					// cycle. Both sides now report what they really charged, so a
					// partially paid side earns commission on exactly the
					// subscriptions that went through.
					let total_charged =
						storage_charged_total.saturating_add(compute_charged_total);
					if total_charged > 0 {
						if let Some(referrer) = Self::referrer_of(&account_id) {
							let commission = Self::referral_commission_credits(total_charged);
							Self::try_pay_referral_commission_tokens(&referrer, commission);
						}
					}

					// Deactivate the subscriptions that could not be paid, refund
					// any unused prepaid months, and report each one individually.
					for sub_id in subs_to_deactivate {
						let Some(failed) = subs.iter().find(|s| s.id == sub_id).cloned() else {
							continue;
						};
						let price = failed.package.price;
						let available_at_attempt = if failed.package.is_any_storage() {
							storage_available_at_attempt
						} else {
							compute_available_at_attempt
						};
						log::warn!(
							target: "runtime::marketplace",
							"monthly {} subscription charge failed for {:?}, sub_id={}: required={}, available_at_attempt={}",
							if failed.package.is_any_storage() { "storage" } else { "compute" },
							account_id,
							sub_id,
							price,
							available_at_attempt
						);
						for sub in subs.iter_mut() {
							if sub.id == sub_id && sub.active {
								sub.active = false;
								// Refund unused prepaid months
								let refund = Self::unused_prepaid_refund_credits(sub);
								if refund > 0 {
									if let Err(e) =
										Self::refund_credits_with_batch(&account_id, refund)
									{
										log::error!(
											target: "runtime::marketplace",
											"failed to refund unused prepaid for sub_id={}: {:?}",
											sub_id,
											e
										);
									}
								}
								break;
							}
						}
						Self::deposit_event(Event::SubscriptionChargeFailed {
							who: account_id.clone(),
							required_credits: price,
							available_credits: available_at_attempt,
						});
					}
				}

				// Persist any advances/deactivations for this account.
				Self::commit_subscriptions(account_id, &before, subs);
			}

			users_charged_or_cancelled > 0
		}

		/// Renewal sweep: drain the accounts actually due, up to the per-run cap.
		///
		/// This replaces a full walk of `UserAllSubscriptionPlans` on every tick.
		/// Date-to-date billing means somebody is due every day rather than only
		/// on the 1st, so the old monthly sweep would have become a daily one —
		/// running an unbounded loop 30x more often, which is strictly worse
		/// than what it replaced. Reading only the due-day prefix makes the work
		/// proportional to who is due instead of to how many subscriptions exist.
		///
		/// The cap is what lets the returned weight be a promise rather than a
		/// report: nothing upstream rejects an overweight `on_initialize`, the
		/// block is simply produced heavier, so the bound has to be applied
		/// before the work rather than measured after it.
		fn handle_all_subscription_charging(current_block: BlockNumberFor<T>) -> Weight {
			let cap = T::MaxSubscriptionChargesPerRun::get() as usize;
			if cap == 0 {
				return T::DbWeight::get().reads(1);
			}

			// Until the backfill has populated the index, an account in its
			// untouched tail has no entry and would go uncharged. Fall back to
			// the pre-index full scan; this branch retires one release later.
			if !BackfillDone::<T>::get() {
				return Self::full_scan_subscription_charging(current_block);
			}

			let today = Self::current_unix_day();
			let mut cursor = DueDayCursor::<T>::get().unwrap_or(today);
			let mut accounts_charged: u64 = 0;
			let mut accounts_seen: u64 = 0;
			let mut day_probes: u32 = 0;

			// Walking a cursor rather than reading only `today` is what stops a
			// day that did not finish draining — or that the chain was down for
			// — from being silently skipped. This is the role
			// `max_catchup_months` plays in the old code and it has to survive.
			while cursor <= today && (accounts_seen as usize) < cap && day_probes < MAX_DAY_PROBES {
				let batch: Vec<T::AccountId> = DueAccounts::<T>::iter_key_prefix(cursor)
					.take(cap.saturating_sub(accounts_seen as usize))
					.collect();

				if batch.is_empty() {
					// Day fully drained. Advancing only on an empty prefix is
					// what keeps the cursor from stranding a day's charges
					// permanently; a charge always moves the due date forward by
					// at least 28 days, so re-filed entries can never land
					// behind it.
					day_probes = day_probes.saturating_add(1);
					cursor = cursor.saturating_add(1);
					continue;
				}

				for account_id in batch {
					// Drop the entry that led us here *before* charging, so the
					// day always makes progress. A stale entry — one whose
					// subscription is no longer due, or no longer exists — would
					// otherwise keep the prefix non-empty forever and wedge the
					// cursor. `commit_subscriptions` re-files the account under
					// whatever days its subscriptions actually hold.
					DueAccounts::<T>::remove(cursor, &account_id);
					accounts_seen = accounts_seen.saturating_add(1);
					if Self::charge_account_due(&account_id, current_block) {
						accounts_charged = accounts_charged.saturating_add(1);
					}
				}
			}

			DueDayCursor::<T>::put(cursor);

			// Bounded by construction: `accounts_seen <= cap`.
			let reads = accounts_seen
				.saturating_mul(6)
				.saturating_add(accounts_charged.saturating_mul(10))
				.saturating_add(u64::from(day_probes))
				.saturating_add(2);
			let writes = accounts_seen.saturating_add(accounts_charged.saturating_mul(10))
				.saturating_add(1);
			T::DbWeight::get().reads_writes(reads, writes)
		}

		/// Populate the due-day index for subscriptions that predate it.
		///
		/// This cannot be a migration. `pallet-migrations` is not configured in
		/// this runtime — there is no `MultiBlockMigrator` and every existing
		/// migration is a single-block `OnRuntimeUpgrade` — so iterating every
		/// subscription there is the same unbounded work this change removes,
		/// relocated into the one block where exceeding the budget breaks the
		/// upgrade itself. Ordinary paginated hook work is bounded by
		/// construction and costs nothing once finished.
		///
		/// Read-only over `UserAllSubscriptionPlans` except for one case: a
		/// legacy `None` due date is written out as the 1st of the current
		/// month, which is precisely what the old sweep already read it to mean.
		/// An index has no key for `None`, so normalising it here is what stops
		/// a survivor from silently never being billed again. That is a value
		/// written through the existing type, not a change of shape.
		fn backfill_due_index() -> Weight {
			if BackfillDone::<T>::get() {
				return T::DbWeight::get().reads(1);
			}

			let limit = T::MaxBackfillAccountsPerRun::get() as usize;
			if limit == 0 {
				return T::DbWeight::get().reads(1);
			}

			// Fix the floor once, on the first tick, so a run that spans days
			// files every account against the same reference point.
			let cursor_start = match DueDayCursor::<T>::get() {
				Some(day) => day,
				None => {
					let start = pallet_calendar::Pallet::<T>::unix_day_of_first_of_month_in(0);
					DueDayCursor::<T>::put(start);
					start
				},
			};

			let batch: Vec<(T::AccountId, Vec<UserPlanSubscription<T>>)> =
				match BackfillCursor::<T>::get() {
					Some(key) => {
						UserAllSubscriptionPlans::<T>::iter_from(key).take(limit).collect()
					},
					None => UserAllSubscriptionPlans::<T>::iter().take(limit).collect(),
				};

			if batch.is_empty() {
				BackfillDone::<T>::put(true);
				BackfillCursor::<T>::kill();
				log::info!(
					target: "runtime::marketplace",
					"due-day index backfill complete; cursor at day {}",
					cursor_start,
				);
				return T::DbWeight::get().reads_writes(2, 2);
			}

			let resume_from =
				UserAllSubscriptionPlans::<T>::hashed_key_for(&batch[batch.len() - 1].0);
			let exhausted = batch.len() < limit;
			let mut normalised: u64 = 0;

			for (account_id, subs) in batch.iter() {
				let before = subs.clone();
				let mut after = subs.clone();
				for sub in after.iter_mut() {
					if sub.active && sub.next_charge_unix_day.is_none() {
						sub.next_charge_unix_day = Some(cursor_start);
						normalised = normalised.saturating_add(1);
					}
				}

				// File each account no earlier than the cursor. An overdue row
				// — one whose day already passed while the chain was down —
				// would otherwise sit behind the cursor where the drain can
				// never reach it. Clamping it up to the cursor charges it on the
				// next tick, which is the same outcome as being overdue, and
				// `max_catchup_months` still bounds the arrears.
				for sub in after.iter().filter(|s| s.active) {
					let day = Self::effective_due_day(sub).max(cursor_start);
					DueAccounts::<T>::insert(day, account_id, ());
				}

				// Only rewrite the stored vector when a `None` was normalised;
				// otherwise this stays a pure index write.
				if normalised > 0 && before.iter().any(|s| s.next_charge_unix_day.is_none()) {
					UserAllSubscriptionPlans::<T>::insert(account_id, after);
				}
			}

			if exhausted {
				BackfillDone::<T>::put(true);
				BackfillCursor::<T>::kill();
				log::info!(target: "runtime::marketplace", "due-day index backfill complete");
			} else {
				BackfillCursor::<T>::put(resume_from);
			}

			let touched = batch.len() as u64;
			T::DbWeight::get().reads_writes(
				touched.saturating_add(3),
				touched
					.saturating_mul(u64::from(T::MaxActiveSubscriptions::get()))
					.saturating_add(normalised)
					.saturating_add(2),
			)
		}

		/// Pre-index behaviour, kept alive only until `BackfillDone` is set.
		///
		/// Unbounded by nature — this is exactly what the index exists to
		/// remove. Delete along with the `BackfillDone` gate one release after
		/// the backfill has completed on every network.
		fn full_scan_subscription_charging(current_block: BlockNumberFor<T>) -> Weight {
			let mut users_seen: u64 = 0;
			let mut users_charged_or_cancelled: u64 = 0;

			let accounts: Vec<T::AccountId> =
				UserAllSubscriptionPlans::<T>::iter_keys().collect();
			for account_id in accounts {
				users_seen = users_seen.saturating_add(1);
				if Self::charge_account_due(&account_id, current_block) {
					users_charged_or_cancelled = users_charged_or_cancelled.saturating_add(1);
				}
			}

			let reads = users_seen
				.saturating_mul(6)
				.saturating_add(users_charged_or_cancelled.saturating_mul(10));
			let writes = users_charged_or_cancelled.saturating_mul(10);
			T::DbWeight::get().reads_writes(reads, writes)
		}

		/// Hourly pay-as-you-go billing for users with no subscription covering
		/// their bytes.
		///
		/// Drive and S3 bytes are metered and billed separately, each exempted by
		/// its own plan: an active Drive plan takes the Drive bytes out of the bill
		/// and an active S3 plan takes the S3 bytes out, so a user holding one plan
		/// still pays hourly for the other side's usage and a user holding both
		/// pays nothing. Both halves settle as a single charge against one
		/// `StorageLastChargedAt` marker, so an account is still billed at most
		/// once an hour.
		///
		/// Each half rounds up to whole GiB on its own, which is deliberate: the
		/// two byte counts come from different backends and are billed as separate
		/// line items, so a partial GiB on each side is a partial GiB of each.
		fn handle_hourly_storage_charging(current_block: BlockNumberFor<T>) -> Weight {
			let mut users_seen: u64 = 0;
			let mut users_charged_or_removed: u64 = 0;
			// Every user the validator metric has ever reported on. Both usage
			// extrinsics write all four Drive/S3 maps together, so this key set
			// covers S3-only users too — their Drive row is simply zero.
			for (user, drive_file_size) in UserTotalDriveFilesSize::<T>::iter() {
				users_seen = users_seen.saturating_add(1);

				// Which halves of the bill a subscription already covers.
				let subscriptions = UserAllSubscriptionPlans::<T>::get(&user);
				let mut has_drive_plan = false;
				let mut has_s3_plan = false;
				for sub in subscriptions.iter().filter(|sub| sub.active) {
					has_drive_plan |= sub.package.is_drive_plan();
					has_s3_plan |= sub.package.is_s3_plan;
				}

				// Both sides covered — nothing left to meter for this user.
				if has_drive_plan && has_s3_plan {
					continue;
				}

				// Check if the time difference is greater than 1 hour
				let last_charged_at = StorageLastChargedAt::<T>::get(user.clone());
				let block_difference = current_block.saturating_sub(last_charged_at);
				if block_difference > T::BlocksPerHour::get().into() {
					let s3_file_size = UserTotalS3FilesSize::<T>::get(&user).unwrap_or(0);

					let billable_drive = if has_drive_plan { 0 } else { drive_file_size };
					let billable_s3 = if has_s3_plan { 0 } else { s3_file_size };

					// Skip if no files to charge
					if billable_drive == 0 && billable_s3 == 0 {
						continue;
					}

					// Get the current price per GB from the marketplace pallet
					let price_per_gb = Self::get_price_per_gb();

					let user_free_credits = CreditsPallet::<T>::get_free_credits(&user);

					// Bill in whole GiB, rounding any partial GiB up.
					let charge_amount =
						times(Credits::new(price_per_gb), gibs_ceil(Bytes::new(billable_drive)))
							.get()
							.saturating_add(
								times(
									Credits::new(price_per_gb),
									gibs_ceil(Bytes::new(billable_s3)),
								)
								.get(),
							);

					if user_free_credits >= charge_amount {
						// Decrease user credits
						let charge_result = Self::consume_credits(user.clone(), charge_amount);

						if charge_result.is_ok() {
							// Commission on money actually collected, same rule
							// as the monthly path. Recorded only — the sweep
							// pays it, so the charge that earned it never waits
							// on the bank.
							Self::accrue_hourly_referral_commission(&user, charge_amount);

							let tx_result = Self::record_credits_transaction(
								&user,
								NativeTransactionType::Subscription,
								charge_amount.into(),
							);
							let ts_result = Self::update_storage_last_charged_at(&user);
							if tx_result.is_ok() && ts_result.is_ok() {
								users_charged_or_removed =
									users_charged_or_removed.saturating_add(1);
							}
						} else {
							log::warn!(
								target: "runtime::marketplace",
								"per-GB charge failed for {:?}: required={}, available={}",
								user,
								charge_amount,
								user_free_credits
							);
							Self::deposit_event(Event::PerGbChargeFailed {
								who: user.clone(),
								charge_amount,
								available_credits: user_free_credits,
							});
						}
					}
				}
			}

			// Conservative weight accounting:
			// - iterating `UserTotalDriveFilesSize` and reading
			//   `UserAllSubscriptionPlans`, `UserTotalS3FilesSize` and
			//   `StorageLastChargedAt` per user
			// - charged/removed paths do writes and call into other pallets
			// - a charged user also walks the referral lookup and writes one
			//   accrual row; paying it out is the sweep's weight, not ours
			let reads = users_seen
				.saturating_mul(5)
				.saturating_add(users_charged_or_removed.saturating_mul(13));
			let writes = users_charged_or_removed.saturating_mul(11);
			T::DbWeight::get().reads_writes(reads, writes)
		}

		/// Helper function to get the current price per GB
		pub fn get_price_per_gb() -> u128 {
			PricePerGbs::<T>::get()
		}

		/// Helper function to get the current price per GB
		pub fn get_price_per_bandwidth() -> u128 {
			PricePerBandwidth::<T>::get()
		}

		/// Cancel a specific subscription by ID (storage or compute).
		/// Marks the subscription inactive, refunds unused prepaid months.
		/// Sets the resubscribe cooldown only when no active subscriptions remain.
		fn do_cancel_subscription_by_id(
			account_id: &T::AccountId,
			subscription_id: SubscriptionId,
		) -> DispatchResult {
			let now = <frame_system::Pallet<T>>::block_number();
			let mut refund: u128 = 0;
			let mut cancelled = false;
			let mut was_storage = false;

			let before = UserAllSubscriptionPlans::<T>::get(account_id);
			let mut after = before.clone();
			for sub in after.iter_mut() {
				if sub.id == subscription_id && sub.active {
					refund = refund.saturating_add(Self::unused_prepaid_refund_credits(sub));
					was_storage = sub.package.is_any_storage();
					sub.active = false;
					cancelled = true;
					break; // Stop looping after finding the subscription
				}
			}

			ensure!(cancelled, Error::<T>::SubscriptionNotFound);

			// Drops the row along with its index key and anchor entry.
			Self::commit_subscriptions(account_id, &before, after);

			if refund > 0 {
				Self::refund_credits_with_batch(account_id, refund)?;
			}

			// Only set cooldown if no active subscriptions remain
			let has_active_subs =
				UserAllSubscriptionPlans::<T>::get(account_id).iter().any(|s| s.active);
			if !has_active_subs {
				LastSubscriptionCancelledAt::<T>::insert(account_id, now);
			}

			if was_storage {
				Self::deposit_event(Event::StorageSubscriptionCancelled {
					who: account_id.clone(),
				});
			} else {
				Self::deposit_event(Event::ComputeSubscriptionCancelled {
					who: account_id.clone(),
				});
			}
			Ok(())
		}

		/// Helper function to remove the last charged timestamp for a user
		pub fn remove_storage_last_charged_at(who: &T::AccountId) {
			// Remove the last charged timestamp for the user
			StorageLastChargedAt::<T>::remove(who);
		}

		/// Remove a specific account from BackupDeleteRequests if it exists
		pub fn remove_user_from_backup_delete_requests(user_id: &T::AccountId) {
			BackupDeleteRequests::<T>::mutate(|delete_requests| {
				delete_requests.retain(|user| user != user_id);
			});
		}

		/// Helper function to update the last charged timestamp for a user
		pub fn update_storage_last_charged_at(who: &T::AccountId) -> DispatchResult {
			// Get the current timestamp from the timestamp pallet
			let now = <frame_system::Pallet<T>>::block_number();

			// Update the last charged timestamp for the user
			StorageLastChargedAt::<T>::insert(who, now);

			Ok(())
		}

		/// Deposit credits and alpha into a new batch
		fn do_deposit(
			sender: T::AccountId,
			credit_amount: u128,
			alpha_amount: u128,
			freeze_for_chargeback: bool,
			code: Option<Vec<u8>>,
		) -> DispatchResult {
			let batch_id = NextBatchId::<T>::get();

			let release_time = if freeze_for_chargeback {
				let block_number = <frame_system::Pallet<T>>::block_number();
				block_number.saturating_add((15u32 * 28800u32).into()) // 15 days
			} else {
				<frame_system::Pallet<T>>::block_number() // No release time
			};

			let batch = Batch {
				owner: sender.clone(),
				credit_amount,
				alpha_amount,
				remaining_credits: credit_amount,
				remaining_alpha: alpha_amount,
				pending_alpha: 0,
				is_frozen: freeze_for_chargeback,
				release_time,
			};

			Batches::<T>::insert(batch_id, batch);
			UserBatches::<T>::append(&sender, batch_id);
			let next = batch_id.saturating_add(1);
			NextBatchId::<T>::put(next);

			AlphaBalances::<T>::mutate(&sender, |alpha| {
				*alpha = alpha.saturating_add(alpha_amount)
			});
			CreditsPallet::<T>::do_mint(sender.clone(), credit_amount, code)?;

			// Route the alpha backing of this deposit to the bank (miner
			// payment funds). Sourced from the marketplace sudo account; never
			// blocks the deposit itself if the transfer cannot be made.
			if alpha_amount > 0 {
				if let Some(sudo_account) = Self::sudo_key() {
					let backing: pallet_hippocampus::BalanceOf<T> = alpha_amount.saturated_into();
					match pallet_hippocampus::Pallet::<T>::deposit_from(
						&sudo_account,
						backing,
						pallet_hippocampus::DepositType::MarketplaceRevenue,
					) {
						Ok(()) => {
							TotalUndistributedBacking::<T>::mutate(|t| {
								*t = t.saturating_add(alpha_amount)
							});
						},
						Err(e) => {
							UnbackedBatchAlpha::<T>::insert(batch_id, alpha_amount);
							log::warn!(
								target: "runtime::marketplace",
								"deposit: routing alpha backing to bank failed: {:?}",
								e
							);
						},
					}
				} else {
					UnbackedBatchAlpha::<T>::insert(batch_id, alpha_amount);
					log::warn!(
						target: "runtime::marketplace",
						"deposit: no sudo key set, alpha backing not routed to bank"
					);
				}
			}

			Self::deposit_event(Event::BatchDeposited { owner: sender, batch_id });

			Ok(())
		}

		/// Consume user credits from their batches
		#[transactional]
		pub fn consume_credits(sender: T::AccountId, credits: u128) -> DispatchResult {
			let block_number = <frame_system::Pallet<T>>::block_number();
			// NOTE: referral discounts should NOT apply here, because this function is used for
			// recurring subscription charges and other billing paths (e.g. per-GB storage charging).
			//
			// Referral discount + referral rewards are handled explicitly at subscription purchase time.
			let mut remaining = credits;

			if let Some(batch_ids) = UserBatches::<T>::get(&sender) {
				for batch_id in batch_ids {
					if remaining == 0 {
						break;
					}

					if let Some(mut batch) = Batches::<T>::get(batch_id) {
						ensure!(batch.owner == sender, Error::<T>::NotAuthorized);

						let credits_to_take = remaining.min(batch.remaining_credits);
						let current = CreditsPallet::<T>::get_free_credits(&batch.owner);
						ensure!(current >= credits_to_take, Error::<T>::InsufficientFreeCredits);

						// Decrease user credits (post-discount total is allocated across batches).
						CreditsPallet::<T>::decrease_user_credits(&batch.owner, credits_to_take);

						// FIXED: Use remaining amounts for accurate alpha calculation
						// This ensures the ratio reflects the current batch state
						let credits_to_take_u256 = U256::from(credits_to_take);
						let remaining_alpha_u256 = U256::from(batch.remaining_alpha);
						let remaining_credits_u256 = U256::from(batch.remaining_credits.max(1));

						// Calculate alpha based on remaining proportion
						let alpha_to_release =
							(credits_to_take_u256 * remaining_alpha_u256) / remaining_credits_u256;

						// Safety: Ensure we don't release more alpha than remaining
						let alpha_to_release_u128 =
							alpha_to_release.min(U256::from(batch.remaining_alpha)).as_u128();

						// Update batch credits first (needed for future calculations in this batch)
						batch.remaining_credits =
							batch.remaining_credits.saturating_sub(credits_to_take);

						// Handle frozen/unfrozen logic
						if batch.is_frozen && block_number < batch.release_time {
							// Batch is still frozen - add to pending
							batch.pending_alpha =
								batch.pending_alpha.saturating_add(alpha_to_release_u128);
						} else {
							// Check if batch just unfroze in this block
							if batch.is_frozen && block_number >= batch.release_time {
								// Batch just unfroze - distribute all pending alpha first
								batch.is_frozen = false;

								if batch.pending_alpha > 0 {
									AlphaBalances::<T>::mutate(&batch.owner, |alpha| {
										*alpha = alpha.saturating_sub(batch.pending_alpha)
									});

									// Alpha stays in bank, no distribution. Track backed portion and release TUB.
									let backed =
										Self::take_backed_portion(batch_id, batch.pending_alpha);
									TotalUndistributedBacking::<T>::mutate(|t| {
										*t = t.saturating_sub(backed)
									});

									batch.pending_alpha = 0;
								}
							}

							// Release current alpha - stays in bank
							if alpha_to_release_u128 > 0 {
								AlphaBalances::<T>::mutate(&batch.owner, |alpha| {
									*alpha = alpha.saturating_sub(alpha_to_release_u128)
								});

								// Alpha stays in bank, no distribution. Track backed portion and release TUB.
								let backed =
									Self::take_backed_portion(batch_id, alpha_to_release_u128);
								TotalUndistributedBacking::<T>::mutate(|t| {
									*t = t.saturating_sub(backed)
								});
							}
						}

						// Update remaining alpha after all operations
						batch.remaining_alpha =
							batch.remaining_alpha.saturating_sub(alpha_to_release_u128);
						// Save updated batch
						Batches::<T>::insert(batch_id, batch);
						remaining = remaining.saturating_sub(credits_to_take);
					}
				}
			}

			ensure!(remaining == 0, Error::<T>::InsufficientFreeCredits);

			Self::deposit_event(Event::CreditsConsumed { owner: sender, credits });

			Ok(())
		}

		/// Consume a release of `released` alpha from `batch_id`: split off the
		/// portion whose backing actually reached the bank at deposit time and
		/// shrink the batch's unbacked marker accordingly. Unbacked alpha is
		/// consumed first, so [`TotalUndistributedBacking`] stays conservative
		/// (never undercounts) across partial releases.
		fn take_backed_portion(batch_id: u64, released: u128) -> u128 {
			let unbacked = UnbackedBatchAlpha::<T>::get(batch_id);
			if unbacked == 0 {
				return released;
			}
			let consumed = unbacked.min(released);
			if consumed == unbacked {
				UnbackedBatchAlpha::<T>::remove(batch_id);
			} else {
				UnbackedBatchAlpha::<T>::insert(batch_id, unbacked.saturating_sub(consumed));
			}
			released.saturating_sub(consumed)
		}

		/// Handle chargeback for a specific batch
		fn handle_chargeback(batch_id: u64) -> DispatchResult {
			// Get the batch from storage
			if let Some(batch) = Batches::<T>::get(batch_id) {
				// Ensure the batch is frozen and the chargeback is valid
				ensure!(
					batch.is_frozen
						&& <frame_system::Pallet<T>>::block_number() < batch.release_time,
					"Invalid chargeback"
				);

				// Decrease the total locked Alpha by the remaining Alpha in the batch
				let total_alpha_to_remove =
					batch.remaining_alpha.saturating_add(batch.pending_alpha);

				if total_alpha_to_remove > 0 {
					AlphaBalances::<T>::mutate(&batch.owner, |alpha| {
						*alpha = alpha.saturating_sub(total_alpha_to_remove);
					});

					// The reversed batch's backing will never be released to
					// the pots: its backed portion (the part that actually
					// reached the bank) stops counting as owed and is refunded
					// from the bank to the sudo account (the buyer is refunded
					// off-chain from there); the unbacked portion just cancels
					// the routing debt recorded at deposit time. Whatever the
					// bank cannot deliver right now — partial pay, rejection,
					// or no sudo key — stays tracked in PendingSudoRefunds
					// (walled off from miner settlement) and is folded into
					// the next chargeback's refund. A chargeback must never
					// fail on bank state.
					let backed = Self::take_backed_portion(batch_id, total_alpha_to_remove);
					TotalUndistributedBacking::<T>::mutate(|t| *t = t.saturating_sub(backed));
					let refund_owed = backed.saturating_add(PendingSudoRefunds::<T>::take());
					let refunded: u128 = if let Some(sudo_account) = Self::sudo_key() {
						if refund_owed > 0 {
							match pallet_hippocampus::Pallet::<T>::request_payment(
								&Self::account_id(),
								&sudo_account,
								refund_owed.saturated_into(),
							) {
								Ok(p) => p.saturated_into(),
								Err(e) => {
									log::warn!(
										target: "runtime::marketplace",
										"chargeback {}: bank rejected refund to sudo: {:?}",
										batch_id,
										e
									);
									0
								},
							}
						} else {
							0
						}
					} else {
						log::warn!(
							target: "runtime::marketplace",
							"chargeback {}: no sudo key, {} backing kept pending in the bank",
							batch_id,
							refund_owed
						);
						0
					};
					let pending = refund_owed.saturating_sub(refunded);
					if pending > 0 {
						log::warn!(
							target: "runtime::marketplace",
							"chargeback {}: refunded {} of {}, remainder kept pending",
							batch_id,
							refunded,
							refund_owed
						);
						PendingSudoRefunds::<T>::put(pending);
					}
				}

				// Remove the batch from the user's batch list
				UserBatches::<T>::mutate(&batch.owner, |batches| {
					if let Some(ref mut batch_vec) = batches {
						batch_vec.retain(|&id| id != batch_id);
					}
				});

				// Remove the batch from storage
				Batches::<T>::remove(batch_id);

				// Burn credits from batch.owner (implementation of burning logic needed)
				let credit_to_burn = batch.remaining_credits;
				let current = CreditsPallet::<T>::get_free_credits(&batch.owner);
				ensure!(current >= credit_to_burn, Error::<T>::InsufficientFreeCredits);
				CreditsPallet::<T>::decrease_user_credits(&batch.owner, credit_to_burn);
				TotalCreditsPurchased::<T>::mutate(|total| {
					*total = total.saturating_sub(credit_to_burn)
				});
			}

			Ok(())
		}

		pub fn get_batches_for_user(
			user: T::AccountId,
		) -> Vec<Batch<T::AccountId, BlockNumberFor<T>>> {
			let batch_ids: Vec<u64> = UserBatches::<T>::get(user).unwrap_or_default();
			batch_ids.iter().filter_map(|id| Batches::<T>::get(*id)).collect()
		}

		pub fn get_batch_by_id(batch_id: u64) -> Option<Batch<T::AccountId, BlockNumberFor<T>>> {
			Batches::<T>::get(batch_id)
		}
	}
}

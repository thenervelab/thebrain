#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;
pub use types::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

mod migrations;
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
    use frame_support::traits::Len;
    use frame_support::{
        pallet_prelude::*,
        traits::OnRuntimeUpgrade,
        traits::StorageVersion,
        traits::{ReservableCurrency, Currency},
        transactional,
        PalletId,
    };
    use frame_system::pallet_prelude::*;
    use sp_runtime::{
        traits::{AtLeast32BitUnsigned, AccountIdConversion, Hash, SaturatedConversion},
        Saturating,
    };
    use pallet_registration::BalanceOf;
    use pallet_registration::Pallet as RegistrationPallet;
    use pallet_registration::NodeType;
    use pallet_credits::Pallet as CreditsPallet;
    use pallet_utils::SubscriptionId;
    use sp_core::H256;
    use sp_std::{vec, vec::Vec};
    use num_traits::float::FloatCore;
    use pallet_rankings::Pallet as RankingsPallet;
    use pallet_credits::AlphaBalances;
    use pallet_credits::TotalCreditsPurchased;
    use frame_system::offchain::SendTransactionTypes;
    use frame_system::offchain::AppCrypto;
    use frame_system::offchain::SendUnsignedTransaction;
    use frame_support::traits::ExistenceRequirement;
    use sp_runtime::traits::Zero;
    use sp_core::U256;
    use sp_runtime::traits::Bounded;
	#[pallet::pallet]
	#[pallet::without_storage_info]
    #[pallet::storage_version(STORAGE_VERSION)]
	pub struct Pallet<T>(_);

    /// The current storage version.
    const STORAGE_VERSION: StorageVersion = StorageVersion::new(1);

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
                weight_used = weight_used.saturating_add(
                    T::DbWeight::get().reads_writes(1, result.unique as u64),
                );
            }

            // Only execute on blocks divisible by the configured interval
            if current_block % T::BlockChargeCheckInterval::get().into() == 0u32.into() {
                weight_used = weight_used.saturating_add(Self::handle_arion_storage_charging(current_block));
                // Monthly subscription charging:
                // - recurring charges happen only on the 1st day of the calendar month
                // - guarded by a unix-day marker so we only run once per day even if multiple blocks hit this interval
                if let Some(unix_day) = Self::should_run_monthly_subscription_charge() {
                    weight_used = weight_used.saturating_add(Self::handle_all_subscription_charging(current_block));
                    // Mark as run for today only after charging completes, to allow retry
                    // if future changes introduce abortable failure paths.
                    LastMonthlySubscriptionChargeDay::<T>::put(unix_day);
                }
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
    }

	// const LOCK_BLOCK_EXPIRATION: u32 = 3;
    // const LOCK_TIMEOUT_EXPIRATION: u32 = 10000;

    #[pallet::storage]
    #[pallet::getter(fn plans)]
    pub type Plans<T: Config> = StorageMap<_, Blake2_128Concat, T::Hash, Plan<T::Hash>, OptionQuery>;

    #[pallet::storage]
    pub(super) type PricePerGbs<T: Config> = StorageValue<_, u128, ValueQuery>;

    #[pallet::storage]
    pub(super) type PricePerBandwidth<T: Config> = StorageValue<_, u128, ValueQuery>;

    /// Storage to track the last charged timestamp for each user
    #[pallet::storage]
    pub(super) type StorageLastChargedAt<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        BlockNumberFor<T>,
        ValueQuery
    >;

    #[pallet::storage]
    #[pallet::getter(fn user_plan_subscription)]
    pub(super) type UserPlanSubscriptions<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        UserPlanSubscription<T>,
        OptionQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn user_all_subscription_plans)]
    pub(super) type UserAllSubscriptionPlans<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        Vec<UserPlanSubscription<T>>,
        ValueQuery,
    >;

    // Storage for OS Disk Image URLs
	#[pallet::storage]
	#[pallet::getter(fn os_disk_image_urls)]
	pub type OSDiskImageUrls<T: Config> = StorageMap<
		_, 
		Blake2_128Concat, 
		Vec<u8>,
		ImageDetails,
	>;

    // Map for batches identified by a unique ID
    #[pallet::storage]
    #[pallet::getter(fn batches)]
    pub type Batches<T: Config> = StorageMap<
        _, 
        Blake2_128Concat, 
        u64, 
        Batch<T::AccountId, BlockNumberFor<T>>
    >;

    // Map for user batches identified by AccountId
    #[pallet::storage]
    #[pallet::getter(fn user_batches)]
    pub type UserBatches<T: Config> = StorageMap<
        _, 
        Blake2_128Concat, 
        T::AccountId, 
        Vec<u64>
    >;

    #[pallet::storage]
    #[pallet::getter(fn is_storage_operations_enabled)]
    pub type IsStorageOperationsEnabled<T: Config> = StorageValue<_, bool, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn is_purchase_plan_enabled)]
    pub type IsPurchasePlanEnabled<T: Config> = StorageValue<_, bool, ValueQuery, GetDefault>;

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
    pub(super) type CdnLocations<T: Config> = StorageMap<_, Blake2_128Concat, u32, CdnLocation, OptionQuery>;

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
    pub(super) type NextTransactionId<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

    #[pallet::storage]
    #[pallet::getter(fn backup_enabled_users)]
    pub(super) type BackupEnabledUsers<T: Config> = StorageValue<
        _,
        Vec<T::AccountId>,
        ValueQuery,
    >;

    #[pallet::storage]
    #[pallet::getter(fn backup_delete_requests)]
    pub(super) type BackupDeleteRequests<T: Config> = StorageValue<
        _,
        Vec<T::AccountId>,
        ValueQuery,
    >;  

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
        CdnLocationAdded { id: u32 },
        /// Auto-renewal status updated
        AutoRenewalUpdated { who: T::AccountId, subscription_id: SubscriptionId, enabled: bool },
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
        PointTransactionRecorded { who: T::AccountId, transaction_type: NativeTransactionType, amount: Points },
        PlanPurchased {
            caller: T::AccountId,
            owner: T::AccountId,
            plan_id: T::Hash,
            location_id: Option<u32>,
            selected_image_name: Option<Vec<u8>>,
            cloud_init_cid: Option<Vec<u8>>,
        },
        PlanPurchaseFailed {
            caller: T::AccountId,
            owner: T::AccountId,
            plan_id: T::Hash,
            error: DispatchError,
        },
        PricePerGbUpdated { price: u128 },
        PricePerBandwidthUpdated { price: u128 },
        StorageSubscriptionCancelled { who: T::AccountId },
        ComputeSubscriptionCancelled { who: T::AccountId },
        BackupEnabled { 
            caller: T::AccountId,
            account: T::AccountId 
        },
        BackupDisabled { 
            caller: T::AccountId,
            account: T::AccountId 
        },
        OSDiskImageUrlSet {
			os_name: Vec<u8>,
			url: Vec<u8>,
		},
        PlanPriceUpdated(T::Hash, u128),
        /// Specific miner request fee updated
        SpecificMinerRequestFeeUpdated { fee: BalanceOf<T> },
        BatchDeposited { owner: T::AccountId, batch_id: u64 },
        DepositFailed {
            authority: T::AccountId,
            account: T::AccountId,
            error: DispatchError,
        },
        CreditsConsumed { owner: T::AccountId, credits: u128 },
        /// Monthly subscription charge could not be collected; subscriptions were deactivated.
        SubscriptionChargeFailed {
            who: T::AccountId,
            required_credits: u128,
            available_credits: u128,
        },
	    StorageOperationsStatusChanged { enabled: bool },
        /// Purchase plan status was changed
        PurchasePlanStatusChanged { enabled: bool },
        StoragePricePerMinerUpdated { price: u128 },
        /// Per-GB storage charging failed after passing the FreeCredits guard.
        PerGbChargeFailed { who: T::AccountId, charge_amount: u128, available_credits: u128 },
        /// Referral reward (credits) could not be minted to the referrer.
        ReferralRewardMintFailed { referrer: T::AccountId, reward_credits: u128 },
		/// User Drive + S3 usage metrics were updated by a validator.
		UserBackendFilesUpdated {
			user: T::AccountId,
			drive_size: u128,
			drive_count: u128,
			s3_size: u128,
			s3_count: u128,
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
		TooManyUpdates,
	}
    
	#[pallet::storage]
	#[pallet::getter(fn user_requests_count)]
	pub type UserRequestsCount<T: Config> = StorageMap<
		_, 
		Blake2_128Concat, 
		T::AccountId, 
		u32, 
		ValueQuery
	>;

    #[pallet::storage]
    #[pallet::getter(fn sudo_key)]
    pub type SudoKey<T: Config> = StorageValue<_, Option<T::AccountId>, ValueQuery>;

    /// Unix-day marker (unix_ms / 86_400_000) of the last time we ran the monthly subscription charge.
    ///
    /// This prevents double-charging on the 1st of the month when `on_initialize` runs multiple times.
    #[pallet::storage]
    #[pallet::getter(fn last_monthly_subscription_charge_day)]
    pub type LastMonthlySubscriptionChargeDay<T: Config> = StorageValue<_, u32, ValueQuery>;
    
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
            storage_limit: Option<u128>,
        ) -> DispatchResult {
            // Ensure the caller is sudo
            ensure_root(origin)?;

            // Generate a unique ID for the plan (you can use a counter or a random hash)
            let plan_id = T::Hashing::hash_of(&plan_name); // Example way to generate a unique ID
            ensure!(
                !Plans::<T>::contains_key(&plan_id),
                Error::<T>::PlanAlreadyExists
            );

            // Create the plan object
            let new_plan = Plan {
                id: plan_id.clone(),
                plan_name: plan_name.clone(),
                plan_description,
                plan_technical_description,
                is_suspended: false, 
                price,
                is_storage_plan,
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
            selected_image_names: Vec<Option<Vec<u8>>>,
            cloud_init_cids: Option<Vec<Option<Vec<u8>>>>,
            pay_upfront: Option<u128>
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;

            // Check if the caller is a whitelisted caller
            let allowed = WhitelistedCallers::<T>::get();
            ensure!(allowed.contains(&who), Error::<T>::SubscriptionCancellationNotAuthorized);
        
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

            ensure!(
                selected_image_names.len() == plan_ids.len(),
                Error::<T>::InvalidInput
            );
            if let Some(ref xs) = location_ids {
                ensure!(xs.len() == plan_ids.len(), Error::<T>::InvalidInput);
            }
            if let Some(ref xs) = cloud_init_cids {
                ensure!(xs.len() == plan_ids.len(), Error::<T>::InvalidInput);
            }

            // Initialize default values for optional parameters
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
                if plan.is_storage_plan {
                    // Handle storage plan purchase
                    Self::do_purchase_storage_plan(
                        owner.clone(),
                        plan_id,
                        pay_upfront
                    )?;
                } else {
                    // For compute plans, image name is required
                    let image_name = selected_image_names[i]
                        .clone()
                        .ok_or(Error::<T>::InvalidImageSelection)?;
                    // Handle compute plan purchase
                    Self::do_purchase_compute_plan(
                        owner.clone(),
                        plan_id,
                        location_ids[i],
                        image_name,
                        cloud_init_cids[i].clone(),
                        pay_upfront
                    )?;
                };

                successful_purchases.push(plan_id);
                // Emit event for successful purchase
                Self::deposit_event(Event::PlanPurchased {
                    caller: owner.clone(),
                    owner: owner.clone(),
                    plan_id,
                    location_id: location_ids[i],
                    selected_image_name: selected_image_names[i].clone(),
                    cloud_init_cid: cloud_init_cids[i].clone(),
                });
            }

            // If we had any successful purchases, we consider the call successful
            Ok(())
        }

        /// Sudo function to set the price per GB for storage
        #[pallet::call_index(8)]
        #[pallet::weight((10_000, Pays::No))]
        pub fn set_price_per_gb(
            origin: OriginFor<T>,
            price: u128,
        ) -> DispatchResult {
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
        pub fn set_bandwidth_price(
            origin: OriginFor<T>,
            price: u128,
        ) -> DispatchResult {
            // Ensure the caller is sudo
            ensure_root(origin)?;

            // Set the price per GB
            PricePerBandwidth::<T>::put(price);

            // Emit an event for the price update
            Self::deposit_event(Event::PricePerBandwidthUpdated { price });

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
            description: Vec<u8>
        ) -> DispatchResultWithPostInfo {
            // Ensure only sudo can set the URL
            ensure_root(origin)?;

            // Validate the URL (basic check)
            ensure!(!url.is_empty(), Error::<T>::InvalidOSDiskImageUrl);

            let os_details = ImageDetails {
                url,
                name,
                description
            };

            // Store the URL for the specified OS
            OSDiskImageUrls::<T>::insert(os_name.clone(), os_details.clone());

            // Emit an event
            Self::deposit_event(Event::OSDiskImageUrlSet { 
                os_name, 
                url: os_details.url 
            });

            Ok(().into())
        }
        
        /// Set the specific miner request fee
        #[pallet::call_index(12)]
        #[pallet::weight((0, Pays::No))]
        pub fn set_specific_miner_request_fee(
            origin: OriginFor<T>,   
            fee: BalanceOf<T>
        ) -> DispatchResult {
            // Ensure the caller is an authority
            let authority = ensure_signed(origin)?;

            // Rate limit: maximum storage requests per block per user
			let max_requests_per_block = T::MaxRequestsPerBlock::get();
			let user_requests_count = UserRequestsCount::<T>::get(&authority);
			ensure!(user_requests_count.saturating_add(1) <= max_requests_per_block, Error::<T>::TooManyRequests);

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
            code: Option<Vec<u8>>
        ) -> DispatchResult {
			let authority = ensure_signed(origin)?;
            CreditsPallet::<T>::ensure_is_authority(&authority)?;

            // Call the existing deposit function
            Self::do_deposit(account, credit_amount, alpha_amount, freeze_for_chargeback, code)?;
            Ok(())
        }


        #[pallet::call_index(15)]
        #[pallet::weight((0, Pays::No))]
        pub fn chargeback(
            origin: OriginFor<T>, 
            batch_id: u64
        ) -> DispatchResult {
            // Ensure the caller is a signed origin (admin check)
            ensure_root(origin)?;

            // Call the existing handle_chargeback function
            Self::handle_chargeback(batch_id)
        }

        #[pallet::call_index(16)]
        #[pallet::weight((0, Pays::No))]
        pub fn set_sudo_key(
            origin: OriginFor<T>, 
            new_sudo_key: T::AccountId
        ) -> DispatchResult {
            // Ensure that the caller is the sudo account
            ensure_root(origin)?;
    
            // Set the new sudo key in storage
            SudoKey::<T>::put(Some(new_sudo_key));
    
            Ok(())
        }

        #[pallet::call_index(17)]
        #[pallet::weight((0, Pays::No))]
        pub fn sudo_set_storage_operations(
            origin: OriginFor<T>,
            enabled: bool
        ) -> DispatchResult {
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
            enabled: bool
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
        pub fn sudo_set_subscription_canceller(
            origin: OriginFor<T>,
            account: T::AccountId,
        ) -> DispatchResult {
            ensure_root(origin)?;
            let mut allowed_accounts = WhitelistedCallers::<T>::get();
            if !allowed_accounts.contains(&account) {
                allowed_accounts.push(account);
            }
            WhitelistedCallers::<T>::put(allowed_accounts);
            Ok(())
        }

        /// Root removes an account from the subscription canceller list.
        #[pallet::call_index(20)]
        #[pallet::weight((0, Pays::No))]
        pub fn sudo_remove_subscription_canceller(
            origin: OriginFor<T>,
            account: T::AccountId,
        ) -> DispatchResult {
            ensure_root(origin)?;
            let mut allowed_accounts = WhitelistedCallers::<T>::get();
            allowed_accounts.retain(|x| x != &account);
            WhitelistedCallers::<T>::put(allowed_accounts);
            Ok(())
        }

        /// Cancel a user's subscription (restricted to whitelisted callers).
        ///
        /// - If `subscription_id` is `Some(id)`: cancels (marks inactive) that specific subscription
        ///   (storage or compute), refunds unused prepaid months.
        /// - If `subscription_id` is `None`: deletes all storage subscriptions (legacy behavior),
        ///   refunds unused prepaid months.
        #[pallet::call_index(21)]
        #[pallet::weight((0, Pays::No))]
        pub fn cancel_user_subscription(
            origin: OriginFor<T>,
            user: T::AccountId,
            subscription_id: Option<SubscriptionId>,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;

            let allowed = WhitelistedCallers::<T>::get();
            ensure!(allowed.contains(&who), Error::<T>::SubscriptionCancellationNotAuthorized);

            match subscription_id {
                Some(id) => Self::do_cancel_subscription_by_id(&user, id),
                None => Self::do_delete_storage_subscription_with_refund(&user),
            }
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
			ensure!(
				allowed.contains(&who),
				Error::<T>::SubscriptionCancellationNotAuthorized
			);

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

			let allowed = WhitelistedCallers::<T>::get();
			ensure!(
				allowed.contains(&who),
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
        pub fn set_storage_price_per_miner(
            origin: OriginFor<T>,
            price: u128,
        ) -> DispatchResult {
            ensure_root(origin)?;
            StoragePricePerMiner::<T>::put(price);
            Self::deposit_event(Event::StoragePricePerMinerUpdated { price });
            Ok(())
        }
	}

    impl<T: Config> Pallet<T> {
        fn try_mint_referral_reward_credits(
            referrer: &T::AccountId,
            reward_credits: u128,
        ) {
            if reward_credits == 0 {
                return;
            }

            // Credits must be spendable; mint them into a new batch (alpha reward intentionally 0).
            if let Err(e) = Self::do_deposit(referrer.clone(), reward_credits, 0, false, None) {
                log::warn!(
                    target: "runtime::marketplace",
                    "referral reward mint failed for {:?}: {:?}",
                    referrer,
                    e
                );
                Self::deposit_event(Event::<T>::ReferralRewardMintFailed {
                    referrer: referrer.clone(),
                    reward_credits,
                });
            }
        }

        fn referral_discount_and_owner(who: &T::AccountId, face_credits: u128) -> (u128, Option<T::AccountId>) {
            if let Some(ref_code) = CreditsPallet::<T>::referred_users(who) {
                let discount = face_credits.saturating_mul(5) / 100u128;
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

            sub.paid_per_month.saturating_mul(months_remaining)
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

        /// Restricted canceller path: remove storage subscriptions from storage (delete rows),
        /// and refund unused prepaid months for any removed *active* storage subs.
        fn do_delete_storage_subscription_with_refund(account_id: &T::AccountId) -> DispatchResult {
            let now = <frame_system::Pallet<T>>::block_number();
            let mut refunded: u128 = 0;
            let mut removed_any = false;

            UserAllSubscriptionPlans::<T>::mutate(account_id, |subscriptions| {
                let original_len = subscriptions.len();

                for sub in subscriptions.iter() {
                    if sub.package.is_storage_plan && sub.active {
                        refunded = refunded.saturating_add(Self::unused_prepaid_refund_credits(sub));
                    }
                }

                *subscriptions = subscriptions
                    .iter()
                    .filter(|sub| !sub.package.is_storage_plan)
                    .cloned()
                    .collect();

                removed_any = subscriptions.len() < original_len;
            });

            ensure!(removed_any, Error::<T>::NoActiveSubscription);

            if refunded > 0 {
                Self::refund_credits_with_batch(account_id, refunded)?;
            }

            LastSubscriptionCancelledAt::<T>::insert(account_id, now);
            Self::deposit_event(Event::StorageSubscriptionCancelled { who: account_id.clone() });
            Ok(())
        }

        /// Returns `Some(unix_day)` when monthly charging should run.
        ///
        /// IMPORTANT: this function is side-effect free; the caller should write the day marker
        /// only after the charging work completes, to avoid skipping the month if charging aborts.
        fn should_run_monthly_subscription_charge() -> Option<u32> {
            let today = Self::current_unix_day();
            if LastMonthlySubscriptionChargeDay::<T>::get() == today {
                return None;
            }

            // Catch-up behavior: if the chain misses the 1st-of-month tick, run on the first
            // available day after restart as long as there exist due subscriptions.
            //
            // Legacy subs (`next_charge_unix_day == None`) are considered due from the 1st of the
            // current month.
            let current_month_first = pallet_calendar::Pallet::<T>::unix_day_of_first_of_month_in(0);
            let any_due = UserAllSubscriptionPlans::<T>::iter().any(|(_who, subs)| {
                subs.iter().any(|sub| {
                    if !sub.active {
                        return false;
                    }
                    match sub.next_charge_unix_day {
                        Some(d) => today >= d,
                        None => current_month_first > 0 && today >= current_month_first,
                    }
                })
            });

            if any_due { Some(today) } else { None }
        }

        /// Helper function to get the current price per GB
        pub fn get_storage_price_per_miner() -> u128 {
            StoragePricePerMiner::<T>::get()
        }
                    
        fn prorated_monthly_price(monthly_price: u128) -> u128 {
            // days_remaining_in_current_month is inclusive of today.
            let dim: u128 = pallet_calendar::Pallet::<T>::days_in_current_month() as u128;
            let drm: u128 = pallet_calendar::Pallet::<T>::days_remaining_in_current_month() as u128;
            if dim == 0 {
                return monthly_price;
            }
            // Use ceil division to avoid rounding tiny (but non-zero) prices down to 0.
            let numerator = monthly_price.saturating_mul(drm);
            numerator
                .saturating_add(dim.saturating_sub(1))
                / dim
        }

        /// Total upfront charge when buying `upfront_months` starting mid-month:
        /// `1 prorated month + (upfront_months - 1) full months`.
        fn upfront_prorated_total(monthly_price: u128, upfront_months: u128) -> u128 {
            let first = Self::prorated_monthly_price(monthly_price);
            let remaining_full_months = upfront_months.saturating_sub(1);
            first.saturating_add(monthly_price.saturating_mul(remaining_full_months))
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

                Self::distribute_alpha(
                    batch.pending_alpha,
                    RankingsPallet::<T>::account_id(),
                    Self::account_id(),
                )?;

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
            let eras_balance: BalanceOf<T> = (eras_in_30_days as u128)
                .try_into()
                .unwrap_or_default();
            // Distribution amount per era
            total_amount / eras_balance
        }

        fn do_purchase_storage_plan(
            who: T::AccountId,
            plan_id: T::Hash,
            pay_upfront: Option<u128>
        ) -> DispatchResult {
            // Check if the ComputeMiner node type is disabled
            ensure!(
                !RegistrationPallet::<T>::is_node_type_disabled(NodeType::StorageMiner),
                Error::<T>::NodeTypeDisabled
            );

            // Check if storage operations are enabled
            ensure!(
                Self::is_purchase_plan_enabled(),
                Error::<T>::PlanOperationDisabled
            );

            // Enforce: only one active storage subscription at a time.
            let existing_subscriptions = UserAllSubscriptionPlans::<T>::get(&who);
            ensure!(
                !existing_subscriptions
                    .iter()
                    .any(|s| s.active && s.package.is_storage_plan),
                Error::<T>::AlreadyHasActiveSubscription
            );

            // Check if plan exists
            let plan = Plans::<T>::get(&plan_id).ok_or(Error::<T>::PlanNotFound)?;

            ensure!(!plan.is_suspended, Error::<T>::PlanSuspended);

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

            Self::consume_credits(
                who.clone(),
                charged_credits,
                Self::account_id().clone(),
                pallet_rankings::Pallet::<T>::account_id().clone(),
            )?;
                    
            // Record transaction
            Self::record_credits_transaction(
                &who,
                NativeTransactionType::Subscription,
                charged_credits.into(),
            )?;

            // Mint referral reward credits to the referral owner (no alpha rewards).
            if let Some(owner) = ref_owner {
                Self::try_mint_referral_reward_credits(&owner, referral_discount);
            }

            // Create subscription (simplified due to removed plan_type)
            let paid_per_month = if CreditsPallet::<T>::referred_users(&who).is_some() {
                plan.price.saturating_mul(9_500u128) / 10_000u128
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
        
            // Get existing subscriptions or create a new vector if none exist
            let mut subscriptions = UserAllSubscriptionPlans::<T>::get(&who);
            let active_count = subscriptions.iter().filter(|s| s.active).count() as u32;
            ensure!(
                active_count < T::MaxActiveSubscriptions::get(),
                Error::<T>::TooManyActiveSubscriptions
            );

            // Add the new subscription
            subscriptions.push(subscription);

            // Save the updated subscriptions list
            UserAllSubscriptionPlans::<T>::insert(&who, subscriptions);

            Ok(())
        }

        fn do_purchase_compute_plan(
            who: T::AccountId,
            plan_id: T::Hash,
            location_id: Option<u32>,
            selected_image_name: Vec<u8>,
            cloud_init_cid: Option<Vec<u8>>,
            pay_upfront: Option<u128>
        ) -> DispatchResult {
            // Check if the ComputeMiner node type is disabled
            ensure!(
                !RegistrationPallet::<T>::is_node_type_disabled(NodeType::ComputeMiner),
                Error::<T>::NodeTypeDisabled
            );

            // Check if plan exists
            let plan = Plans::<T>::get(&plan_id).ok_or(Error::<T>::PlanNotFound)?;

            ensure!(!plan.is_suspended, Error::<T>::PlanSuspended);

            // Check if the selected image name exists in OSDiskImageUrls storage
            ensure!(
                Self::os_disk_image_urls(selected_image_name.clone()).is_some(), 
                Error::<T>::InvalidImageSelection
            );

            let image_url = Self::os_disk_image_urls(selected_image_name.clone()).unwrap();

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

            Self::consume_credits(
                who.clone(),
                charged_credits,
                Self::account_id().clone(),
                pallet_rankings::Pallet::<T, pallet_rankings::Instance2>::account_id().clone(),
            )?;
                    
            // Record transaction
            Self::record_credits_transaction(
                &who,
                NativeTransactionType::Subscription,
                charged_credits.into(),
            )?;

            // Mint referral reward credits to the referral owner (no alpha rewards).
            if let Some(owner) = ref_owner {
                Self::try_mint_referral_reward_credits(&owner, referral_discount);
            }

            // Create subscription (simplified due to removed plan_type)
            let paid_per_month = if CreditsPallet::<T>::referred_users(&who).is_some() {
                plan.price.saturating_mul(9_500u128) / 10_000u128
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
                selected_image_name : Some(selected_image_name),
                next_charge_unix_day,
                paid_per_month,
                _phantom: PhantomData,
            };
        
            // Get existing subscriptions or create a new vector if none exist
            let mut subscriptions = UserAllSubscriptionPlans::<T>::get(&who);
            let active_count = subscriptions.iter().filter(|s| s.active).count() as u32;
            ensure!(
                active_count < T::MaxActiveSubscriptions::get(),
                Error::<T>::TooManyActiveSubscriptions
            );

            // Add the new subscription
            subscriptions.push(subscription);

            // Save the updated subscriptions list
            UserAllSubscriptionPlans::<T>::insert(&who, subscriptions);

            Ok(())
        }

        fn record_credits_transaction(
            who: &T::AccountId,
            transaction_type: NativeTransactionType,
            amount: Points,
        ) -> DispatchResult {
            let transaction_id = NextTransactionId::<T>::try_mutate(who, |id| -> Result<u32, DispatchError> {
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
        fn handle_all_subscription_charging(current_block: BlockNumberFor<T>) -> Weight {
            let mut users_seen: u64 = 0;
            let mut users_with_active_subs: u64 = 0;
            let mut users_charged_or_cancelled: u64 = 0;
            let today = Self::current_unix_day();
            // Cap catch-up to avoid heavy loops after long downtime.
            let max_catchup_months: u32 = 3;

            // Iterate through all users with subscriptions once
            for (account_id, subscriptions) in UserAllSubscriptionPlans::<T>::iter() {
                users_seen = users_seen.saturating_add(1);
                // Work on a local copy so we can do catch-up charging deterministically.
                let mut subs = subscriptions;

                // If no active subs, skip.
                if !subs.iter().any(|s| s.active) {
                    continue;
                }
                users_with_active_subs = users_with_active_subs.saturating_add(1);

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
                        .filter(|sub| sub.package.is_storage_plan)
                        .filter(due)
                        .cloned()
                        .collect();
                    let compute_subs_to_charge: Vec<_> = active_subs
                        .iter()
                        .filter(|sub| !sub.package.is_storage_plan)
                        .filter(due)
                        .cloned()
                        .collect();

                    // Nothing due (or nothing active) → stop catch-up for this account.
                    if storage_subs_to_charge.is_empty() && compute_subs_to_charge.is_empty() {
                        break;
                    }

                    // Calculate total charge amounts (one month worth for each due sub).
                    let mut total_storage_charge = 0u128;
                    let mut total_compute_charge = 0u128;

                    for sub in &storage_subs_to_charge {
                        total_storage_charge = total_storage_charge.saturating_add(sub.package.price);
                    }
                    for sub in &compute_subs_to_charge {
                        total_compute_charge = total_compute_charge.saturating_add(sub.package.price);
                    }

                    if total_storage_charge == 0 && total_compute_charge == 0 {
                        break;
                    }
                    users_charged_or_cancelled = users_charged_or_cancelled.saturating_add(1);

                    // Charge storage and compute independently; deactivate only the failed side.
                    let mut storage_ok = total_storage_charge == 0;
                    let mut compute_ok = total_compute_charge == 0;

                    let storage_available_at_attempt =
                        CreditsPallet::<T>::get_free_credits(&account_id);
                    if total_storage_charge > 0 {
                        if storage_available_at_attempt >= total_storage_charge {
                            storage_ok = Self::consume_credits(
                                account_id.clone(),
                                total_storage_charge,
                                Self::account_id().clone(),
                                pallet_rankings::Pallet::<T>::account_id().clone(),
                            )
                            .and_then(|_| {
                                Self::record_credits_transaction(
                                    &account_id,
                                    NativeTransactionType::Subscription,
                                    total_storage_charge.into(),
                                )
                            })
                            .is_ok();
                        } else {
                            storage_ok = false;
                        }
                    }

                    // Charge compute subscriptions individually in ascending order of subscription ID
                    let mut compute_charged_total = 0u128;
                    let mut compute_subs_to_deactivate = Vec::new();
                    let mut compute_available = CreditsPallet::<T>::get_free_credits(&account_id);
                    let mut sorted_compute_subs = compute_subs_to_charge.clone();
                    sorted_compute_subs.sort_by_key(|sub| sub.id);
                    for sub in &sorted_compute_subs {
                        let price = sub.package.price;
                        if compute_available >= price {
                            if Self::consume_credits(
                                account_id.clone(),
                                price,
                                Self::account_id().clone(),
                                pallet_rankings::Pallet::<T, pallet_rankings::Instance2>::account_id().clone(),
                            )
                            .and_then(|_| {
                                Self::record_credits_transaction(
                                    &account_id,
                                    NativeTransactionType::Subscription,
                                    price.into(),
                                )
                            })
                            .is_ok() {
                                compute_charged_total = compute_charged_total.saturating_add(price);
                                compute_available = compute_available.saturating_sub(price);
                                // Mark as charged below
                            } else {
                                compute_subs_to_deactivate.push(sub.id);
                            }
                        } else {
                            compute_subs_to_deactivate.push(sub.id);
                        }
                    }
                    compute_ok = compute_subs_to_deactivate.is_empty();

                    // Update successfully charged subscriptions (only those due),
                    // advancing from the previous `next_charge_unix_day`, not from "now".
                    for sub in subs.iter_mut() {
                        if !sub.active {
                            continue;
                        }
                        if !sub.next_charge_unix_day.map_or(true, |d| today >= d) {
                            continue;
                        }

                        let ok_for_type =
                            if sub.package.is_storage_plan { storage_ok } else { 
                                !compute_subs_to_deactivate.contains(&sub.id)
                            };
                        if ok_for_type {
                            sub.last_charged_at = current_block;
                            let prev_next = sub.next_charge_unix_day.unwrap_or(today);
                            sub.next_charge_unix_day = Some(
                                pallet_calendar::Pallet::<T>::unix_day_of_first_of_month_after(
                                    prev_next,
                                ),
                            );
                        }
                    }

                    // Apply referral commission: 5% of total charged to referrer, if referred.
                    let total_charged = if storage_ok { total_storage_charge } else { 0 } +
                                        compute_charged_total;
                    if total_charged > 0 {
                        if let Some(ref_code) = CreditsPallet::<T>::referred_users(&account_id) {
                            if let Some(referrer) = CreditsPallet::<T>::referral_codes(ref_code) {
                                let commission = total_charged.saturating_mul(5) / 100;
                                Self::try_mint_referral_reward_credits(&referrer, commission);
                            }
                        }
                    }

                    // Deactivate failed side(s) and emit failure events.
                    if !storage_ok && total_storage_charge > 0 {
                        log::warn!(
                            target: "runtime::marketplace",
                            "monthly storage subscription charge failed for {:?}: required={}, available_at_attempt={}",
                            account_id,
                            total_storage_charge,
                            storage_available_at_attempt
                        );
                        for sub in subs.iter_mut() {
                            if sub.active
                                && sub.package.is_storage_plan
                                && sub.next_charge_unix_day.map_or(true, |d| today >= d)
                            {
                                sub.active = false;
                            }
                        }
                        Self::deposit_event(Event::SubscriptionChargeFailed {
                            who: account_id.clone(),
                            required_credits: total_storage_charge,
                            available_credits: storage_available_at_attempt,
                        });
                    }

                    // Deactivate compute subs that failed individually, refund unused prepaid
                    for sub_id in compute_subs_to_deactivate {
                        let price = sorted_compute_subs.iter().find(|s| s.id == sub_id).unwrap().package.price;
                        log::warn!(
                            target: "runtime::marketplace",
                            "monthly compute subscription charge failed for {:?}, sub_id={}: required={}, available_at_attempt={}",
                            account_id,
                            sub_id,
                            price,
                            compute_available
                        );
                        for sub in subs.iter_mut() {
                            if sub.id == sub_id && sub.active {
                                sub.active = false;
                                // Refund unused prepaid months
                                let refund = Self::unused_prepaid_refund_credits(sub);
                                if refund > 0 {
                                    if let Err(e) = Self::refund_credits_with_batch(&account_id, refund) {
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
                            available_credits: compute_available,
                        });
                    }
                }

                // Persist any advances/deactivations for this account.
                UserAllSubscriptionPlans::<T>::insert(&account_id, subs);
            }

            // Conservative weight accounting:
            // - `iter()` reads each user's subscription entry once.
            // - for users we charge/cancel, we likely write back to subscriptions and touch credits/tx records.
            // Use intentionally-high multipliers to avoid undercounting as state grows.
            let reads = users_seen
                .saturating_add(users_with_active_subs.saturating_mul(5))
                .saturating_add(users_charged_or_cancelled.saturating_mul(10));
            let writes = users_charged_or_cancelled.saturating_mul(10);
            T::DbWeight::get().reads_writes(reads, writes)
        }
        
        // charges users for storage
        fn handle_arion_storage_charging(current_block: BlockNumberFor<T>) -> Weight {
            let mut users_seen: u64 = 0;
            let mut users_charged_or_removed: u64 = 0;
            // Get all users who requested storage
            let all_users_who_requested_storage = pallet_arion::Pallet::<T>::get_all_users();
            for user in all_users_who_requested_storage {
                users_seen = users_seen.saturating_add(1);
                // Check if user has any active storage plan subscription
                let subscriptions = UserAllSubscriptionPlans::<T>::get(&user);
                if !subscriptions.is_empty() && subscriptions.iter().any(|sub| sub.active && sub.package.is_storage_plan) {
                    // Skip charging this user as they have an active storage plan
                    continue;
                }

                // Check if the time difference is greater than 1 hour
                let last_charged_at = StorageLastChargedAt::<T>::get(user.clone());
                let block_difference = current_block.saturating_sub(last_charged_at);
                if block_difference > T::BlocksPerHour::get().into() {
                    // Variables to track total file size and fulfilled requests for updating
                    let total_file_size_in_bs: u128 = pallet_arion::Pallet::<T>::user_total_files_size(&user).unwrap_or(0);
            
                    // Skip if no files to charge
                    if total_file_size_in_bs == 0 {
                        continue;
                    }
            
                    // Get the current price per GB from the marketplace pallet
                    let price_per_gb = Self::get_price_per_gb();
                    
                    let user_free_credits = CreditsPallet::<T>::get_free_credits(&user);
                        
                    // Round up to the nearest whole number of GBs
                    let rounded_gbs: u128 = total_file_size_in_bs
                        .saturating_add(1_073_741_823)
                        / 1_073_741_824;
                    let charge_amount = price_per_gb.saturating_mul(rounded_gbs);            
            
                    if user_free_credits >= charge_amount {
                        // Decrease user credits
                        let charge_result = Self::consume_credits(
                            user.clone(),
                            charge_amount,
                            Self::account_id().clone(),
                            RankingsPallet::<T>::account_id().clone(),
                        );

                        if charge_result.is_ok() {
                            let tx_result = Self::record_credits_transaction(
                                &user,
                                NativeTransactionType::Subscription,
                                charge_amount.into(),
                            );
                            let ts_result = Self::update_storage_last_charged_at(&user);
                            if tx_result.is_ok() && ts_result.is_ok() {
                                users_charged_or_removed = users_charged_or_removed.saturating_add(1);
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
            // - reading `UserAllSubscriptionPlans` and `StorageLastChargedAt` per user
            // - charged/removed paths do writes and call into other pallets
            let reads = users_seen.saturating_mul(5).saturating_add(users_charged_or_removed.saturating_mul(10));
            let writes = users_charged_or_removed.saturating_mul(10);
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

        /// Retrieve active compute subscriptions specifically
        fn get_active_compute_subscriptions() -> Vec<(T::AccountId, UserPlanSubscription<T>)> {
            UserAllSubscriptionPlans::<T>::iter()
                .flat_map(|(account_id, subscriptions)| {
                    subscriptions.into_iter()
                        .filter(|sub| sub.active && !sub.package.is_storage_plan)
                        .map(move |sub| (account_id.clone(), sub))
                })
                .collect()
        }

        /// Cancel a specific subscription by ID (storage or compute).
        /// Marks the subscription inactive, refunds unused prepaid months.
        /// Sets the resubscribe cooldown only when no active subscriptions remain.
        fn do_cancel_subscription_by_id(account_id: &T::AccountId, subscription_id: SubscriptionId) -> DispatchResult {
            let now = <frame_system::Pallet<T>>::block_number();
            let mut refund: u128 = 0;
            let mut cancelled = false;
            let mut was_storage = false;

            UserAllSubscriptionPlans::<T>::mutate(account_id, |subscriptions| {
                for sub in subscriptions.iter_mut() {
                    if sub.id == subscription_id && sub.active {
                        refund = refund.saturating_add(Self::unused_prepaid_refund_credits(sub));
                        was_storage = sub.package.is_storage_plan;
                        sub.active = false;
                        cancelled = true;
                    }
                }
            });

            ensure!(cancelled, Error::<T>::SubscriptionNotFound);

            if refund > 0 {
                Self::refund_credits_with_batch(account_id, refund)?;
            }

            // Only set cooldown if no active subscriptions remain
            let has_active_subs = UserAllSubscriptionPlans::<T>::get(account_id).iter().any(|s| s.active);
            if !has_active_subs {
                LastSubscriptionCancelledAt::<T>::insert(account_id, now);
            }

            if was_storage {
                Self::deposit_event(Event::StorageSubscriptionCancelled { who: account_id.clone() });
            } else {
                Self::deposit_event(Event::ComputeSubscriptionCancelled { who: account_id.clone() });
            }
            Ok(())
        }

        /// Helper function to remove the last charged timestamp for a user
        pub fn remove_storage_last_charged_at(who: &T::AccountId)  {
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
            code: Option<Vec<u8>>
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
            
            AlphaBalances::<T>::mutate(&sender, |alpha| *alpha = alpha.saturating_add(alpha_amount));
            CreditsPallet::<T>::do_mint(sender.clone(), credit_amount, code)?;

            Self::deposit_event(Event::BatchDeposited { owner: sender, batch_id });

            Ok(())
        }

        /// Consume user credits from their batches
        #[transactional]
        pub fn consume_credits(
            sender: T::AccountId,
            credits: u128,
            marketplace_account: T::AccountId,
            ranking_account: T::AccountId
        ) -> DispatchResult {
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
                        let alpha_to_release_u128 = alpha_to_release
                            .min(U256::from(batch.remaining_alpha))
                            .as_u128();
                        
                        // Update batch credits first (needed for future calculations in this batch)
                        batch.remaining_credits = batch.remaining_credits.saturating_sub(credits_to_take);
                        
                        // Handle frozen/unfrozen logic
                        if batch.is_frozen && block_number < batch.release_time {
                            
                            // Batch is still frozen - add to pending
                            batch.pending_alpha = batch.pending_alpha.saturating_add(alpha_to_release_u128);
                            
                        } else {
                            
                            // Check if batch just unfroze in this block
                            if batch.is_frozen && block_number >= batch.release_time {
       
                                // Batch just unfroze - distribute all pending alpha first
                                batch.is_frozen = false;
 
                                if batch.pending_alpha > 0 {
                                    AlphaBalances::<T>::mutate(
                                        &batch.owner,
                                        |alpha| *alpha = alpha.saturating_sub(batch.pending_alpha)
                                    );
        
                                    Self::distribute_alpha(
                                        batch.pending_alpha,
                                        ranking_account.clone(),
                                        marketplace_account.clone(),
                                    )?;
        
                                    batch.pending_alpha = 0;
                                }
                            }
        
                            // Distribute current alpha
                            if alpha_to_release_u128 > 0 {
                                AlphaBalances::<T>::mutate(
                                    &batch.owner,
                                    |alpha| *alpha = alpha.saturating_sub(alpha_to_release_u128)
                                );
            
                                Self::distribute_alpha(
                                    alpha_to_release_u128,
                                    ranking_account.clone(),
                                    marketplace_account.clone(),
                                )?;
                        }
                        }
        
                        // Update remaining alpha after all operations
                        batch.remaining_alpha = batch.remaining_alpha.saturating_sub(alpha_to_release_u128);
                        // Save updated batch
                        Batches::<T>::insert(batch_id, batch);
                        remaining = remaining.saturating_sub(credits_to_take);
                    }
                }
            }
        
            ensure!(remaining == 0, Error::<T>::InsufficientFreeCredits);
        
            Self::deposit_event(Event::CreditsConsumed {
                owner: sender,
                credits
            });
        
            Ok(())
        }

        fn distribute_alpha(
            alpha_to_release: u128,
            ranking_account: T::AccountId,
            marketplace_account: T::AccountId,
        ) -> DispatchResult {
        
            let rankings_amount = alpha_to_release
                .checked_mul(70)
                .and_then(|x| x.checked_div(100))
                .unwrap_or_default();
        
            let marketplace_amount = alpha_to_release
                .saturating_sub(rankings_amount);

            if let Some(sudo_account) = Self::sudo_key() {

                <pallet_balances::Pallet<T>>::transfer(
                    &sudo_account,
                    &ranking_account,
                    rankings_amount.try_into().unwrap_or_default(),
                    ExistenceRequirement::AllowDeath
                )?;
        
                <pallet_balances::Pallet<T>>::transfer(
                    &sudo_account,
                    &marketplace_account,
                    marketplace_amount.try_into().unwrap_or_default(),
                    ExistenceRequirement::AllowDeath
                )?;
            }
        
            Ok(())
        }

        /// Handle chargeback for a specific batch
        fn handle_chargeback(batch_id: u64) -> DispatchResult {
            // Get the batch from storage
            if let Some(batch) = Batches::<T>::get(batch_id) {
                // Ensure the batch is frozen and the chargeback is valid
                ensure!(batch.is_frozen && <frame_system::Pallet<T>>::block_number() < batch.release_time, "Invalid chargeback");
 
                // Decrease the total locked Alpha by the remaining Alpha in the batch
                let total_alpha_to_remove = batch.remaining_alpha
                    .saturating_add(batch.pending_alpha);

                if total_alpha_to_remove > 0 {
                    AlphaBalances::<T>::mutate(&batch.owner, |alpha| {
                        *alpha = alpha.saturating_sub(total_alpha_to_remove);
                    });
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
                TotalCreditsPurchased::<T>::mutate(|total| *total = total.saturating_sub(credit_to_burn));
            }

            Ok(())
        }
        
        pub fn get_batches_for_user(user: T::AccountId) -> Vec<Batch<T::AccountId, BlockNumberFor<T>>> {
            let batch_ids: Vec<u64> = UserBatches::<T>::get(user).unwrap_or_default(); 
            batch_ids.iter()
                .filter_map(|id| Batches::<T>::get(*id))
                .collect()
        }

        pub fn get_batch_by_id(batch_id: u64) -> Option<Batch<T::AccountId, BlockNumberFor<T>>> {
            Batches::<T>::get(batch_id)
        }

   }
}

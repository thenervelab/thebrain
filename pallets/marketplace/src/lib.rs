#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;
pub use types::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

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
	pub struct Pallet<T>(_);

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
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
                if Self::should_run_monthly_subscription_charge() {
                    weight_used = weight_used.saturating_add(Self::handle_all_subscription_charging(current_block));
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
        SubscriptionCancelled { who: T::AccountId },
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
	    StorageOperationsStatusChanged { enabled: bool },
        /// Purchase plan status was changed
        PurchasePlanStatusChanged { enabled: bool },
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
        /// No active compute subscription found for the user
        NoActiveComputeSubscription,
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
            plan_ids: Vec<T::Hash>,
            location_ids: Option<Vec<Option<u32>>>,
            selected_image_names: Vec<Option<Vec<u8>>>,
            cloud_init_cids: Option<Vec<Option<Vec<u8>>>>,
            miner_ids: Option<Vec<Option<Vec<u8>>>>
        ) -> DispatchResult {
            let owner = ensure_signed(origin)?;

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
            if let Some(ref xs) = miner_ids {
                ensure!(xs.len() == plan_ids.len(), Error::<T>::InvalidInput);
            }

            // Initialize default values for optional parameters
            let location_ids = location_ids.unwrap_or_else(|| vec![None; plan_ids.len()]);
            let cloud_init_cids = cloud_init_cids.unwrap_or_else(|| vec![None; plan_ids.len()]);
            let miner_ids = miner_ids.unwrap_or_else(|| vec![None; plan_ids.len()]);

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
                        miner_ids[i].clone()
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
                        miner_ids[i].clone()
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
            SudoKey::<T>::put(Some(new_sudo_key)); // Store as Some(value)
    
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
        pub fn sudo_set_purchase_plan(
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

        /// User cancels their own subscription
        #[pallet::call_index(19)]
        #[pallet::weight((0, Pays::No))]
        pub fn cancel_my_subscription(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            
            // Call the helper function to cancel the subscription
            Self::do_cancel_storage_subscription(&who)?;
            
            Ok(())
        }        
	}

    impl<T: Config> Pallet<T> {
        /// Current UNIX millis as `u64`.
        fn now_ms() -> u64 {
            pallet_timestamp::Pallet::<T>::get().saturated_into::<u64>()
        }

        fn current_unix_day() -> u32 {
            // unix day number since epoch (UTC)
            (Self::now_ms() / 86_400_000u64) as u32
        }

        fn is_first_day_of_month() -> bool {
            let dim = pallet_calendar::Pallet::<T>::days_in_current_month();
            let drm = pallet_calendar::Pallet::<T>::days_remaining_in_current_month();
            dim > 0 && drm == dim
        }

        fn should_run_monthly_subscription_charge() -> bool {
            if !Self::is_first_day_of_month() {
                return false;
            }

            let today = Self::current_unix_day();
            if LastMonthlySubscriptionChargeDay::<T>::get() == today {
                return false;
            }

            // Mark as run for today (even if no one ends up being charged),
            // to prevent repeated full-scan loops within the same day.
            LastMonthlySubscriptionChargeDay::<T>::put(today);
            true
        }

        fn prorated_monthly_price(monthly_price: u128) -> u128 {
            // days_remaining_in_current_month is inclusive of today.
            let dim: u128 = pallet_calendar::Pallet::<T>::days_in_current_month() as u128;
            let drm: u128 = pallet_calendar::Pallet::<T>::days_remaining_in_current_month() as u128;
            if dim == 0 {
                return monthly_price;
            }
            monthly_price.saturating_mul(drm) / dim
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
            miner_id: Option<Vec<u8>>
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


            let pay_upfront: Option<u128> = None;
            // Check if plan exists
            let plan = Plans::<T>::get(&plan_id).ok_or(Error::<T>::PlanNotFound)?;

            ensure!(!plan.is_suspended, Error::<T>::PlanSuspended);

            // Determine the (monthly) price for the plan.
            // If the user purchases mid-month, charge a pro-rated amount based on remaining days
            // in the current month (inclusive of today).
            let mut plan_price_native = Self::prorated_monthly_price(plan.price);
                
            if let Some(upfront_months) = pay_upfront {
                plan_price_native = plan_price_native.saturating_mul(upfront_months);
            }
        
            // Check user's native token balance 
            let user_free_credits = CreditsPallet::<T>::get_free_credits(&who);
            ensure!(user_free_credits >= plan_price_native, Error::<T>::InsufficientFreeCredits);

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
                plan_price_native,
                Self::account_id().clone(),
                pallet_rankings::Pallet::<T>::account_id().clone(),
            )?;
                    
            // Record transaction
            Self::record_credits_transaction(
                &who,
                NativeTransactionType::Subscription,
                (plan_price_native).into(),
            )?;

            // Create subscription (simplified due to removed plan_type)
            let subscription = UserPlanSubscription {
                id: subscription_id,
                owner: who.clone(),
                package: plan.clone(),
                cdn_location_id: None,
                active: true,
                last_charged_at: current_block_number,
                selected_image_name: None,
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
            miner_id: Option<Vec<u8>>
        ) -> DispatchResult {
            // Check if the ComputeMiner node type is disabled
            ensure!(
                !RegistrationPallet::<T>::is_node_type_disabled(NodeType::ComputeMiner),
                Error::<T>::NodeTypeDisabled
            );

            // Enforce: only one active compute subscription at a time.
            let existing_subscriptions = UserAllSubscriptionPlans::<T>::get(&who);
            ensure!(
                !existing_subscriptions
                    .iter()
                    .any(|s| s.active && !s.package.is_storage_plan),
                Error::<T>::AlreadyHasActiveSubscription
            );

            let pay_upfront: Option<u128> = None;
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
                plan_price_native = plan_price_native.saturating_mul(upfront_months);
            }
        
            // Check user's native token balance 
            let user_free_credits = CreditsPallet::<T>::get_free_credits(&who);
            ensure!(user_free_credits >= plan_price_native, Error::<T>::InsufficientFreeCredits);

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
                plan_price_native,
                Self::account_id().clone(),
                pallet_rankings::Pallet::<T, pallet_rankings::Instance2>::account_id().clone(),
            )?;
                    
            // Record transaction
            Self::record_credits_transaction(
                &who,
                NativeTransactionType::Subscription,
                (plan_price_native).into(),
            )?;

            // Create subscription (simplified due to removed plan_type)
            let subscription = UserPlanSubscription {
                id: subscription_id,
                owner: who.clone(),
                package: plan.clone(),
                cdn_location_id: location_id,
                active: true,
                last_charged_at: current_block_number,
                selected_image_name : Some(selected_image_name),
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

            // Iterate through all users with subscriptions once
            for (account_id, subscriptions) in UserAllSubscriptionPlans::<T>::iter() {
                users_seen = users_seen.saturating_add(1);
                // Filter only active subscriptions
                let active_subs: Vec<UserPlanSubscription<T>> = subscriptions
                    .iter()
                    .filter(|sub| sub.active)
                    .cloned()
                    .collect();
                
                if active_subs.is_empty() {
                    continue;
                }
                users_with_active_subs = users_with_active_subs.saturating_add(1);

                // Monthly charging: on the 1st day of the month we charge all active subscriptions.
                // (The outer caller guards execution to month-start only.)
                let storage_subs_to_charge: Vec<_> = active_subs
                    .iter()
                    .filter(|sub| sub.package.is_storage_plan)
                    .cloned()
                    .collect();
                let compute_subs_to_charge: Vec<_> = active_subs
                    .iter()
                    .filter(|sub| !sub.package.is_storage_plan)
                    .cloned()
                    .collect();

                // Calculate total charge amounts
                let mut total_storage_charge = 0u128;
                let mut total_compute_charge = 0u128;

                for sub in &storage_subs_to_charge {
                    total_storage_charge = total_storage_charge.saturating_add(sub.package.price);
                }

                for sub in &compute_subs_to_charge {
                    total_compute_charge = total_compute_charge.saturating_add(sub.package.price);
                }

                let total_charge = total_storage_charge.saturating_add(total_compute_charge);
                let user_free_credits = CreditsPallet::<T>::get_free_credits(&account_id);

                if user_free_credits >= total_charge {
                    // User has enough credits, charge them
                    users_charged_or_cancelled = users_charged_or_cancelled.saturating_add(1);
                    let mut storage_charged_ok = false;
                    let mut compute_charged_ok = false;
                    
                    // Charge for storage subscriptions
                    if total_storage_charge > 0 {
                        let storage_charge_result = Self::consume_credits(
                            account_id.clone(), 
                            total_storage_charge,
                            Self::account_id().clone(), 
                            pallet_rankings::Pallet::<T>::account_id().clone()
                        );
                        if storage_charge_result.is_ok() {
                            let tx_result = Self::record_credits_transaction(
                                &account_id,
                                NativeTransactionType::Subscription,
                                total_storage_charge.into(),
                            );
                            storage_charged_ok = tx_result.is_ok();
                        }
                    }

                    // Charge for compute subscriptions
                    if total_compute_charge > 0 {
                        let compute_charge_result = Self::consume_credits(
                            account_id.clone(), 
                            total_compute_charge,
                            Self::account_id().clone(), 
                            pallet_rankings::Pallet::<T, pallet_rankings::Instance2>::account_id().clone()
                        );
                        if compute_charge_result.is_ok() {
                            let tx_result = Self::record_credits_transaction(
                                &account_id,
                                NativeTransactionType::Subscription,
                                total_compute_charge.into(),
                            );
                            compute_charged_ok = tx_result.is_ok();
                        }
                    }

                    // Update all charged subscriptions
                    UserAllSubscriptionPlans::<T>::mutate(&account_id, |subs| {
                        for sub in subs.iter_mut() {
                            if sub.active {
                                // Only update timestamps if the relevant charge path succeeded.
                                if sub.package.is_storage_plan {
                                    if storage_charged_ok {
                                        sub.last_charged_at = current_block;
                                    }
                                } else {
                                    if compute_charged_ok {
                                        sub.last_charged_at = current_block;
                                    }
                                }
                            }
                        }
                    });
                }
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

        /// Cancel a user's subscription
        fn do_cancel_storage_subscription(account_id: &T::AccountId) -> DispatchResult {
            let now = <frame_system::Pallet<T>>::block_number();
            UserAllSubscriptionPlans::<T>::mutate(account_id, |subscriptions| {
                let original_len = subscriptions.len();
                // Filter out storage plans
                *subscriptions = subscriptions
                    .iter()
                    .filter(|sub| !sub.package.is_storage_plan)
                    .cloned()
                    .collect();
        
                if subscriptions.is_empty() {
                    // If no subscriptions left, we could clear it if needed
                    // But since we're already setting it to an empty vector, we might not need to
                }
        
                // Emit event if any storage plans were removed
                if subscriptions.len() < original_len {
                    LastSubscriptionCancelledAt::<T>::insert(account_id, now);
                    Self::deposit_event(Event::StorageSubscriptionCancelled {
                        who: account_id.clone(),
                    });
                    Ok(())
                } else {
                    Err(Error::<T>::NoActiveSubscription.into())
                }
            })
        }
        
        fn do_cancel_subscription(account_id: &T::AccountId) -> DispatchResult {
            let now = <frame_system::Pallet<T>>::block_number();
            UserAllSubscriptionPlans::<T>::mutate(account_id, |subscriptions| {
                let original_len = subscriptions.len();
                // Keep only storage plans
                *subscriptions = subscriptions
                    .iter()
                    .filter(|sub| sub.package.is_storage_plan)
                    .cloned()
                    .collect();
        
                // Emit event if any non-storage plans were removed
                if subscriptions.len() < original_len {
                    LastSubscriptionCancelledAt::<T>::insert(account_id, now);
                    Self::deposit_event(Event::SubscriptionCancelled {
                        who: account_id.clone(),
                    });
                    Ok(())
                } else {
                    Err(Error::<T>::NoActiveSubscription.into())
                }
            })
        }

        /// Cancel ALL subscriptions for a user (both storage and compute)
        fn do_cancel_all_subscriptions(account_id: &T::AccountId) -> DispatchResult {
            let now = <frame_system::Pallet<T>>::block_number();
            UserAllSubscriptionPlans::<T>::mutate(account_id, |subscriptions| {
                let had_subscriptions = !subscriptions.is_empty();
                
                // Clear all subscriptions
                subscriptions.clear();
        
                if had_subscriptions {
                    LastSubscriptionCancelledAt::<T>::insert(account_id, now);
                    // Emit events for both types being cancelled
                    Self::deposit_event(Event::StorageSubscriptionCancelled {
                        who: account_id.clone(),
                    });
                    Self::deposit_event(Event::ComputeSubscriptionCancelled {
                        who: account_id.clone(),
                    });
                    Ok(())
                } else {
                    Err(Error::<T>::NoActiveSubscription.into())
                }
            })
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
            
            let mut remaining = credits;
            let block_number = <frame_system::Pallet<T>>::block_number();
            
            if let Some(batch_ids) = UserBatches::<T>::get(&sender) {
                for batch_id in batch_ids {
                    
                    if remaining == 0 {
                        break;
                    }
                    
                    if let Some(mut batch) = Batches::<T>::get(batch_id) {
                        
                        ensure!(batch.owner == sender, "Not your batch");
                        
                        let credits_to_take = remaining.min(batch.remaining_credits);
                        let current = CreditsPallet::<T>::get_free_credits(&batch.owner);
                        ensure!(current >= credits_to_take, Error::<T>::InsufficientFreeCredits);
                        
                        // Referral logic
                        let mut total_discount = 0u128;
                        if let Some(previous_referral) = CreditsPallet::<T>::referred_users(&batch.owner) {
                            CreditsPallet::<T>::apply_referral_discount(
                                &previous_referral,
                                credits_to_take,
                                &mut total_discount
                            )?;
                        }
                        let effective_charge = credits_to_take.saturating_sub(total_discount);
                        ensure!(current >= effective_charge, Error::<T>::InsufficientFreeCredits);

                        // Decrease user credits (apply discount)
                        CreditsPallet::<T>::decrease_user_credits(&batch.owner, effective_charge);
                        
                        // FIXED: Use remaining amounts for accurate alpha calculation
                        // This ensures the ratio reflects the current batch state
                        let credits_to_take_u256 = U256::from(effective_charge);
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
                        batch.remaining_credits = batch.remaining_credits.saturating_sub(effective_charge);
                        
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
                        remaining = remaining.saturating_sub(effective_charge);
                    }
                }
            }
        
            ensure!(remaining == 0, "Not enough credits");
        
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
                .checked_mul(75)
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
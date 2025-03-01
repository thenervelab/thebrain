
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

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;
#[frame_support::pallet]
pub mod pallet {
    use super::*;
    use frame_support::traits::Len;
    use frame_support::{
        pallet_prelude::*,
        traits::{ReservableCurrency, Currency},
        PalletId,
    };
    use frame_system::pallet_prelude::*;
    use sp_runtime::{
        traits::{AtLeast32BitUnsigned, AccountIdConversion, Hash},
        Saturating,
    };
    use pallet_registration::BalanceOf;
    use pallet_ipfs_pin::FileHash;
    use pallet_credits::Pallet as CreditsPallet;
    use pallet_utils::SubscriptionId;
    use sp_core::H256;
    use sp_std::{vec, vec::Vec};
    use pallet_ipfs_pin::StorageRequest;
    use num_traits::float::FloatCore;
    use pallet_rankings::Pallet as RankingsPallet;
    use pallet_subaccount::traits::SubAccounts;
    use frame_system::offchain::Signer;
    use sp_runtime::offchain::storage_lock::StorageLock;
    // use frame_system::offchain::SignedPayload;
    use sp_runtime::offchain::Duration;
    use sp_runtime::offchain::storage_lock::BlockAndTime;
    use frame_system::offchain::SendTransactionTypes;
    use frame_system::offchain::AppCrypto;
    use frame_system::offchain::SigningTypes;
    use frame_system::offchain::SendUnsignedTransaction;
    use frame_support::traits::ExistenceRequirement;
    use pallet_compute::ComputeRequestStatus;
    use sp_runtime::traits::Zero;

	#[pallet::pallet]
	#[pallet::without_storage_info]
	pub struct Pallet<T>(_);

    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
        fn on_initialize(current_block: BlockNumberFor<T>) -> Weight {    

            // Only execute on blocks divisible by the configured interval
            if current_block % T::BlockChargeCheckInterval::get().into() == 0u32.into() {
                Self::handle_storage_subscription_charging(current_block);
                Self::handle_compute_subscription_charging(current_block);
            }

            // Return some weight (adjust based on actual implementation)
            T::DbWeight::get().reads_writes(1, 1)
        }
    }
    
    #[pallet::config]
    pub trait Config: frame_system::Config + 
                    pallet_registration::Config + 
                    pallet_credits::Config + 
                    pallet_ipfs_pin::Config + 
                    pallet_balances::Config + 
                    pallet_notifications::Config +
                    pallet_compute::Config +
                    pallet_storage::Config +
                    pallet_rankings::Config +
                    pallet_subaccount::Config +
                    pallet_rankings::Config<pallet_rankings::Instance2>+ 
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

        #[pallet::constant]
        type StorageGracePeriod: Get<u32>;

        #[pallet::constant]
        type ComputeGracePeriod: Get<u32>;

        /// Custom hash type for this pallet
        type CustomHash: Parameter + Default + From<H256>;

        /// The block interval for executing certain pallet operations
        type BlockChargeCheckInterval: Get<u32>;
    }

	const LOCK_BLOCK_EXPIRATION: u32 = 3;
    const LOCK_TIMEOUT_EXPIRATION: u32 = 10000;

    #[pallet::storage]
    #[pallet::getter(fn plans)]
    pub type Plans<T: Config> = StorageMap<_, Blake2_128Concat, T::Hash, Plan<T::Hash>, OptionQuery>;

    #[pallet::storage]
    pub(super) type PricePerGbs<T: Config> = StorageValue<_, u128, ValueQuery>;

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
    #[pallet::getter(fn registry_cid_delete_requests)]
    pub(super) type RegistryCidDeleteRequests<T: Config> = StorageValue<_, Vec<Vec<u8>>, ValueQuery>;

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
    #[pallet::getter(fn user_file_hashes)]
    pub type UserFileHashes<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        Vec<Vec<u8>>,
        ValueQuery
    >;

    // Storage for OS Disk Image URLs
	#[pallet::storage]
	#[pallet::getter(fn os_disk_image_urls)]
	pub type OSDiskImageUrls<T: Config> = StorageMap<
		_, 
		Blake2_128Concat, 
		Vec<u8>, // OS Name (e.g., "ubuntu", "fedora")
		ImageDetails, // URL for the OS disk image
	>;
    
    #[derive(Encode, Decode, Clone, PartialEq, Eq, Default, TypeInfo)]
    pub struct ImageDetails {
        pub url: Vec<u8>,
        pub description: Vec<u8>,
        pub name: Vec<u8>,
    }

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
        PinRequested {
			who: T::AccountId,
			file_hash: FileHash,
			replicas: u32
		},
        UnpinRequestAdded { 
            caller: T::AccountId,
            owner: T::AccountId,
            file_hash: FileHash,
        },
        StorageRequestAdded {
            caller: T::AccountId,
            owner: T::AccountId,
            files_input: Vec<FileInput>,
        },
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
            selected_image_name: Vec<u8>,
            cloud_init_cid: Option<Vec<u8>>,
        },
        FileHashCleanedUp {
            subscription_id: SubscriptionId,
            file_hash: FileHash,
        },
        PricePerGbUpdated { price: u128 },
        SubscriptionCancelled { who: T::AccountId },
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
	}

	#[pallet::error]
	pub enum Error<T> {
		NoneValue,
        NotSubscriptionOwner,
        SubscriptionNotFound,
        TooManySharedUsers,
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
        InvalidStorageReduction,
        InvalidSubscriptionUsage,
        ComputeResourceExceeded,
        NoActiveSubscription,
        BackupAlreadyEnabled,
        InvalidImageSelection,
        /// No active compute subscription found for the user
        NoActiveComputeSubscription,
        /// The plan does not match the user's active subscription
        InvalidPlanForSubscription,
        InvalidPlanConfiguration,
        InvalidOSDiskImageUrl,
        /// No subscription found for the given user
        NoSubscriptionFound,
	}


    #[pallet::validate_unsigned]
    impl<T: Config> ValidateUnsigned for Pallet<T> {
        type Call = Call<T>;
        fn validate_unsigned(_source: TransactionSource, call: &Self::Call) -> TransactionValidity {
            match call {
                Call::process_storage_request_approval { owner, storage_cost, signature: _} => {
					let current_block = frame_system::Pallet::<T>::block_number();
					// Create a unique hash combining all relevant data
					let mut data = Vec::new();
					data.extend_from_slice(&current_block.encode());
					data.extend_from_slice(&owner.encode());
                    data.extend_from_slice(&storage_cost.encode());
					let unique_hash = sp_io::hashing::blake2_256(&data);
	
					ValidTransaction::with_tag_prefix("MarketPlaceOffchain")
						.priority(TransactionPriority::max_value())
						.and_provides(("process_storage_request_approval", unique_hash))
						.longevity(5)
						.propagate(true)
						.build()
                },
                _ => InvalidTransaction::Call.into(),
            }
        }
    }


	#[pallet::call]
	impl<T: Config> Pallet<T> {
        /// Enable backup for a user's subscription from marketplace pallet
        #[pallet::call_index(0)]
        #[pallet::weight((0, Pays::No))]
        pub fn enable_vms_backup(
            origin: OriginFor<T>,
        ) -> DispatchResult {
            // Ensure the caller is signed
            let account_id = ensure_signed(origin)?;

            // Check if the account is a sub-account, and if so, use the main account
            let main_account = match <pallet_subaccount::Pallet<T> as SubAccounts<T::AccountId>>::get_main_account(account_id.clone()) {
                Ok(main) => main,
                Err(_) => account_id.clone(), // If not a sub-account, use the original account
            };

            // Call the marketplace pallet's enable_backup function
            Self::enable_backup(main_account.clone())?;

            // Emit an event in the backup pallet
            Self::deposit_event(Event::BackupEnabled { 
                caller: account_id,
                account: main_account 
            });

            Ok(())
        }

        /// Disable backup for a user's subscription
        #[pallet::call_index(1)]
        #[pallet::weight((0, Pays::No))]
        pub fn disable_vms_backup(
            origin: OriginFor<T>,
        ) -> DispatchResult {
            // Ensure the caller is signed
            let account_id = ensure_signed(origin)?;

            // Check if the account is a sub-account, and if so, use the main account
            let main_account = match <pallet_subaccount::Pallet<T> as SubAccounts<T::AccountId>>::get_main_account(account_id.clone()) {
                Ok(main) => main,
                Err(_) => account_id.clone(), // If not a sub-account, use the original account
            };

            // Call the disable_backup function
            Self::disable_backup(main_account.clone())?;

            // Emit an event (optional, but recommended)
            Self::deposit_event(Event::BackupDisabled { 
                caller: account_id,
                account: main_account 
            });

            Ok(())
        }

        #[pallet::call_index(2)] 
        #[pallet::weight((0, Pays::No))]
        pub fn burn_tokens( 
            origin: OriginFor<T>,
            amount: BalanceOf<T>,
        ) -> DispatchResult {
            // Ensure the caller is signed
            let _who = ensure_signed(origin)?;
            Self::do_burn_tokens(amount)
        }

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

        // file owner request to save their files given replicas and file hashes 
		#[pallet::call_index(4)]
		#[pallet::weight((0, Pays::No))]
		pub fn storage_request(
			origin: OriginFor<T>,
			files_input: Vec<FileInput>,
            miner_ids: Option<Vec<u8>>
		) -> DispatchResult {
			let caller = ensure_signed(origin)?;

            // Check if a specific miner is requested and charge an additional fee
            if miner_ids.is_some() {
                // Define a fixed fee for requesting a specific miner
                let specific_miner_fee = Self::specific_miner_request_fee();

                let length_as_balance: <T as pallet::Config>::Balance = TryFrom::try_from(miner_ids.len() as u128)
                .unwrap_or_else(|_| Zero::zero());
            
                let total_fee = specific_miner_fee.saturating_mul(length_as_balance.into());

                // Ensure the payer has sufficient balance
                ensure!(
                    <pallet_balances::Pallet<T>>::free_balance(&caller) >= total_fee,
                    Error::<T>::InsufficientBalance
                );

                // Charge the specific miner request fee
                <pallet_balances::Pallet<T>>::transfer(
                    &caller.clone(), 
                    &Self::account_id(), 
                    total_fee, 
                    ExistenceRequirement::KeepAlive
                )?;
            }

            // Check if the account is a sub-account, and if so, use the main account
            let owner = match <pallet_subaccount::Pallet<T> as SubAccounts<T::AccountId>>::get_main_account(caller.clone()) {
                Ok(main) => main,
                Err(_) => caller.clone(), // If not a sub-account, use the original account
            };

            Self::process_storage_requests(&owner.clone(), &files_input.clone(), miner_ids)?;

            // Emit an event for the storage request
            Self::deposit_event(Event::StorageRequestAdded {
                caller,
                owner,
                files_input,
            });

			Ok(())
		}

        #[pallet::call_index(5)]
        #[pallet::weight((0, Pays::No))]
        pub fn storage_unpin_request(
            origin: OriginFor<T>,
            file_hash: FileHash,
        ) -> DispatchResult {
            let caller = ensure_signed(origin)?;

            // Check if the account is a sub-account, and if so, use the main account
            let owner = match <pallet_subaccount::Pallet<T> as SubAccounts<T::AccountId>>::get_main_account(caller.clone()) {
                Ok(main) => main,
                Err(_) => caller.clone(), // If not a sub-account, use the original account
            };

            // Convert file hash to a hex-encoded string
            let file_hash_encoded = hex::encode(file_hash.clone());
            let encoded_file_hash: Vec<u8> = file_hash_encoded.clone().into();

            // Get storage request by file hash
            let requested_storage = pallet_ipfs_pin::Pallet::<T>::get_storage_request_by_hash(owner.clone(),encoded_file_hash.clone());
            ensure!(requested_storage.is_some(), Error::<T>::StorageRequestNotFound);

            // Call the helper function to process the unpin request
            let processed_hash = pallet_ipfs_pin::Pallet::<T>::process_unpin_request(owner.clone(), file_hash)?;

            // Emit the event for unpin request
            Self::deposit_event(Event::UnpinRequestAdded {
                caller,
                owner,
                file_hash: processed_hash,
            });

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
            // miner_id: Option<Vec<u8>>,
            price: u128,
            name: Vec<u8>,
            // created_date: Vec<u8>,
        ) -> DispatchResult {
            // Ensure the caller is sudo
            ensure_root(origin)?;

            // Generate a unique ID for the plan (you can use a counter or a random hash)
            let plan_id = T::Hashing::hash_of(&plan_name); // Example way to generate a unique ID


            // Create the plan object
            let new_plan = Plan {
                id: plan_id.clone(),
                plan_name: plan_name.clone(),
                plan_description,
                plan_technical_description,
                is_suspended: false, // By default, the plan is active
                price,
                name,
            };

            // Insert the new plan into storage
            Plans::<T>::insert(plan_id.clone(), new_plan);

            Ok(())
        }

        /// Purchase a plan (storage or compute) using points
        #[pallet::call_index(7)]
        #[pallet::weight((0, Pays::No))]
        pub fn purchase_plan(
            origin: OriginFor<T>,
            plan_id: T::Hash,
            location_id: Option<u32>,
            selected_image_name: Vec<u8>,
            cloud_init_cid: Option<Vec<u8>>,
            pay_for: Option<T::AccountId>,
            miner_id: Option<Vec<u8>>
        ) -> DispatchResult {
            let caller = ensure_signed(origin)?;

            // Check if a specific miner is requested and charge an additional fee
            if miner_id.is_some() {
                // Define a fixed fee for requesting a specific miner
                let specific_miner_fee = Self::specific_miner_request_fee();
                
                // Ensure the payer has sufficient balance
                ensure!(
                    <pallet_balances::Pallet<T>>::free_balance(&caller) >= specific_miner_fee,
                    Error::<T>::InsufficientBalance
                );

                // Charge the specific miner request fee
                <pallet_balances::Pallet<T>>::transfer(
                    &caller.clone(), 
                    &Self::account_id(), 
                    specific_miner_fee, 
                    ExistenceRequirement::KeepAlive
                )?;
            }
            
            // Check if the caller is a sub-account, and if so, use the main account
            let caller_main_account = match <pallet_subaccount::Pallet<T> as SubAccounts<T::AccountId>>::get_main_account(caller.clone()) {
                Ok(main) => main,
                Err(_) => caller.clone(), // If not a sub-account, use the original account
            };

            // Determine the owner of the plan
            let owner = match pay_for {
                Some(specified_owner) => {
                    // If a specific owner is provided, check if it's a sub-account
                    match <pallet_subaccount::Pallet<T> as SubAccounts<T::AccountId>>::get_main_account(specified_owner.clone()) {
                        Ok(main) => main,
                        Err(_) => specified_owner.clone(), // If not a sub-account, use the original account
                    }

                },
                None => caller_main_account.clone(),
            };

            let result = Self::do_purchase_plan(
                owner.clone(),
                plan_id,
                location_id,
                selected_image_name.clone(),
                cloud_init_cid.clone(),
                miner_id
            )?;

            // Emit an event for the plan purchase
            Self::deposit_event(Event::PlanPurchased {
                caller,
                owner,
                plan_id,
                location_id,
                selected_image_name,
                cloud_init_cid,
            });

            Ok(result)
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

        // Sudo function to increase the price of an existing plan
        #[pallet::call_index(10)]
        #[pallet::weight((10_000, Pays::No))]
        pub fn increase_plan_price(
            origin: OriginFor<T>,
            plan_id: T::Hash,
            new_price: u128,
        ) -> DispatchResult {
            ensure_none(origin)?;
            
            // Retrieve the existing plan
            let mut plan = <Plans<T>>::get(plan_id)
                .ok_or(Error::<T>::NoneValue)?;

            // Update the plan's price
            plan.price = new_price;

            // Store the updated plan
            <Plans<T>>::insert(plan_id, plan);

            // Emit an event (optional, you might want to add this to the Events enum)
            Self::deposit_event(Event::PlanPriceUpdated(plan_id, new_price));

            Ok(())
        }

		#[pallet::call_index(11)]
		#[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
        pub fn process_storage_request_approval(
            origin: OriginFor<T>,
            owner: T::AccountId,
            storage_cost: u128,
            _signature: <T as SigningTypes>::Signature,
		) -> DispatchResultWithPostInfo {
            ensure_none(origin)?;
			// Handle referral logic
			let mut total_discount = 0u128;
			if let Some(previous_referral) = CreditsPallet::<T>::referred_users(owner.clone()) {
				let _ = CreditsPallet::<T>::apply_referral_discount(&previous_referral, storage_cost, &mut total_discount);
			}
			log::info!("total discount is : {}", total_discount);					

			// Calculate the amounts for Compute Rankings and Marketplace
			let total_charge = storage_cost;
			let rankings_amount = total_charge
				.checked_mul(70u32.into())
				.and_then(|x| x.checked_div(100u32.into()))
				.unwrap_or_default();
						
			let marketplace_amount = total_charge - rankings_amount;

			// Mint 70% to Compute Rankings
			let _ = pallet_balances::Pallet::<T>::deposit_creating(
				&RankingsPallet::<T>::account_id(), 
				rankings_amount.try_into().unwrap_or_default()
			);

			// Emit event for rankings account reward
			RankingsPallet::<T>::deposit_event(pallet_rankings::Event::RewardDistributed {
				account: RankingsPallet::<T>::account_id(),
				amount: rankings_amount.try_into().unwrap_or_default()
			});

			// Deposit remaining 30% amount to marketplace account
			let _ = pallet_balances::Pallet::<T>::deposit_creating(
				&Self::account_id(), 
				marketplace_amount.try_into().unwrap_or_default()
			);	

			// Emit event for marketplace account reward
			RankingsPallet::<T>::deposit_event(pallet_rankings::Event::RewardDistributed {
				account: Self::account_id(),
				amount: marketplace_amount.try_into().unwrap_or_default()
			});

            Ok(().into())
        }
        
        /// Set the specific miner request fee
        #[pallet::call_index(15)]
        #[pallet::weight((0, Pays::No))]
        pub fn set_specific_miner_request_fee(
            origin: OriginFor<T>,
            fee: BalanceOf<T>
        ) -> DispatchResult {
            // Ensure the caller is an authority
            let authority = ensure_signed(origin)?;
            CreditsPallet::<T>::ensure_is_authority(&authority)?;

            // Update the SpecificMinerRequestFee storage value
            <SpecificMinerRequestFee<T>>::put(fee);

            // Deposit an event to notify about the fee update
            Self::deposit_event(Event::<T>::SpecificMinerRequestFeeUpdated { fee });

            Ok(())
        }
	}

    impl<T: Config> Pallet<T> {
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

        /// Burns the specified amount of native tokens from the pallet's account
        pub fn do_burn_tokens( amount: BalanceOf<T>) -> DispatchResult {
            // Get the pallet's account
            let pallet_account = Self::account_id();
            
            // Ensure the pallet has enough balance using pallet_balances
            ensure!(
                pallet_balances::Pallet::<T>::free_balance(&pallet_account) >= amount,
                Error::<T>::InsufficientBalance
            );

            // Call the burn function from balances pallet
            pallet_balances::Pallet::<T>::burn(
                frame_system::RawOrigin::Signed(pallet_account).into(),
                amount,
                false, // keep_alive set to false to allow burning entire balance
            )?;

            // Emit an event
            Self::deposit_event(Event::TokensBurned {
                amount,
            });

            Ok(())
        }

        fn do_purchase_plan(
            who: T::AccountId,
            plan_id: T::Hash,
            location_id: Option<u32>,
            selected_image_name: Vec<u8>,
            cloud_init_cid: Option<Vec<u8>>,
            miner_id: Option<Vec<u8>>
            // referral_code: Option<Vec<u8>>,
            // pay_upfront: Option<u128>,
        ) -> DispatchResult {
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

            // Convert marketplace ImageMetadata to compute pallet ImageMetadata
            let compute_image_metadata = pallet_compute::ImageMetadata {
                name: selected_image_name.clone(),
                image_url: image_url.url,
            };

            // Determine the price (using the price from the plan)
            let mut plan_price_native = plan.price;
                
            if let Some(upfront_months) = pay_upfront {
                plan_price_native = plan_price_native.saturating_mul(upfront_months);
            }
        
            // Check user's native token balance 
            let user_free_credits = CreditsPallet::<T>::get_free_credits(&who);
            ensure!(user_free_credits >= plan_price_native, Error::<T>::InsufficientFreeCredits);
        
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

            // Handle referral logic
            let mut total_discount = 0u128;
            if let Some(previous_referral) = CreditsPallet::<T>::referred_users(&who.clone()) {
                let _ = CreditsPallet::<T>::apply_referral_discount(&previous_referral, plan_price_native, &mut total_discount);
            }

            // charge the user 
            CreditsPallet::<T>::decrease_user_credits(&who, plan_price_native);

            // Deposit charge to marketplace account
            let _ = pallet_balances::Pallet::<T>::deposit_creating(&Self::account_id(), (plan_price_native).try_into().unwrap_or_default());

            // Calculate the amounts for Compute Rankings and Marketplace
            let total_charge = plan_price_native;
            let rankings_amount = total_charge
                .checked_mul(70u32.into())
                .and_then(|x| x.checked_div(100u32.into()))
                .unwrap_or_default();
                                            
            let marketplace_amount = total_charge - rankings_amount;
            
            // Mint 70% to Compute Rankings
            let _ = pallet_balances::Pallet::<T>::deposit_creating(
                &RankingsPallet::<T>::account_id(), 
                rankings_amount.try_into().unwrap_or_default()
            );
            
            // Deposit remaining 30% amount to marketplace account
            let _ = pallet_balances::Pallet::<T>::deposit_creating(
                &Self::account_id(), 
                marketplace_amount.try_into().unwrap_or_default()
            );
                    
            // Record transaction
            Self::record_native_transaction(
                &who,
                NativeTransactionType::Subscription,
                (plan_price_native).into(),
            )?;

            let current_block_number = <frame_system::Pallet<T>>::block_number();
			
            // Create subscription (simplified due to removed plan_type)
            let subscription = UserPlanSubscription {
                id: subscription_id,
                owner: who.clone(),
                package: plan.clone(),
                cdn_location_id: location_id,
                active: true,
                last_charged_at: current_block_number,
                selected_image_name,
                _phantom: PhantomData,
            };
        
            // Store subscription
            UserPlanSubscriptions::<T>::insert(&who, subscription);

            pallet_compute::Pallet::<T>::create_compute_request(
                who.clone(),
                plan.plan_technical_description.clone(),
                plan_id,
                compute_image_metadata,
                cloud_init_cid,
                miner_id
            )?;

            Ok(())
        }

        fn record_native_transaction(
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

        // Add this new helper function
        fn handle_unpin_of_inactive_subscription(account: &T::AccountId, file_hash_to_unpin: Vec<u8>)  {
            if let Ok(file_hash) = hex::decode(&file_hash_to_unpin) {
                let update_unpin_hash: Vec<u8> = file_hash.into();
                // Mutate the existing file hashes for the account
                UserFileHashes::<T>::mutate(account, |file_hashes| {
                    // Remove the specific file hash if it exists
                    file_hashes.retain(|hash| *hash != update_unpin_hash);
                });

                // Add the file hash to the registry CID delete request
                RegistryCidDeleteRequests::<T>::mutate(|cid_delete_requests| {
                    if !cid_delete_requests.contains(&update_unpin_hash) {
                        cid_delete_requests.push(update_unpin_hash.clone());
                    }
                });
            }

            let mut unpin_request = pallet_ipfs_pin::Pallet::<T>::unpin_requests();
            
            // Add files to unpin
            unpin_request.push(file_hash_to_unpin);
            
            // Update unpin items storage
            pallet_ipfs_pin::Pallet::<T>::overwrite_unpin_requests(unpin_request);  
        }
        
        // charging logic for compute
        fn handle_compute_subscription_charging(current_block: BlockNumberFor<T>) {
            // Retrieve all fulfilled compute requests
            let compute_subscriptions = Self::get_active_compute_subscriptions();
            for (account_id, mut subscription) in compute_subscriptions {
                // Check if the time difference is greater than 1 hour
                let block_difference = current_block.saturating_sub(subscription.last_charged_at);
                if block_difference > T::BlocksPerHour::get().into() {
                    
                    // only charge if the request is fulfilled by assigned minner 
                    let minner_compute_request = pallet_compute::Pallet::<T>::get_miner_compute_request(
                        account_id.clone(),
                        subscription.package.id
                    );
                    if minner_compute_request.is_some() {
                        let compute_request = minner_compute_request.unwrap();
                        if compute_request.fullfilled {
                            // charge the user for the compute requests for blocks in an hour 
                            let price_per_block = &subscription.package.price;
                            let user_free_credits = CreditsPallet::<T>::get_free_credits(&account_id);

                            // total blocks in an hour
                            let charge_amount = price_per_block * ( T::BlocksPerHour::get() as u128);
                        
                            if user_free_credits >= charge_amount {
                                // Decrease user credits
                                CreditsPallet::<T>::decrease_user_credits(&account_id, charge_amount);

                                // Handle referral logic
                                let mut total_discount = 0u128;
                                if let Some(previous_referral) = CreditsPallet::<T>::referred_users(&account_id) {
                                    let _ = CreditsPallet::<T>::apply_referral_discount(&previous_referral, charge_amount, &mut total_discount);
                                }
                                
                                // Calculate the amounts for Compute Rankings and Marketplace
                                let total_charge = charge_amount - total_discount;
                                let rankings_amount = total_charge
                                    .checked_mul(70u32.into())
                                    .and_then(|x| x.checked_div(100u32.into()))
                                    .unwrap_or_default();
                                
                                let marketplace_amount = total_charge - rankings_amount;

                                // Mint 70% to Compute Rankings
                                let _ = pallet_balances::Pallet::<T>::deposit_creating(
                                    &pallet_rankings::Pallet::<T, pallet_rankings::Instance2>::account_id(), 
                                    rankings_amount.try_into().unwrap_or_default()
                                );

                                // Deposit remaining 30% amount to marketplace account
                                let _ = pallet_balances::Pallet::<T>::deposit_creating(
                                    &Self::account_id(), 
                                    marketplace_amount.try_into().unwrap_or_default()
                                );
                                
                                // Record transaction
                                let _ = Self::record_native_transaction(
                                    &account_id,
                                    NativeTransactionType::Subscription,
                                    (charge_amount - total_discount).into(),
                                );

                                // Update the storage (Gbs Used , Last Charged at) for this subscription
                                subscription.last_charged_at = current_block;
                                UserPlanSubscriptions::<T>::insert(&account_id, subscription.clone());

                                let compute_request = pallet_compute::Pallet::<T>::get_compute_request_by_id(compute_request.request_id);

                                if compute_request.unwrap().status == ComputeRequestStatus::Stopped {
                                    let _ = pallet_compute::Pallet::<T>::add_miner_compute_boot_request(
                                        account_id.clone(),
                                        subscription.package.id
                                    );
                                }
                            } else {
                                if Self::is_compute_request_in_grace_period(subscription.last_charged_at, current_block) {
                                    // Still within grace period
                                    // Check if a stop request exists before creating one
                                    if !pallet_compute::Pallet::<T>::compute_stop_request_exists(&account_id, &subscription.package.id) {
                                        let _ = pallet_compute::Pallet::<T>::add_miner_compute_stop_request(
                                            account_id.clone(),
                                            subscription.package.id
                                        );
                                    }
                                } else {
                                    // Cancel subscription if no credits
                                    let _ = Self::do_cancel_subscription(&account_id);
                                }
                            }
                        }
                    }
                }
            }
        }

        fn handle_storage_subscription_charging(current_block: BlockNumberFor<T>) {
            // Get all users who requested storage
            let all_users_who_requested_storage = pallet_ipfs_pin::Pallet::<T>::storage_request_users();
            for user in all_users_who_requested_storage {
                // Check if the time difference is greater than 1 hour
                let last_charged_at = StorageLastChargedAt::<T>::get(user.clone());
                let block_difference = current_block.saturating_sub(last_charged_at);
                if block_difference > T::BlocksPerHour::get().into() {
                    // Retrieve all fulfilled storage requests for this user
                    let fulfilled_requests = pallet_ipfs_pin::Pallet::<T>::get_owner_fulfilled_requests(user.clone());

                    // Variables to track total file size and fulfilled requests for updating
                    let mut total_file_size_in_bs: u128 = 0;
                    let mut requests_to_update: Vec<StorageRequest<T::AccountId, BlockNumberFor<T>>> = Vec::new();
            
                    // Calculate total file size for requests older than 1 hour
                    for request in fulfilled_requests {
                        let block_difference = current_block.saturating_sub(request.last_charged_at);
                        if block_difference > T::BlocksPerHour::get().into() {
                            if let Some(file_size_in_bytes) = pallet_ipfs_pin::Pallet::<T>::get_file_size(request.file_hash.clone()) {
                                total_file_size_in_bs += file_size_in_bytes as u128;
                                requests_to_update.push(request);
                            }
                        }
                    }
                
                    // Skip if no files to charge
                    if total_file_size_in_bs == 0 {
                        continue;
                    }
                
                    // Convert total file size to gigabytes
                    let total_file_size_in_gbs = total_file_size_in_bs as f64 / 1_073_741_824.0;
            
                    // Get the current price per GB from the marketplace pallet
                    let price_per_gb = Self::get_price_per_gb();
                    
                    let user_free_credits = CreditsPallet::<T>::get_free_credits(&user);
                        
                    // Round up to the nearest whole number of GBs
                    let rounded_gbs = ((total_file_size_in_gbs).floor() as u128) + 1;
                    let charge_amount = price_per_gb * rounded_gbs;                    
            
                    if user_free_credits >= charge_amount {
                        // Decrease user credits
                        CreditsPallet::<T>::decrease_user_credits(&user, charge_amount);

                        // Handle referral logic
                        let mut total_discount = 0u128;
                        if let Some(previous_referral) = CreditsPallet::<T>::referred_users(&user) {
                            let _ = CreditsPallet::<T>::apply_referral_discount(&previous_referral, charge_amount, &mut total_discount);
                        }
                        
                        // Calculate the amounts for Storage Rankings and Marketplace
                        let total_charge = charge_amount - total_discount;
                        let rankings_amount = total_charge
                            .checked_mul(70u32.into())
                            .and_then(|x| x.checked_div(100u32.into()))
                            .unwrap_or_default();
                                    
                        let marketplace_amount = total_charge - rankings_amount;

                        // Mint 70% to Storage Rankings
                        let _ = pallet_balances::Pallet::<T>::deposit_creating(
                            &RankingsPallet::<T>::account_id(), 
                            rankings_amount.try_into().unwrap_or_default()
                        );

                        // Deposit remaining 30% amount to marketplace account
                        let _ = pallet_balances::Pallet::<T>::deposit_creating(
                            &Self::account_id(), 
                            marketplace_amount.try_into().unwrap_or_default()
                        );

                        // Record transaction
                        let _ = Self::record_native_transaction(
                            &user,
                            NativeTransactionType::Subscription,
                            (charge_amount - total_discount).into(),
                        );

                        // Update last charged block for each request
                        for mut request in requests_to_update.clone() {
                            request.last_charged_at = current_block;
                            // update user subscription last charged at
                            pallet_ipfs_pin::Pallet::<T>::update_storage_request(request.owner.clone(), request.file_hash.clone(), Some(request));
                        }
                        let _ = Self::update_storage_last_charged_at(&user);
                    } else {
                        // Iterate through requests to update
                        for request in requests_to_update {
                            // Check if the request has exceeded the storage grace period
                            let subscription = UserPlanSubscriptions::<T>::get(&user);
                            if let Some(subscription) = subscription {
                                let blocks_per_hour = T::BlocksPerHour::get();
                                let grace_period_blocks = T::StorageGracePeriod::get();
                                
                                // Calculate grace period start after hourly charging
                                let grace_period_start = last_charged_at.saturating_add(blocks_per_hour.into());
                                
                                // Check if the current block is within the grace period
                                if current_block.saturating_sub(grace_period_start) <= grace_period_blocks.into() {
                                    // Still within grace period
                                    log::info!(
                                        "Storage request for user {:?} is in grace period",
                                        user
                                    );
                                } else {
                                    // Cancel the request after grace period
                                    pallet_ipfs_pin::Pallet::<T>::update_storage_request(request.owner.clone(), request.file_hash.clone(), None);
            
                                    // request to delete all backups of user 
                                    Self::move_user_to_backup_delete_requests(&user);
            
                                    // Handle unpinning for inactive subscription
                                    Self::handle_unpin_of_inactive_subscription(&user, request.file_hash.clone());

                                    Self::remove_storage_last_charged_at(&user);
                                }
                            }
                        }
                    }
                }
            }
        }
        
        // fn handle_storage_subscription_charging(current_block: BlockNumberFor<T>) {
        //     // get total files stores , charge users every hour
        //     let users_with_buckets = pallet_storage::get_users_with_buckets();
        //     for user in users_with_buckets {
        //         // last_charged_at
        //         let last_charged_at = pallet_storage::last_charged_at(user);
        //         let block_difference = current_block.saturating_sub(last_charged_at);
        //         if block_difference > T::BlocksPerHour::get().into() {
        //             // check user last charged_at and if the hour has passed 
        //             let bucket_names = BucketNames::<T>::get(&user);
        //             // Track total size for the user's buckets
        //             let mut user_total_size: u64 = 0;
    
        //             // Process the bucket names
        //             for bucket_name in bucket_names {
        //                 let bucket_name_str = String::from_utf8_lossy(&bucket_name);
    
        //                 // Perform HTTP request to list bucket contents
        //                 match pallet_storage::get_bucket_size_in_bytes(&bucket_name_str) {
        //                     Ok((_response, bucket_size)) => {
        //                         log::info!(
        //                             "Bucket {} size: {} bytes",
        //                             bucket_name_str,
        //                             bucket_size
        //                         );
        //                         // Accumulate total size for the user
        //                         user_total_size += bucket_size;
        //                     },
        //                     Err(err) => {
        //                         log::error!(
        //                             "Failed to list contents for bucket {}: {:?}",
        //                             bucket_name_str,
        //                             err
        //                         );
        //                     }
        //                 }
        //             }

        //             // Skip if no files to charge
        //             if user_total_size == 0 {
        //                 continue;
        //             }

        //             // Convert total file size to gigabytes
        //             let total_file_size_in_gbs = user_total_size as f64 / 1_073_741_824.0;

        //             // Get the current price per GB from the marketplace pallet
        //             let price_per_gb = Self::get_price_per_gb();
                    
        //             let user_free_credits = CreditsPallet::<T>::get_free_credits(&user);
                    
        //             // Round up to the nearest whole number of GBs
        //             let rounded_gbs = ((total_file_size_in_gbs).floor() as u128) + 1;
        //             let charge_amount = price_per_gb * rounded_gbs;                    

        //             if user_free_credits >= charge_amount {
        //                 // Decrease user credits
        //                 CreditsPallet::<T>::decrease_user_credits(&user, charge_amount);

        //                 // Handle referral logic
        //                 let mut total_discount = 0u128;
        //                 if let Some(previous_referral) = CreditsPallet::<T>::referred_users(&user) {
        //                     let _ = CreditsPallet::<T>::apply_referral_discount(&previous_referral, charge_amount, &mut total_discount);
        //                 }
                        
        //                 // Calculate the amounts for Compute Rankings and Marketplace
        //                 let total_charge = charge_amount - total_discount;
        //                 let rankings_amount = total_charge
        //                     .checked_mul(70u32.into())
        //                     .and_then(|x| x.checked_div(100u32.into()))
        //                     .unwrap_or_default();
                                    
        //                 let marketplace_amount = total_charge - rankings_amount;

        //                 // Mint 70% to Compute Rankings
        //                 let _ = pallet_balances::Pallet::<T>::deposit_creating(
        //                     &RankingsPallet::<T>::account_id(), 
        //                     rankings_amount.try_into().unwrap_or_default()
        //                 );

        //                 // Deposit remaining 30% amount to marketplace account
        //                 let _ = pallet_balances::Pallet::<T>::deposit_creating(
        //                     &Self::account_id(), 
        //                     marketplace_amount.try_into().unwrap_or_default()
        //                 );

        //                 // Record transaction
        //                 let _ = Self::record_native_transaction(
        //                     &user,
        //                     NativeTransactionType::Subscription,
        //                     (charge_amount - total_discount).into(),
        //                 );

        //                 // Update last charged block for each request
        //                 update_last_charged_at(&user, current_block);
                        
        //             } else {
        //                 let blocks_per_hour = T::BlocksPerHour::get();
        //                 let grace_period_blocks = T::StorageGracePeriod::get();
                        
        //                 // Calculate grace period start after hourly charging
        //                 let grace_period_start = last_charged_at.saturating_add(blocks_per_hour.into());
                        
        //                 // Check if the current block is within the grace period
        //                 if current_block.saturating_sub(grace_period_start) <= grace_period_blocks.into() {
        //                     // Still within grace period
        //                     log::info!(
        //                         "Storage request for user {:?} is in grace period",
        //                         user
        //                     );
        //                 } else {
        //                     // Cancel the request after grace period
        //                     // and delete storage 
        //                 }
        //             }
        //         }
        //     }
        // }

        /// Helper function to get the current price per GB
        pub fn get_price_per_gb() -> u128 {
            PricePerGbs::<T>::get()
        }

        /// Retrieve active compute subscriptions specifically
        fn get_active_compute_subscriptions() -> Vec<(T::AccountId, UserPlanSubscription<T>)> {
            UserPlanSubscriptions::<T>::iter()
                .filter(|(_, subscription)| {
                    subscription.active 
                })
                .collect()
        }

        /// Process storage requests for given file hashes
        pub fn process_storage_requests(
            owner: &T::AccountId, 
            file_inputs: &[FileInput],
            miner_ids: Option<Vec<u8>>
        ) -> DispatchResult {
            // Process each file hash
            for file_input in file_inputs.iter() {
                // Call the helper function for each file
                pallet_ipfs_pin::Pallet::<T>::process_storage_request(
                    owner.clone(), 
                    file_input.file_hash.clone(),
                    file_input.file_name.clone(),
                    miner_ids.clone()
                )?;

                // Emit event for each file
                Self::deposit_event(Event::PinRequested {
                    who: owner.clone(),
                    file_hash: file_input.file_hash.clone(),
                    replicas: 3u32,
                });
            }

            // Add file hashes to user's file hashes
            UserFileHashes::<T>::mutate(owner.clone(), |existing_hashes| {
                let new_hashes: Vec<Vec<u8>> = file_inputs
                    .iter()
                    .map(|file_input| file_input.file_hash.clone())
                    .collect();
                
                existing_hashes.extend(new_hashes);
            });

            Ok(())
        }

        /// Update the last charged block for a user's subscription
        pub fn update_user_subscription_last_charged_at(
            account: &T::AccountId, 
            block_number: BlockNumberFor<T>
        ) -> DispatchResult {
            // Retrieve the existing subscription
            UserPlanSubscriptions::<T>::try_mutate(account, |subscription_opt| {
                // Check if subscription exists
                if let Some(subscription) = subscription_opt {
                    // Update the last_charged_at field
                    subscription.last_charged_at = block_number;
                    Ok(())
                } else {
                    // Return a dispatch error if no subscription exists
                    Err(DispatchError::Other("No subscription found for user".into()))
                }
            })
        }

        /// Cancel a user's subscription
        fn do_cancel_subscription(account_id: &T::AccountId) -> DispatchResult {
            // Retrieve the current subscription
            let subscription = UserPlanSubscriptions::<T>::get(account_id)
                .ok_or(Error::<T>::NoActiveSubscription)?;

            // Remove the subscription from storage
            UserPlanSubscriptions::<T>::remove(account_id);

            // Delete associated miner compute request
            let _ = pallet_compute::Pallet::<T>::add_delete_miner_compute_request(
                subscription.package.id, 
                account_id.clone()
            );

            // Emit an event about subscription cancellation
            Self::deposit_event(Event::SubscriptionCancelled {
                who: account_id.clone(),
            });

            Ok(())
        }

          /// Disable backup for a user's subscription
          pub fn disable_backup(
            account_id: T::AccountId,
        ) -> DispatchResult {
            // Retrieve the user's subscription
            let subscription = UserPlanSubscriptions::<T>::get(&account_id)
                .ok_or(Error::<T>::SubscriptionNotFound)?;

            // Ensure the caller is the subscription owner
            ensure!(subscription.owner == account_id, Error::<T>::NotSubscriptionOwner);

            // Remove user from backup enabled users list
            BackupEnabledUsers::<T>::mutate(|users| {
                // Find and remove the user if present
                if let Some(index) = users.iter().position(|id| id == &account_id) {
                    users.remove(index);
                }
            });

            Ok(())
        }

        /// Move a user from BackupEnabledUsers to BackupDeleteRequests
        fn move_user_to_backup_delete_requests(user_id: &T::AccountId) {
            // Remove the user from BackupEnabledUsers
            BackupEnabledUsers::<T>::mutate(|enabled_users| {
                enabled_users.retain(|user| user != user_id);
            });

            BackupDeleteRequests::<T>::mutate(|delete_requests| {
                if !delete_requests.contains(user_id) {
                    delete_requests.push(user_id.clone());
                }
            });
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

        /// Enable backup for a user's subscription
        pub fn enable_backup(
            account_id: T::AccountId,
        ) -> DispatchResult {
            // Retrieve the user's subscription
            let subscription = UserPlanSubscriptions::<T>::get(&account_id)
                .ok_or(Error::<T>::SubscriptionNotFound)?;

            // Ensure the caller is the subscription owner
            ensure!(subscription.owner == account_id, Error::<T>::NotSubscriptionOwner);

            // Add user to backup enabled users list
            BackupEnabledUsers::<T>::try_mutate(|users| -> DispatchResult {
                // Check if user is already in the list
                ensure!(!users.contains(&account_id), Error::<T>::BackupAlreadyEnabled);
                
                // Attempt to add the user
                users.push(account_id.clone());
                
                // Return Ok(()) to satisfy the DispatchResult
                Ok(())
            })?;

            Ok(())
        }        

        /// Remove a specific file hash from the registry CID delete request
        pub fn remove_cid_delete_request(file_hash: Vec<u8>) -> DispatchResult {
            RegistryCidDeleteRequests::<T>::mutate(|cid_delete_requests| {
                // Remove all instances of the file hash
                cid_delete_requests.retain(|hash| *hash != file_hash);
            });

            Ok(())
        }

        /// Helper function to get the configured compute grace period
		pub fn get_compute_grace_period() -> BlockNumberFor<T> {
			T::ComputeGracePeriod::get().into()
		}

		/// Checks if a compute request is within the grace period
		pub fn is_compute_request_in_grace_period(
			last_charged_at: BlockNumberFor<T>, 
			current_block: BlockNumberFor<T>
		) -> bool {
			let blocks_per_hour = T::BlocksPerHour::get().into();
			let grace_period_blocks = Self::get_compute_grace_period();
			
			// Calculate grace period start after hourly charging
			let grace_period_start = last_charged_at.saturating_add(blocks_per_hour);
			
			// Check if the current block is within the grace period
			current_block.saturating_sub(grace_period_start) <= grace_period_blocks
		}

        /// Helper function to update the last charged timestamp for a user
        pub fn update_storage_last_charged_at(who: &T::AccountId) -> DispatchResult {
            // Get the current timestamp from the timestamp pallet
            let now = <frame_system::Pallet<T>>::block_number();

            // Update the last charged timestamp for the user
            StorageLastChargedAt::<T>::insert(who, now);

            Ok(())
        }

        pub fn call_storage_request_approval_charging(account_id: T::AccountId, storage_cost: u128) {
			// Create a unique lock for the save hardware info operation
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"Marketplace::storage_request_approval_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);
		
            if let Ok(_guard) = lock.try_lock() {
                // Fetch signer accounts using AuthorityId
                let signer = Signer::<T, <T as pallet::Config>::AuthorityId>::all_accounts();
                if !signer.can_sign() {
                    log::warn!("No accounts available for signing in signer.");
                    return;
                }
            
                // Prepare and sign the payload
                let results = signer.send_unsigned_transaction(
                    |account| {
                        StorageApprovalPayload {
                            account_id: account_id.clone(),
                            storage_cost: storage_cost.clone(),
                            public: account.public.clone(),
                            _marker: PhantomData,
                        }
                    },
                    |payload, signature| {
                        Call::process_storage_request_approval {
                            owner: payload.account_id,
                            storage_cost: payload.storage_cost,
                            signature,
                        }
                    },
                );
            
                // Process results of the transaction submission
                for (acc, res) in &results {
                    match res {
                        Ok(()) => log::info!("[{:?}] Successfully submitted signed hardware update", acc.id),
                        Err(e) => log::error!("[{:?}] Error submitting hardware update: {:?}", acc.id, e),
                    }
                }

            } else {
				log::error!("❌ Could not acquire lock for updating metrics data");
			};
		}

   }
}
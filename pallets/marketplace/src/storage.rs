
use frame_support::{pallet_prelude::*, storage::types::*};
use frame_system::pallet_prelude::*;
use parity_scale_codec::{Decode, Encode};
use sp_std::vec::Vec;

#[pallet::storage]
#[pallet::getter(fn plans)]
pub type Plans<T: Config> = StorageMap<_, Blake2_128Concat, T::Hash, Plan<T::Hash>, OptionQuery>;

// Mapping to track the lifetime rewards earned by each referral code
#[pallet::storage]
pub(super) type ReferralCodeRewards<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>,  u32, ValueQuery>;

// Mapping to track the number of times a referral code has been used
#[pallet::storage]
pub(super) type ReferralCodeUsageCount<T: Config> = StorageMap<_, Blake2_128Concat, Vec<u8>, u32, ValueQuery>;

// Mapping to track the total number of referral codes created
#[pallet::storage]
pub(super) type TotalReferralCodes<T: Config> = StorageValue<_, u32, ValueQuery>;

// Mapping to store the last block number a user created a referral code
#[pallet::storage]
#[pallet::getter(fn last_referral_creation_block)]
pub type LastReferralCreationBlock<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, BlockNumberFor<T>>;

// Mapping to track the total referral rewards earned
#[pallet::storage]
pub(super) type TotalReferralRewards<T: Config> = StorageValue<_, u32, ValueQuery>;

#[pallet::storage]
#[pallet::getter(fn user_subscriptions)]
pub(super) type UserSubscriptions<T: Config> = StorageMap<
    _,
    Blake2_128Concat,
    T::AccountId,
    UserSubscription<T>,
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

#[pallet::storage]
#[pallet::getter(fn cdn_locations)]
pub(super) type CdnLocations<T: Config> = StorageMap<_, Blake2_128Concat, u32, CdnLocation, OptionQuery>;

#[pallet::storage]
pub type SubscriptionFileHashes<T: Config> = StorageMap<
    _,
    Blake2_128Concat,
    SubscriptionId,
    Vec<Vec<u8>>,
    ValueQuery
>;

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
#[pallet::getter(fn subscription_permissions)]
pub(super) type SubscriptionPermissions<T: Config> = StorageDoubleMap<
    _,
    Blake2_128Concat,
    SubscriptionId,
    Blake2_128Concat,
    T::AccountId,
    AccessControl<T>,
    OptionQuery,
>;
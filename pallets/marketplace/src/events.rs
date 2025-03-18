use frame_support::pallet_prelude::*;
use crate::{
    Config,
    types::{PackageTier, Points, PointTransactionType},
    storage::BalanceOf,
};

#[pallet::event]
#[pallet::generate_deposit(pub(super) fn deposit_event)]
pub enum Event<T: Config> {
    /// Points purchased with USDC. [account, points_amount]
    PointsPurchased {
        account: T::AccountId,
        amount: Points,
    },
    /// Points spent on subscription. [account, points_amount, subscription_id]
    PointsSpent {
        account: T::AccountId,
        amount: Points,
        subscription_id: u32,
    },
    /// Points transferred between accounts. [from, to, amount]
    PointsTransferred {
        from: T::AccountId,
        to: T::AccountId,
        amount: Points,
    },
    /// Points refunded. [account, points_amount, subscription_id]
    PointsRefunded {
        account: T::AccountId,
        amount: Points,
        subscription_id: u32,
    },
    /// Point transaction recorded. [account, transaction_type, amount]
    PointTransactionRecorded {
        account: T::AccountId,
        transaction_type: PointTransactionType,
        amount: Points,
    },
    /// Storage package purchased. [account, package_tier]
    PackagePurchased {
        account: T::AccountId,
        tier: PackageTier,
    },
    /// Storage usage updated. [account, used_storage]
    StorageUsageUpdated {
        account: T::AccountId,
        storage_used: u32,
    },
    /// Subscription renewed. [account, package_tier]
    SubscriptionRenewed {
        account: T::AccountId,
        tier: PackageTier,
    },
    /// CDN location added. [location_id]
    CdnLocationAdded {
        location_id: u32,
    },
    /// Auto-renewal status updated. [account, status]
    AutoRenewalUpdated {
        account: T::AccountId,
        enabled: bool,
    },
}

#[pallet::error]
pub enum Error<T> {
    /// Not enough points to purchase package
    InsufficientPoints,
    /// Package not found
    PackageNotFound,
    /// User already has active subscription
    AlreadySubscribed,
    /// Subscription not found
    SubscriptionNotFound,
    /// Storage limit exceeded
    StorageLimitExceeded,
    /// Invalid subscription duration
    InvalidDuration,
    /// CDN location not found
    CdnLocationNotFound,
    /// Subscription expired
    SubscriptionExpired,
    /// Invalid package configuration
    InvalidPackageConfig,
    /// Not enough points for transfer
    InsufficientPointsForTransfer,
    /// Cannot transfer points to self
    CannotTransferToSelf,
    /// Invalid point amount
    InvalidPointAmount,
    /// Transaction ID not found
    TransactionNotFound,
    /// Point system error
    PointSystemError,
}

// Helper function to convert tuple event params to struct form
impl<T: Config> Event<T> {
    pub fn points_purchased(account: T::AccountId, amount: Points) -> Self {
        Self::PointsPurchased { account, amount }
    }

    pub fn points_spent(account: T::AccountId, amount: Points, subscription_id: u32) -> Self {
        Self::PointsSpent { account, amount, subscription_id }
    }

    pub fn points_transferred(from: T::AccountId, to: T::AccountId, amount: Points) -> Self {
        Self::PointsTransferred { from, to, amount }
    }

    pub fn points_refunded(account: T::AccountId, amount: Points, subscription_id: u32) -> Self {
        Self::PointsRefunded { account, amount, subscription_id }
    }

    pub fn point_transaction_recorded(account: T::AccountId, transaction_type: PointTransactionType, amount: Points) -> Self {
        Self::PointTransactionRecorded { account, transaction_type, amount }
    }

    pub fn package_purchased(account: T::AccountId, tier: PackageTier) -> Self {
        Self::PackagePurchased { account, tier }
    }

    pub fn storage_updated(account: T::AccountId, storage_used: u32) -> Self {
        Self::StorageUsageUpdated { account, storage_used }
    }

    pub fn subscription_renewed(account: T::AccountId, tier: PackageTier) -> Self {
        Self::SubscriptionRenewed { account, tier }
    }

    pub fn cdn_added(location_id: u32) -> Self {
        Self::CdnLocationAdded { location_id }
    }

    pub fn auto_renewal_updated(account: T::AccountId, enabled: bool) -> Self {
        Self::AutoRenewalUpdated { account, enabled }
    }
}

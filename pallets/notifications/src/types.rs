use codec::{Decode, Encode};
use scale_info::TypeInfo;
use frame_support::pallet_prelude::{RuntimeDebug, MaxEncodedLen};

#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub enum NotificationType {
    SubscriptionEndingSoon,
    SubscriptionHasEnded,
    General,
    // Custom(Vec<u8>), // For custom notification types with metadata
}


#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
pub struct Notification<AccountId,BlockNumberFor> {
    /// The sender of the notification
    pub sender: AccountId,
    /// The block number when the notification is sent
    pub block_to_send: BlockNumberFor,
    /// Recurrence flag
    pub recurrence: bool,
    /// Starting recurrence block number
    pub starting_recurrence: Option<BlockNumberFor>,
    /// Frequency in blocks for recurrence
    pub frequency: Option<BlockNumberFor>,
    /// Read status of the notification
    pub read: bool,
    /// Notification type
    pub notification_type: NotificationType,
}
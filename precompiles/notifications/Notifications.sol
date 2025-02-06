// SPDX-License-Identifier: GPL-3.0-only
pragma solidity >=0.8.0;

address constant INotifications_ADDRESS = 0x0000000000000000000000000000000000000825;

INotifications constant INotifications_CONTRACT = INotifications(
    INotifications_ADDRESS
);


/// @title Interface for interacting with the Notifications precompile
/// @author The Brain Team
interface INotifications {
    /// @notice Send a notification to a recipient
    /// @param recipient The address of the recipient
    /// @param blockToSend The block number when the notification should be sent
    /// @param recurrence Whether the notification should recur
    /// @param startingRecurrence The block number when recurrence should start
    /// @param frequency The frequency of recurrence in blocks
    function sendNotification(
        address recipient,
        uint256 blockToSend,
        bool recurrence,
        uint256 startingRecurrence,
        uint256 frequency
    ) external;

    /// @notice Mark a notification as read
    /// @param index The index of the notification to mark as read
    function markAsRead(uint32 index) external;
}
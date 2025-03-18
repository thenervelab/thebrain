use crate::{mock::*, Error,  NotificationType};
use frame_support::{assert_noop, assert_ok};
use frame_system::RawOrigin;

/// Test sending a basic notification
#[test]
fn send_notification_works() {
    new_test_ext().execute_with(|| {
        let sender = 1;
        let recipient = 2;
        let block_to_send = 10;

        // Send a notification
        assert_ok!(
            Notifications::send_notification(
                RuntimeOrigin::signed(sender),
                recipient,
                block_to_send,
                false,
                None,
                None
            )
        );

        // Check that the notification was added
        let notifications = Notifications::notifications(&recipient);
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].sender, sender);
        assert_eq!(notifications[0].block_to_send, block_to_send);
        assert!(!notifications[0].read);
    });
}

/// Test sending multiple notifications
#[test]
fn send_multiple_notifications() {
    new_test_ext().execute_with(|| {
        let sender = 1;
        let recipient = 2;

        // Set initial block number
        frame_system::Pallet::<Test>::set_block_number(1);

        // Send multiple notifications with block progression to bypass cooldown
        for block in 10..15 {
            // Move past cooldown period
            frame_system::Pallet::<Test>::set_block_number(block * 11);

            assert_ok!(
                Notifications::send_notification(
                    RuntimeOrigin::signed(sender),
                    recipient,
                    block,
                    false,
                    None,
                    None
                )
            );
        }

        // Check that all notifications were added
        let notifications = Notifications::notifications(&recipient);
        assert_eq!(notifications.len(), 5);
    });
}

/// Test marking a notification as read
#[test]
fn mark_notification_as_read() {
    new_test_ext().execute_with(|| {
        let sender = 1;
        let recipient = 2;
        let block_to_send = 10;

        // Send a notification
        assert_ok!(
            Notifications::send_notification(
                RuntimeOrigin::signed(sender),
                recipient,
                block_to_send,
                false,
                None,
                None
            )
        );

        // Mark the notification as read
        assert_ok!(
            Notifications::mark_as_read(RuntimeOrigin::signed(recipient), 0)
        );

        // Check that the notification is marked as read
        let notifications = Notifications::notifications(&recipient);
        assert!(notifications[0].read);
    });
}

/// Test marking a non-existent notification fails
#[test]
fn mark_non_existent_notification_fails() {
    new_test_ext().execute_with(|| {
        let recipient = 2;

        // Try to mark a non-existent notification
        assert_noop!(
            Notifications::mark_as_read(RuntimeOrigin::signed(recipient), 0),
            Error::<Test>::InvalidNotificationIndex
        );
    });
}

/// Test sudo update of a notification
#[test]
fn sudo_update_notification_works() {
    new_test_ext().execute_with(|| {
        let sender = 1;
        let recipient = 2;
        let block_to_send = 10;

        // Send a notification
        assert_ok!(
            Notifications::send_notification(
                RuntimeOrigin::signed(sender),
                recipient,
                block_to_send,
                false,
                None,
                None
            )
        );

        // Sudo update the notification
        let new_block = 20;
        assert_ok!(
            Notifications::sudo_update_notification(
                RawOrigin::Root.into(),
                recipient,
                0,
                new_block,
                true,
                Some(5),
                Some(10)
            )
        );

        // Check that the notification was updated
        let notifications = Notifications::notifications(&recipient);
        assert_eq!(notifications[0].block_to_send, new_block);
        assert!(notifications[0].recurrence);
        assert_eq!(notifications[0].starting_recurrence, Some(5));
        assert_eq!(notifications[0].frequency, Some(10));
    });
}

/// Test sudo update fails for non-existent notification
#[test]
fn sudo_update_non_existent_notification_fails() {
    new_test_ext().execute_with(|| {
        // Try to update a non-existent notification
        assert_noop!(
            Notifications::sudo_update_notification(
                RawOrigin::Root.into(),
                2,
                0,
                20,
                true,
                Some(5),
                Some(10)
            ),
            Error::<Test>::InvalidNotificationIndex
        );
    });
}

/// Test cooldown period between notifications
#[test]
fn cooldown_period_works() {
    new_test_ext().execute_with(|| {
        let sender = 1;
        let recipient = 2;
        let block_to_send = 10;

        // Set initial block number
        frame_system::Pallet::<Test>::set_block_number(1);

        // First notification should work
        assert_ok!(
            Notifications::send_notification(
                RuntimeOrigin::signed(sender),
                recipient,
                block_to_send,
                false,
                None,
                None
            )
        );

        // Try to send another notification before cooldown period
        assert_noop!(
            Notifications::send_notification(
                RuntimeOrigin::signed(sender),
                recipient,
                block_to_send + 1,
                false,
                None,
                None
            ),
            Error::<Test>::CooldownNotElapsed
        );

        // Move past cooldown period (assuming cooldown is 10 blocks)
        frame_system::Pallet::<Test>::set_block_number(12);

        // Second notification should now work
        assert_ok!(
            Notifications::send_notification(
                RuntimeOrigin::signed(sender),
                recipient,
                block_to_send + 1,
                false,
                None,
                None
            )
        );
    });
}

/// Test banning an account prevents notifications
#[test]
fn ban_account_prevents_notifications() {
    new_test_ext().execute_with(|| {
        let banned_sender = 1;
        let recipient = 2;
        let block_to_send = 10;

        // Ban the account
        assert_ok!(
            Notifications::ban_account(RawOrigin::Root.into(), banned_sender)
        );

        // Try to send a notification from banned account
        assert_noop!(
            Notifications::send_notification(
                RuntimeOrigin::signed(banned_sender),
                recipient,
                block_to_send,
                false,
                None,
                None
            ),
            Error::<Test>::AccountBanned
        );
    });
}

/// Test subscription ended notification
#[test]
fn subscription_ended_notification() {
    new_test_ext().execute_with(|| {
        let sender = 1;
        let recipient = 2;
        let subscription_id = 42;

        // Simulate subscription ended notification
        Notifications::notify_subscription_ended(sender, recipient, subscription_id);

        // Check the notification was added
        let notifications = Notifications::notifications(&recipient);
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].notification_type, NotificationType::SubscriptionHasEnded);
        assert_eq!(notifications[0].sender, sender);
    });
}

/// Test subscription ending soon notification
#[test]
fn subscription_ending_soon_notification() {
    new_test_ext().execute_with(|| {
        let sender = 1;
        let recipient = 2;
        let subscription_id = 42;

        // Simulate subscription ending soon notification
        Notifications::notify_subscription_ending_soon(sender, recipient, subscription_id);

        // Check the notification was added
        let notifications = Notifications::notifications(&recipient);
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0].notification_type, NotificationType::SubscriptionEndingSoon);
        assert_eq!(notifications[0].sender, sender);
    });
}
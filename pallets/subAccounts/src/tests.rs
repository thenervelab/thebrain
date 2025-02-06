use crate::{mock::*, Error, Event};
use frame_support::{assert_noop, assert_ok};

/// Sender tries to add a sub account without being the main account or a sub account of a profile
#[test]
fn add_sub_account_not_allowed() {
    new_test_ext().execute_with(|| {
        let user = 0;
        let sender = 1;

        // create user in profile pallet
        // create_user(user);

        // The sender adds a sub account for the user. Should fail
        assert_noop!(
            SubAccounts::add_sub_account(RuntimeOrigin::signed(sender), user, sender),
            Error::<Test>::NoSubAccount
        );
    });
}

/// Sender tries to add a sub account for a different profile
#[test]
fn add_sub_account_for_different_profile() {
    new_test_ext().execute_with(|| {
        let profile_main_1 = 0;
        let profile_main_2 = 1;
        let unregistered_user = 2;

        // The main account 1 tries to add a sub account for the profile 2. Should fail
        assert_noop!(
            SubAccounts::add_sub_account(
                RuntimeOrigin::signed(profile_main_1),
                profile_main_2,
                unregistered_user
            ),
            Error::<Test>::NoSubAccount
        );
    });
}

/// Sender tries to add a sub account that is already a sub account of another profile
#[test]
fn add_existent_sub_account() {
    new_test_ext().execute_with(|| {
        let profile_main_1 = 0;
        let profile_main_2 = 1;

        // Add profile_main_2 as a sub account of profile_main_1
        assert_ok!(SubAccounts::add_sub_account(
            RuntimeOrigin::signed(profile_main_1),
            profile_main_1,
            profile_main_2
        ));

        // Try to add the same sub account again should fail
        assert_noop!(
            SubAccounts::add_sub_account(
                RuntimeOrigin::signed(profile_main_1),
                profile_main_1,
                profile_main_2
            ),
            Error::<Test>::AlreadySubAccount
        );
    });
}

/// Cannot remove all the connected accounts for a profile
#[test]
fn remove_all_connected_accounts() {
    new_test_ext().execute_with(|| {
        let profile_main_1 = 0;

        // Try to remove the main account as a sub account should fail
        assert_noop!(
            SubAccounts::remove_sub_account(
                RuntimeOrigin::signed(profile_main_1),
                profile_main_1,
                profile_main_1
            ),
            Error::<Test>::NoAccountsLeft
        );
    });
}

/// Sub account can be added by the main account and sub accounts
#[test]
fn add_sub_accounts() {
    new_test_ext().execute_with(|| {
        let profile_main_1 = 0;
        let sub_account_1 = 1;
        let sub_account_2 = 2;

        // Main account adds the first sub account
        assert_ok!(
            SubAccounts::add_sub_account(
                RuntimeOrigin::signed(profile_main_1),
                profile_main_1,
                sub_account_1
            )
        );

        // Check that the sub account was added successfully
        assert_eq!(SubAccounts::sub_account(sub_account_1), Some(profile_main_1));

        // Sub account 1 adds another sub account
        assert_ok!(
            SubAccounts::add_sub_account(
                RuntimeOrigin::signed(sub_account_1),
                profile_main_1,
                sub_account_2
            )
        );

        // Check events for both additions
        System::assert_last_event(
            Event::SubAccountAdded { 
                main: profile_main_1, 
                sub: sub_account_2 
            }.into()
        );
    });
}

/// Remove a sub account successfully
#[test]
fn remove_sub_account_works() {
    new_test_ext().execute_with(|| {
        let profile_main_1 = 0;
        let sub_account_1 = 1;

        // Add a sub account
        assert_ok!(
            SubAccounts::add_sub_account(
                RuntimeOrigin::signed(profile_main_1),
                profile_main_1,
                sub_account_1
            )
        );

        // Remove the sub account
        assert_ok!(
            SubAccounts::remove_sub_account(
                RuntimeOrigin::signed(profile_main_1),
                profile_main_1,
                sub_account_1
            )
        );

        // Check that the sub account was removed
        assert_eq!(SubAccounts::sub_account(sub_account_1), None);

        // Check event
        System::assert_last_event(
            Event::SubAccountRemoved { 
                main: profile_main_1, 
                sub: sub_account_1 
            }.into()
        );
    });
}

/// Cannot remove a sub account that doesn't exist
#[test]
fn remove_non_existent_sub_account() {
    new_test_ext().execute_with(|| {
        let profile_main_1 = 0;
        let non_existent_sub = 1;

        // Try to remove a non-existent sub account
        assert_noop!(
            SubAccounts::remove_sub_account(
                RuntimeOrigin::signed(profile_main_1),
                profile_main_1,
                non_existent_sub
            ),
            Error::<Test>::NoSubAccount
        );
    });
}

/// Cannot remove a sub account from a different profile
#[test]
fn remove_sub_account_from_different_profile() {
    new_test_ext().execute_with(|| {
        let profile_main_1 = 0;
        let profile_main_2 = 1;
        let sub_account_1 = 2;

        // Add sub account to profile_main_1
        assert_ok!(
            SubAccounts::add_sub_account(
                RuntimeOrigin::signed(profile_main_1),
                profile_main_1,
                sub_account_1
            )
        );

        // Try to remove the sub account from a different profile
        assert_noop!(
            SubAccounts::remove_sub_account(
                RuntimeOrigin::signed(profile_main_2),
                profile_main_1,
                sub_account_1
            ),
            Error::<Test>::NoSubAccount
        );
    });
}
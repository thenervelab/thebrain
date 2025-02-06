use crate::{mock::*, Error, Event};
use frame_support::{assert_noop, assert_ok};
use sp_runtime::AccountId32;

// Helper function to create a test account
fn create_account(seed: u8) -> AccountId32 {
    let mut account_bytes = [0u8; 32];
    account_bytes[31] = seed;
    AccountId32::from(account_bytes)
}

#[test]
fn set_public_item_works() {
    new_test_ext().execute_with(|| {
        let account = create_account(1);
        let public_item = b"0x1234abcd".to_vec();

        // Set a public item
        assert_ok!(AccountProfile::set_public_item(
            RuntimeOrigin::signed(account.clone()),
            public_item.clone()
        ));

        // Check storage
        let stored_item = AccountProfile::user_public_storage(&account);
        assert_eq!(stored_item, b"1234abcd".to_vec());

        // Check event
        System::assert_last_event(Event::PublicItemSet(account, b"1234abcd".to_vec()).into());
    });
}

#[test]
fn set_public_item_fails_with_invalid_hex() {
    new_test_ext().execute_with(|| {
        let account = create_account(1);
        let invalid_hex = b"not-a-hex".to_vec();

        // Attempt to set an invalid hex item
        assert_noop!(
            AccountProfile::set_public_item(
                RuntimeOrigin::signed(account),
                invalid_hex
            ),
            Error::<Test>::InvalidHexString
        );
    });
}

#[test]
fn set_private_item_works() {
    new_test_ext().execute_with(|| {
        let account = create_account(1);
        let private_item = b"0x5678ef".to_vec();

        // Set a private item
        assert_ok!(AccountProfile::set_private_item(
            RuntimeOrigin::signed(account.clone()),
            private_item.clone()
        ));

        // Check storage
        let stored_item = AccountProfile::user_private_storage(&account);
        assert_eq!(stored_item, b"5678ef".to_vec());

        // Check event
        System::assert_last_event(Event::PrivateItemSet(account, b"5678ef".to_vec()).into());
    });
}

#[test]
fn set_private_item_fails_with_invalid_hex() {
    new_test_ext().execute_with(|| {
        let account = create_account(1);
        let invalid_hex = b"not-a-hex".to_vec();

        // Attempt to set an invalid hex item
        assert_noop!(
            AccountProfile::set_private_item(
                RuntimeOrigin::signed(account),
                invalid_hex
            ),
            Error::<Test>::InvalidHexString
        );
    });
}

#[test]
fn set_username_works() {
    new_test_ext().execute_with(|| {
        let account = create_account(1);
        let username = b"testuser".to_vec();

        // Set a username
        assert_ok!(AccountProfile::set_username(
            RuntimeOrigin::signed(account.clone()),
            username.clone()
        ));

        // Check username storage
        let stored_username = AccountProfile::get_username_by_account_id(&account);
        assert_eq!(stored_username, Some(b"testuser".to_vec()));

        let account_by_username = AccountProfile::get_account_id_by_username(&b"testuser".to_vec());
        assert_eq!(account_by_username, Some(account.clone()));

        // Check event
        System::assert_last_event(Event::UsernameSet(account, username).into());
    });
}

#[test]
fn set_username_fails_with_duplicate_username() {
    new_test_ext().execute_with(|| {
        let account1 = create_account(1);
        let account2 = create_account(2);
        let username = b"testuser".to_vec();

        // First username set should work
        assert_ok!(AccountProfile::set_username(
            RuntimeOrigin::signed(account1.clone()),
            username.clone()
        ));

        // Second username set with same username should fail
        assert_noop!(
            AccountProfile::set_username(
                RuntimeOrigin::signed(account2),
                username
            ),
            Error::<Test>::UsernameAlreadyTaken
        );
    });
}

#[test]
fn set_username_fails_when_already_set() {
    new_test_ext().execute_with(|| {
        let account = create_account(1);
        let first_username = b"firstuser".to_vec();
        let second_username = b"seconduser".to_vec();

        // First username set should work
        assert_ok!(AccountProfile::set_username(
            RuntimeOrigin::signed(account.clone()),
            first_username
        ));

        // Second username set for same account should fail
        assert_noop!(
            AccountProfile::set_username(
                RuntimeOrigin::signed(account),
                second_username
            ),
            Error::<Test>::UsernameAlreadySet
        );
    });
}

#[test]
fn set_data_public_key_works() {
    new_test_ext().execute_with(|| {
        let account = create_account(1);
        let data_public_key = b"0x1234567890abcdef".to_vec();

        // Set a data public key
        assert_ok!(AccountProfile::set_data_public_key(
            RuntimeOrigin::signed(account.clone()),
            data_public_key.clone()
        ));

        // Check storage
        let stored_key = AccountProfile::get_data_public_key(&account);
        assert_eq!(stored_key, Some(b"1234567890abcdef".to_vec()));

        // Check event
        System::assert_last_event(Event::DataPublicKeySet(account).into());
    });
}

#[test]
fn set_message_public_key_works() {
    new_test_ext().execute_with(|| {
        let account = create_account(1);
        let message_public_key = b"0xfedcba0987654321".to_vec();

        // Set a message public key
        assert_ok!(AccountProfile::set_message_public_key(
            RuntimeOrigin::signed(account.clone()),
            message_public_key.clone()
        ));

        // Check storage
        let stored_key = AccountProfile::get_message_public_key(&account);
        assert_eq!(stored_key, Some(b"fedcba0987654321".to_vec()));

        // Check event
        System::assert_last_event(Event::MessagePublicKeySet(account).into());
    });
}
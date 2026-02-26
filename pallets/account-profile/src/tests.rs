use crate::AccountProfile;
use crate::{mock::*, AccountProfileChallenge, Error, Event};
use frame_support::{assert_noop, assert_ok};
use frame_system::pallet_prelude::BlockNumberFor;
use frame_system::pallet_prelude::*;
use sp_core::{ed25519, H256};
use sp_io::hashing::blake2_256;
use sp_runtime::AccountId32;

// Helper function to create a test account
fn create_account(seed: u8) -> AccountId32 {
	let mut account_bytes = [0u8; 32];
	account_bytes[31] = seed;
	AccountId32::from(account_bytes)
}

// Helper function to create a test node ID
fn create_node_id(seed: u8) -> Vec<u8> {
	let mut node_id = vec![0u8; 38];
	node_id[0..6].copy_from_slice(&[0x00, 0x24, 0x08, 0x01, 0x12, 0x20]);
	node_id[6..38].fill(seed);
	node_id
}

// Helper function to create a test public key and signature
fn create_test_keypair() -> (ed25519::Pair, Vec<u8>, Vec<u8>) {
	let pair = ed25519::Pair::generate().0;
	let public_key = pair.public().0.to_vec();
	let signature = vec![0u8; 64]; // Dummy signature for tests that don't verify
	(pair, public_key, signature)
}

// Helper function to create a valid challenge
fn create_valid_challenge<T: Config>(
	account: AccountId32,
	node_id: &[u8],
	expires_at: BlockNumberFor<T>,
) -> Vec<u8> {
	let challenge = AccountProfileChallenge {
		domain: *b"HIPPIUS::PROFILE::v1\0\0\0\0",
		account: account.clone(),
		expires_at,
		genesis_hash: [0u8; 32], // Mock genesis hash
		node_id_hash: H256::from(blake2_256(node_id)),
	};
	challenge.encode()
}

#[test]
fn set_public_item_works() {
	new_test_ext().execute_with(|| {
		let account = create_account(1);
		let public_item = b"0x1234abcd".to_vec();

		// Mock registration pallet to return a validator node
		// (You'll need to implement mock_registration_setup in your mock.rs)

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

		assert_noop!(
			AccountProfile::set_public_item(RuntimeOrigin::signed(account), invalid_hex),
			Error::<Test>::InvalidHexString
		);
	});
}

#[test]
fn set_public_item_fails_without_registered_node() {
	new_test_ext().execute_with(|| {
		let account = create_account(1);
		let public_item = b"0x1234abcd".to_vec();

		// Don't set up registration - should fail
		assert_noop!(
			AccountProfile::set_public_item(RuntimeOrigin::signed(account), public_item),
			Error::<Test>::NodeNotRegistered
		);
	});
}

#[test]
fn set_private_item_works() {
	new_test_ext().execute_with(|| {
		let account = create_account(1);
		let private_item = b"0x5678ef".to_vec();

		assert_ok!(AccountProfile::set_private_item(
			RuntimeOrigin::signed(account.clone()),
			private_item.clone()
		));

		let stored_item = AccountProfile::user_private_storage(&account);
		assert_eq!(stored_item, b"5678ef".to_vec());

		System::assert_last_event(Event::PrivateItemSet(account, b"5678ef".to_vec()).into());
	});
}

#[test]
fn set_username_works() {
	new_test_ext().execute_with(|| {
		let account = create_account(1);
		let username = b"testuser".to_vec();

		assert_ok!(AccountProfile::set_username(
			RuntimeOrigin::signed(account.clone()),
			username.clone()
		));

		let stored_username = AccountProfile::get_username_by_account_id(&account);
		assert_eq!(stored_username, Some(b"testuser".to_vec()));

		let account_by_username = AccountProfile::get_account_id_by_username(&b"testuser".to_vec());
		assert_eq!(account_by_username, Some(account.clone()));

		System::assert_last_event(Event::UsernameSet(account, username).into());
	});
}

#[test]
fn set_username_fails_with_duplicate_username() {
	new_test_ext().execute_with(|| {
		let account1 = create_account(1);
		let account2 = create_account(2);
		let username = b"testuser".to_vec();

		assert_ok!(AccountProfile::set_username(
			RuntimeOrigin::signed(account1.clone()),
			username.clone()
		));

		assert_noop!(
			AccountProfile::set_username(RuntimeOrigin::signed(account2), username),
			Error::<Test>::UsernameAlreadyTaken
		);
	});
}

#[test]
fn set_data_public_key_works() {
	new_test_ext().execute_with(|| {
		let account = create_account(1);
		let data_public_key = b"0x1234567890abcdef".to_vec();

		assert_ok!(AccountProfile::set_data_public_key(
			RuntimeOrigin::signed(account.clone()),
			data_public_key.clone()
		));

		let stored_key = AccountProfile::get_data_public_key(&account);
		assert_eq!(stored_key, Some(b"1234567890abcdef".to_vec()));

		System::assert_last_event(Event::DataPublicKeySet(account).into());
	});
}

#[test]
fn set_message_public_key_works() {
	new_test_ext().execute_with(|| {
		let account = create_account(1);
		let message_public_key = b"0xfedcba0987654321".to_vec();

		assert_ok!(AccountProfile::set_message_public_key(
			RuntimeOrigin::signed(account.clone()),
			message_public_key.clone()
		));

		let stored_key = AccountProfile::get_message_public_key(&account);
		assert_eq!(stored_key, Some(b"fedcba0987654321".to_vec()));

		System::assert_last_event(Event::MessagePublicKeySet(account).into());
	});
}

// Tests for the updated account profile extrinsics with signature verification

#[test]
fn set_account_profile_works() {
	new_test_ext().execute_with(|| {
		let account = create_account(1);
		let node_id = create_node_id(1);
		let encryption_key = b"0xencryptionkey123".to_vec();
		let node_id_hex = node_id.clone(); // In real scenario, this would be hex encoding

		let current_block = System::block_number();
		let challenge_bytes =
			create_valid_challenge::<Test>(account.clone(), &node_id_hex, current_block + 100);

		let (_, public_key, signature) = create_test_keypair();

		// Mock registration pallet to return node info with matching node_id
		// and stored identity

		assert_ok!(AccountProfile::set_account_profile(
			RuntimeOrigin::signed(account.clone()),
			node_id.clone(),
			encryption_key.clone(),
			challenge_bytes,
			signature,
			public_key,
			node_id_hex,
		));

		// Check storage
		let stored_profile = AccountProfile::account_profiles(&account).unwrap();
		assert_eq!(stored_profile.node_id, node_id);
		assert_eq!(stored_profile.encryption_key, encryption_key);

		// Check event
		System::assert_last_event(Event::AccountProfileSet(account).into());
	});
}

#[test]
fn update_account_profile_works() {
	new_test_ext().execute_with(|| {
		let account = create_account(1);
		let node_id = create_node_id(1);
		let encryption_key = b"0xnewencryptionkey456".to_vec();
		let node_id_hex = node_id.clone();

		let current_block = System::block_number();
		let challenge_bytes =
			create_valid_challenge::<Test>(account.clone(), &node_id_hex, current_block + 100);

		let (_, public_key, signature) = create_test_keypair();

		// First set a profile
		let initial_profile =
			AccountProfile { node_id: node_id.clone(), encryption_key: b"0xoldkey".to_vec() };
		AccountProfiles::<Test>::insert(&account, initial_profile);

		// Now update it
		assert_ok!(AccountProfile::update_account_profile(
			RuntimeOrigin::signed(account.clone()),
			node_id.clone(),
			encryption_key.clone(),
			challenge_bytes,
			signature,
			public_key,
			node_id_hex,
		));

		// Check storage was updated
		let stored_profile = AccountProfile::account_profiles(&account).unwrap();
		assert_eq!(stored_profile.encryption_key, encryption_key);

		System::assert_last_event(Event::AccountProfileUpdated(account).into());
	});
}

#[test]
fn set_account_profile_fails_with_expired_challenge() {
	new_test_ext().execute_with(|| {
		let account = create_account(1);
		let node_id = create_node_id(1);
		let encryption_key = b"0xencryptionkey123".to_vec();
		let node_id_hex = node_id.clone();

		let current_block = System::block_number();
		// Challenge expires in the past
		let challenge_bytes =
			create_valid_challenge::<Test>(account.clone(), &node_id_hex, current_block - 1);

		let (_, public_key, signature) = create_test_keypair();

		assert_noop!(
			AccountProfile::set_account_profile(
				RuntimeOrigin::signed(account.clone()),
				node_id,
				encryption_key,
				challenge_bytes,
				signature,
				public_key,
				node_id_hex,
			),
			Error::<Test>::ChallengeExpired
		);
	});
}

#[test]
fn set_account_profile_fails_with_node_id_mismatch() {
	new_test_ext().execute_with(|| {
		let account = create_account(1);
		let node_id = create_node_id(1);
		let wrong_node_id = create_node_id(2);
		let encryption_key = b"0xencryptionkey123".to_vec();
		let node_id_hex = wrong_node_id.clone(); // Wrong hex

		let current_block = System::block_number();
		let challenge_bytes = create_valid_challenge::<Test>(
			account.clone(),
			&node_id_hex, // This will create hash of wrong_node_id
			current_block + 100,
		);

		let (_, public_key, signature) = create_test_keypair();

		assert_noop!(
			AccountProfile::set_account_profile(
				RuntimeOrigin::signed(account.clone()),
				node_id, // Actual node_id is different
				encryption_key,
				challenge_bytes,
				signature,
				public_key,
				node_id_hex,
			),
			Error::<Test>::NodeIdMismatch
		);
	});
}

#[test]
fn set_account_profile_fails_with_reused_challenge() {
	new_test_ext().execute_with(|| {
		let account = create_account(1);
		let node_id = create_node_id(1);
		let encryption_key = b"0xencryptionkey123".to_vec();
		let node_id_hex = node_id.clone();

		let current_block = System::block_number();
		let challenge_bytes =
			create_valid_challenge::<Test>(account.clone(), &node_id_hex, current_block + 100);

		let (_, public_key, signature) = create_test_keypair();

		// First use should work
		assert_ok!(AccountProfile::set_account_profile(
			RuntimeOrigin::signed(account.clone()),
			node_id.clone(),
			encryption_key.clone(),
			challenge_bytes.clone(),
			signature.clone(),
			public_key.clone(),
			node_id_hex.clone(),
		));

		// Second use with same challenge should fail
		assert_noop!(
			AccountProfile::set_account_profile(
				RuntimeOrigin::signed(account.clone()),
				node_id,
				encryption_key,
				challenge_bytes,
				signature,
				public_key,
				node_id_hex,
			),
			Error::<Test>::ChallengeReused
		);
	});
}

#[test]
fn set_account_profile_fails_with_invalid_challenge_domain() {
	new_test_ext().execute_with(|| {
		let account = create_account(1);
		let node_id = create_node_id(1);
		let encryption_key = b"0xencryptionkey123".to_vec();
		let node_id_hex = node_id.clone();

		let current_block = System::block_number();
		let mut challenge = AccountProfileChallenge::<AccountId32, u64> {
			domain: *b"WRONG::DOMAIN::v1\0\0\0\0\0\0", // Wrong domain
			account: account.clone(),
			expires_at: current_block + 100,
			genesis_hash: [0u8; 32],
			node_id_hash: H256::from(blake2_256(&node_id_hex)),
		};
		let challenge_bytes = challenge.encode();

		let (_, public_key, signature) = create_test_keypair();

		assert_noop!(
			AccountProfile::set_account_profile(
				RuntimeOrigin::signed(account.clone()),
				node_id,
				encryption_key,
				challenge_bytes,
				signature,
				public_key,
				node_id_hex,
			),
			Error::<Test>::InvalidChallenge
		);
	});
}

#[test]
fn set_account_profile_fails_with_wrong_account_in_challenge() {
	new_test_ext().execute_with(|| {
		let account = create_account(1);
		let wrong_account = create_account(2);
		let node_id = create_node_id(1);
		let encryption_key = b"0xencryptionkey123".to_vec();
		let node_id_hex = node_id.clone();

		let current_block = System::block_number();
		let mut challenge = AccountProfileChallenge::<AccountId32, u64> {
			domain: *b"HIPPIUS::PROFILE::v1\0\0\0\0",
			account: wrong_account, // Wrong account
			expires_at: current_block + 100,
			genesis_hash: [0u8; 32],
			node_id_hash: H256::from(blake2_256(&node_id_hex)),
		};
		let challenge_bytes = challenge.encode();

		let (_, public_key, signature) = create_test_keypair();

		assert_noop!(
			AccountProfile::set_account_profile(
				RuntimeOrigin::signed(account.clone()),
				node_id,
				encryption_key,
				challenge_bytes,
				signature,
				public_key,
				node_id_hex,
			),
			Error::<Test>::InvalidChallenge
		);
	});
}

#[test]
fn set_account_profile_fails_without_registered_node() {
	new_test_ext().execute_with(|| {
		let account = create_account(1);
		let node_id = create_node_id(1);
		let encryption_key = b"0xencryptionkey123".to_vec();
		let node_id_hex = node_id.clone();

		let current_block = System::block_number();
		let challenge_bytes =
			create_valid_challenge::<Test>(account.clone(), &node_id_hex, current_block + 100);

		let (_, public_key, signature) = create_test_keypair();

		// Don't set up registration - should fail
		assert_noop!(
			AccountProfile::set_account_profile(
				RuntimeOrigin::signed(account.clone()),
				node_id,
				encryption_key,
				challenge_bytes,
				signature,
				public_key,
				node_id_hex,
			),
			Error::<Test>::NodeNotRegistered
		);
	});
}

#[test]
fn hex_strip_prefix_works() {
	new_test_ext().execute_with(|| {
		let account = create_account(1);

		// Test with 0x prefix
		let with_prefix = b"0x1234".to_vec();
		assert_ok!(AccountProfile::set_public_item(
			RuntimeOrigin::signed(account.clone()),
			with_prefix
		));

		// Test without prefix
		let without_prefix = b"5678".to_vec();
		assert_ok!(AccountProfile::set_public_item(RuntimeOrigin::signed(account), without_prefix));

		// Both should work
	});
}

#[test]
fn username_lowercase_conversion_works() {
	new_test_ext().execute_with(|| {
		let account = create_account(1);
		let mixed_case_username = b"TestUser".to_vec();

		assert_ok!(AccountProfile::set_username(
			RuntimeOrigin::signed(account.clone()),
			mixed_case_username
		));

		// Username should be stored in lowercase
		let stored_username = AccountProfile::get_username_by_account_id(&account);
		assert_eq!(stored_username, Some(b"testuser".to_vec()));

		// Should be retrievable by lowercase username
		let account_by_username = AccountProfile::get_account_id_by_username(&b"testuser".to_vec());
		assert_eq!(account_by_username, Some(account));
	});
}

#[test]
fn multiple_users_with_different_usernames_works() {
	new_test_ext().execute_with(|| {
		let account1 = create_account(1);
		let account2 = create_account(2);

		assert_ok!(AccountProfile::set_username(
			RuntimeOrigin::signed(account1.clone()),
			b"user1".to_vec()
		));

		assert_ok!(AccountProfile::set_username(
			RuntimeOrigin::signed(account2.clone()),
			b"user2".to_vec()
		));

		assert_eq!(AccountProfile::get_account_id_by_username(&b"user1".to_vec()), Some(account1));
		assert_eq!(AccountProfile::get_account_id_by_username(&b"user2".to_vec()), Some(account2));
	});
}

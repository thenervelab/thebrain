//! HIP-01 guards: only trusted validators write rankings (#1), and a coldkey registration must
//! be signed by the claimed owner (#2).

use frame_support::{assert_noop, assert_ok};
use hippius_mainnet_runtime::{
	AccountId, RankingStorage, Registration, Runtime, RuntimeOrigin, System,
};
use pallet_registration::{
	ColdkeyNodeInfoLite, ColdkeyNodeRegistrationV2, Libp2pKeyType, NodeType, OwnerToNode, Status,
};
use sp_core::crypto::Ss58Codec;
use sp_runtime::{AccountId32, BuildStorage};

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

fn trusted_validator() -> AccountId {
	AccountId32::from_ss58check(pallet_utils::TRUSTED_VALIDATORS[0])
		.expect("TRUSTED_VALIDATORS holds valid SS58 addresses")
}

fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| System::set_block_number(1));
	ext
}

/// Writes an online Validator node for `owner` straight into registration storage, skipping
/// the stake, UID, and libp2p checks `register_node_with_coldkey` would need.
fn register_validator(owner: &AccountId, node_id: &[u8]) {
	ColdkeyNodeRegistrationV2::<Runtime>::insert(
		node_id.to_vec(),
		Some(ColdkeyNodeInfoLite {
			node_id: node_id.to_vec(),
			node_type: NodeType::Validator,
			status: Status::Online,
			registered_at: 1,
			owner: owner.clone(),
		}),
	);
	OwnerToNode::<Runtime>::insert(owner, vec![node_id.to_vec()]);
}

fn update_rankings(who: AccountId) -> sp_runtime::DispatchResult {
	RankingStorage::update_rankings(RuntimeOrigin::signed(who), vec![], vec![], vec![], vec![])
}

#[test]
fn every_trusted_validator_parses_and_is_trusted() {
	for ss58 in pallet_utils::TRUSTED_VALIDATORS {
		let account = AccountId32::from_ss58check(ss58).unwrap();
		assert!(pallet_utils::is_trusted_validator(&account), "{ss58}");
	}
	assert!(!pallet_utils::is_trusted_validator(&account(7)));
	// Non-32-byte encodings are rejected rather than truncated or padded.
	assert!(!pallet_utils::is_trusted_validator(&7u64));
}

#[test]
fn untrusted_registered_validator_cannot_update_rankings() {
	new_test_ext().execute_with(|| {
		let outsider = account(7);
		register_validator(&outsider, b"outsider-node");

		assert_noop!(
			update_rankings(outsider),
			pallet_rankings::Error::<Runtime>::NotAllowedValidator
		);
	});
}

#[test]
fn trusted_registered_validator_can_update_rankings() {
	new_test_ext().execute_with(|| {
		let validator = trusted_validator();
		register_validator(&validator, b"trusted-node");

		assert_ok!(update_rankings(validator));
	});
}

#[test]
fn trusted_but_unregistered_account_cannot_update_rankings() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			update_rankings(trusted_validator()),
			pallet_rankings::Error::<Runtime>::NodeNotRegistered
		);
	});
}

#[test]
fn coldkey_registration_rejects_signer_other_than_owner() {
	new_test_ext().execute_with(|| {
		let attacker = account(1);
		let victim = account(2);

		assert_noop!(
			Registration::register_node_with_coldkey(
				RuntimeOrigin::signed(attacker),
				NodeType::StorageMiner,
				b"node".to_vec(),
				false,
				victim,
				Libp2pKeyType::Ed25519,
				vec![],
				vec![],
				vec![],
				vec![],
			),
			pallet_registration::Error::<Runtime>::NotNodeOwner
		);
	});
}

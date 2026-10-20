//! `Registration::register_node_with_coldkey` must be signed by `owner`
//! (the HIP-01 #2 guard from #73, which returns `NotNodeOwner`).
//!
//! The node's libp2p signature over the challenge binds it to `owner`, but it
//! is not the owner's consent: anyone holding a node key could otherwise
//! register under someone else's account.

use frame_support::assert_noop;
use hippius_mainnet_runtime::{AccountId, Registration, Runtime, RuntimeOrigin, System};
use pallet_registration::{Libp2pKeyType, NodeType};
use sp_runtime::{AccountId32, BuildStorage};

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| System::set_block_number(1));
	ext
}

fn call(
	signer: &AccountId,
	owner: &AccountId,
) -> frame_support::dispatch::DispatchResultWithPostInfo {
	Registration::register_node_with_coldkey(
		RuntimeOrigin::signed(signer.clone()),
		NodeType::StorageMiner,
		b"node".to_vec(),
		false,
		owner.clone(),
		Libp2pKeyType::Ed25519,
		vec![0u8; 32],
		vec![0u8; 64],
		vec![], // undecodable challenge
		b"node".to_vec(),
	)
}

#[test]
fn rejects_a_signer_other_than_owner() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			call(&account(1), &account(2)),
			pallet_registration::Error::<Runtime>::NotNodeOwner
		);
	});
}

#[test]
fn owner_signed_call_passes_the_origin_check() {
	new_test_ext().execute_with(|| {
		// Same garbage arguments, signed by the owner: it gets past the origin
		// check and fails on the challenge instead.
		assert_noop!(
			call(&account(2), &account(2)),
			pallet_registration::Error::<Runtime>::InvalidChallenge
		);
	});
}

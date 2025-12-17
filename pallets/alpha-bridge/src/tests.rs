use crate::{mock::*, Error, Event, Pallet as AlphaBridge};
use frame_support::{assert_noop, assert_ok};

fn next_checkpoint_nonce() -> u64 {
	match crate::LastCheckpointNonce::<Test>::get() {
		Some(nonce) => nonce.checked_add(1).expect("checkpoint nonce overflow in tests"),
		None => 0,
	}
}

#[test]
fn test_successful_deposit_proposal_by_guardian() {
	new_test_ext().execute_with(|| {
		let deposits = vec![create_deposit_proposal(bob(), 1000)];
		let deposit_id = deposits[0].id;

		assert_ok!(AlphaBridge::<Test>::propose_deposits(
			RuntimeOrigin::signed(alice()),
			deposits.clone(),
			next_checkpoint_nonce(),
		));

		// Check event
		System::assert_has_event(
			Event::DepositsProposed { deposit_ids: vec![deposit_id], proposer: alice() }.into(),
		);

		assert!(crate::PendingDeposits::<Test>::contains_key(deposit_id));
	});
}

#[test]
fn test_deposit_proposal_by_non_guardian_fails() {
	new_test_ext().execute_with(|| {
		let deposits = vec![create_deposit_proposal(user2(), 1000)];

		assert_noop!(
			AlphaBridge::<Test>::propose_deposits(
				RuntimeOrigin::signed(user2()),
				deposits,
				next_checkpoint_nonce(),
			),
			Error::<Test>::NotGuardian
		);
	});
}

#[test]
fn test_batch_deposit_proposal() {
	new_test_ext().execute_with(|| {
		let deposits = vec![
			create_deposit_proposal(user1(), 1000),
			create_deposit_proposal(user2(), 2000),
			create_deposit_proposal(user1(), 500),
		];
		let deposit_ids: Vec<_> = deposits.iter().map(|d| d.id).collect();

		assert_ok!(AlphaBridge::<Test>::propose_deposits(
			RuntimeOrigin::signed(alice()),
			deposits,
			next_checkpoint_nonce(),
		));

		// Check that all deposits were created
		System::assert_has_event(Event::DepositsProposed { deposit_ids, proposer: alice() }.into());
	});
}

#[test]
fn test_deposit_approval_with_threshold_votes() {
	new_test_ext().execute_with(|| {
		// Set threshold to 2
		crate::ApproveThreshold::<Test>::put(2);

		let deposits = vec![create_deposit_proposal(user1(), 1000)];
		let deposit_id = deposits[0].id;
		assert_ok!(AlphaBridge::<Test>::propose_deposits(
			RuntimeOrigin::signed(alice()),
			deposits,
			next_checkpoint_nonce(),
		));

		// First vote (alice() already voted when proposing)
		// Second vote should approve
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			true
		));

		// Check that deposit was approved and minted
		assert_eq!(Balances::free_balance(&user1()) - INITIAL_BALANCE, 1000);

		// Check event
		System::assert_has_event(
			Event::BridgeMinted { deposit_id, recipient: user1(), amount: 1000 }.into(),
		);
	});
}

#[test]
fn test_deposit_minting_to_new_account() {
	new_test_ext().execute_with(|| {
		crate::ApproveThreshold::<Test>::put(DEFAULT_APPROVE_THRESHOLD);

		// Use new_account() which is NOT in genesis
		let new_recipient = new_account();

		// Verify account doesn't exist yet (balance is 0)
		assert_eq!(Balances::free_balance(&new_recipient), 0);

		let deposits = vec![create_deposit_proposal(new_recipient.clone(), 1000)];
		let deposit_id = deposits[0].id;

		assert_ok!(AlphaBridge::<Test>::propose_deposits(
			RuntimeOrigin::signed(alice()),
			deposits,
			next_checkpoint_nonce(),
		));

		// Second vote to approve
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			true
		));

		// Verify account was created with minted balance
		// Since account started at 0, balance should equal minted amount
		assert_eq!(Balances::free_balance(&new_recipient), 1000);

		// Verify event
		System::assert_has_event(
			Event::BridgeMinted { deposit_id, recipient: new_recipient, amount: 1000 }.into(),
		);

		// Verify total minted tracking
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 1000);
	});
}

#[test]
fn test_deposit_below_existential_deposit_fails() {
	new_test_ext().execute_with(|| {
		crate::ApproveThreshold::<Test>::put(DEFAULT_APPROVE_THRESHOLD);

		let new_recipient = new_account();

		// Try to mint 0 tokens (below existential deposit of 1)
		let deposits = vec![create_deposit_proposal(new_recipient.clone(), 0)];
		let deposit_id = deposits[0].id;

		assert_ok!(AlphaBridge::<Test>::propose_deposits(
			RuntimeOrigin::signed(alice()),
			deposits,
			next_checkpoint_nonce(),
		));

		// This should fail when trying to create account with 0 balance
		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(RuntimeOrigin::signed(bob()), deposit_id, true),
			Error::<Test>::DepositMintFailed
		);

		// Verify account was NOT created
		assert_eq!(Balances::free_balance(&new_recipient), 0);

		// Verify no minting occurred
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 0);
	});
}

#[test]
fn test_deposit_denial_with_threshold_votes() {
	new_test_ext().execute_with(|| {
		// Set thresholds
		crate::ApproveThreshold::<Test>::put(2);
		crate::DenyThreshold::<Test>::put(2);

		let deposits = vec![create_deposit_proposal(user1(), 1000)];
		let deposit_id = deposits[0].id;
		assert_ok!(AlphaBridge::<Test>::propose_deposits(
			RuntimeOrigin::signed(alice()),
			deposits,
			next_checkpoint_nonce(),
		));

		// Two deny votes
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			false
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(charlie()),
			deposit_id,
			false
		));

		// Check that deposit was denied (not minted)
		assert_eq!(Balances::free_balance(&user1()), INITIAL_BALANCE);

		// Check event
		System::assert_has_event(
			Event::BridgeDenied { deposit_id, recipient: user1(), amount: 1000 }.into(),
		);
	});
}

#[test]
fn test_deposit_attestation_by_non_guardian_fails() {
	new_test_ext().execute_with(|| {
		crate::ApproveThreshold::<Test>::put(2);

		let deposits = vec![create_deposit_proposal(user1(), 1000)];
		let deposit_id = deposits[0].id;
		assert_ok!(AlphaBridge::<Test>::propose_deposits(
			RuntimeOrigin::signed(alice()),
			deposits,
			next_checkpoint_nonce(),
		));

		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(RuntimeOrigin::signed(user1()), deposit_id, true),
			Error::<Test>::NotGuardian
		);
	});
}

#[test]
fn test_double_voting_on_deposit_fails() {
	new_test_ext().execute_with(|| {
		crate::ApproveThreshold::<Test>::put(3);

		let deposits = vec![create_deposit_proposal(user1(), 1000)];
		let deposit_id = deposits[0].id;
		assert_ok!(AlphaBridge::<Test>::propose_deposits(
			RuntimeOrigin::signed(alice()),
			deposits,
			next_checkpoint_nonce(),
		));

		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			true
		));

		// Try to vote again
		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(RuntimeOrigin::signed(bob()), deposit_id, true),
			Error::<Test>::AlreadyVoted
		);
	});
}

#[test]
fn test_voting_on_non_existent_deposit_fails() {
	new_test_ext().execute_with(|| {
		use sp_core::H256;
		let fake_deposit_id = H256::from_low_u64_be(99);
		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(
				RuntimeOrigin::signed(alice()),
				fake_deposit_id,
				true
			),
			Error::<Test>::DepositNotPending
		);
	});
}

#[test]
fn test_empty_deposit_batch_fails() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			AlphaBridge::<Test>::propose_deposits(
				RuntimeOrigin::signed(alice()),
				vec![],
				next_checkpoint_nonce(),
			),
			Error::<Test>::EmptyDepositBatch
		);
	});
}

#[test]
fn test_deposit_while_paused_fails() {
	new_test_ext().execute_with(|| {
		crate::Paused::<Test>::put(true);

		let deposits = vec![create_deposit_proposal(user1(), 1000)];

		assert_noop!(
			AlphaBridge::<Test>::propose_deposits(
				RuntimeOrigin::signed(alice()),
				deposits,
				next_checkpoint_nonce(),
			),
			Error::<Test>::BridgePaused
		);
	});
}

#[test]
fn test_checkpoint_nonce_must_start_at_zero() {
	new_test_ext().execute_with(|| {
		let deposits = vec![create_deposit_proposal(user1(), 1000)];

		assert_noop!(
			AlphaBridge::<Test>::propose_deposits(RuntimeOrigin::signed(alice()), deposits, 1,),
			Error::<Test>::InvalidCheckpointNonce
		);
	});
}

#[test]
fn test_duplicate_checkpoint_nonce_fails() {
	new_test_ext().execute_with(|| {
		let first_deposits = vec![create_deposit_proposal(user1(), 1000)];
		let first_nonce = next_checkpoint_nonce();
		assert_ok!(AlphaBridge::<Test>::propose_deposits(
			RuntimeOrigin::signed(alice()),
			first_deposits,
			first_nonce,
		));

		let second_deposits = vec![create_deposit_proposal(user2(), 2000)];
		assert_noop!(
			AlphaBridge::<Test>::propose_deposits(
				RuntimeOrigin::signed(alice()),
				second_deposits,
				first_nonce,
			),
			Error::<Test>::InvalidCheckpointNonce
		);
	});
}

#[test]
fn test_skipped_checkpoint_nonce_fails() {
	new_test_ext().execute_with(|| {
		let first_deposits = vec![create_deposit_proposal(user1(), 1000)];
		let first_nonce = next_checkpoint_nonce();
		assert_ok!(AlphaBridge::<Test>::propose_deposits(
			RuntimeOrigin::signed(alice()),
			first_deposits,
			first_nonce,
		));

		let second_deposits = vec![create_deposit_proposal(user2(), 2000)];
		assert_noop!(
			AlphaBridge::<Test>::propose_deposits(
				RuntimeOrigin::signed(alice()),
				second_deposits,
				2,
			),
			Error::<Test>::InvalidCheckpointNonce
		);
	});
}

#[test]
fn test_user_unlock_request() {
	new_test_ext().execute_with(|| {
		let initial_balance = Balances::free_balance(&user1());

		assert_ok!(AlphaBridge::<Test>::request_unlock(RuntimeOrigin::signed(user1()), 1000));

		// Check that balance was locked (transferred to escrow)
		assert_eq!(Balances::free_balance(&user1()), initial_balance - 1000);

		// Get the burn_id that was generated
		let burn_id = {
			// There should be exactly one pending unlock
			let pending: Vec<_> = crate::PendingUnlocks::<Test>::iter().collect();
			assert_eq!(pending.len(), 1);
			pending[0].0
		};

		// Check event
		System::assert_has_event(
			Event::UnlockRequested { burn_id, requester: user1(), amount: 1000 }.into(),
		);

		// Check that unlock is pending
		assert!(crate::PendingUnlocks::<Test>::contains_key(burn_id));
	});
}

#[test]
fn test_unlock_request_with_insufficient_balance_fails() {
	new_test_ext().execute_with(|| {
		let balance = Balances::free_balance(&user1());
		let amount = balance + 1000;

		assert_noop!(
			AlphaBridge::<Test>::request_unlock(RuntimeOrigin::signed(user1()), amount),
			sp_runtime::TokenError::FundsUnavailable
		);
	});
}

#[test]
fn test_unlock_approval_with_threshold_votes() {
	new_test_ext().execute_with(|| {
		let initial_balance = Balances::free_balance(&user1());

		assert_ok!(AlphaBridge::<Test>::request_unlock(RuntimeOrigin::signed(user1()), 1000));

		// Get the burn_id that was generated
		let burn_id = {
			let pending: Vec<_> = crate::PendingUnlocks::<Test>::iter().collect();
			assert_eq!(pending.len(), 1);
			pending[0].0
		};

		// Approve with threshold (2 votes)
		assert_ok!(AlphaBridge::<Test>::attest_unlock(
			RuntimeOrigin::signed(alice()),
			burn_id,
			true
		));
		assert_ok!(AlphaBridge::<Test>::attest_unlock(RuntimeOrigin::signed(bob()), burn_id, true));

		// Check that unlock was approved (balance burned from escrow, not restored to user)
		assert_eq!(Balances::free_balance(&user1()), initial_balance - 1000);

		// Check event
		System::assert_has_event(
			Event::UnlockApproved { burn_id, requester: user1(), amount: 1000 }.into(),
		);
	});
}

#[test]
fn test_unlock_denial_with_threshold_votes() {
	new_test_ext().execute_with(|| {
		let initial_balance = Balances::free_balance(&user1());

		assert_ok!(AlphaBridge::<Test>::request_unlock(RuntimeOrigin::signed(user1()), 1000));

		// Get the burn_id that was generated
		let burn_id = {
			let pending: Vec<_> = crate::PendingUnlocks::<Test>::iter().collect();
			assert_eq!(pending.len(), 1);
			pending[0].0
		};

		// Deny with threshold (2 votes)
		assert_ok!(AlphaBridge::<Test>::attest_unlock(
			RuntimeOrigin::signed(alice()),
			burn_id,
			false
		));
		assert_ok!(AlphaBridge::<Test>::attest_unlock(
			RuntimeOrigin::signed(bob()),
			burn_id,
			false
		));

		// Check that balance was restored
		assert_eq!(Balances::free_balance(&user1()), initial_balance);

		// Check event
		System::assert_has_event(
			Event::UnlockDenied { burn_id, requester: user1(), amount: 1000 }.into(),
		);
	});
}

#[test]
fn test_unlock_attestation_by_non_guardian_fails() {
	new_test_ext().execute_with(|| {
		assert_ok!(AlphaBridge::<Test>::request_unlock(RuntimeOrigin::signed(user1()), 1000));

		let burn_id = {
			let pending: Vec<_> = crate::PendingUnlocks::<Test>::iter().collect();
			assert_eq!(pending.len(), 1);
			pending[0].0
		};

		assert_noop!(
			AlphaBridge::<Test>::attest_unlock(RuntimeOrigin::signed(user2()), burn_id, true),
			Error::<Test>::NotGuardian
		);
	});
}

#[test]
fn test_double_voting_on_unlock_fails() {
	new_test_ext().execute_with(|| {
		crate::ApproveThreshold::<Test>::put(3);

		assert_ok!(AlphaBridge::<Test>::request_unlock(RuntimeOrigin::signed(user1()), 1000));

		let burn_id = {
			let pending: Vec<_> = crate::PendingUnlocks::<Test>::iter().collect();
			assert_eq!(pending.len(), 1);
			pending[0].0
		};

		assert_ok!(AlphaBridge::<Test>::attest_unlock(
			RuntimeOrigin::signed(alice()),
			burn_id,
			true
		));

		// Try to vote again
		assert_noop!(
			AlphaBridge::<Test>::attest_unlock(RuntimeOrigin::signed(alice()), burn_id, true),
			Error::<Test>::AlreadyVoted
		);
	});
}

#[test]
fn test_unlock_while_paused_fails() {
	new_test_ext().execute_with(|| {
		crate::Paused::<Test>::put(true);

		assert_noop!(
			AlphaBridge::<Test>::request_unlock(RuntimeOrigin::signed(user1()), 1000),
			Error::<Test>::BridgePaused
		);
	});
}

#[test]
fn test_deposit_expiration_after_ttl() {
	new_test_ext().execute_with(|| {
		crate::ApproveThreshold::<Test>::put(3);
		crate::SignatureTTLBlocks::<Test>::put(10);

		let deposits = vec![create_deposit_proposal(user1(), 1000)];
		let deposit_id = deposits[0].id;
		let checkpoint_nonce = next_checkpoint_nonce();
		assert_ok!(AlphaBridge::<Test>::propose_deposits(
			RuntimeOrigin::signed(alice()),
			deposits,
			checkpoint_nonce,
		));

		// Run past TTL
		run_to_block(15);

		// Anyone can expire
		assert_ok!(AlphaBridge::<Test>::expire_pending_deposit(
			RuntimeOrigin::signed(user2()),
			deposit_id
		));

		// Check event
		System::assert_has_event(
			Event::DepositExpired { deposit_id, recipient: user1(), amount: 1000 }.into(),
		);

		// Check that deposit is no longer pending
		assert!(!crate::PendingDeposits::<Test>::contains_key(deposit_id));
	});
}

#[test]
fn test_unlock_expiration_after_ttl() {
	new_test_ext().execute_with(|| {
		crate::ApproveThreshold::<Test>::put(3);
		crate::SignatureTTLBlocks::<Test>::put(10);
		let initial_balance = Balances::free_balance(&user1());

		assert_ok!(AlphaBridge::<Test>::request_unlock(RuntimeOrigin::signed(user1()), 1000));

		let burn_id = {
			let pending: Vec<_> = crate::PendingUnlocks::<Test>::iter().collect();
			assert_eq!(pending.len(), 1);
			pending[0].0
		};

		// Balance should be locked (transferred to escrow)
		assert_eq!(Balances::free_balance(&user1()), initial_balance - 1000);

		// Run past TTL
		run_to_block(15);

		// Anyone can expire
		assert_ok!(AlphaBridge::<Test>::expire_pending_unlock(
			RuntimeOrigin::signed(user2()),
			burn_id
		));

		// Check that balance was restored
		assert_eq!(Balances::free_balance(&user1()), initial_balance);

		// Check event
		System::assert_has_event(Event::UnlockExpired { burn_id }.into());
	});
}

#[test]
fn test_expiration_before_ttl_fails() {
	new_test_ext().execute_with(|| {
		crate::ApproveThreshold::<Test>::put(3);
		crate::SignatureTTLBlocks::<Test>::put(100);

		let deposits = vec![create_deposit_proposal(user1(), 1000)];
		let deposit_id = deposits[0].id;
		let checkpoint_nonce = next_checkpoint_nonce();
		assert_ok!(AlphaBridge::<Test>::propose_deposits(
			RuntimeOrigin::signed(alice()),
			deposits,
			checkpoint_nonce,
		));

		run_to_block(5);

		assert_noop!(
			AlphaBridge::<Test>::expire_pending_deposit(RuntimeOrigin::signed(user2()), deposit_id),
			Error::<Test>::ProposalNotExpired
		);
	});
}

#[test]
fn test_add_guardian_by_root() {
	new_test_ext().execute_with(|| {
		assert_ok!(AlphaBridge::<Test>::add_guardian(RuntimeOrigin::root(), dave()));

		assert!(crate::Guardians::<Test>::get().contains(&dave()));

		System::assert_has_event(Event::GuardianAdded { guardian: dave() }.into());
	});
}

#[test]
fn test_add_guardian_by_non_root_fails() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			AlphaBridge::<Test>::add_guardian(RuntimeOrigin::signed(alice()), dave()),
			sp_runtime::DispatchError::BadOrigin
		);
	});
}

#[test]
fn test_remove_guardian() {
	new_test_ext().execute_with(|| {
		// Add more guardians first to ensure thresholds are still achievable
		assert_ok!(AlphaBridge::<Test>::add_guardian(RuntimeOrigin::root(), dave()));
		assert_ok!(AlphaBridge::<Test>::add_guardian(RuntimeOrigin::root(), user2()));

		assert_ok!(AlphaBridge::<Test>::remove_guardian(RuntimeOrigin::root(), charlie()));

		assert!(!crate::Guardians::<Test>::get().contains(&charlie()));

		System::assert_has_event(Event::GuardianRemoved { guardian: charlie() }.into());
	});
}

#[test]
fn test_duplicate_guardian_fails() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			AlphaBridge::<Test>::add_guardian(RuntimeOrigin::root(), alice()),
			Error::<Test>::GuardianAlreadyExists
		);
	});
}

#[test]
fn test_removing_non_existent_guardian_fails() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			AlphaBridge::<Test>::remove_guardian(RuntimeOrigin::root(), dave()),
			Error::<Test>::GuardianNotFound
		);
	});
}

#[test]
fn test_set_approve_threshold() {
	new_test_ext().execute_with(|| {
		assert_ok!(AlphaBridge::<Test>::set_approve_threshold(RuntimeOrigin::root(), 3));

		assert_eq!(crate::ApproveThreshold::<Test>::get(), 3);

		System::assert_has_event(Event::ApproveThresholdUpdated { new_threshold: 3 }.into());
	});
}

#[test]
fn test_set_deny_threshold() {
	new_test_ext().execute_with(|| {
		assert_ok!(AlphaBridge::<Test>::set_deny_threshold(RuntimeOrigin::root(), 3));

		assert_eq!(crate::DenyThreshold::<Test>::get(), 3);

		System::assert_has_event(Event::DenyThresholdUpdated { new_threshold: 3 }.into());
	});
}

#[test]
fn test_threshold_exceeding_guardian_count_fails() {
	new_test_ext().execute_with(|| {
		// Only 3 guardians: alice(), bob(), charlie()
		assert_noop!(
			AlphaBridge::<Test>::set_approve_threshold(RuntimeOrigin::root(), 10),
			Error::<Test>::ThresholdTooHigh
		);
	});
}

#[test]
fn test_threshold_zero_fails() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			AlphaBridge::<Test>::set_approve_threshold(RuntimeOrigin::root(), 0),
			Error::<Test>::ThresholdTooLow
		);
	});
}

#[test]
fn test_pause_bridge() {
	new_test_ext().execute_with(|| {
		assert_ok!(AlphaBridge::<Test>::set_paused(RuntimeOrigin::root(), true));

		assert_eq!(crate::Paused::<Test>::get(), true);

		System::assert_has_event(Event::BridgePaused { paused: true }.into());
	});
}

#[test]
fn test_unpause_bridge() {
	new_test_ext().execute_with(|| {
		crate::Paused::<Test>::put(true);

		assert_ok!(AlphaBridge::<Test>::set_paused(RuntimeOrigin::root(), false));

		assert_eq!(crate::Paused::<Test>::get(), false);

		System::assert_has_event(Event::BridgePaused { paused: false }.into());
	});
}

#[test]
fn test_set_global_mint_cap() {
	new_test_ext().execute_with(|| {
		assert_ok!(AlphaBridge::<Test>::set_global_mint_cap(RuntimeOrigin::root(), 50_000_000));

		assert_eq!(crate::GlobalMintCap::<Test>::get(), 50_000_000);

		System::assert_has_event(Event::GlobalMintCapUpdated { new_cap: 50_000_000 }.into());
	});
}

#[test]
fn test_minting_exceeding_cap_fails() {
	new_test_ext().execute_with(|| {
		// Set a low cap
		crate::GlobalMintCap::<Test>::put(500);

		let deposits = vec![create_deposit_proposal(user1(), 1000)];

		assert_noop!(
			AlphaBridge::<Test>::propose_deposits(
				RuntimeOrigin::signed(alice()),
				deposits,
				next_checkpoint_nonce(),
			),
			Error::<Test>::GlobalMintCapExceeded
		);
	});
}

#[test]
fn test_finalize_rechecks_mint_cap() {
	new_test_ext().execute_with(|| {
		crate::GlobalMintCap::<Test>::put(1_000);

		let deposit_a = create_deposit_proposal(user1(), 600);
		let deposit_b = create_deposit_proposal(user2(), 600);
		let checkpoint_nonce = next_checkpoint_nonce();

		assert_ok!(AlphaBridge::<Test>::propose_deposits(
			RuntimeOrigin::signed(alice()),
			vec![deposit_a.clone(), deposit_b.clone()],
			checkpoint_nonce,
		));

		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_a.id,
			true
		));
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 600);

		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(RuntimeOrigin::signed(bob()), deposit_b.id, true),
			Error::<Test>::GlobalMintCapExceeded
		);
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 600);
		assert!(crate::PendingDeposits::<Test>::contains_key(deposit_b.id));
	});
}

#[test]
fn test_set_signature_ttl() {
	new_test_ext().execute_with(|| {
		assert_ok!(AlphaBridge::<Test>::set_signature_ttl(RuntimeOrigin::root(), 200));

		assert_eq!(crate::SignatureTTLBlocks::<Test>::get(), 200);

		System::assert_has_event(Event::SignatureTTLUpdated { new_ttl: 200 }.into());
	});
}

#[test]
fn test_full_deposit_cycle() {
	new_test_ext().execute_with(|| {
		crate::ApproveThreshold::<Test>::put(2);

		// 1. Propose deposits
		let deposits = vec![create_deposit_proposal(user1(), 1000)];
		let deposit_id = deposits[0].id;
		let checkpoint_nonce = next_checkpoint_nonce();
		assert_ok!(AlphaBridge::<Test>::propose_deposits(
			RuntimeOrigin::signed(alice()),
			deposits,
			checkpoint_nonce,
		));

		// 2. Attest to reach threshold
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			true
		));

		// 3. Verify minting occurred
		assert_eq!(Balances::free_balance(&user1()) - INITIAL_BALANCE, 1000);

		// 4. Verify deposit is no longer pending
		assert!(!crate::PendingDeposits::<Test>::contains_key(deposit_id));

		// 5. Verify deposit is processed
		assert!(crate::ProcessedDeposits::<Test>::contains_key(deposit_id));

		// 6. Verify total minted updated
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 1000);
	});
}

#[test]
fn test_full_unlock_cycle() {
	new_test_ext().execute_with(|| {
		crate::ApproveThreshold::<Test>::put(2);

		let initial_balance = Balances::free_balance(&user1());

		// 1. Request unlock
		assert_ok!(AlphaBridge::<Test>::request_unlock(RuntimeOrigin::signed(user1()), 1000));

		let burn_id = {
			let pending: Vec<_> = crate::PendingUnlocks::<Test>::iter().collect();
			assert_eq!(pending.len(), 1);
			pending[0].0
		};

		// Verify balance locked
		assert_eq!(Balances::free_balance(&user1()), initial_balance - 1000);

		// 2. Attest to reach threshold
		assert_ok!(AlphaBridge::<Test>::attest_unlock(
			RuntimeOrigin::signed(alice()),
			burn_id,
			true
		));
		assert_ok!(AlphaBridge::<Test>::attest_unlock(RuntimeOrigin::signed(bob()), burn_id, true));

		// 3. Verify burning occurred (balance stays locked, burned from escrow)
		assert_eq!(Balances::free_balance(&user1()), initial_balance - 1000);

		// 4. Verify unlock is no longer pending
		assert!(!crate::PendingUnlocks::<Test>::contains_key(burn_id));

		// 5. Verify burn was enqueued in ordered burn queue
		let burn_nonce =
			crate::BurnNonceById::<Test>::get(burn_id).expect("burn nonce must exist for burn_id");
		let queue_item =
			crate::BurnQueue::<Test>::get(burn_nonce).expect("burn queue item should exist");
		assert_eq!(queue_item.burn_id, burn_id);
		assert_eq!(queue_item.requester, user1());
		assert_eq!(queue_item.amount, 1000);
	});
}

#[test]
fn test_denied_deposit_doesnt_mint() {
	new_test_ext().execute_with(|| {
		crate::ApproveThreshold::<Test>::put(2);
		crate::DenyThreshold::<Test>::put(2);

		let deposits = vec![create_deposit_proposal(user1(), 1000)];
		let deposit_id = deposits[0].id;
		let checkpoint_nonce = next_checkpoint_nonce();
		assert_ok!(AlphaBridge::<Test>::propose_deposits(
			RuntimeOrigin::signed(alice()),
			deposits,
			checkpoint_nonce,
		));

		// Deny
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			false
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(charlie()),
			deposit_id,
			false
		));

		// Verify no minting
		assert_eq!(Balances::free_balance(&user1()), INITIAL_BALANCE);
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 0);
	});
}

#[test]
fn test_denied_unlock_restores_funds() {
	new_test_ext().execute_with(|| {
		crate::ApproveThreshold::<Test>::put(2);
		crate::DenyThreshold::<Test>::put(2);

		let initial_balance = Balances::free_balance(&user1());

		assert_ok!(AlphaBridge::<Test>::request_unlock(RuntimeOrigin::signed(user1()), 1000));

		let burn_id = {
			let pending: Vec<_> = crate::PendingUnlocks::<Test>::iter().collect();
			assert_eq!(pending.len(), 1);
			pending[0].0
		};

		// Balance locked
		assert_eq!(Balances::free_balance(&user1()), initial_balance - 1000);

		// Deny
		assert_ok!(AlphaBridge::<Test>::attest_unlock(
			RuntimeOrigin::signed(alice()),
			burn_id,
			false
		));
		assert_ok!(AlphaBridge::<Test>::attest_unlock(
			RuntimeOrigin::signed(bob()),
			burn_id,
			false
		));

		// Verify funds restored
		assert_eq!(Balances::free_balance(&user1()), initial_balance);
	});
}

#[test]
fn test_multiple_sequential_deposits() {
	new_test_ext().execute_with(|| {
		crate::ApproveThreshold::<Test>::put(2);

		// First batch
		let deposits1 = vec![create_deposit_proposal(user1(), 1000)];
		let deposit_id1 = deposits1[0].id;
		assert_ok!(AlphaBridge::<Test>::propose_deposits(
			RuntimeOrigin::signed(alice()),
			deposits1,
			next_checkpoint_nonce(),
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id1,
			true
		));

		// Second batch
		let deposits2 = vec![create_deposit_proposal(user2(), 2000)];
		let deposit_id2 = deposits2[0].id;
		assert_ok!(AlphaBridge::<Test>::propose_deposits(
			RuntimeOrigin::signed(alice()),
			deposits2,
			next_checkpoint_nonce(),
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id2,
			true
		));

		// Verify both users received minted tokens
		assert_eq!(Balances::free_balance(&user1()) - INITIAL_BALANCE, 1000);
		assert_eq!(Balances::free_balance(&user2()) - INITIAL_BALANCE, 2000);

		// Verify total minted
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 3000);
	});
}
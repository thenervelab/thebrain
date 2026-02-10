use crate::{mock::*, pallet::DepositStatus, Error, Event, Pallet as AlphaBridge};
use frame_support::{assert_noop, assert_ok};

// ============================================================================
// Deposit Flow Tests (Guardian attests deposit -> hAlpha minted to recipient)
// ============================================================================

#[test]
fn test_first_attestation_creates_deposit_record() {
	new_test_ext().execute_with(|| {
		let recipient = user1();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, nonce);

		// First guardian attestation creates the deposit record
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			amount,
			nonce,
		));

		// Check that deposit was created
		let deposit = crate::Deposits::<Test>::get(deposit_id).expect("Deposit should exist");
		assert_eq!(deposit.recipient, recipient);
		assert_eq!(deposit.amount, amount);
		assert_eq!(deposit.status, DepositStatus::Pending);
		assert!(deposit.votes.contains(&alice()));

		// Check event
		System::assert_has_event(
			Event::DepositAttested { id: deposit_id, guardian: alice() }.into(),
		);
	});
}

#[test]
fn test_deposit_completes_when_threshold_reached() {
	new_test_ext().execute_with(|| {
		// Threshold is 2 by default
		let recipient = user1();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, nonce);
		let initial_balance = Balances::free_balance(&recipient);

		// First attestation
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			amount,
			nonce,
		));

		// Second attestation should complete the deposit
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient.clone(),
			amount,
			nonce,
		));

		// Check that hAlpha was minted
		assert_eq!(Balances::free_balance(&recipient), initial_balance + amount);

		// Check deposit status
		let deposit = crate::Deposits::<Test>::get(deposit_id).expect("Deposit should exist");
		assert_eq!(deposit.status, DepositStatus::Completed);

		// Check events
		System::assert_has_event(
			Event::DepositCompleted { id: deposit_id, recipient, amount }.into(),
		);

		// Check total minted tracking
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 1000);
	});
}

#[test]
fn test_deposit_attestation_by_non_guardian_fails() {
	new_test_ext().execute_with(|| {
		let recipient = user2();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, nonce);

		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(
				RuntimeOrigin::signed(user1()),
				deposit_id,
				recipient,
				amount,
				nonce,
			),
			Error::<Test>::NotGuardian
		);
	});
}

#[test]
fn test_double_voting_on_deposit_fails() {
	new_test_ext().execute_with(|| {
		// Set threshold to 3 so deposit doesn't complete on second vote
		crate::ApproveThreshold::<Test>::put(3);

		let recipient = user1();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, nonce);

		// First attestation
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			amount,
			nonce,
		));

		// Same guardian tries to vote again
		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(
				RuntimeOrigin::signed(alice()),
				deposit_id,
				recipient,
				amount,
				nonce,
			),
			Error::<Test>::AlreadyVoted
		);
	});
}

#[test]
fn test_attestation_on_completed_deposit_fails() {
	new_test_ext().execute_with(|| {
		let recipient = user1();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, nonce);

		// Complete the deposit with threshold votes
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			amount,
			nonce,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient.clone(),
			amount,
			nonce,
		));

		// Third guardian tries to attest on completed deposit
		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(
				RuntimeOrigin::signed(charlie()),
				deposit_id,
				recipient,
				amount,
				nonce,
			),
			Error::<Test>::DepositAlreadyCompleted
		);
	});
}

#[test]
fn test_deposit_while_paused_fails() {
	new_test_ext().execute_with(|| {
		crate::Paused::<Test>::put(true);

		let recipient = user1();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, nonce);

		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(
				RuntimeOrigin::signed(alice()),
				deposit_id,
				recipient,
				amount,
				nonce,
			),
			Error::<Test>::BridgePaused
		);
	});
}

#[test]
fn test_deposit_exceeding_mint_cap_fails() {
	new_test_ext().execute_with(|| {
		// Set a low cap
		crate::GlobalMintCap::<Test>::put(500);

		let recipient = user1();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, nonce);

		// First attestation succeeds (just creates record)
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			amount,
			nonce,
		));

		// Second attestation should fail at finalization due to cap
		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(
				RuntimeOrigin::signed(bob()),
				deposit_id,
				recipient,
				amount,
				nonce,
			),
			Error::<Test>::CapExceeded
		);
	});
}

#[test]
fn test_deposit_minting_to_new_account() {
	new_test_ext().execute_with(|| {
		let new_recipient = new_account();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&new_recipient, amount, nonce);

		// Verify account doesn't exist yet
		assert_eq!(Balances::free_balance(&new_recipient), 0);

		// Complete deposit
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			new_recipient.clone(),
			amount,
			nonce,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			new_recipient.clone(),
			amount,
			nonce,
		));

		// Verify account was created with minted balance
		assert_eq!(Balances::free_balance(&new_recipient), amount);

		// Verify total minted tracking
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), amount);
	});
}

// ============================================================================
// Poisoning Prevention Tests (InvalidRequestId — hash-based ID verification)
// ============================================================================

#[test]
fn test_attestation_with_wrong_recipient_fails() {
	new_test_ext().execute_with(|| {
		let original_recipient = user1();
		let wrong_recipient = user2();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&original_recipient, amount, nonce);

		// First guardian creates the deposit
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			original_recipient.clone(),
			amount,
			nonce,
		));

		// Second guardian tries to attest with different recipient — fails ID verification
		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(
				RuntimeOrigin::signed(bob()),
				deposit_id,
				wrong_recipient,
				amount,
				nonce,
			),
			Error::<Test>::InvalidRequestId
		);

		// Deposit should still be pending with original recipient
		let deposit = crate::Deposits::<Test>::get(deposit_id).expect("Deposit should exist");
		assert_eq!(deposit.recipient, original_recipient);
		assert_eq!(deposit.status, DepositStatus::Pending);
	});
}

#[test]
fn test_attestation_with_wrong_amount_fails() {
	new_test_ext().execute_with(|| {
		let recipient = user1();
		let original_amount = 1000u128;
		let wrong_amount = 2000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, original_amount, nonce);

		// First guardian creates the deposit
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			original_amount,
			nonce,
		));

		// Second guardian tries to attest with different amount — fails ID verification
		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(
				RuntimeOrigin::signed(bob()),
				deposit_id,
				recipient,
				wrong_amount,
				nonce,
			),
			Error::<Test>::InvalidRequestId
		);

		// Deposit should still be pending with original amount
		let deposit = crate::Deposits::<Test>::get(deposit_id).expect("Deposit should exist");
		assert_eq!(deposit.amount, original_amount);
		assert_eq!(deposit.status, DepositStatus::Pending);
	});
}

#[test]
fn test_attestation_with_matching_details_succeeds() {
	new_test_ext().execute_with(|| {
		crate::ApproveThreshold::<Test>::put(3);

		let recipient = user1();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, nonce);

		// All three guardians attest with same details
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			amount,
			nonce,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient.clone(),
			amount,
			nonce,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(charlie()),
			deposit_id,
			recipient.clone(),
			amount,
			nonce,
		));

		// Deposit should complete successfully
		let deposit = crate::Deposits::<Test>::get(deposit_id).expect("Deposit should exist");
		assert_eq!(deposit.status, DepositStatus::Completed);
		assert_eq!(deposit.votes.len(), 3);
	});
}

// ============================================================================
// Withdrawal Flow Tests (User burns hAlpha -> Guardian confirms release)
// ============================================================================

#[test]
fn test_user_withdraw_burns_halpha() {
	new_test_ext().execute_with(|| {
		// First mint some hAlpha to the user via deposit
		let recipient = user1();
		let deposit_amount = 5_000_000_000_000u128; // Must be divisible by 1e9
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, deposit_amount, nonce);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			deposit_amount,
			nonce,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient.clone(),
			deposit_amount,
			nonce,
		));

		let balance_after_deposit = Balances::free_balance(&user1());
		let total_minted_after_deposit = crate::TotalMintedByBridge::<Test>::get();

		let withdraw_amount = 1_000_000_000_000u128; // Must be divisible by 1e9

		// Now user withdraws (recipient = sender automatically)
		assert_ok!(AlphaBridge::<Test>::withdraw(
			RuntimeOrigin::signed(user1()),
			withdraw_amount,
		));

		// Balance should be reduced
		assert_eq!(Balances::free_balance(&user1()), balance_after_deposit - withdraw_amount);

		// Total minted should be reduced
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), total_minted_after_deposit - withdraw_amount);

		// Check that withdrawal request was created
		let requests: Vec<_> = crate::WithdrawalRequests::<Test>::iter().collect();
		assert_eq!(requests.len(), 1);
		let (request_id, request) = &requests[0];
		assert_eq!(request.sender, user1());
		assert_eq!(request.recipient, user1());
		assert_eq!(request.amount, withdraw_amount);

		// Check event
		System::assert_has_event(
			Event::WithdrawalRequestCreated {
				id: *request_id,
				sender: user1(),
				recipient: user1(),
				amount: withdraw_amount,
			}
			.into(),
		);
	});
}

#[test]
fn test_withdraw_with_insufficient_balance_fails() {
	new_test_ext().execute_with(|| {
		let balance = Balances::free_balance(&user1());
		// Round up to nearest multiple of 1e9
		let excessive_amount = ((balance + 1000 + 999_999_999) / 1_000_000_000) * 1_000_000_000;

		assert_noop!(
			AlphaBridge::<Test>::withdraw(
				RuntimeOrigin::signed(user1()),
				excessive_amount,
			),
			Error::<Test>::InsufficientBalance
		);
	});
}

#[test]
fn test_withdraw_while_paused_fails() {
	new_test_ext().execute_with(|| {
		crate::Paused::<Test>::put(true);

		assert_noop!(
			AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 1_000_000_000),
			Error::<Test>::BridgePaused
		);
	});
}

// ============================================================================
// Minimum Withdrawal Amount Tests (AmountTooSmall error)
// ============================================================================

#[test]
fn test_withdraw_below_minimum_fails() {
	new_test_ext().execute_with(|| {
		// Set a specific minimum for this test
		crate::MinWithdrawalAmount::<Test>::put(1_000_000_000u128);

		assert_noop!(
			AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 0),
			Error::<Test>::AmountTooSmall
		);
		assert_noop!(
			AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 999_999_999),
			Error::<Test>::AmountTooSmall
		);
	});
}

#[test]
fn test_withdraw_at_minimum_succeeds() {
	new_test_ext().execute_with(|| {
		// Set a specific minimum for this test
		crate::MinWithdrawalAmount::<Test>::put(1_000_000_000u128);
		// Set TotalMintedByBridge so the accounting works
		crate::TotalMintedByBridge::<Test>::put(10_000_000_000u128);

		assert_ok!(AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 1_000_000_000));
	});
}

// ============================================================================
// Dust Amount Tests (AmountNotBridgeable error)
// ============================================================================

#[test]
fn test_withdraw_dust_amount_fails() {
	new_test_ext().execute_with(|| {
		crate::MinWithdrawalAmount::<Test>::put(1u128);
		crate::TotalMintedByBridge::<Test>::put(10_000_000_000u128);

		// Amount not divisible by 1_000_000_000
		assert_noop!(
			AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 1_500_000_000),
			Error::<Test>::AmountNotBridgeable
		);
		assert_noop!(
			AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 1),
			Error::<Test>::AmountNotBridgeable
		);
	});
}

#[test]
fn test_withdraw_clean_amount_succeeds() {
	new_test_ext().execute_with(|| {
		crate::MinWithdrawalAmount::<Test>::put(1u128);
		crate::TotalMintedByBridge::<Test>::put(10_000_000_000u128);

		// Exactly divisible by 1_000_000_000
		assert_ok!(AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 2_000_000_000));
	});
}

// ============================================================================
// Admin Functions Tests
// ============================================================================

#[test]
fn test_admin_cancel_deposit() {
	new_test_ext().execute_with(|| {
		let recipient = user1();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, nonce);

		// Create a pending deposit
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient,
			amount,
			nonce,
		));

		// Admin cancels
		assert_ok!(AlphaBridge::<Test>::admin_cancel_deposit(
			RuntimeOrigin::root(),
			deposit_id,
			crate::pallet::CancelReason::AdminEmergency,
		));

		// Check status
		let deposit = crate::Deposits::<Test>::get(deposit_id).unwrap();
		assert_eq!(deposit.status, DepositStatus::Cancelled);

		// Check event
		System::assert_has_event(
			Event::DepositCancelled {
				id: deposit_id,
				reason: crate::pallet::CancelReason::AdminEmergency,
			}
			.into(),
		);
	});
}

#[test]
fn test_admin_cancel_completed_deposit_fails() {
	new_test_ext().execute_with(|| {
		let recipient = user1();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, nonce);

		// Complete a deposit
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			amount,
			nonce,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient,
			amount,
			nonce,
		));

		// Try to cancel completed deposit
		assert_noop!(
			AlphaBridge::<Test>::admin_cancel_deposit(
				RuntimeOrigin::root(),
				deposit_id,
				crate::pallet::CancelReason::AdminEmergency,
			),
			Error::<Test>::InvalidStatus
		);
	});
}

#[test]
fn test_admin_fail_withdrawal_request_mints_back() {
	new_test_ext().execute_with(|| {
		// Setup: mint and withdraw
		let recipient = user1();
		let deposit_amount = 5_000_000_000_000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, deposit_amount, nonce);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			deposit_amount,
			nonce,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient,
			deposit_amount,
			nonce,
		));

		let balance_after_deposit = Balances::free_balance(&user1());

		let withdraw_amount = 1_000_000_000_000u128;

		// Withdraw
		assert_ok!(AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), withdraw_amount));

		let balance_after_withdraw = Balances::free_balance(&user1());
		assert_eq!(balance_after_withdraw, balance_after_deposit - withdraw_amount);

		let total_minted_after_withdraw = crate::TotalMintedByBridge::<Test>::get();

		let request_id = crate::WithdrawalRequests::<Test>::iter().next().unwrap().0;

		// Admin fails the withdrawal
		assert_ok!(AlphaBridge::<Test>::admin_fail_withdrawal_request(RuntimeOrigin::root(), request_id));

		// Balance should be restored
		assert_eq!(Balances::free_balance(&user1()), balance_after_deposit);

		// TotalMintedByBridge should increase back
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), total_minted_after_withdraw + withdraw_amount);

		// Check status
		let request = crate::WithdrawalRequests::<Test>::get(request_id).unwrap();
		assert_eq!(request.status, crate::pallet::WithdrawalRequestStatus::Failed);

		// Check events
		System::assert_has_event(Event::WithdrawalRequestFailed { id: request_id }.into());
		System::assert_has_event(
			Event::AdminManualMint { recipient: user1(), amount: withdraw_amount, deposit_id: None }.into(),
		);
	});
}

#[test]
fn test_admin_fail_withdrawal_respects_mint_cap() {
	new_test_ext().execute_with(|| {
		// Setup: mint and withdraw
		crate::TotalMintedByBridge::<Test>::put(5_000_000_000_000u128);
		assert_ok!(AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 1_000_000_000_000));

		// Now set a low cap that would be exceeded by minting back
		crate::GlobalMintCap::<Test>::put(4_500_000_000_000u128); // Below what would be needed

		let request_id = crate::WithdrawalRequests::<Test>::iter().next().unwrap().0;

		// Admin try to fail the withdrawal - should fail due to cap
		assert_noop!(
			AlphaBridge::<Test>::admin_fail_withdrawal_request(RuntimeOrigin::root(), request_id),
			Error::<Test>::CapExceeded
		);
	});
}

#[test]
fn test_admin_manual_mint() {
	new_test_ext().execute_with(|| {
		let initial_balance = Balances::free_balance(&user1());
		let initial_total_minted = crate::TotalMintedByBridge::<Test>::get();

		assert_ok!(AlphaBridge::<Test>::admin_manual_mint(RuntimeOrigin::root(), user1(), 500, None));

		// Balance should increase
		assert_eq!(Balances::free_balance(&user1()), initial_balance + 500);

		// TotalMintedByBridge should increase
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), initial_total_minted + 500);

		// Check event
		System::assert_has_event(Event::AdminManualMint { recipient: user1(), amount: 500, deposit_id: None }.into());
	});
}

#[test]
fn test_admin_manual_mint_respects_cap() {
	new_test_ext().execute_with(|| {
		crate::GlobalMintCap::<Test>::put(100);
		crate::TotalMintedByBridge::<Test>::put(50);

		// Try to mint more than remaining cap
		assert_noop!(
			AlphaBridge::<Test>::admin_manual_mint(RuntimeOrigin::root(), user1(), 100, None),
			Error::<Test>::CapExceeded
		);

		// Mint exactly up to cap should succeed
		assert_ok!(AlphaBridge::<Test>::admin_manual_mint(RuntimeOrigin::root(), user1(), 50, None));
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 100);
	});
}

#[test]
fn test_admin_manual_mint_by_non_root_fails() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			AlphaBridge::<Test>::admin_manual_mint(RuntimeOrigin::signed(alice()), user1(), 500, None),
			sp_runtime::DispatchError::BadOrigin
		);
	});
}

// ============================================================================
// Guardian Management Tests
// ============================================================================

#[test]
fn test_set_guardians_and_threshold() {
	new_test_ext().execute_with(|| {
		let new_guardians = vec![alice(), bob(), dave()];
		assert_ok!(AlphaBridge::<Test>::set_guardians_and_threshold(
			RuntimeOrigin::root(),
			new_guardians.clone(),
			2,
		));

		assert_eq!(crate::Guardians::<Test>::get(), new_guardians);
		assert_eq!(crate::ApproveThreshold::<Test>::get(), 2);

		System::assert_has_event(
			Event::GuardiansUpdated { guardians: new_guardians, approve_threshold: 2 }.into(),
		);
	});
}

#[test]
fn test_set_guardians_and_threshold_non_root_fails() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			AlphaBridge::<Test>::set_guardians_and_threshold(
				RuntimeOrigin::signed(alice()),
				vec![alice(), bob()],
				1,
			),
			sp_runtime::DispatchError::BadOrigin
		);
	});
}

#[test]
fn test_set_guardians_and_threshold_threshold_zero_fails() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			AlphaBridge::<Test>::set_guardians_and_threshold(
				RuntimeOrigin::root(),
				vec![alice(), bob()],
				0,
			),
			Error::<Test>::ThresholdTooLow
		);
	});
}

#[test]
fn test_set_guardians_and_threshold_threshold_too_high_fails() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			AlphaBridge::<Test>::set_guardians_and_threshold(
				RuntimeOrigin::root(),
				vec![alice(), bob()],
				3,
			),
			Error::<Test>::ThresholdTooHigh
		);
	});
}

#[test]
fn test_set_guardians_and_threshold_too_many_guardians_fails() {
	new_test_ext().execute_with(|| {
		let too_many: Vec<_> = (0..11u64).map(|i| {
			use sp_core::Hasher;
			use sp_runtime::traits::BlakeTwo256;
			let hash = BlakeTwo256::hash(&i.to_le_bytes());
			sp_runtime::AccountId32::new(hash.0)
		}).collect();
		assert_noop!(
			AlphaBridge::<Test>::set_guardians_and_threshold(
				RuntimeOrigin::root(),
				too_many,
				1,
			),
			Error::<Test>::TooManyGuardians
		);
	});
}

#[test]
fn test_set_guardians_and_threshold_replaces_existing() {
	new_test_ext().execute_with(|| {
		// Initial guardians are alice, bob, charlie with threshold 2
		assert_eq!(crate::Guardians::<Test>::get().len(), 3);

		// Replace with just alice, bob and threshold 1
		let new_guardians = vec![alice(), bob()];
		assert_ok!(AlphaBridge::<Test>::set_guardians_and_threshold(
			RuntimeOrigin::root(),
			new_guardians.clone(),
			1,
		));

		assert_eq!(crate::Guardians::<Test>::get(), new_guardians);
		assert_eq!(crate::ApproveThreshold::<Test>::get(), 1);
	});
}

// ============================================================================
// Threshold Tests
// ============================================================================

#[test]
fn test_single_guardian_threshold() {
	new_test_ext().execute_with(|| {
		// Set threshold to 1
		crate::ApproveThreshold::<Test>::put(1);

		let recipient = user1();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, nonce);
		let initial_balance = Balances::free_balance(&recipient);

		// Single attestation should complete the deposit
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			amount,
			nonce,
		));

		// Should be completed immediately
		let deposit = crate::Deposits::<Test>::get(deposit_id).expect("Deposit should exist");
		assert_eq!(deposit.status, DepositStatus::Completed);
		assert_eq!(Balances::free_balance(&recipient), initial_balance + 1000);
	});
}

// ============================================================================
// Pause Tests
// ============================================================================

#[test]
fn test_pause_bridge() {
	new_test_ext().execute_with(|| {
		assert_ok!(AlphaBridge::<Test>::pause(RuntimeOrigin::root()));

		assert!(crate::Paused::<Test>::get());

		System::assert_has_event(Event::Paused.into());
	});
}

#[test]
fn test_unpause_bridge() {
	new_test_ext().execute_with(|| {
		crate::Paused::<Test>::put(true);

		assert_ok!(AlphaBridge::<Test>::unpause(RuntimeOrigin::root()));

		assert!(!crate::Paused::<Test>::get());

		System::assert_has_event(Event::Unpaused.into());
	});
}

// ============================================================================
// Mint Cap Tests
// ============================================================================

#[test]
fn test_set_global_mint_cap() {
	new_test_ext().execute_with(|| {
		assert_ok!(AlphaBridge::<Test>::set_global_mint_cap(RuntimeOrigin::root(), 50_000_000));

		assert_eq!(crate::GlobalMintCap::<Test>::get(), 50_000_000);

		System::assert_has_event(Event::GlobalMintCapUpdated { new_cap: 50_000_000 }.into());
	});
}

#[test]
fn test_set_mint_cap_below_total_minted_fails() {
	new_test_ext().execute_with(|| {
		crate::TotalMintedByBridge::<Test>::put(1000);

		assert_noop!(
			AlphaBridge::<Test>::set_global_mint_cap(RuntimeOrigin::root(), 500),
			Error::<Test>::CapExceeded
		);
	});
}

// ============================================================================
// TotalMintedByBridge Accounting Tests
// ============================================================================

#[test]
fn test_total_minted_increases_on_deposit() {
	new_test_ext().execute_with(|| {
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 0);

		let recipient = user1();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, nonce);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			amount,
			nonce,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient,
			amount,
			nonce,
		));

		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 1000);
	});
}

#[test]
fn test_total_minted_decreases_on_withdraw() {
	new_test_ext().execute_with(|| {
		// Setup: deposit first
		let recipient = user1();
		let deposit_amount = 5_000_000_000_000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, deposit_amount, nonce);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			deposit_amount,
			nonce,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient,
			deposit_amount,
			nonce,
		));

		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), deposit_amount);

		// Withdraw
		let withdraw_amount = 1_000_000_000_000u128;
		assert_ok!(AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), withdraw_amount));

		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), deposit_amount - withdraw_amount);
	});
}

#[test]
fn test_withdraw_accounting_underflow_fails() {
	new_test_ext().execute_with(|| {
		// TotalMintedByBridge is 0, but user has balance from genesis
		// This simulates an accounting bug scenario
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 0);

		// Trying to withdraw should fail due to underflow
		assert_noop!(
			AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 1_000_000_000),
			Error::<Test>::AccountingUnderflow
		);
	});
}

#[test]
fn test_multiple_deposits_and_withdrawals_accounting() {
	new_test_ext().execute_with(|| {
		let deposit_amount_1 = 3_000_000_000_000u128;
		let deposit_amount_2 = 2_000_000_000_000u128;

		// Deposit 1
		let deposit_id1 = generate_deposit_id_for(&user1(), deposit_amount_1, 0);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id1,
			user1(),
			deposit_amount_1,
			0,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id1,
			user1(),
			deposit_amount_1,
			0,
		));
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), deposit_amount_1);

		// Deposit 2
		let deposit_id2 = generate_deposit_id_for(&user2(), deposit_amount_2, 1);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id2,
			user2(),
			deposit_amount_2,
			1,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id2,
			user2(),
			deposit_amount_2,
			1,
		));
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), deposit_amount_1 + deposit_amount_2);

		// Withdraw 1
		let withdraw_1 = 1_000_000_000_000u128;
		assert_ok!(AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), withdraw_1));
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), deposit_amount_1 + deposit_amount_2 - withdraw_1);

		// Withdraw 2
		let withdraw_2 = 1_000_000_000_000u128;
		assert_ok!(AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user2()), withdraw_2));
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), deposit_amount_1 + deposit_amount_2 - withdraw_1 - withdraw_2);
	});
}

// ============================================================================
// Full Cycle Tests
// ============================================================================

#[test]
fn test_full_deposit_cycle() {
	new_test_ext().execute_with(|| {
		let recipient = user1();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, nonce);
		let initial_balance = Balances::free_balance(&recipient);

		// Guardian 1 attests (creates record)
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			amount,
			nonce,
		));

		// Verify pending state
		let deposit = crate::Deposits::<Test>::get(deposit_id).unwrap();
		assert_eq!(deposit.status, DepositStatus::Pending);
		assert_eq!(Balances::free_balance(&recipient), initial_balance);

		// Guardian 2 attests (completes deposit)
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient.clone(),
			amount,
			nonce,
		));

		// Verify completed state
		let deposit = crate::Deposits::<Test>::get(deposit_id).unwrap();
		assert_eq!(deposit.status, DepositStatus::Completed);
		assert_eq!(Balances::free_balance(&recipient), initial_balance + 1000);
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 1000);
	});
}

#[test]
fn test_full_withdrawal_cycle() {
	new_test_ext().execute_with(|| {
		// Setup: deposit first
		let recipient = user1();
		let deposit_amount = 5_000_000_000_000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, deposit_amount, nonce);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			deposit_amount,
			nonce,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient,
			deposit_amount,
			nonce,
		));

		let balance_after_deposit = Balances::free_balance(&user1());

		let withdraw_amount = 1_000_000_000_000u128;

		// User initiates withdrawal (burns hAlpha)
		assert_ok!(AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), withdraw_amount));

		// Verify burn
		assert_eq!(Balances::free_balance(&user1()), balance_after_deposit - withdraw_amount);
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), deposit_amount - withdraw_amount);

		// Get withdrawal request ID
		let request_id = crate::WithdrawalRequests::<Test>::iter().next().unwrap().0;

		// Verify withdrawal request is in Requested status (guardians will attest on Bittensor)
		let request = crate::WithdrawalRequests::<Test>::get(request_id).unwrap();
		assert_eq!(request.status, crate::pallet::WithdrawalRequestStatus::Requested);
		assert_eq!(request.sender, user1());
		assert_eq!(request.recipient, user1());
		assert_eq!(request.amount, withdraw_amount);
	});
}

#[test]
fn test_failed_withdrawal_recovery_cycle() {
	new_test_ext().execute_with(|| {
		// Setup: deposit first
		let recipient = user1();
		let deposit_amount = 5_000_000_000_000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, deposit_amount, nonce);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			deposit_amount,
			nonce,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient,
			deposit_amount,
			nonce,
		));

		let balance_after_deposit = Balances::free_balance(&user1());

		let withdraw_amount = 1_000_000_000_000u128;

		// User initiates withdrawal
		assert_ok!(AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), withdraw_amount));
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), deposit_amount - withdraw_amount);

		let request_id = crate::WithdrawalRequests::<Test>::iter().next().unwrap().0;

		// Withdrawal fails on Bittensor side, admin recovers
		assert_ok!(AlphaBridge::<Test>::admin_fail_withdrawal_request(RuntimeOrigin::root(), request_id));

		// Verify recovery - user gets hAlpha back
		assert_eq!(Balances::free_balance(&user1()), balance_after_deposit);
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), deposit_amount);

		let request = crate::WithdrawalRequests::<Test>::get(request_id).unwrap();
		assert_eq!(request.status, crate::pallet::WithdrawalRequestStatus::Failed);
	});
}

// ============================================================================
// TTL Cleanup Tests
// ============================================================================

#[test]
fn test_cleanup_deposit_after_ttl() {
	new_test_ext().execute_with(|| {
		// Set a small TTL for testing
		assert_ok!(AlphaBridge::<Test>::set_cleanup_ttl(RuntimeOrigin::root(), 10));

		// Create and complete a deposit
		let recipient = user1();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, nonce);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			amount,
			nonce,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient,
			amount,
			nonce,
		));

		// Verify deposit exists and is completed
		let deposit = crate::Deposits::<Test>::get(deposit_id).expect("Deposit should exist");
		assert_eq!(deposit.status, DepositStatus::Completed);
		assert!(deposit.finalized_at_block.is_some());

		// Advance block number past TTL
		let finalized_at = deposit.finalized_at_block.unwrap();
		System::set_block_number(finalized_at + 11);

		// Cleanup should succeed (guardian only)
		assert_ok!(AlphaBridge::<Test>::cleanup_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
		));

		// Deposit should be removed
		assert!(crate::Deposits::<Test>::get(deposit_id).is_none());

		// Check event
		System::assert_has_event(Event::DepositCleanedUp { id: deposit_id }.into());
	});
}

#[test]
fn test_cleanup_deposit_before_ttl_fails() {
	new_test_ext().execute_with(|| {
		// Set a TTL for testing
		assert_ok!(AlphaBridge::<Test>::set_cleanup_ttl(RuntimeOrigin::root(), 100));

		// Create and complete a deposit
		let recipient = user1();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, nonce);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			amount,
			nonce,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient,
			amount,
			nonce,
		));

		// Try to cleanup immediately (before TTL expires) as guardian
		assert_noop!(
			AlphaBridge::<Test>::cleanup_deposit(
				RuntimeOrigin::signed(alice()),
				deposit_id,
			),
			Error::<Test>::TTLNotExpired
		);

		// Deposit should still exist
		assert!(crate::Deposits::<Test>::get(deposit_id).is_some());
	});
}

#[test]
fn test_cleanup_pending_deposit_fails() {
	new_test_ext().execute_with(|| {
		// Set TTL to 1 so we can't fail due to TTL
		assert_ok!(AlphaBridge::<Test>::set_cleanup_ttl(RuntimeOrigin::root(), 1));

		// Create a pending deposit (only one attestation, threshold is 2)
		let recipient = user1();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, nonce);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient,
			amount,
			nonce,
		));

		// Verify deposit is pending
		let deposit = crate::Deposits::<Test>::get(deposit_id).expect("Deposit should exist");
		assert_eq!(deposit.status, DepositStatus::Pending);

		// Advance past TTL so we hit RecordNotFinalized, not TTLNotExpired
		System::set_block_number(100);

		// Try to cleanup pending deposit as guardian
		assert_noop!(
			AlphaBridge::<Test>::cleanup_deposit(
				RuntimeOrigin::signed(alice()),
				deposit_id,
			),
			Error::<Test>::RecordNotFinalized
		);
	});
}

#[test]
fn test_cleanup_withdrawal_request_after_ttl() {
	new_test_ext().execute_with(|| {
		// Set a small TTL for testing
		assert_ok!(AlphaBridge::<Test>::set_cleanup_ttl(RuntimeOrigin::root(), 10));

		// Setup: deposit first to get hAlpha
		let recipient = user1();
		let deposit_amount = 5_000_000_000_000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, deposit_amount, nonce);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			deposit_amount,
			nonce,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient,
			deposit_amount,
			nonce,
		));

		// Create a withdrawal request
		let withdraw_amount = 1_000_000_000_000u128;
		assert_ok!(AlphaBridge::<Test>::withdraw(
			RuntimeOrigin::signed(user1()),
			withdraw_amount,
		));

		// Get the request
		let request_id = crate::WithdrawalRequests::<Test>::iter().next().unwrap().0;
		let request = crate::WithdrawalRequests::<Test>::get(request_id).expect("Request should exist");

		// Advance block number past TTL
		System::set_block_number(request.created_at_block + 11);

		// Cleanup should succeed (guardian only, no status check for withdrawal requests)
		assert_ok!(AlphaBridge::<Test>::cleanup_withdrawal_request(
			RuntimeOrigin::signed(alice()),
			request_id,
		));

		// Request should be removed
		assert!(crate::WithdrawalRequests::<Test>::get(request_id).is_none());

		// Check event
		System::assert_has_event(Event::WithdrawalRequestCleanedUp { id: request_id }.into());
	});
}

#[test]
fn test_cleanup_withdrawal_request_before_ttl_fails() {
	new_test_ext().execute_with(|| {
		// Set a TTL for testing
		assert_ok!(AlphaBridge::<Test>::set_cleanup_ttl(RuntimeOrigin::root(), 100));

		// Setup: deposit first to get hAlpha
		let recipient = user1();
		let deposit_amount = 5_000_000_000_000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, deposit_amount, nonce);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			deposit_amount,
			nonce,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient,
			deposit_amount,
			nonce,
		));

		// Create a withdrawal request
		let withdraw_amount = 1_000_000_000_000u128;
		assert_ok!(AlphaBridge::<Test>::withdraw(
			RuntimeOrigin::signed(user1()),
			withdraw_amount,
		));

		let request_id = crate::WithdrawalRequests::<Test>::iter().next().unwrap().0;

		// Try to cleanup immediately (before TTL expires) as guardian
		assert_noop!(
			AlphaBridge::<Test>::cleanup_withdrawal_request(
				RuntimeOrigin::signed(alice()),
				request_id,
			),
			Error::<Test>::TTLNotExpired
		);

		// Request should still exist
		assert!(crate::WithdrawalRequests::<Test>::get(request_id).is_some());
	});
}

#[test]
fn test_cleanup_deposit_non_guardian_fails() {
	new_test_ext().execute_with(|| {
		// Set a small TTL for testing
		assert_ok!(AlphaBridge::<Test>::set_cleanup_ttl(RuntimeOrigin::root(), 10));

		// Create and complete a deposit
		let recipient = user1();
		let amount = 1000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, nonce);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			amount,
			nonce,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient,
			amount,
			nonce,
		));

		// Advance block number past TTL
		let deposit = crate::Deposits::<Test>::get(deposit_id).expect("Deposit should exist");
		let finalized_at = deposit.finalized_at_block.unwrap();
		System::set_block_number(finalized_at + 11);

		// Non-guardian should fail
		assert_noop!(
			AlphaBridge::<Test>::cleanup_deposit(
				RuntimeOrigin::signed(user1()),
				deposit_id,
			),
			Error::<Test>::NotGuardian
		);

		// Deposit should still exist
		assert!(crate::Deposits::<Test>::get(deposit_id).is_some());
	});
}

#[test]
fn test_cleanup_withdrawal_request_non_guardian_fails() {
	new_test_ext().execute_with(|| {
		// Set a small TTL for testing
		assert_ok!(AlphaBridge::<Test>::set_cleanup_ttl(RuntimeOrigin::root(), 10));

		// Setup: deposit first to get hAlpha
		let recipient = user1();
		let deposit_amount = 5_000_000_000_000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id_for(&recipient, deposit_amount, nonce);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			deposit_amount,
			nonce,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient,
			deposit_amount,
			nonce,
		));

		// Create a withdrawal request
		let withdraw_amount = 1_000_000_000_000u128;
		assert_ok!(AlphaBridge::<Test>::withdraw(
			RuntimeOrigin::signed(user1()),
			withdraw_amount,
		));

		// Get the request
		let request_id = crate::WithdrawalRequests::<Test>::iter().next().unwrap().0;
		let request = crate::WithdrawalRequests::<Test>::get(request_id).expect("Request should exist");

		// Advance block number past TTL
		System::set_block_number(request.created_at_block + 11);

		// Non-guardian should fail
		assert_noop!(
			AlphaBridge::<Test>::cleanup_withdrawal_request(
				RuntimeOrigin::signed(user1()),
				request_id,
			),
			Error::<Test>::NotGuardian
		);

		// Request should still exist
		assert!(crate::WithdrawalRequests::<Test>::get(request_id).is_some());
	});
}

#[test]
fn test_set_cleanup_ttl() {
	new_test_ext().execute_with(|| {
		// Initial TTL should be the default (100800 blocks, ~7 days)
		assert_eq!(crate::CleanupTTLBlocks::<Test>::get(), 100800);

		// Set a new TTL (different from default)
		assert_ok!(AlphaBridge::<Test>::set_cleanup_ttl(RuntimeOrigin::root(), 50400));

		// Verify TTL was updated
		assert_eq!(crate::CleanupTTLBlocks::<Test>::get(), 50400);

		// Check event
		System::assert_has_event(
			Event::CleanupTTLUpdated { old_ttl: 100800, new_ttl: 50400 }.into(),
		);
	});
}

#[test]
fn test_set_cleanup_ttl_zero_fails() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			AlphaBridge::<Test>::set_cleanup_ttl(RuntimeOrigin::root(), 0),
			Error::<Test>::InvalidTTL
		);
	});
}

#[test]
fn test_set_cleanup_ttl_non_root_fails() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			AlphaBridge::<Test>::set_cleanup_ttl(RuntimeOrigin::signed(alice()), 100),
			sp_runtime::DispatchError::BadOrigin
		);
	});
}

// ============================================================================
// Min Withdrawal Amount Tests
// ============================================================================

#[test]
fn test_set_min_withdrawal_amount() {
	new_test_ext().execute_with(|| {
		// Mock sets MinWithdrawalAmount to 1
		assert_eq!(crate::MinWithdrawalAmount::<Test>::get(), 1);

		assert_ok!(AlphaBridge::<Test>::set_min_withdrawal_amount(RuntimeOrigin::root(), 2_000_000_000));

		assert_eq!(crate::MinWithdrawalAmount::<Test>::get(), 2_000_000_000);

		System::assert_has_event(
			Event::MinWithdrawalAmountUpdated { old_amount: 1, new_amount: 2_000_000_000 }.into(),
		);
	});
}

#[test]
fn test_set_min_withdrawal_amount_non_root_fails() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			AlphaBridge::<Test>::set_min_withdrawal_amount(RuntimeOrigin::signed(alice()), 2_000_000_000),
			sp_runtime::DispatchError::BadOrigin
		);
	});
}

#[test]
fn test_set_min_withdrawal_amount_zero_fails() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			AlphaBridge::<Test>::set_min_withdrawal_amount(RuntimeOrigin::root(), 0),
			Error::<Test>::AmountTooSmall
		);
	});
}

// ============================================================================
// Request ID Verification Tests
// ============================================================================

#[test]
fn test_attest_deposit_with_wrong_nonce_fails() {
	new_test_ext().execute_with(|| {
		let recipient = user1();
		let amount = 1000u128;
		let correct_nonce = 5u64;
		let wrong_nonce = 99u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, correct_nonce);

		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(
				RuntimeOrigin::signed(alice()),
				deposit_id,
				recipient,
				amount,
				wrong_nonce,
			),
			Error::<Test>::InvalidRequestId
		);
	});
}

#[test]
fn test_attest_deposit_with_fabricated_id_fails() {
	new_test_ext().execute_with(|| {
		let fabricated_id = sp_core::H256::from([0xAB; 32]);

		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(
				RuntimeOrigin::signed(alice()),
				fabricated_id,
				user1(),
				1000,
				0,
			),
			Error::<Test>::InvalidRequestId
		);
	});
}

#[test]
fn test_attest_deposit_with_correct_nonce_passes() {
	new_test_ext().execute_with(|| {
		let recipient = user1();
		let amount = 1000u128;
		let nonce = 42u64;
		let deposit_id = generate_deposit_id_for(&recipient, amount, nonce);

		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient,
			amount,
			nonce,
		));
	});
}

use crate::{mock::*, pallet::DepositStatus, Error, Event, Pallet as AlphaBridge};
use frame_support::{assert_noop, assert_ok};

// ============================================================================
// Deposit Flow Tests (Guardian attests deposit -> hAlpha minted to recipient)
// ============================================================================

#[test]
fn test_first_attestation_creates_deposit_record() {
	new_test_ext().execute_with(|| {
		let deposit_id = generate_deposit_id(1);
		let recipient = user1();
		let amount = 1000u64;

		// First guardian attestation creates the deposit record
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			amount,
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
		let deposit_id = generate_deposit_id(1);
		let recipient = user1();
		let amount = 1000u64;
		let initial_balance = Balances::free_balance(&recipient);

		// First attestation
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			amount,
		));

		// Second attestation should complete the deposit
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient.clone(),
			amount,
		));

		// Check that hAlpha was minted
		assert_eq!(Balances::free_balance(&recipient), initial_balance + amount as u128);

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
		let deposit_id = generate_deposit_id(1);

		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(
				RuntimeOrigin::signed(user1()),
				deposit_id,
				user2(),
				1000,
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

		let deposit_id = generate_deposit_id(1);
		let recipient = user1();

		// First attestation
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			1000,
		));

		// Same guardian tries to vote again
		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(
				RuntimeOrigin::signed(alice()),
				deposit_id,
				recipient,
				1000,
			),
			Error::<Test>::AlreadyVoted
		);
	});
}

#[test]
fn test_attestation_on_completed_deposit_fails() {
	new_test_ext().execute_with(|| {
		let deposit_id = generate_deposit_id(1);
		let recipient = user1();

		// Complete the deposit with threshold votes
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			1000,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient.clone(),
			1000,
		));

		// Third guardian tries to attest on completed deposit
		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(
				RuntimeOrigin::signed(charlie()),
				deposit_id,
				recipient,
				1000,
			),
			Error::<Test>::DepositAlreadyCompleted
		);
	});
}

#[test]
fn test_deposit_while_paused_fails() {
	new_test_ext().execute_with(|| {
		crate::Paused::<Test>::put(true);

		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(
				RuntimeOrigin::signed(alice()),
				generate_deposit_id(1),
				user1(),
				1000,
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

		let deposit_id = generate_deposit_id(1);
		let recipient = user1();

		// First attestation succeeds (just creates record)
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			1000,
		));

		// Second attestation should fail at finalization due to cap
		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(
				RuntimeOrigin::signed(bob()),
				deposit_id,
				recipient,
				1000,
			),
			Error::<Test>::CapExceeded
		);
	});
}

#[test]
fn test_deposit_minting_to_new_account() {
	new_test_ext().execute_with(|| {
		let new_recipient = new_account();
		let deposit_id = generate_deposit_id(1);
		let amount = 1000u64;

		// Verify account doesn't exist yet
		assert_eq!(Balances::free_balance(&new_recipient), 0);

		// Complete deposit
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			new_recipient.clone(),
			amount,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			new_recipient.clone(),
			amount,
		));

		// Verify account was created with minted balance
		assert_eq!(Balances::free_balance(&new_recipient), amount as u128);

		// Verify total minted tracking
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), amount as u128);
	});
}

// ============================================================================
// Poisoning Prevention Tests (InvalidDepositDetails error)
// ============================================================================

#[test]
fn test_attestation_with_wrong_recipient_fails() {
	new_test_ext().execute_with(|| {
		let deposit_id = generate_deposit_id(1);
		let original_recipient = user1();
		let wrong_recipient = user2();

		// First guardian creates the deposit
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			original_recipient.clone(),
			1000,
		));

		// Second guardian tries to attest with different recipient (poisoning attempt)
		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(
				RuntimeOrigin::signed(bob()),
				deposit_id,
				wrong_recipient,
				1000,
			),
			Error::<Test>::InvalidDepositDetails
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
		let deposit_id = generate_deposit_id(1);
		let recipient = user1();
		let original_amount = 1000u64;
		let wrong_amount = 2000u64;

		// First guardian creates the deposit
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			original_amount,
		));

		// Second guardian tries to attest with different amount (poisoning attempt)
		assert_noop!(
			AlphaBridge::<Test>::attest_deposit(
				RuntimeOrigin::signed(bob()),
				deposit_id,
				recipient,
				wrong_amount,
			),
			Error::<Test>::InvalidDepositDetails
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

		let deposit_id = generate_deposit_id(1);
		let recipient = user1();
		let amount = 1000u64;

		// All three guardians attest with same details
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			amount,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			recipient.clone(),
			amount,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(charlie()),
			deposit_id,
			recipient.clone(),
			amount,
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
		let deposit_id = generate_deposit_id(1);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			user1(),
			5000,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			user1(),
			5000,
		));

		let balance_after_deposit = Balances::free_balance(&user1());
		let total_minted_after_deposit = crate::TotalMintedByBridge::<Test>::get();

		// Now user withdraws (recipient = sender automatically)
		assert_ok!(AlphaBridge::<Test>::withdraw(
			RuntimeOrigin::signed(user1()),
			1000,
		));

		// Balance should be reduced
		assert_eq!(Balances::free_balance(&user1()), balance_after_deposit - 1000);

		// Total minted should be reduced
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), total_minted_after_deposit - 1000);

		// Check that withdrawal request was created
		let requests: Vec<_> = crate::WithdrawalRequests::<Test>::iter().collect();
		assert_eq!(requests.len(), 1);
		let (request_id, request) = &requests[0];
		assert_eq!(request.sender, user1());
		assert_eq!(request.recipient, user1());
		assert_eq!(request.amount, 1000);

		// Check event
		System::assert_has_event(
			Event::WithdrawalRequestCreated {
				id: *request_id,
				sender: user1(),
				recipient: user1(),
				amount: 1000,
			}
			.into(),
		);
	});
}

#[test]
fn test_withdraw_with_insufficient_balance_fails() {
	new_test_ext().execute_with(|| {
		let balance = Balances::free_balance(&user1());
		let excessive_amount = (balance + 1000) as u64;

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
			AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 1000),
			Error::<Test>::BridgePaused
		);
	});
}

// ============================================================================
// Zero Amount Rejection Tests (AmountTooSmall error)
// ============================================================================

#[test]
fn test_withdraw_zero_amount_fails() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 0),
			Error::<Test>::AmountTooSmall
		);
	});
}

#[test]
fn test_withdraw_positive_amount_succeeds() {
	new_test_ext().execute_with(|| {
		// First, make sure TotalMintedByBridge is set so the accounting works
		crate::TotalMintedByBridge::<Test>::put(10000);

		assert_ok!(AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 1));
	});
}

// ============================================================================
// Admin Functions Tests
// ============================================================================

#[test]
fn test_admin_cancel_deposit() {
	new_test_ext().execute_with(|| {
		let deposit_id = generate_deposit_id(1);

		// Create a pending deposit
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			user1(),
			1000,
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
		let deposit_id = generate_deposit_id(1);

		// Complete a deposit
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			user1(),
			1000,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			user1(),
			1000,
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
		let deposit_id = generate_deposit_id(1);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			user1(),
			5000,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			user1(),
			5000,
		));

		let balance_after_deposit = Balances::free_balance(&user1());

		// Withdraw
		assert_ok!(AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 1000));

		let balance_after_withdraw = Balances::free_balance(&user1());
		assert_eq!(balance_after_withdraw, balance_after_deposit - 1000);

		let total_minted_after_withdraw = crate::TotalMintedByBridge::<Test>::get();

		let request_id = crate::WithdrawalRequests::<Test>::iter().next().unwrap().0;

		// Admin fails the withdrawal
		assert_ok!(AlphaBridge::<Test>::admin_fail_withdrawal_request(RuntimeOrigin::root(), request_id));

		// Balance should be restored
		assert_eq!(Balances::free_balance(&user1()), balance_after_deposit);

		// TotalMintedByBridge should increase back
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), total_minted_after_withdraw + 1000);

		// Check status
		let request = crate::WithdrawalRequests::<Test>::get(request_id).unwrap();
		assert_eq!(request.status, crate::pallet::WithdrawalRequestStatus::Failed);

		// Check events
		System::assert_has_event(Event::WithdrawalRequestFailed { id: request_id }.into());
		System::assert_has_event(
			Event::AdminManualMint { recipient: user1(), amount: 1000, deposit_id: None }.into(),
		);
	});
}

#[test]
fn test_admin_fail_withdrawal_respects_mint_cap() {
	new_test_ext().execute_with(|| {
		// Setup: mint and withdraw
		crate::TotalMintedByBridge::<Test>::put(5000);
		assert_ok!(AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 1000));

		// Now set a low cap that would be exceeded by minting back
		crate::GlobalMintCap::<Test>::put(4500); // Below what would be needed

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

// ============================================================================
// Threshold Tests
// ============================================================================

#[test]
fn test_set_approve_threshold() {
	new_test_ext().execute_with(|| {
		assert_ok!(AlphaBridge::<Test>::set_approve_threshold(RuntimeOrigin::root(), 3));

		assert_eq!(crate::ApproveThreshold::<Test>::get(), 3);

		System::assert_has_event(Event::ApproveThresholdUpdated { new_threshold: 3 }.into());
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
fn test_single_guardian_threshold() {
	new_test_ext().execute_with(|| {
		// Set threshold to 1
		crate::ApproveThreshold::<Test>::put(1);

		let deposit_id = generate_deposit_id(1);
		let recipient = user1();
		let initial_balance = Balances::free_balance(&recipient);

		// Single attestation should complete the deposit
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			1000,
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
		assert_ok!(AlphaBridge::<Test>::set_paused(RuntimeOrigin::root(), true));

		assert!(crate::Paused::<Test>::get());

		System::assert_has_event(Event::BridgePaused { paused: true }.into());
	});
}

#[test]
fn test_unpause_bridge() {
	new_test_ext().execute_with(|| {
		crate::Paused::<Test>::put(true);

		assert_ok!(AlphaBridge::<Test>::set_paused(RuntimeOrigin::root(), false));

		assert!(!crate::Paused::<Test>::get());

		System::assert_has_event(Event::BridgePaused { paused: false }.into());
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

		let deposit_id = generate_deposit_id(1);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			user1(),
			1000,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			user1(),
			1000,
		));

		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 1000);
	});
}

#[test]
fn test_total_minted_decreases_on_withdraw() {
	new_test_ext().execute_with(|| {
		// Setup: deposit first
		let deposit_id = generate_deposit_id(1);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			user1(),
			5000,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			user1(),
			5000,
		));

		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 5000);

		// Withdraw
		assert_ok!(AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 1000));

		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 4000);
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
			AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 1000),
			Error::<Test>::AccountingUnderflow
		);
	});
}

#[test]
fn test_multiple_deposits_and_withdrawals_accounting() {
	new_test_ext().execute_with(|| {
		// Deposit 1
		let deposit_id1 = generate_deposit_id(1);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id1,
			user1(),
			3000,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id1,
			user1(),
			3000,
		));
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 3000);

		// Deposit 2
		let deposit_id2 = generate_deposit_id(2);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id2,
			user2(),
			2000,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id2,
			user2(),
			2000,
		));
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 5000);

		// Withdraw 1
		assert_ok!(AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 1000));
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 4000);

		// Withdraw 2
		assert_ok!(AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user2()), 500));
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 3500);
	});
}

// ============================================================================
// Full Cycle Tests
// ============================================================================

#[test]
fn test_full_deposit_cycle() {
	new_test_ext().execute_with(|| {
		let deposit_id = generate_deposit_id(1);
		let recipient = user1();
		let initial_balance = Balances::free_balance(&recipient);

		// Guardian 1 attests (creates record)
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			recipient.clone(),
			1000,
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
			1000,
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
		let deposit_id = generate_deposit_id(1);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			user1(),
			5000,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			user1(),
			5000,
		));

		let balance_after_deposit = Balances::free_balance(&user1());

		// User initiates withdrawal (burns hAlpha)
		assert_ok!(AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 1000));

		// Verify burn
		assert_eq!(Balances::free_balance(&user1()), balance_after_deposit - 1000);
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 4000);

		// Get withdrawal request ID
		let request_id = crate::WithdrawalRequests::<Test>::iter().next().unwrap().0;

		// Verify withdrawal request is in Requested status (guardians will attest on Bittensor)
		let request = crate::WithdrawalRequests::<Test>::get(request_id).unwrap();
		assert_eq!(request.status, crate::pallet::WithdrawalRequestStatus::Requested);
		assert_eq!(request.sender, user1());
		assert_eq!(request.recipient, user1());
		assert_eq!(request.amount, 1000);
	});
}

#[test]
fn test_failed_withdrawal_recovery_cycle() {
	new_test_ext().execute_with(|| {
		// Setup: deposit first
		let deposit_id = generate_deposit_id(1);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			user1(),
			5000,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			user1(),
			5000,
		));

		let balance_after_deposit = Balances::free_balance(&user1());

		// User initiates withdrawal
		assert_ok!(AlphaBridge::<Test>::withdraw(RuntimeOrigin::signed(user1()), 1000));
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 4000);

		let request_id = crate::WithdrawalRequests::<Test>::iter().next().unwrap().0;

		// Withdrawal fails on Bittensor side, admin recovers
		assert_ok!(AlphaBridge::<Test>::admin_fail_withdrawal_request(RuntimeOrigin::root(), request_id));

		// Verify recovery - user gets hAlpha back
		assert_eq!(Balances::free_balance(&user1()), balance_after_deposit);
		assert_eq!(crate::TotalMintedByBridge::<Test>::get(), 5000);

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
		let deposit_id = generate_deposit_id(1);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			user1(),
			1000,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			user1(),
			1000,
		));

		// Verify deposit exists and is completed
		let deposit = crate::Deposits::<Test>::get(deposit_id).expect("Deposit should exist");
		assert_eq!(deposit.status, DepositStatus::Completed);
		assert!(deposit.finalized_at_block.is_some());

		// Advance block number past TTL
		let finalized_at = deposit.finalized_at_block.unwrap();
		System::set_block_number(finalized_at + 11);

		// Cleanup should succeed
		assert_ok!(AlphaBridge::<Test>::cleanup_deposit(
			RuntimeOrigin::signed(user1()),  // Anyone can call
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
		let deposit_id = generate_deposit_id(1);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			user1(),
			1000,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			user1(),
			1000,
		));

		// Try to cleanup immediately (before TTL expires)
		assert_noop!(
			AlphaBridge::<Test>::cleanup_deposit(
				RuntimeOrigin::signed(user1()),
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
		let deposit_id = generate_deposit_id(1);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			user1(),
			1000,
		));

		// Verify deposit is pending
		let deposit = crate::Deposits::<Test>::get(deposit_id).expect("Deposit should exist");
		assert_eq!(deposit.status, DepositStatus::Pending);

		// Advance past TTL so we hit RecordNotFinalized, not TTLNotExpired
		System::set_block_number(100);

		// Try to cleanup pending deposit
		assert_noop!(
			AlphaBridge::<Test>::cleanup_deposit(
				RuntimeOrigin::signed(user1()),
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
		let deposit_id = generate_deposit_id(1);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			user1(),
			5000,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			user1(),
			5000,
		));

		// Create a withdrawal request
		assert_ok!(AlphaBridge::<Test>::withdraw(
			RuntimeOrigin::signed(user1()),
			1000,
		));

		// Get the request
		let request_id = crate::WithdrawalRequests::<Test>::iter().next().unwrap().0;
		let request = crate::WithdrawalRequests::<Test>::get(request_id).expect("Request should exist");

		// Advance block number past TTL
		System::set_block_number(request.created_at_block + 11);

		// Cleanup should succeed (no status check for withdrawal requests)
		assert_ok!(AlphaBridge::<Test>::cleanup_withdrawal_request(
			RuntimeOrigin::signed(user2()),  // Anyone can call
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
		let deposit_id = generate_deposit_id(1);
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(alice()),
			deposit_id,
			user1(),
			5000,
		));
		assert_ok!(AlphaBridge::<Test>::attest_deposit(
			RuntimeOrigin::signed(bob()),
			deposit_id,
			user1(),
			5000,
		));

		// Create a withdrawal request
		assert_ok!(AlphaBridge::<Test>::withdraw(
			RuntimeOrigin::signed(user1()),
			1000,
		));

		let request_id = crate::WithdrawalRequests::<Test>::iter().next().unwrap().0;

		// Try to cleanup immediately (before TTL expires)
		assert_noop!(
			AlphaBridge::<Test>::cleanup_withdrawal_request(
				RuntimeOrigin::signed(user1()),
				request_id,
			),
			Error::<Test>::TTLNotExpired
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

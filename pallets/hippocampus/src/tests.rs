use crate::{
	mock::*, DepositType, Error, Event, TotalDeposited, TotalPaidByRequester, TotalPaidOut,
};
use frame_support::{assert_noop, assert_ok};

#[test]
fn deposit_works() {
	new_test_ext().execute_with(|| {
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(alice()),
			1_000,
			DepositType::MarketplaceRevenue
		));
		assert_eq!(Balances::free_balance(hippocampus_account()), 1_000);
		assert_eq!(Balances::free_balance(alice()), INITIAL_BALANCE - 1_000);
		assert_eq!(TotalDeposited::<Test>::get(DepositType::MarketplaceRevenue), 1_000);
		System::assert_last_event(
			Event::Deposited {
				who: alice(),
				amount: 1_000,
				deposit_type: DepositType::MarketplaceRevenue,
			}
			.into(),
		);

		// A second deposit with a different type is accounted separately.
		assert_ok!(Hippocampus::deposit(RuntimeOrigin::signed(bob()), 500, DepositType::Grant));
		assert_eq!(Balances::free_balance(hippocampus_account()), 1_500);
		assert_eq!(TotalDeposited::<Test>::get(DepositType::Grant), 500);
		assert_eq!(TotalDeposited::<Test>::get(DepositType::MarketplaceRevenue), 1_000);
	});
}

#[test]
fn deposit_zero_fails() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Hippocampus::deposit(RuntimeOrigin::signed(alice()), 0, DepositType::Other),
			Error::<Test>::ZeroAmount
		);
	});
}

#[test]
fn whitelist_management_works() {
	new_test_ext().execute_with(|| {
		// Only the admin origin can manage the whitelist.
		assert_noop!(
			Hippocampus::add_requester(RuntimeOrigin::signed(alice()), charlie()),
			sp_runtime::DispatchError::BadOrigin
		);

		assert_ok!(Hippocampus::add_requester(RuntimeOrigin::root(), charlie()));
		System::assert_last_event(Event::RequesterAdded { who: charlie() }.into());
		assert_noop!(
			Hippocampus::add_requester(RuntimeOrigin::root(), charlie()),
			Error::<Test>::AlreadyWhitelisted
		);

		assert_noop!(
			Hippocampus::remove_requester(RuntimeOrigin::signed(alice()), charlie()),
			sp_runtime::DispatchError::BadOrigin
		);
		assert_ok!(Hippocampus::remove_requester(RuntimeOrigin::root(), charlie()));
		System::assert_last_event(Event::RequesterRemoved { who: charlie() }.into());
		assert_noop!(
			Hippocampus::remove_requester(RuntimeOrigin::root(), charlie()),
			Error::<Test>::NotWhitelisted
		);
	});
}

#[test]
fn request_payment_requires_whitelist() {
	new_test_ext().execute_with(|| {
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(alice()),
			1_000,
			DepositType::MarketplaceRevenue
		));
		assert_noop!(
			Hippocampus::request_payment(&charlie(), &bob(), 100),
			Error::<Test>::RequesterNotWhitelisted
		);
	});
}

#[test]
fn request_payment_pays_in_full_when_funded() {
	new_test_ext().execute_with(|| {
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(alice()),
			1_000,
			DepositType::MarketplaceRevenue
		));
		assert_ok!(Hippocampus::add_requester(RuntimeOrigin::root(), charlie()));

		let bob_before = Balances::free_balance(bob());
		let paid = Hippocampus::request_payment(&charlie(), &bob(), 400).unwrap();
		assert_eq!(paid, 400);
		assert_eq!(Balances::free_balance(bob()), bob_before + 400);
		assert_eq!(Balances::free_balance(hippocampus_account()), 600);
		assert_eq!(TotalPaidOut::<Test>::get(), 400);
		assert_eq!(TotalPaidByRequester::<Test>::get(charlie()), 400);
		System::assert_last_event(
			Event::PaymentReleased { requester: charlie(), dest: bob(), requested: 400, paid: 400 }
				.into(),
		);
	});
}

#[test]
fn request_payment_is_capped_at_available_balance() {
	new_test_ext().execute_with(|| {
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(alice()),
			1_000,
			DepositType::MarketplaceRevenue
		));
		assert_ok!(Hippocampus::add_requester(RuntimeOrigin::root(), charlie()));

		// Asks for more than the bank holds: pays balance minus ED, never fails.
		let paid = Hippocampus::request_payment(&charlie(), &bob(), 5_000).unwrap();
		assert_eq!(paid, 1_000 - EXISTENTIAL_DEPOSIT);
		assert_eq!(Balances::free_balance(hippocampus_account()), EXISTENTIAL_DEPOSIT);
		assert_eq!(TotalPaidOut::<Test>::get(), 1_000 - EXISTENTIAL_DEPOSIT);
		System::assert_last_event(
			Event::PaymentReleased {
				requester: charlie(),
				dest: bob(),
				requested: 5_000,
				paid: 1_000 - EXISTENTIAL_DEPOSIT,
			}
			.into(),
		);

		// Hippocampus is now empty (only ED left): next request pays zero.
		let paid = Hippocampus::request_payment(&charlie(), &bob(), 100).unwrap();
		assert_eq!(paid, 0);
	});
}

#[test]
fn request_payment_zero_is_noop() {
	new_test_ext().execute_with(|| {
		assert_ok!(Hippocampus::add_requester(RuntimeOrigin::root(), charlie()));
		let paid = Hippocampus::request_payment(&charlie(), &bob(), 0).unwrap();
		assert_eq!(paid, 0);
		assert_eq!(TotalPaidOut::<Test>::get(), 0);
	});
}

#[test]
fn invariant_no_money_flows_when_distribution_disabled() {
	new_test_ext().execute_with(|| {
		// Fund the bank with plenty of money
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(alice()),
			10_000,
			DepositType::MarketplaceRevenue
		));
		assert_ok!(Hippocampus::add_requester(RuntimeOrigin::root(), charlie()));

		// Verify that payment works when distribution is enabled (default)
		let bob_before = Balances::free_balance(bob());
		let paid = Hippocampus::request_payment(&charlie(), &bob(), 1_000).unwrap();
		assert_eq!(paid, 1_000);
		assert_eq!(Balances::free_balance(bob()), bob_before + 1_000);

		// Disable distribution
		assert_ok!(Hippocampus::set_distribution_enabled(RuntimeOrigin::root(), false));

		// Verify the switch is off
		assert!(!Hippocampus::distribution_enabled());

		// Attempt to request payment when switch is OFF should fail
		let bob_before_disabled = Balances::free_balance(bob());
		assert_noop!(
			Hippocampus::request_payment(&charlie(), &bob(), 500),
			Error::<Test>::DistributionDisabled
		);
		// Verify no money was transferred
		assert_eq!(Balances::free_balance(bob()), bob_before_disabled);
		// Verify total paid out remains unchanged
		assert_eq!(TotalPaidOut::<Test>::get(), 1_000);

		// Re-enable distribution
		assert_ok!(Hippocampus::set_distribution_enabled(RuntimeOrigin::root(), true));
		assert!(Hippocampus::distribution_enabled());

		// Verify payment works again after re-enabling
		let bob_before_reenabled = Balances::free_balance(bob());
		let paid = Hippocampus::request_payment(&charlie(), &bob(), 500).unwrap();
		assert_eq!(paid, 500);
		assert_eq!(Balances::free_balance(bob()), bob_before_reenabled + 500);
	});
}

#[test]
fn set_distribution_enabled_requires_admin_origin() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Hippocampus::set_distribution_enabled(RuntimeOrigin::signed(alice()), false),
			sp_runtime::DispatchError::BadOrigin
		);
		// A rejected call must leave the switch untouched.
		assert!(Hippocampus::distribution_enabled());
	});
}

#[test]
fn set_distribution_enabled_emits_event() {
	new_test_ext().execute_with(|| {
		assert_ok!(Hippocampus::set_distribution_enabled(RuntimeOrigin::root(), false));
		System::assert_last_event(Event::DistributionEnabledChanged { enabled: false }.into());

		assert_ok!(Hippocampus::set_distribution_enabled(RuntimeOrigin::root(), true));
		System::assert_last_event(Event::DistributionEnabledChanged { enabled: true }.into());
	});
}

#[test]
fn disabled_check_precedes_whitelist_check() {
	new_test_ext().execute_with(|| {
		// A non-whitelisted requester while disabled must see DistributionDisabled,
		// not RequesterNotWhitelisted: the kill switch outranks per-requester state.
		assert_ok!(Hippocampus::set_distribution_enabled(RuntimeOrigin::root(), false));
		assert_noop!(
			Hippocampus::request_payment(&charlie(), &bob(), 100),
			Error::<Test>::DistributionDisabled
		);
	});
}

use crate::{
	mock::*, DepositType, EmissionPaidOut, Error, Event, MinerPaymentWhitelist, TotalDeposited,
	TotalPaidByRequester, TotalPaidOut,
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

#[test]
fn pay_storage_miners_distributes_pro_rata() {
	new_test_ext().execute_with(|| {
		// The Grant covers the bank's ED so the full emission amount is payable.
		assert_ok!(Hippocampus::deposit(RuntimeOrigin::signed(alice()), 10, DepositType::Grant));
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(alice()),
			1_000,
			DepositType::Emission
		));
		set_ranked_miners(vec![(charlie(), 1), (dave(), 3)]);
		whitelist_miner_payment_caller(alice());

		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 1_000));

		assert_eq!(Balances::free_balance(charlie()), 250);
		assert_eq!(Balances::free_balance(dave()), 750);
		assert_eq!(EmissionPaidOut::<Test>::get(), 1_000);
		assert_eq!(Hippocampus::emission_available(), 0);
		assert_eq!(TotalPaidOut::<Test>::get(), 1_000);
		System::assert_last_event(
			Event::StorageMinersPaid {
				requested: 1_000,
				paid: 1_000,
				miners_paid: 2,
				miners_skipped: 0,
			}
			.into(),
		);

		// Each successful transfer also emits its per-miner indexing event.
		let events = System::events();
		for expected in [
			Event::MinerPaymentPaid { miner: charlie(), amount: 250 },
			Event::MinerPaymentPaid { miner: dave(), amount: 750 },
		] {
			let expected = expected.into();
			assert!(
				events.iter().any(|record| record.event == expected),
				"missing per-miner event: {expected:?}"
			);
		}
	});
}

#[test]
fn pay_storage_miners_skips_failed_transfers() {
	new_test_ext().execute_with(|| {
		assert_ok!(Hippocampus::deposit(RuntimeOrigin::signed(alice()), 10, DepositType::Grant));
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(alice()),
			1_000,
			DepositType::Emission
		));
		// charlie's share (101 * 1/20 = 5) is below the ED of 10 and charlie
		// holds no account, so the transfer itself fails and is skipped;
		// dave's share (95) clears the ED and pays out.
		set_ranked_miners(vec![(charlie(), 1), (dave(), 19)]);
		whitelist_miner_payment_caller(alice());

		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 101));

		assert_eq!(Balances::free_balance(charlie()), 0);
		assert_eq!(Balances::free_balance(dave()), 95);
		// Only what actually moved is booked; the failed share stays in the
		// compartment.
		assert_eq!(EmissionPaidOut::<Test>::get(), 95);
		assert_eq!(Hippocampus::emission_available(), 905);
		System::assert_last_event(
			Event::StorageMinersPaid {
				requested: 101,
				paid: 95,
				miners_paid: 1,
				miners_skipped: 1,
			}
			.into(),
		);
	});
}

#[test]
fn pay_storage_miners_conserves_every_planck() {
	// Invariant sweep: whatever the weight vector, exactly `paid` leaves the
	// bank, all of it lands with miners, both ledgers book the same figure,
	// and floor-division dust is bounded by one planck per miner.
	let cases: &[(&[u16], u128)] = &[
		(&[1, 3], 1_000),
		(&[7, 7, 7], 1_000),
		(&[1, 65_535], 9_999),
		(&[13, 29, 58], 101),
		(&[5], 10),
		(&[1, 2, 3, 4, 5, 6, 7], 9_973),
	];
	for (weights, amount) in cases {
		new_test_ext().execute_with(|| {
			assert_ok!(Hippocampus::deposit(
				RuntimeOrigin::signed(alice()),
				10,
				DepositType::Grant
			));
			assert_ok!(Hippocampus::deposit(
				RuntimeOrigin::signed(alice()),
				10_000,
				DepositType::Emission
			));
			let miners: Vec<(AccountId, u16)> = weights
				.iter()
				.enumerate()
				.map(|(i, w)| {
					let index = u8::try_from(i).expect("small test vector");
					(sp_runtime::AccountId32::new([100 + index; 32]), *w)
				})
				.collect();
			set_ranked_miners(miners.clone());
			whitelist_miner_payment_caller(alice());

			let bank_before = Balances::free_balance(hippocampus_account());
			assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), *amount));

			let received: u128 =
				miners.iter().map(|(miner, _)| Balances::free_balance(miner)).sum();
			let bank_delta = bank_before - Balances::free_balance(hippocampus_account());
			assert_eq!(received, bank_delta, "weights {weights:?}");
			assert_eq!(EmissionPaidOut::<Test>::get(), received, "weights {weights:?}");
			assert_eq!(TotalPaidOut::<Test>::get(), received, "weights {weights:?}");
			assert!(received <= *amount);
			assert!(
				*amount - received < weights.len() as u128,
				"dust exceeded bound for weights {weights:?}: paid {received} of {amount}"
			);
		});
	}
}

#[test]
fn pay_storage_miners_requires_whitelisted_caller() {
	new_test_ext().execute_with(|| {
		// Non-whitelisted caller should fail
		assert_noop!(
			Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 100),
			Error::<Test>::PaymentCallerNotWhitelisted
		);
	});
}

#[test]
fn pay_storage_miners_rejects_zero_amount() {
	new_test_ext().execute_with(|| {
		whitelist_miner_payment_caller(alice());
		assert_noop!(
			Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 0),
			Error::<Test>::ZeroAmount
		);
	});
}

#[test]
fn pay_storage_miners_respects_distribution_switch() {
	new_test_ext().execute_with(|| {
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(alice()),
			1_000,
			DepositType::Emission
		));
		set_ranked_miners(vec![(charlie(), 1)]);
		whitelist_miner_payment_caller(alice());
		assert_ok!(Hippocampus::set_distribution_enabled(RuntimeOrigin::root(), false));
		assert_noop!(
			Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 100),
			Error::<Test>::DistributionDisabled
		);
	});
}

#[test]
fn pay_storage_miners_cannot_spend_other_compartments() {
	new_test_ext().execute_with(|| {
		// A fat bank funded by everything except emission pays nothing.
		assert_ok!(Hippocampus::deposit(RuntimeOrigin::signed(alice()), 1_000, DepositType::Grant));
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(alice()),
			1_000,
			DepositType::MarketplaceRevenue
		));
		set_ranked_miners(vec![(charlie(), 1)]);
		whitelist_miner_payment_caller(alice());
		assert_noop!(
			Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 100),
			Error::<Test>::InsufficientEmissionFunds
		);
	});
}

#[test]
fn pay_storage_miners_cannot_overdraw_the_bank() {
	new_test_ext().execute_with(|| {
		// Compartment says 1_000 but another consumer already drained the
		// account below that — reject rather than pay partially.
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(alice()),
			1_000,
			DepositType::Emission
		));
		assert_ok!(Hippocampus::add_requester(RuntimeOrigin::root(), bob()));
		assert_ok!(Hippocampus::request_payment(&bob(), &charlie(), 600));
		set_ranked_miners(vec![(dave(), 1)]);
		whitelist_miner_payment_caller(alice());
		assert_noop!(
			Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 1_000),
			Error::<Test>::InsufficientBankBalance
		);
	});
}

#[test]
fn pay_storage_miners_rejects_empty_or_zero_weight_ranking() {
	new_test_ext().execute_with(|| {
		assert_ok!(Hippocampus::deposit(RuntimeOrigin::signed(alice()), 10, DepositType::Grant));
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(alice()),
			1_000,
			DepositType::Emission
		));
		whitelist_miner_payment_caller(alice());
		assert_noop!(
			Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 100),
			Error::<Test>::NoEligibleMiners
		);
		set_ranked_miners(vec![(charlie(), 0), (dave(), 0)]);
		whitelist_miner_payment_caller(alice());
		assert_noop!(
			Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 100),
			Error::<Test>::NoEligibleMiners
		);
	});
}

#[test]
fn pay_storage_miners_bounds_the_miner_list() {
	new_test_ext().execute_with(|| {
		assert_ok!(Hippocampus::deposit(RuntimeOrigin::signed(alice()), 10, DepositType::Grant));
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(alice()),
			1_000,
			DepositType::Emission
		));
		// MaxMinersPerPayout is 16 in the mock; 17 entries must reject.
		let miners: Vec<_> =
			(1u8..=17).map(|i| (sp_runtime::AccountId32::new([i; 32]), 1u16)).collect();
		set_ranked_miners(miners);
		whitelist_miner_payment_caller(alice());
		assert_noop!(
			Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 100),
			Error::<Test>::TooManyMiners
		);
	});
}

#[test]
fn pay_storage_miners_dust_stays_in_compartment() {
	new_test_ext().execute_with(|| {
		assert_ok!(Hippocampus::deposit(RuntimeOrigin::signed(alice()), 10, DepositType::Grant));
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(alice()),
			1_000,
			DepositType::Emission
		));
		set_ranked_miners(vec![(charlie(), 3), (dave(), 7)]);
		whitelist_miner_payment_caller(alice());

		// 101 * 3/10 = 30, 101 * 7/10 = 70 — one planck of dust remains.
		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 101));

		assert_eq!(Balances::free_balance(charlie()), 30);
		assert_eq!(Balances::free_balance(dave()), 70);
		assert_eq!(EmissionPaidOut::<Test>::get(), 100);
		assert_eq!(Hippocampus::emission_available(), 900);
		System::assert_last_event(
			Event::StorageMinersPaid {
				requested: 101,
				paid: 100,
				miners_paid: 2,
				miners_skipped: 0,
			}
			.into(),
		);
	});
}

#[test]
fn pay_storage_miners_skips_zero_shares() {
	new_test_ext().execute_with(|| {
		assert_ok!(Hippocampus::deposit(RuntimeOrigin::signed(alice()), 10, DepositType::Grant));
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(alice()),
			100_000,
			DepositType::Emission
		));
		// 10_000 * 1/65_536 floors to zero: charlie is skipped, not fatal.
		set_ranked_miners(vec![(charlie(), 1), (dave(), 65_535)]);
		whitelist_miner_payment_caller(alice());

		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 10_000));

		assert_eq!(Balances::free_balance(charlie()), 0);
		assert_eq!(Balances::free_balance(dave()), 9_999);
		System::assert_last_event(
			Event::StorageMinersPaid {
				requested: 10_000,
				paid: 9_999,
				miners_paid: 1,
				miners_skipped: 1,
			}
			.into(),
		);
	});
}

#[test]
fn pay_storage_miners_compartment_is_cumulative() {
	new_test_ext().execute_with(|| {
		assert_ok!(Hippocampus::deposit(RuntimeOrigin::signed(alice()), 10, DepositType::Grant));
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(alice()),
			1_000,
			DepositType::Emission
		));
		set_ranked_miners(vec![(charlie(), 1)]);
		whitelist_miner_payment_caller(alice());

		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 600));
		assert_eq!(Hippocampus::emission_available(), 400);

		// The second call may spend only what the compartment still holds.
		assert_noop!(
			Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 401),
			Error::<Test>::InsufficientEmissionFunds
		);
		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 400));
		assert_eq!(Hippocampus::emission_available(), 0);
		assert_eq!(Balances::free_balance(charlie()), 1_000);
	});
}

#[test]
fn pay_storage_miners_respects_24hour_cap() {
	new_test_ext().execute_with(|| {
		let cap = Max24HourMinerPayout::get();

		assert_ok!(Hippocampus::deposit(RuntimeOrigin::signed(alice()), 10, DepositType::Grant));
		// Twice the cap in the compartment, so only the rate limit can reject.
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(alice()),
			cap * 2,
			DepositType::Emission
		));
		set_ranked_miners(vec![(charlie(), 1)]);
		whitelist_miner_payment_caller(alice());

		// A single request above the cap rejects outright.
		assert_noop!(
			Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), cap + 1),
			Error::<Test>::ExceedsDaily24HourMinerPayoutLimit
		);

		// Spend most of the budget, then reject when the combined total
		// would cross the cap.
		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), cap - 100));
		assert_noop!(
			Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 101),
			Error::<Test>::ExceedsDaily24HourMinerPayoutLimit
		);

		// Exactly reaching the cap is allowed.
		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 100));
		assert_eq!(Balances::free_balance(charlie()), cap);
	});
}

#[test]
fn add_miner_payment_caller_works() {
	new_test_ext().execute_with(|| {
		let caller = alice();
		assert!(!MinerPaymentWhitelist::<Test>::contains_key(&caller));

		assert_ok!(Hippocampus::add_miner_payment_caller(RuntimeOrigin::root(), caller.clone()));
		assert!(MinerPaymentWhitelist::<Test>::contains_key(&caller));

		System::assert_last_event(Event::MinerPaymentCallerAdded { who: caller }.into());
	});
}

#[test]
fn add_miner_payment_caller_requires_admin() {
	new_test_ext().execute_with(|| {
		let caller = alice();
		assert_noop!(
			Hippocampus::add_miner_payment_caller(RuntimeOrigin::signed(caller.clone()), caller),
			sp_runtime::DispatchError::BadOrigin
		);
	});
}

#[test]
fn remove_miner_payment_caller_works() {
	new_test_ext().execute_with(|| {
		let caller = alice();
		assert_ok!(Hippocampus::add_miner_payment_caller(RuntimeOrigin::root(), caller.clone()));
		assert!(MinerPaymentWhitelist::<Test>::contains_key(&caller));

		assert_ok!(Hippocampus::remove_miner_payment_caller(RuntimeOrigin::root(), caller.clone()));
		assert!(!MinerPaymentWhitelist::<Test>::contains_key(&caller));

		System::assert_last_event(Event::MinerPaymentCallerRemoved { who: caller.clone() }.into());

		// A removed caller can no longer trigger payouts.
		assert_noop!(
			Hippocampus::pay_storage_miners(RuntimeOrigin::signed(caller), 100),
			Error::<Test>::PaymentCallerNotWhitelisted
		);
	});
}

#[test]
fn pay_storage_miners_resets_24hour_period() {
	new_test_ext().execute_with(|| {
		let cap = Max24HourMinerPayout::get();

		assert_ok!(Hippocampus::deposit(RuntimeOrigin::signed(alice()), 10, DepositType::Grant));
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(alice()),
			cap * 2,
			DepositType::Emission
		));
		set_ranked_miners(vec![(charlie(), 1)]);
		whitelist_miner_payment_caller(alice());

		// Exhaust the whole 24-hour budget; the next planck rejects.
		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), cap));
		assert_noop!(
			Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 1),
			Error::<Test>::ExceedsDaily24HourMinerPayoutLimit
		);

		// One block short of the period boundary the budget is still spent.
		// The first period is anchored at block 0 (MinerPayoutPeriodStart
		// default), so the boundary falls exactly on BlocksPer24Hours.
		System::set_block_number(BlocksPer24Hours::get() - 1);
		assert_noop!(
			Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 1),
			Error::<Test>::ExceedsDaily24HourMinerPayoutLimit
		);

		// At the boundary the period resets and a full budget is available.
		System::set_block_number(BlocksPer24Hours::get());
		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), cap));
		assert_eq!(Hippocampus::emission_available(), 0);
		assert_eq!(Balances::free_balance(charlie()), cap * 2);
	});
}

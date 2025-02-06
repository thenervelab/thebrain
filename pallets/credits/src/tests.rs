use crate::{mock::*, Error, Event};
use frame_support::{assert_noop, assert_ok};

#[test]
fn convert_balance_to_credits_works() {
    new_test_ext().execute_with(|| {
        // Initial setup
        let account_id = 1;
        let initial_balance = 100;
        let conversion_amount = 50;

        // Verify initial state
        assert_eq!(Balances::free_balance(account_id), initial_balance);
        assert_eq!(Credits::get_free_credits(&account_id), 0);

        // Convert balance to credits
        assert_ok!(Credits::convert_balance_to_credits(
            RuntimeOrigin::signed(account_id), 
            conversion_amount
        ));

        // Check updated balance and credits
        assert_eq!(Balances::free_balance(account_id), initial_balance - (conversion_amount as u64));
        assert_eq!(Credits::get_free_credits(&account_id), conversion_amount);

        // Check event was emitted
        System::assert_last_event(
            Event::ConvertedToCredits { 
                who: account_id, 
                amount: conversion_amount 
            }.into()
        );
    });
}

#[test]
fn convert_balance_to_credits_fails_with_insufficient_balance() {
    new_test_ext().execute_with(|| {
        let account_id = 1;
        let conversion_amount = 150; // More than initial balance

        assert_noop!(
            Credits::convert_balance_to_credits(
                RuntimeOrigin::signed(account_id), 
                conversion_amount
            ),
            Error::<Test>::InsufficientBalance
        );
    });
}

#[test]
fn convert_balance_to_credits_fails_with_zero_amount() {
    new_test_ext().execute_with(|| {
        let account_id = 1;

        assert_noop!(
            Credits::convert_balance_to_credits(
                RuntimeOrigin::signed(account_id), 
                0
            ),
            Error::<Test>::InvalidConversionAmount
        );
    });
}

#[test]
fn mint_credits_works() {
    new_test_ext().execute_with(|| {
        // Add an authority first
        let authority = 2;
        let user = 1;
        let mint_amount = 50;

        // Add authority (in a real scenario, this would be done via sudo)
        assert_ok!(Credits::add_authority(RuntimeOrigin::root(), authority));

        // Mint credits as an authority
        assert_ok!(Credits::mint(RuntimeOrigin::signed(authority), user, mint_amount));

        // Check credits were minted
        assert_eq!(Credits::get_free_credits(&user), mint_amount);

        // Check event was emitted
        System::assert_last_event(
            Event::MinetdAccountCredits { 
                who: user, 
                amount: mint_amount 
            }.into()
        );
    });
}

#[test]
fn burn_credits_works() {
    new_test_ext().execute_with(|| {
        // Add an authority first
        let authority = 2;
        let user = 1;
        let mint_amount = 50;
        let burn_amount = 20;

        // Add authority (in a real scenario, this would be done via sudo)
        assert_ok!(Credits::add_authority(RuntimeOrigin::root(), authority));

        // Mint credits first
        assert_ok!(Credits::mint(RuntimeOrigin::signed(authority), user, mint_amount));

        // Burn credits as an authority
        assert_ok!(Credits::burn(RuntimeOrigin::signed(authority), user, burn_amount));

        // Check credits were burned
        assert_eq!(Credits::get_free_credits(&user), mint_amount - burn_amount);

        // Check event was emitted
        System::assert_last_event(
            Event::BurnedAccountCredits { 
                who: user, 
                amount: mint_amount - burn_amount 
            }.into()
        );
    });
}

#[test]
fn burn_credits_fails_with_insufficient_credits() {
    new_test_ext().execute_with(|| {
        // Add an authority first
        let authority = 2;
        let user = 1;
        let burn_amount = 50;

        // Add authority (in a real scenario, this would be done via sudo)
        assert_ok!(Credits::add_authority(RuntimeOrigin::root(), authority));

        // Attempt to burn credits without minting first
        assert_noop!(
            Credits::burn(RuntimeOrigin::signed(authority), user, burn_amount),
            Error::<Test>::InsufficientFreeCredits
        );
    });
}
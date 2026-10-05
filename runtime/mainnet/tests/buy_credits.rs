//! `buy_credits`: a user pays native tokens into the bank and is credited at
//! `Credits.AlphaPrice`.
//!
//! What the chain owes that call is that the tokens and the credits move
//! together or not at all, that the price it uses is one an authority set
//! recently, that the buyer never gets fewer credits than they signed for, and
//! that the buyer's account is never reaped by it. The authority `deposit`
//! shares its batch path, so the last tests pin that `deposit` behaves exactly
//! as before.

use frame_support::{
	assert_noop, assert_ok,
	dispatch::{GetDispatchInfo, Pays},
	traits::{Currency, InstanceFilter, OnRuntimeUpgrade},
};
use hippius_mainnet_runtime::{
	migrations::InitAlphaPriceUpdatedAt, AccountId, Balances, Credits, Hippocampus, Marketplace,
	MaxAlphaPriceAge, MaxBatchesPerBuyer, MinBuyAlpha, ProxyType, Runtime, RuntimeCall,
	RuntimeEvent, RuntimeOrigin, System,
};
use pallet_credits::{AlphaPrice, AlphaPriceUpdatedAt, ReferralCodes, ReferredUsers};
use pallet_hippocampus::DepositType;
use pallet_marketplace::{
	weights::WeightInfo, Batches, BuyCreditsEnabled, Error, Event as MarketplaceEvent,
	LastPurchaseBatch, NextBatchId, TotalUndistributedBacking, UnbackedBatchAlpha, UserBatches,
};
use sp_runtime::{AccountId32, ArithmeticError, BuildStorage, DispatchError, TokenError};

/// One alpha, in planck.
const ALPHA: u128 = 1_000_000_000_000_000_000;

/// $0.37 per alpha, 18 decimals. Odd on purpose, so rounding shows.
const PRICE: u128 = 370_000_000_000_000_000;

/// Runtime existential deposit.
const ED: u128 = 500;

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

/// Sets the price (pallet-credits authority).
fn oracle() -> AccountId {
	account(1)
}

fn buyer() -> AccountId {
	account(2)
}

fn referrer() -> AccountId {
	account(3)
}

fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| {
		System::set_block_number(1);
		assert_ok!(Credits::add_authority(RuntimeOrigin::root(), oracle()));
		assert_ok!(Marketplace::sudo_set_buy_credits_enabled(RuntimeOrigin::root(), true));
		assert_ok!(Credits::set_alpha_price(RuntimeOrigin::signed(oracle()), PRICE));
		let _ = Balances::deposit_creating(&buyer(), 10 * ALPHA);
	});
	ext
}

fn credits_for(alpha: u128) -> u128 {
	alpha * PRICE / ALPHA
}

fn buy(alpha: u128, min_credits: u128) -> Result<(), DispatchError> {
	Marketplace::buy_credits(RuntimeOrigin::signed(buyer()), alpha, min_credits, None)
		.map_err(Into::into)
}

fn bank() -> AccountId {
	Hippocampus::account_id()
}

fn marketplace_events() -> Vec<MarketplaceEvent<Runtime>> {
	System::events()
		.into_iter()
		.filter_map(|r| match r.event {
			RuntimeEvent::Marketplace(e) => Some(e),
			_ => None,
		})
		.collect()
}

#[test]
fn buy_moves_tokens_to_bank_and_mints_a_batch() {
	new_test_ext().execute_with(|| {
		let batch_id = NextBatchId::<Runtime>::get();
		let backing_before = TotalUndistributedBacking::<Runtime>::get();
		let expected = credits_for(ALPHA);
		assert_eq!(expected, 370_000_000_000_000_000);

		assert_ok!(buy(ALPHA, expected));

		assert_eq!(Balances::free_balance(buyer()), 9 * ALPHA);
		assert_eq!(Balances::free_balance(bank()), ALPHA);
		assert_eq!(Credits::get_free_credits(&buyer()), expected);
		assert_eq!(pallet_credits::AlphaBalances::<Runtime>::get(buyer()), ALPHA);
		assert_eq!(TotalUndistributedBacking::<Runtime>::get(), backing_before + ALPHA);
		assert_eq!(NextBatchId::<Runtime>::get(), batch_id + 1);
		assert_eq!(UserBatches::<Runtime>::get(buyer()), Some(vec![batch_id]));
		assert!(!UnbackedBatchAlpha::<Runtime>::contains_key(batch_id));
		let batch = Batches::<Runtime>::get(batch_id).unwrap();
		assert_eq!(batch.owner, buyer());
		assert_eq!(batch.credit_amount, expected);
		assert_eq!(batch.alpha_amount, ALPHA);
		assert_eq!(batch.remaining_credits, expected);
		assert_eq!(batch.remaining_alpha, ALPHA);
		assert_eq!(batch.pending_alpha, 0);
		assert!(!batch.is_frozen);
		assert_eq!(batch.release_time, 1);

		let events = marketplace_events();
		assert!(events.contains(&MarketplaceEvent::BatchDeposited { owner: buyer(), batch_id }));
		assert_eq!(
			events.last(),
			Some(&MarketplaceEvent::CreditsBought {
				who: buyer(),
				alpha_amount: ALPHA,
				credit_amount: expected,
				batch_id,
				alpha_price: PRICE,
			})
		);
		assert!(System::events().iter().any(|r| r.event
			== RuntimeEvent::Hippocampus(pallet_hippocampus::Event::Deposited {
				who: buyer(),
				amount: ALPHA,
				deposit_type: DepositType::MarketplaceRevenue,
			})));
	});
}

#[test]
fn buy_is_fee_paying_and_priced_by_weight_info() {
	new_test_ext().execute_with(|| {
		let call = RuntimeCall::Marketplace(pallet_marketplace::Call::buy_credits {
			alpha_amount: ALPHA,
			min_credits: 0,
			code: None,
		});
		let info = call.get_dispatch_info();
		assert_eq!(info.pays_fee, Pays::Yes);
		assert_eq!(
			info.weight,
			pallet_marketplace::weights::SubstrateWeight::<Runtime>::buy_credits()
		);
	});
}

#[test]
fn credits_round_down() {
	new_test_ext().execute_with(|| {
		// 0.01 alpha + 1 planck at $0.37: the planck is worth 0.37 of a unit.
		let alpha = MinBuyAlpha::get() + 1;
		assert_ok!(buy(alpha, 0));
		assert_eq!(Credits::get_free_credits(&buyer()), 3_700_000_000_000_000);
		assert_eq!(Marketplace::alpha_to_credits(alpha, PRICE), Ok(3_700_000_000_000_000));
	});
}

#[test]
fn credit_amount_overflow_is_refused_not_wrapped() {
	new_test_ext().execute_with(|| {
		assert_eq!(
			Marketplace::alpha_to_credits(u128::MAX, u128::MAX),
			Err(Error::<Runtime>::CreditAmountOverflow.into())
		);
		// Products past u128 are still exact when the quotient fits.
		assert_eq!(Marketplace::alpha_to_credits(u128::MAX, ALPHA), Ok(u128::MAX));
	});
}

#[test]
fn credits_that_would_not_land_are_refused() {
	new_test_ext().execute_with(|| {
		assert_ok!(Credits::do_mint(buyer(), u128::MAX - 1, None));
		assert_noop!(buy(ALPHA, 0), Error::<Runtime>::CreditAmountOverflow);
	});
}

#[test]
fn disabled_refuses() {
	new_test_ext().execute_with(|| {
		assert_ok!(Marketplace::sudo_set_buy_credits_enabled(RuntimeOrigin::root(), false));
		assert_noop!(buy(ALPHA, 0), Error::<Runtime>::BuyCreditsDisabled);
	});
}

#[test]
fn enable_switch_is_root_only_and_off_by_default() {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	sp_io::TestExternalities::new(t).execute_with(|| {
		System::set_block_number(1);
		assert!(!BuyCreditsEnabled::<Runtime>::get());
		assert_noop!(
			Marketplace::sudo_set_buy_credits_enabled(RuntimeOrigin::signed(buyer()), true),
			DispatchError::BadOrigin
		);
		assert_ok!(Marketplace::sudo_set_buy_credits_enabled(RuntimeOrigin::root(), true));
		assert!(BuyCreditsEnabled::<Runtime>::get());
		assert_eq!(
			marketplace_events().last(),
			Some(&MarketplaceEvent::BuyCreditsStatusChanged { enabled: true })
		);
	});
}

#[test]
fn zero_and_dust_amounts_refused() {
	new_test_ext().execute_with(|| {
		assert_noop!(buy(0, 0), Error::<Runtime>::BuyAmountTooLow);
		assert_noop!(buy(MinBuyAlpha::get() - 1, 0), Error::<Runtime>::BuyAmountTooLow);
		assert_ok!(buy(MinBuyAlpha::get(), 0));
	});
}

#[test]
fn unset_price_refused() {
	new_test_ext().execute_with(|| {
		AlphaPrice::<Runtime>::kill();
		assert_noop!(buy(ALPHA, 0), Error::<Runtime>::AlphaPriceNotSet);
	});
}

#[test]
fn set_alpha_price_stamps_the_block() {
	new_test_ext().execute_with(|| {
		assert_eq!(AlphaPriceUpdatedAt::<Runtime>::get(), Some(1));
		System::set_block_number(42);
		assert_ok!(Credits::set_alpha_price(RuntimeOrigin::signed(oracle()), PRICE));
		assert_eq!(AlphaPriceUpdatedAt::<Runtime>::get(), Some(42));
	});
}

#[test]
fn stale_price_refused() {
	new_test_ext().execute_with(|| {
		let max_age = MaxAlphaPriceAge::get();
		assert_eq!(max_age, 28_800, "two days of 6s blocks");

		System::set_block_number(1 + max_age);
		assert_ok!(buy(ALPHA, 0));

		System::set_block_number(2 + max_age);
		assert_noop!(buy(ALPHA, 0), Error::<Runtime>::AlphaPriceStale);

		// Refreshing the price reopens it.
		assert_ok!(Credits::set_alpha_price(RuntimeOrigin::signed(oracle()), PRICE));
		assert_ok!(buy(ALPHA, 0));
	});
}

#[test]
fn price_without_update_record_is_stale() {
	new_test_ext().execute_with(|| {
		AlphaPriceUpdatedAt::<Runtime>::kill();
		assert_noop!(buy(ALPHA, 0), Error::<Runtime>::AlphaPriceStale);
	});
}

#[test]
fn slippage_bound_holds() {
	new_test_ext().execute_with(|| {
		let expected = credits_for(ALPHA);
		assert_noop!(buy(ALPHA, expected + 1), Error::<Runtime>::SlippageExceeded);
		assert_ok!(buy(ALPHA, expected));
	});
}

#[test]
fn insufficient_balance_leaves_no_trace() {
	new_test_ext().execute_with(|| {
		// `assert_noop` compares the whole storage root: no batch, no
		// credits, no request-count bump.
		assert_noop!(buy(11 * ALPHA, 0), ArithmeticError::Underflow);
	});
}

#[test]
fn buy_never_reaps_the_buyer() {
	new_test_ext().execute_with(|| {
		// The whole balance would take the account below ED.
		assert_noop!(buy(10 * ALPHA, 0), TokenError::NotExpendable);

		assert_ok!(buy(10 * ALPHA - ED, 0));
		assert_eq!(Balances::free_balance(buyer()), ED);
		assert!(System::account_exists(&buyer()));
	});
}

#[test]
fn referral_code_recorded() {
	new_test_ext().execute_with(|| {
		ReferralCodes::<Runtime>::insert(b"HIPPIUSREF".to_vec(), referrer());
		assert_ok!(Marketplace::buy_credits(
			RuntimeOrigin::signed(buyer()),
			ALPHA,
			0,
			Some(b"HIPPIUSREF".to_vec()),
		));
		assert_eq!(ReferredUsers::<Runtime>::get(buyer()), Some(b"HIPPIUSREF".to_vec()));
	});
}

#[test]
fn bad_referral_code_fails_the_whole_purchase() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Marketplace::buy_credits(
				RuntimeOrigin::signed(buyer()),
				ALPHA,
				0,
				Some(b"NOPE".to_vec())
			),
			pallet_credits::Error::<Runtime>::InvalidReferralCode
		);

		ReferralCodes::<Runtime>::insert(b"MINE".to_vec(), buyer());
		assert_noop!(
			Marketplace::buy_credits(
				RuntimeOrigin::signed(buyer()),
				ALPHA,
				0,
				Some(b"MINE".to_vec())
			),
			pallet_credits::Error::<Runtime>::InvalidRefferalOwner
		);
	});
}

#[test]
fn oversized_referral_code_refused_up_front() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Marketplace::buy_credits(
				RuntimeOrigin::signed(buyer()),
				ALPHA,
				0,
				Some(vec![b'A'; 65])
			),
			Error::<Runtime>::ReferralCodeTooLong
		);
	});
}

fn backend_deposit(n: u32) {
	for _ in 0..n {
		assert_ok!(Marketplace::deposit(
			RuntimeOrigin::signed(oracle()),
			buyer(),
			1,
			0,
			false,
			None,
		));
	}
}

#[test]
fn at_the_batch_cap_purchases_top_up_the_last_purchase_batch() {
	new_test_ext().execute_with(|| {
		let cap = MaxBatchesPerBuyer::get();
		backend_deposit(cap - 2);
		assert_ok!(buy(ALPHA, 0));
		let first = NextBatchId::<Runtime>::get() - 1;
		assert_eq!(LastPurchaseBatch::<Runtime>::get(buyer()), Some(first));
		// Still below the cap: a new batch.
		assert_ok!(buy(ALPHA, 0));
		let last = NextBatchId::<Runtime>::get() - 1;
		assert_ne!(last, first);
		assert_eq!(LastPurchaseBatch::<Runtime>::get(buyer()), Some(last));
		assert_eq!(UserBatches::<Runtime>::get(buyer()).unwrap().len() as u32, cap);
		let credits_before = Credits::get_free_credits(&buyer());
		let backing_before = TotalUndistributedBacking::<Runtime>::get();
		System::reset_events();

		// At the cap: same batch, grown.
		assert_ok!(buy(2 * ALPHA, 0));
		let added = credits_for(2 * ALPHA);
		assert_eq!(NextBatchId::<Runtime>::get() - 1, last);
		assert_eq!(UserBatches::<Runtime>::get(buyer()).unwrap().len() as u32, cap);
		let batch = Batches::<Runtime>::get(last).unwrap();
		assert_eq!(batch.alpha_amount, 3 * ALPHA);
		assert_eq!(batch.remaining_alpha, 3 * ALPHA);
		assert_eq!(batch.credit_amount, credits_for(ALPHA) + added);
		assert_eq!(batch.remaining_credits, credits_for(ALPHA) + added);
		assert!(!batch.is_frozen);
		assert_eq!(Credits::get_free_credits(&buyer()), credits_before + added);
		assert_eq!(pallet_credits::AlphaBalances::<Runtime>::get(buyer()), 4 * ALPHA);
		assert_eq!(Balances::free_balance(bank()), 4 * ALPHA);
		assert_eq!(Balances::free_balance(buyer()), 6 * ALPHA);
		assert_eq!(TotalUndistributedBacking::<Runtime>::get(), backing_before + 2 * ALPHA);
		assert_eq!(
			marketplace_events(),
			vec![
				MarketplaceEvent::BatchDeposited { owner: buyer(), batch_id: last },
				MarketplaceEvent::CreditsBought {
					who: buyer(),
					alpha_amount: 2 * ALPHA,
					credit_amount: added,
					batch_id: last,
					alpha_price: PRICE,
				},
			]
		);
	});
}

#[test]
fn topped_up_batch_is_consumed_like_any_other() {
	new_test_ext().execute_with(|| {
		let cap = MaxBatchesPerBuyer::get();
		backend_deposit(cap - 1);
		assert_ok!(buy(ALPHA, 0));
		assert_ok!(buy(ALPHA, 0));
		let id = LastPurchaseBatch::<Runtime>::get(buyer()).unwrap();
		let all = Credits::get_free_credits(&buyer());
		let backing_before = TotalUndistributedBacking::<Runtime>::get();

		assert_ok!(Marketplace::consume_credits(buyer(), all));

		let batch = Batches::<Runtime>::get(id).unwrap();
		assert_eq!(batch.remaining_credits, 0);
		assert_eq!(batch.remaining_alpha, 0);
		assert_eq!(TotalUndistributedBacking::<Runtime>::get(), backing_before - 2 * ALPHA);
		assert_eq!(pallet_credits::AlphaBalances::<Runtime>::get(buyer()), 0);
	});
}

#[test]
fn at_the_batch_cap_without_a_purchase_batch_refuses() {
	new_test_ext().execute_with(|| {
		backend_deposit(MaxBatchesPerBuyer::get());
		assert_noop!(buy(MinBuyAlpha::get(), 0), Error::<Runtime>::TooManyBatches);
	});
}

#[test]
fn non_transfer_proxy_cannot_buy_credits() {
	let call = RuntimeCall::Marketplace(pallet_marketplace::Call::buy_credits {
		alpha_amount: ALPHA,
		min_credits: 0,
		code: None,
	});
	assert!(!ProxyType::NonTransfer.filter(&call));
	assert!(ProxyType::Any.filter(&call));
}

#[test]
fn rate_limited_like_other_user_calls() {
	new_test_ext().execute_with(|| {
		for _ in 0..5 {
			assert_ok!(buy(MinBuyAlpha::get(), 0));
		}
		assert_noop!(buy(MinBuyAlpha::get(), 0), Error::<Runtime>::TooManyRequests);
	});
}

#[test]
fn authority_deposit_still_routes_from_sudo() {
	new_test_ext().execute_with(|| {
		let sudo = account(9);
		let _ = Balances::deposit_creating(&sudo, 10 * ALPHA);
		assert_ok!(Marketplace::set_sudo_key(RuntimeOrigin::root(), sudo.clone()));
		let batch_id = NextBatchId::<Runtime>::get();
		let backing_before = TotalUndistributedBacking::<Runtime>::get();

		assert_ok!(Marketplace::deposit(
			RuntimeOrigin::signed(oracle()),
			buyer(),
			1_000,
			ALPHA,
			true,
			None,
		));

		assert_eq!(Balances::free_balance(sudo), 9 * ALPHA);
		assert_eq!(Balances::free_balance(buyer()), 10 * ALPHA);
		assert_eq!(Balances::free_balance(bank()), ALPHA);
		assert_eq!(TotalUndistributedBacking::<Runtime>::get(), backing_before + ALPHA);
		assert_eq!(Credits::get_free_credits(&buyer()), 1_000);
		let batch = Batches::<Runtime>::get(batch_id).unwrap();
		assert!(batch.is_frozen);
		assert_eq!(batch.release_time, 1 + 15 * 28_800);
		let events = marketplace_events();
		assert_eq!(
			events.last(),
			Some(&MarketplaceEvent::BatchDeposited { owner: buyer(), batch_id })
		);
		assert!(!events.iter().any(|e| matches!(e, MarketplaceEvent::CreditsBought { .. })));
	});
}

#[test]
fn authority_deposit_without_sudo_is_still_best_effort() {
	new_test_ext().execute_with(|| {
		let batch_id = NextBatchId::<Runtime>::get();
		assert_ok!(Marketplace::deposit(
			RuntimeOrigin::signed(oracle()),
			buyer(),
			1_000,
			ALPHA,
			false,
			None,
		));
		assert_eq!(UnbackedBatchAlpha::<Runtime>::get(batch_id), ALPHA);
		assert_eq!(Balances::free_balance(bank()), 0);
		assert_eq!(Credits::get_free_credits(&buyer()), 1_000);
	});
}

#[test]
fn authority_deposit_works_while_buy_credits_is_off() {
	new_test_ext().execute_with(|| {
		assert_ok!(Marketplace::sudo_set_buy_credits_enabled(RuntimeOrigin::root(), false));
		AlphaPrice::<Runtime>::kill();
		assert_ok!(Marketplace::deposit(
			RuntimeOrigin::signed(oracle()),
			buyer(),
			1_000,
			0,
			false,
			None,
		));
		assert_eq!(Credits::get_free_credits(&buyer()), 1_000);
	});
}

#[test]
fn migration_stamps_an_existing_price_once() {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	sp_io::TestExternalities::new(t).execute_with(|| {
		System::set_block_number(500);
		// Pre-upgrade state: a price with no update record.
		AlphaPrice::<Runtime>::put(PRICE);
		assert_eq!(AlphaPriceUpdatedAt::<Runtime>::get(), None);

		InitAlphaPriceUpdatedAt::<Runtime>::on_runtime_upgrade();
		assert_eq!(AlphaPriceUpdatedAt::<Runtime>::get(), Some(500));

		// Idempotent: a later run leaves the record alone.
		System::set_block_number(900);
		InitAlphaPriceUpdatedAt::<Runtime>::on_runtime_upgrade();
		assert_eq!(AlphaPriceUpdatedAt::<Runtime>::get(), Some(500));
	});
}

#[test]
fn migration_leaves_an_unset_price_unset() {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	sp_io::TestExternalities::new(t).execute_with(|| {
		System::set_block_number(500);
		InitAlphaPriceUpdatedAt::<Runtime>::on_runtime_upgrade();
		assert_eq!(AlphaPriceUpdatedAt::<Runtime>::get(), None);
	});
}

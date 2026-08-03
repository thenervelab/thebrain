//! End-to-end tests for the Arion miner payment flow, run against the real
//! runtime: stats accrual → settlement → bank withdrawal → transfer → staking
//! bond, plus the marketplace deposit → bank revenue routing.

use frame_support::traits::{Currency, Hooks};
use hippius_mainnet_runtime::{
	AccountId, Arion, Balances, Bank, BlockNumber, Credits, Marketplace, Runtime, RuntimeEvent,
	RuntimeOrigin, System,
};
use pallet_arion::{
	ChildMinerUid, ChildRegistration, ChildRegistrations, ChildStatus, FamilyArrears,
	MinerAccruals, MinerStats, MinerStatsUpdate, SettlementSkipReason,
};
use sp_core::crypto::Ss58Codec;
use sp_runtime::{AccountId32, BuildStorage};

const UNIT: u128 = 1_000_000_000_000_000_000; // 18 decimals
const GIB: u128 = 1 << 30;
/// Must match `ArionSettlementInterval` in the runtime.
const SETTLEMENT_BLOCK: BlockNumber = 14_400;

/// The hardcoded `ArionAdminMembers` account (all arion/bank admin origins).
fn admin() -> AccountId {
	AccountId32::from_ss58check("5CVXqxb7mhFTtZVw5BJ8M2ujND9PFymSDxF8bkod6Sm4XJTW").unwrap()
}

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| System::set_block_number(1));
	ext
}

/// Register an active child miner directly in storage (bypasses the
/// signature/proxy/deposit machinery of `register_child`, which is not what
/// these tests exercise).
fn register_miner(family: &AccountId, child: &AccountId, uid: u32) {
	ChildRegistrations::<Runtime>::insert(
		child,
		ChildRegistration {
			family: family.clone(),
			node_id: [uid as u8; 32],
			status: ChildStatus::Active,
			deposit: 0u128,
			unbonding_end: 0u32.into(),
		},
	);
	ChildMinerUid::<Runtime>::insert(child, uid);
}

/// Submit miner stats for `uid` at the current block (bucket = block number).
fn submit_stats(uid: u32, bytes: u128) {
	let bucket: u32 = System::block_number().try_into().unwrap_or(u32::MAX);
	let updates = vec![MinerStatsUpdate {
		uid,
		stats: MinerStats { shard_data_bytes: bytes, ..Default::default() },
	}];
	Arion::submit_miner_stats(
		RuntimeOrigin::signed(admin()),
		bucket,
		updates.try_into().expect("bounded"),
		None,
	)
	.expect("stats submission");
}

/// Enable payments: price + token price feed + funded, whitelisted bank.
fn enable_payments(price_usd_per_gb_block: u128, alpha_price: u128, bank_funds: u128) {
	Arion::set_miner_price(RuntimeOrigin::signed(admin()), price_usd_per_gb_block)
		.expect("set price");
	pallet_credits::AlphaPrice::<Runtime>::put(alpha_price);
	let _ = Balances::deposit_creating(&Bank::account_id(), bank_funds);
	Bank::add_requester(RuntimeOrigin::signed(admin()), Arion::account_id())
		.expect("whitelist arion escrow");
}

fn settle_at(n: BlockNumber) {
	System::set_block_number(n);
	Arion::on_initialize(n);
}

fn staking_ledger_active(who: &AccountId) -> u128 {
	pallet_staking::Pallet::<Runtime>::ledger(sp_staking::StakingAccount::Stash(who.clone()))
		.expect("ledger exists")
		.active
}

#[test]
fn accrual_integrates_bytes_over_blocks() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		register_miner(&family, &child, 7);

		// First submission creates the accrual entry (nothing accrued yet).
		submit_stats(7, 100 * GIB);
		let acc = MinerAccruals::<Runtime>::get(7).expect("entry created");
		assert_eq!(acc.byte_blocks, 0);

		// Ten blocks later: the *previous* value is integrated over 10 blocks.
		System::set_block_number(11);
		submit_stats(7, 42);
		let acc = MinerAccruals::<Runtime>::get(7).expect("entry exists");
		assert_eq!(acc.byte_blocks, 100 * GIB * 10);
	});
}

#[test]
fn settlement_pays_family_and_bonds_stake() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		register_miner(&family, &child, 7);
		submit_stats(7, 100 * GIB); // stored from block 1 onwards

		let price = 10_000_000_000_u128; // $1e-8 per GiB per block
		let alpha_price = UNIT / 20; // token at $0.05
		enable_payments(price, alpha_price, 10 * UNIT);

		settle_at(SETTLEMENT_BLOCK);

		// Settlement accrues 100 GiB from block 1 to SETTLEMENT_BLOCK.
		let byte_blocks = 100 * GIB * (SETTLEMENT_BLOCK as u128 - 1);
		let expected = pallet_arion::tokens_for_byte_blocks(byte_blocks, price, alpha_price);
		assert!(expected > 0);

		// Paid in full and locked as bonded stake on the family account.
		assert_eq!(Balances::free_balance(&family), expected);
		assert_eq!(staking_ledger_active(&family), expected);
		assert_eq!(FamilyArrears::<Runtime>::get(&family), 0);
		// Accrual was reset by the settlement.
		assert_eq!(MinerAccruals::<Runtime>::get(7).expect("entry").byte_blocks, 0);
		// Bank accounting.
		assert_eq!(pallet_bank::TotalPaidOut::<Runtime>::get(), expected);
		assert_eq!(
			pallet_bank::TotalPaidByRequester::<Runtime>::get(Arion::account_id()),
			expected
		);

		let family_paid_ok = System::events().iter().any(|r| {
			matches!(
				&r.event,
				RuntimeEvent::Arion(pallet_arion::Event::FamilyPaid {
					family: f,
					tokens,
					staked: true,
				}) if *f == family && *tokens == expected
			)
		});
		assert!(family_paid_ok, "FamilyPaid {{ staked: true }} event expected");
	});
}

#[test]
fn shortfall_pays_pro_rata_and_carries_arrears() {
	new_test_ext().execute_with(|| {
		let (family_a, child_a) = (account(1), account(2));
		let (family_b, child_b) = (account(3), account(4));
		register_miner(&family_a, &child_a, 1);
		register_miner(&family_b, &child_b, 2);
		submit_stats(1, 200 * GIB);
		submit_stats(2, 100 * GIB);

		let price = 10_000_000_000_u128;
		let alpha_price = UNIT / 20;
		let elapsed = SETTLEMENT_BLOCK as u128 - 1;
		let due_a = pallet_arion::tokens_for_byte_blocks(200 * GIB * elapsed, price, alpha_price);
		let due_b = pallet_arion::tokens_for_byte_blocks(100 * GIB * elapsed, price, alpha_price);
		let total_due = due_a + due_b;

		// Fund the bank with only half of what is owed (+ ED it always keeps).
		let half = total_due / 2;
		enable_payments(price, alpha_price, half + 500);

		settle_at(SETTLEMENT_BLOCK);

		let paid_a = due_a * half / total_due;
		let paid_b = due_b * half / total_due;
		assert_eq!(Balances::free_balance(&family_a), paid_a);
		assert_eq!(Balances::free_balance(&family_b), paid_b);
		assert_eq!(FamilyArrears::<Runtime>::get(&family_a), due_a - paid_a);
		assert_eq!(FamilyArrears::<Runtime>::get(&family_b), due_b - paid_b);

		// Stop further accrual (previous bytes still accrue for one block).
		System::set_block_number(SETTLEMENT_BLOCK + 1);
		submit_stats(1, 0);
		submit_stats(2, 0);
		let extra_a = pallet_arion::tokens_for_byte_blocks(200 * GIB, price, alpha_price);
		let extra_b = pallet_arion::tokens_for_byte_blocks(100 * GIB, price, alpha_price);

		// Refill the bank: next settlement clears arrears (+ the one-block tail).
		let _ = Balances::deposit_creating(&Bank::account_id(), total_due);
		settle_at(2 * SETTLEMENT_BLOCK);

		assert_eq!(FamilyArrears::<Runtime>::get(&family_a), 0);
		assert_eq!(FamilyArrears::<Runtime>::get(&family_b), 0);
		assert_eq!(Balances::free_balance(&family_a), due_a + extra_a);
		assert_eq!(Balances::free_balance(&family_b), due_b + extra_b);
		assert_eq!(staking_ledger_active(&family_a), due_a + extra_a);
	});
}

#[test]
fn settlement_skips_when_price_or_feed_is_zero() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		register_miner(&family, &child, 7);
		submit_stats(7, GIB);

		// Price unset (0): skip, accruals untouched.
		settle_at(SETTLEMENT_BLOCK);
		let skipped_price = System::events().iter().any(|r| {
			matches!(
				&r.event,
				RuntimeEvent::Arion(pallet_arion::Event::MinerPaymentSkipped {
					reason: SettlementSkipReason::PriceUnset,
				})
			)
		});
		assert!(skipped_price);

		// Price set but token price feed at zero: skip as well.
		Arion::set_miner_price(RuntimeOrigin::signed(admin()), 1).expect("set price");
		pallet_credits::AlphaPrice::<Runtime>::put(0);
		settle_at(2 * SETTLEMENT_BLOCK);
		let skipped_feed = System::events().iter().any(|r| {
			matches!(
				&r.event,
				RuntimeEvent::Arion(pallet_arion::Event::MinerPaymentSkipped {
					reason: SettlementSkipReason::TokenPriceUnavailable,
				})
			)
		});
		assert!(skipped_feed);
		assert_eq!(Balances::free_balance(&family), 0);
	});
}

#[test]
fn extreme_values_saturate_without_panicking() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		register_miner(&family, &child, 7);
		submit_stats(7, u128::MAX);

		// Absurd price and cheap token: every intermediate would overflow u128.
		enable_payments(u128::MAX, 1, UNIT);

		// Must complete: saturating math end to end, payment capped by the bank.
		settle_at(SETTLEMENT_BLOCK);

		// Bank paid out everything it could (down to its ED)...
		assert_eq!(Balances::free_balance(&Bank::account_id()), 500);
		assert_eq!(Balances::free_balance(&family), UNIT - 500);
		// ...and the un-payable remainder is carried as (saturated) arrears.
		assert!(FamilyArrears::<Runtime>::get(&family) > 0);
	});
}

#[test]
fn marketplace_deposit_routes_alpha_backing_to_bank() {
	new_test_ext().execute_with(|| {
		let authority = account(10);
		let sudo = account(11);
		let user = account(12);
		Credits::add_authority(RuntimeOrigin::root(), authority.clone()).expect("add authority");
		pallet_marketplace::SudoKey::<Runtime>::put(Some(sudo.clone()));
		let _ = Balances::deposit_creating(&sudo, 100 * UNIT);

		Marketplace::deposit(
			RuntimeOrigin::signed(authority.clone()),
			user.clone(),
			5 * UNIT, // credits minted to the user
			3 * UNIT, // alpha backing routed to the bank
			false,
			None,
		)
		.expect("marketplace deposit");

		assert_eq!(Balances::free_balance(&Bank::account_id()), 3 * UNIT);
		assert_eq!(Balances::free_balance(&sudo), 97 * UNIT);
		assert_eq!(
			pallet_bank::TotalDeposited::<Runtime>::get(
				pallet_bank::DepositType::MarketplaceRevenue
			),
			3 * UNIT
		);

		// Without a sudo key the deposit itself still succeeds — the routing
		// is skipped, never blocking credit purchases.
		pallet_marketplace::SudoKey::<Runtime>::put(None::<AccountId>);
		Marketplace::deposit(
			RuntimeOrigin::signed(authority),
			account(13),
			5 * UNIT,
			3 * UNIT,
			false,
			None,
		)
		.expect("deposit without sudo");
		assert_eq!(Balances::free_balance(&Bank::account_id()), 3 * UNIT);
	});
}

#[test]
fn deregistration_forfeits_unpaid_accrual() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		register_miner(&family, &child, 7);
		submit_stats(7, 100 * GIB);
		System::set_block_number(11);

		// Explicit forfeiture (deregistration = loss, by design): the accrual
		// is dropped and the event carries the exact unpaid amount.
		Arion::clear_miner_uid_and_stats_for_child(&child);
		assert!(MinerAccruals::<Runtime>::get(7).is_none());
		let forfeited = System::events().iter().any(|r| {
			matches!(
				&r.event,
				RuntimeEvent::Arion(pallet_arion::Event::MinerAccrualForfeited {
					family: f,
					uid: 7,
					byte_blocks,
				}) if *f == family && *byte_blocks == 100 * GIB * 10
			)
		});
		assert!(forfeited, "MinerAccrualForfeited event with exact byte-blocks expected");

		// Settlement pays nothing afterwards: the uid mapping is gone.
		enable_payments(10_000_000_000, UNIT / 20, 10 * UNIT);
		settle_at(SETTLEMENT_BLOCK);
		assert_eq!(Balances::free_balance(&family), 0);
	});
}

#[test]
fn consumption_distributes_revenue_from_bank_single_sudo_debit() {
	new_test_ext().execute_with(|| {
		let authority = account(10);
		let sudo = account(11);
		let user = account(12);
		let ranking_pot = account(20);
		let marketplace_pot = account(21);
		Credits::add_authority(RuntimeOrigin::root(), authority.clone()).expect("add authority");
		pallet_marketplace::SudoKey::<Runtime>::put(Some(sudo.clone()));
		let _ = Balances::deposit_creating(&sudo, 100 * UNIT);

		// Deposit: 5 credits backed by 3 alpha — 100% routed to the bank.
		Marketplace::deposit(
			RuntimeOrigin::signed(authority),
			user.clone(),
			5 * UNIT,
			3 * UNIT,
			false,
			None,
		)
		.expect("marketplace deposit");
		assert_eq!(Balances::free_balance(&Bank::account_id()), 3 * UNIT);

		// The marketplace pallet account distributes from the bank — whitelist it.
		Bank::add_requester(RuntimeOrigin::signed(admin()), Marketplace::account_id())
			.expect("whitelist marketplace");

		// Consume 2 of the 5 credits → releases 2×3/5 = 1.2 alpha, paid by the
		// bank to the pots (70/30) — the sudo account is not touched again.
		Marketplace::consume_credits(
			user.clone(),
			2 * UNIT,
			marketplace_pot.clone(),
			ranking_pot.clone(),
		)
		.expect("consume credits");

		let released = 2 * UNIT * 3 / 5;
		let ranking_share = released * 70 / 100;
		let marketplace_share = released - ranking_share;
		assert_eq!(Balances::free_balance(&ranking_pot), ranking_share);
		assert_eq!(Balances::free_balance(&marketplace_pot), marketplace_share);
		// Sudo was debited exactly once, at deposit time.
		assert_eq!(Balances::free_balance(&sudo), 97 * UNIT);
		assert_eq!(Balances::free_balance(&Bank::account_id()), 3 * UNIT - released);
		assert_eq!(
			pallet_bank::TotalPaidByRequester::<Runtime>::get(Marketplace::account_id()),
			released
		);
	});
}

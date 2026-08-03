//! End-to-end tests for the Arion miner payment flow, run against the real
//! runtime: stats accrual → settlement → direct bank payout → staking bond,
//! plus the marketplace consumption → bank revenue routing.

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
fn deregistered_child_accrual_is_paid_at_next_settlement() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		register_miner(&family, &child, 7);
		submit_stats(7, 100 * GIB); // stored from block 1 onwards

		let price = 10_000_000_000_u128;
		let alpha_price = UNIT / 20;
		enable_payments(price, alpha_price, 1_000_000 * UNIT);

		// The family deregisters its child mid-window: service earned up to
		// this point must survive losing the uid mapping and stats pruning.
		System::set_block_number(7_200);
		Arion::deregister_child(RuntimeOrigin::signed(family.clone()), child)
			.expect("deregister child");

		// A stats-pruning tick runs long before the next settlement.
		System::set_block_number(7_300);
		Arion::on_initialize(7_300);

		settle_at(SETTLEMENT_BLOCK);

		// 100 GiB held from block 1 to deregistration at block 7200.
		let expected =
			pallet_arion::tokens_for_byte_blocks(100 * GIB * 7_199, price, alpha_price);
		assert!(expected > 0);
		assert_eq!(Balances::free_balance(&family), expected);
		assert_eq!(staking_ledger_active(&family), expected);
	});
}

#[test]
fn failed_family_payout_is_not_double_pulled_from_bank() {
	new_test_ext().execute_with(|| {
		let (family_a, child_a) = (account(1), account(2));
		let (family_b, child_b) = (account(3), account(4));
		register_miner(&family_a, &child_a, 1);
		register_miner(&family_b, &child_b, 2);
		submit_stats(1, 100 * GIB);
		submit_stats(2, 1); // one byte: due lands below the existential deposit

		let price = 1_000_000_u128;
		let alpha_price = UNIT / 20;
		let elapsed = SETTLEMENT_BLOCK as u128 - 1;
		let due_a = pallet_arion::tokens_for_byte_blocks(100 * GIB * elapsed, price, alpha_price);
		let due_b = pallet_arion::tokens_for_byte_blocks(elapsed, price, alpha_price);
		// A's one-block accrual tail between the two settlements below.
		let extra_a = pallet_arion::tokens_for_byte_blocks(100 * GIB, price, alpha_price);
		let ed = Balances::minimum_balance();
		assert!(due_a > ed, "family A payout must be deliverable");
		assert!(due_b > 0 && due_b < ed, "family B payout must be undeliverable (below ED)");

		enable_payments(price, alpha_price, due_a + due_b + extra_a + 10 * ed);
		let bank_before = Balances::free_balance(&Bank::account_id());

		settle_at(SETTLEMENT_BLOCK);

		// B's payout cannot be delivered (below the ED of a fresh account):
		// the debt is carried as arrears and the tokens never leave the bank —
		// nothing may be stranded on (or burned from) the arion escrow.
		assert_eq!(Balances::free_balance(&family_b), 0);
		assert_eq!(FamilyArrears::<Runtime>::get(&family_b), due_b);
		assert_eq!(Balances::free_balance(&Arion::account_id()), 0);
		assert_eq!(Balances::free_balance(&Bank::account_id()), bank_before - due_a);

		// Stop further accrual, then settle again: retrying B's arrears must
		// not debit the bank for amounts it already released.
		System::set_block_number(SETTLEMENT_BLOCK + 1);
		submit_stats(1, 0);
		submit_stats(2, 0);

		settle_at(2 * SETTLEMENT_BLOCK);

		assert_eq!(
			Balances::free_balance(&Bank::account_id()),
			bank_before - due_a - extra_a,
			"bank must only be debited for delivered payouts"
		);
		assert_eq!(Balances::free_balance(&Arion::account_id()), 0);
		assert_eq!(FamilyArrears::<Runtime>::get(&family_b), due_b);
	});
}

#[test]
fn consumed_alpha_routes_bank_share_once() {
	new_test_ext().execute_with(|| {
		let authority = account(10);
		let sudo = account(11);
		let user = account(12);
		let ranking = account(14);
		let marketplace = account(15);
		Credits::add_authority(RuntimeOrigin::root(), authority.clone()).expect("add authority");
		pallet_marketplace::SudoKey::<Runtime>::put(Some(sudo.clone()));
		let _ = Balances::deposit_creating(&sudo, 100 * UNIT);

		Marketplace::deposit(
			RuntimeOrigin::signed(authority),
			user.clone(),
			5 * UNIT, // credits minted to the user
			3 * UNIT, // alpha backing
			false,
			None,
		)
		.expect("marketplace deposit");

		// Depositing only records the batch: real funds leave the sudo pot
		// once, when the credits are consumed.
		assert_eq!(Balances::free_balance(&sudo), 100 * UNIT);
		assert_eq!(Balances::free_balance(&Bank::account_id()), 0);

		Marketplace::consume_credits(user, 5 * UNIT, marketplace.clone(), ranking.clone())
			.expect("consume credits");

		// Single sudo debit for the full backing, split 30% to the bank and
		// the remainder 70/30 between ranking and marketplace.
		let bank_share = 3 * UNIT * 30 / 100;
		let remainder = 3 * UNIT - bank_share;
		let ranking_share = remainder * 70 / 100;
		let marketplace_share = remainder - ranking_share;
		assert_eq!(Balances::free_balance(&Bank::account_id()), bank_share);
		assert_eq!(Balances::free_balance(&ranking), ranking_share);
		assert_eq!(Balances::free_balance(&marketplace), marketplace_share);
		assert_eq!(Balances::free_balance(&sudo), 97 * UNIT);
		assert_eq!(
			pallet_bank::TotalDeposited::<Runtime>::get(
				pallet_bank::DepositType::MarketplaceRevenue
			),
			bank_share
		);
	});
}

#[test]
fn consume_without_sudo_key_skips_revenue_routing() {
	new_test_ext().execute_with(|| {
		let authority = account(10);
		let user = account(13);
		Credits::add_authority(RuntimeOrigin::root(), authority.clone()).expect("add authority");
		pallet_marketplace::SudoKey::<Runtime>::put(None::<AccountId>);

		Marketplace::deposit(
			RuntimeOrigin::signed(authority),
			user.clone(),
			5 * UNIT,
			3 * UNIT,
			false,
			None,
		)
		.expect("deposit without sudo");

		// Without a sudo key billing still succeeds — every revenue transfer
		// (bank, ranking, marketplace) is skipped rather than blocking it.
		Marketplace::consume_credits(user, 5 * UNIT, account(15), account(14))
			.expect("consume without sudo");
		assert_eq!(Balances::free_balance(&Bank::account_id()), 0);
	});
}

//! End-to-end tests for the Arion miner payment flow, run against the real
//! runtime: stats accrual → settlement → bank withdrawal → transfer → staking
//! bond, plus the marketplace deposit → bank revenue routing.

use frame_support::traits::{Currency, Hooks, OnRuntimeUpgrade};
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
	pallet_arion::MinerUidToChild::<Runtime>::insert(uid, child.clone());
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

/// Expected payout for raw byte-blocks — the same shared money math the
/// pallet settles with (formerly `pallet_arion::tokens_for_byte_blocks`).
fn tokens_for(byte_blocks: u128, price: u128, token_price: u128) -> u128 {
	payment_math::tokens_for(
		payment_math::ByteBlocks::new(byte_blocks),
		payment_math::UsdPerGibBlock::new(price),
		payment_math::Usd::new(token_price),
	)
	.get()
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
		let expected = tokens_for(byte_blocks, price, alpha_price);
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
		let due_a = tokens_for(200 * GIB * elapsed, price, alpha_price);
		let due_b = tokens_for(100 * GIB * elapsed, price, alpha_price);
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
		let extra_a = tokens_for(200 * GIB, price, alpha_price);
		let extra_b = tokens_for(100 * GIB, price, alpha_price);

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
		assert!(pallet_arion::MinerUidToChild::<Runtime>::get(7).is_none());
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

#[test]
fn failed_deposit_routing_creates_distribution_arrears_then_retries() {
	new_test_ext().execute_with(|| {
		let authority = account(10);
		let sudo = account(11); // set but unfunded: deposit routing fails
		let user = account(12);
		let funder = account(30);
		let ranking_pot = account(20);
		let marketplace_pot = account(21);
		Credits::add_authority(RuntimeOrigin::root(), authority.clone()).expect("add authority");
		pallet_marketplace::SudoKey::<Runtime>::put(Some(sudo.clone()));
		let _ = Balances::deposit_creating(&sudo, 1_000);
		let _ = Balances::deposit_creating(&funder, 100 * UNIT);
		Bank::add_requester(RuntimeOrigin::signed(admin()), Marketplace::account_id())
			.expect("whitelist marketplace");

		// Routing fails (sudo unfunded): deposit still succeeds, nothing is
		// counted as backing, the bank is empty.
		Marketplace::deposit(
			RuntimeOrigin::signed(authority),
			user.clone(),
			5 * UNIT,
			3 * UNIT,
			false,
			None,
		)
		.expect("deposit succeeds despite failed routing");
		assert_eq!(pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(), 0);
		assert_eq!(Balances::free_balance(&Bank::account_id()), 0);

		// Billing still succeeds; the pots get nothing — but the shortfall is
		// carried as arrears instead of silently discarded.
		Marketplace::consume_credits(
			user.clone(),
			2 * UNIT,
			marketplace_pot.clone(),
			ranking_pot.clone(),
		)
		.expect("billing succeeds");
		let released_1 = 2 * UNIT * 3 / 5;
		let ranking_owed_1 = released_1 * 70 / 100;
		let marketplace_owed_1 = released_1 - ranking_owed_1;
		assert_eq!(Balances::free_balance(&ranking_pot), 0);
		assert_eq!(
			pallet_marketplace::DistributionArrears::<Runtime>::get(&ranking_pot),
			ranking_owed_1
		);
		assert_eq!(
			pallet_marketplace::DistributionArrears::<Runtime>::get(&marketplace_pot),
			marketplace_owed_1
		);

		// Refill the bank; the next distribution pays arrears + new share.
		Bank::deposit(RuntimeOrigin::signed(funder), 10 * UNIT, pallet_bank::DepositType::Grant)
			.expect("refill");
		Marketplace::consume_credits(
			user.clone(),
			1 * UNIT,
			marketplace_pot.clone(),
			ranking_pot.clone(),
		)
		.expect("second consume");
		let released_2 = 1 * UNIT * 3 / 5;
		let total_released = released_1 + released_2;
		let ranking_total = ranking_owed_1 + released_2 * 70 / 100;
		let marketplace_total = total_released - ranking_total;
		assert_eq!(Balances::free_balance(&ranking_pot), ranking_total);
		assert_eq!(Balances::free_balance(&marketplace_pot), marketplace_total);
		assert_eq!(pallet_marketplace::DistributionArrears::<Runtime>::get(&ranking_pot), 0);
		assert_eq!(pallet_marketplace::DistributionArrears::<Runtime>::get(&marketplace_pot), 0);
	});
}

#[test]
fn miner_settlement_cannot_drain_pot_backing() {
	new_test_ext().execute_with(|| {
		let authority = account(10);
		let sudo = account(11);
		let user = account(12);
		let funder = account(30);
		let ranking_pot = account(20);
		let marketplace_pot = account(21);
		let (family, child) = (account(1), account(2));
		Credits::add_authority(RuntimeOrigin::root(), authority.clone()).expect("add authority");
		pallet_marketplace::SudoKey::<Runtime>::put(Some(sudo.clone()));
		let _ = Balances::deposit_creating(&sudo, 100 * UNIT);
		let _ = Balances::deposit_creating(&funder, 100 * UNIT);
		register_miner(&family, &child, 7);
		submit_stats(7, 100 * GIB);

		// 3 UNIT of pot backing + 2 UNIT of miner budget in the bank.
		Marketplace::deposit(
			RuntimeOrigin::signed(authority),
			user.clone(),
			5 * UNIT,
			3 * UNIT,
			false,
			None,
		)
		.expect("deposit");
		Bank::deposit(RuntimeOrigin::signed(funder), 2 * UNIT, pallet_bank::DepositType::Grant)
			.expect("miner budget");
		Bank::add_requester(RuntimeOrigin::signed(admin()), Marketplace::account_id())
			.expect("whitelist marketplace");

		// Miner due far above the bank balance: even a runaway due can only
		// take the miner budget, never the backing owed to the pots.
		enable_payments(1_000_000_000_000, UNIT / 20, 0);
		settle_at(SETTLEMENT_BLOCK);
		assert_eq!(Balances::free_balance(&family), 2 * UNIT - 500);
		assert_eq!(Balances::free_balance(&Bank::account_id()), 3 * UNIT + 500);
		assert!(pallet_arion::FamilyArrears::<Runtime>::get(&family) > 0);

		// The pots are paid in full from the protected backing.
		Marketplace::consume_credits(
			user.clone(),
			2 * UNIT,
			marketplace_pot.clone(),
			ranking_pot.clone(),
		)
		.expect("consume");
		let released = 2 * UNIT * 3 / 5;
		assert_eq!(Balances::free_balance(&ranking_pot), released * 70 / 100);
		assert_eq!(Balances::free_balance(&marketplace_pot), released - released * 70 / 100);
	});
}

#[test]
fn migration_builds_uid_reverse_index_and_drops_duplicates() {
	new_test_ext().execute_with(|| {
		use frame_support::traits::{GetStorageVersion, StorageVersion};
		let (child_a, child_b, child_c) = (account(1), account(2), account(3));
		// Pre-migration state: forward mappings only, uid 7 claimed twice.
		ChildMinerUid::<Runtime>::insert(&child_a, 7u32);
		ChildMinerUid::<Runtime>::insert(&child_b, 7u32);
		ChildMinerUid::<Runtime>::insert(&child_c, 9u32);

		<Arion as OnRuntimeUpgrade>::on_runtime_upgrade();

		// Reverse index built; uid 9 is unambiguous.
		assert_eq!(pallet_arion::MinerUidToChild::<Runtime>::get(9), Some(child_c));
		// Exactly one claimant kept uid 7; the duplicate lost its mapping.
		let winner = pallet_arion::MinerUidToChild::<Runtime>::get(7).expect("one winner");
		let a_kept = ChildMinerUid::<Runtime>::get(&child_a).is_some();
		let b_kept = ChildMinerUid::<Runtime>::get(&child_b).is_some();
		assert!(a_kept ^ b_kept, "exactly one forward mapping survives");
		assert_eq!(ChildMinerUid::<Runtime>::get(&winner), Some(7));
		// Version bumped: running again is a no-op.
		assert_eq!(
			<pallet_arion::Pallet<Runtime> as GetStorageVersion>::on_chain_storage_version(),
			StorageVersion::new(1)
		);
	});
}

#[test]
fn chargeback_refunds_backing_from_bank_to_sudo() {
	new_test_ext().execute_with(|| {
		let authority = account(10);
		let sudo = account(11);
		let user = account(12);
		Credits::add_authority(RuntimeOrigin::root(), authority.clone()).expect("add authority");
		pallet_marketplace::SudoKey::<Runtime>::put(Some(sudo.clone()));
		let _ = Balances::deposit_creating(&sudo, 100 * UNIT);
		Bank::add_requester(RuntimeOrigin::signed(admin()), Marketplace::account_id())
			.expect("whitelist marketplace");

		let batch_id = pallet_marketplace::NextBatchId::<Runtime>::get();
		// Frozen deposit (chargeback window open): backing goes to the bank.
		Marketplace::deposit(
			RuntimeOrigin::signed(authority),
			user.clone(),
			5 * UNIT,
			3 * UNIT,
			true,
			None,
		)
		.expect("frozen deposit");
		assert_eq!(Balances::free_balance(&Bank::account_id()), 3 * UNIT);
		assert_eq!(pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(), 3 * UNIT);

		// Chargeback: the reversed backing is refunded to sudo (minus the ED
		// the bank always keeps) and no longer counted as owed to the pots.
		Marketplace::chargeback(RuntimeOrigin::root(), batch_id).expect("chargeback");
		assert_eq!(Balances::free_balance(&sudo), 100 * UNIT - 500);
		assert_eq!(Balances::free_balance(&Bank::account_id()), 500);
		assert_eq!(pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(), 0);
		// The user's credits from the reversed batch were burned.
		assert_eq!(Credits::get_free_credits(&user), 0);
	});
}

#[test]
fn unbacked_batch_release_does_not_reduce_backed_ledger() {
	new_test_ext().execute_with(|| {
		let authority = account(10);
		let sudo = account(11);
		let (user_a, user_b) = (account(12), account(13));
		let ranking_pot = account(20);
		let marketplace_pot = account(21);
		Credits::add_authority(RuntimeOrigin::root(), authority.clone()).expect("add authority");
		pallet_marketplace::SudoKey::<Runtime>::put(Some(sudo.clone()));
		let _ = Balances::deposit_creating(&sudo, 100 * UNIT);
		Bank::add_requester(RuntimeOrigin::signed(admin()), Marketplace::account_id())
			.expect("whitelist marketplace");

		// Batch A is backed: its 3 UNIT of alpha backing reach the bank.
		Marketplace::deposit(
			RuntimeOrigin::signed(authority.clone()),
			user_a.clone(),
			5 * UNIT,
			3 * UNIT,
			false,
			None,
		)
		.expect("backed deposit");
		assert_eq!(pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(), 3 * UNIT);

		// Batch B is unbacked: no sudo key, nothing reaches the bank.
		pallet_marketplace::SudoKey::<Runtime>::kill();
		let batch_b = pallet_marketplace::NextBatchId::<Runtime>::get();
		Marketplace::deposit(
			RuntimeOrigin::signed(authority),
			user_b.clone(),
			5 * UNIT,
			2 * UNIT,
			false,
			None,
		)
		.expect("unbacked deposit");
		assert_eq!(pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(), 3 * UNIT);
		assert_eq!(pallet_marketplace::UnbackedBatchAlpha::<Runtime>::get(batch_b), 2 * UNIT);

		// Releasing batch B pays the pots from the shared pool, but must not
		// reduce the ledger of backing owed for batch A — batch B contributed
		// nothing to it.
		Marketplace::consume_credits(
			user_b.clone(),
			5 * UNIT,
			marketplace_pot.clone(),
			ranking_pot.clone(),
		)
		.expect("consume unbacked batch");
		let released = 2 * UNIT;
		assert_eq!(
			Balances::free_balance(&ranking_pot) + Balances::free_balance(&marketplace_pot),
			released
		);
		assert_eq!(pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(), 3 * UNIT);
		assert_eq!(pallet_marketplace::UnbackedBatchAlpha::<Runtime>::get(batch_b), 0);
	});
}

#[test]
fn chargeback_bank_shortfall_keeps_refund_pending_and_retries() {
	new_test_ext().execute_with(|| {
		let authority = account(10);
		let sudo = account(11);
		let user = account(12);
		Credits::add_authority(RuntimeOrigin::root(), authority.clone()).expect("add authority");
		pallet_marketplace::SudoKey::<Runtime>::put(Some(sudo.clone()));
		let _ = Balances::deposit_creating(&sudo, 100 * UNIT);
		Bank::add_requester(RuntimeOrigin::signed(admin()), Marketplace::account_id())
			.expect("whitelist marketplace");

		// First chargeback: the bank keeps its ED, so 500 of the refund cannot
		// be delivered — it must stay tracked, not silently vanish.
		let batch_1 = pallet_marketplace::NextBatchId::<Runtime>::get();
		Marketplace::deposit(
			RuntimeOrigin::signed(authority.clone()),
			user.clone(),
			5 * UNIT,
			3 * UNIT,
			true,
			None,
		)
		.expect("frozen deposit 1");
		Marketplace::chargeback(RuntimeOrigin::root(), batch_1).expect("chargeback 1");
		assert_eq!(pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(), 0);
		assert_eq!(pallet_marketplace::PendingSudoRefunds::<Runtime>::get(), 500);
		assert_eq!(Balances::free_balance(&Bank::account_id()), 500);

		// Second chargeback folds the pending remainder into its own refund.
		let batch_2 = pallet_marketplace::NextBatchId::<Runtime>::get();
		Marketplace::deposit(
			RuntimeOrigin::signed(authority),
			user.clone(),
			5 * UNIT,
			2 * UNIT,
			true,
			None,
		)
		.expect("frozen deposit 2");
		Marketplace::chargeback(RuntimeOrigin::root(), batch_2).expect("chargeback 2");
		// Owed 2 UNIT + 500 pending; the bank can deliver everything but its ED.
		assert_eq!(pallet_marketplace::PendingSudoRefunds::<Runtime>::get(), 500);
		assert_eq!(pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(), 0);
		assert_eq!(Balances::free_balance(&sudo), 100 * UNIT - 500);
	});
}

#[test]
fn chargeback_without_sudo_key_walls_refund_from_miners() {
	new_test_ext().execute_with(|| {
		let authority = account(10);
		let sudo = account(11);
		let user = account(12);
		let (family, child) = (account(1), account(2));
		Credits::add_authority(RuntimeOrigin::root(), authority.clone()).expect("add authority");
		pallet_marketplace::SudoKey::<Runtime>::put(Some(sudo.clone()));
		let _ = Balances::deposit_creating(&sudo, 100 * UNIT);
		Bank::add_requester(RuntimeOrigin::signed(admin()), Marketplace::account_id())
			.expect("whitelist marketplace");
		register_miner(&family, &child, 7);
		submit_stats(7, 100 * GIB);

		let batch_id = pallet_marketplace::NextBatchId::<Runtime>::get();
		Marketplace::deposit(
			RuntimeOrigin::signed(authority),
			user.clone(),
			5 * UNIT,
			3 * UNIT,
			true,
			None,
		)
		.expect("frozen deposit");

		// Sudo key removed before the chargeback: the refund cannot be
		// delivered, so the full backing stays in the bank, tracked as a
		// pending refund.
		pallet_marketplace::SudoKey::<Runtime>::kill();
		Marketplace::chargeback(RuntimeOrigin::root(), batch_id).expect("chargeback");
		assert_eq!(pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(), 0);
		assert_eq!(pallet_marketplace::PendingSudoRefunds::<Runtime>::get(), 3 * UNIT);
		assert_eq!(Balances::free_balance(&Bank::account_id()), 3 * UNIT);
		assert_eq!(Balances::free_balance(&sudo), 100 * UNIT - 3 * UNIT);

		// The pending refund is walled off from miner settlement exactly like
		// undistributed backing: a runaway miner due must not consume it.
		enable_payments(1_000_000_000_000, UNIT / 20, 0);
		settle_at(SETTLEMENT_BLOCK);
		assert_eq!(Balances::free_balance(&family), 0);
		assert!(FamilyArrears::<Runtime>::get(&family) > 0);
		assert_eq!(Balances::free_balance(&Bank::account_id()), 3 * UNIT);
	});
}

#[test]
fn activation_migration_whitelists_and_seeds_backing() {
	new_test_ext().execute_with(|| {
		let authority = account(10);
		let sudo = account(11);
		let user = account(12);
		Credits::add_authority(RuntimeOrigin::root(), authority.clone()).expect("add authority");

		// Pre-upgrade state: batches exist but their backing never reached the
		// bank (the old runtime had no routing). Deposit with no sudo key,
		// then wipe the unbacked markers the new runtime writes — pre-upgrade
		// code wrote neither markers nor ledger entries.
		Marketplace::deposit(
			RuntimeOrigin::signed(authority.clone()),
			user.clone(),
			5 * UNIT,
			3 * UNIT,
			false,
			None,
		)
		.expect("pre-upgrade deposit");
		Marketplace::deposit(
			RuntimeOrigin::signed(authority),
			user.clone(),
			5 * UNIT,
			2 * UNIT,
			true,
			None,
		)
		.expect("pre-upgrade frozen deposit");
		let _ = pallet_marketplace::UnbackedBatchAlpha::<Runtime>::clear(u32::MAX, None);
		pallet_marketplace::SudoKey::<Runtime>::put(Some(sudo.clone()));
		let _ = Balances::deposit_creating(&sudo, 100 * UNIT);

		hippius_mainnet_runtime::migrations::ActivateMinerPaymentBank::<Runtime>::on_runtime_upgrade();

		// Both pallet accounts are whitelisted as bank requesters.
		assert!(pallet_bank::WhitelistedRequesters::<Runtime>::contains_key(Arion::account_id()));
		assert!(pallet_bank::WhitelistedRequesters::<Runtime>::contains_key(
			Marketplace::account_id()
		));
		// The outstanding backing (remaining + pending across batches) moved
		// from sudo to the bank and is counted as owed to the pots.
		assert_eq!(pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(), 5 * UNIT);
		assert_eq!(Balances::free_balance(&Bank::account_id()), 5 * UNIT);
		assert_eq!(Balances::free_balance(&sudo), 95 * UNIT);

		// Idempotent: a second run (next runtime upgrade) must not seed again.
		hippius_mainnet_runtime::migrations::ActivateMinerPaymentBank::<Runtime>::on_runtime_upgrade();
		assert_eq!(pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(), 5 * UNIT);
		assert_eq!(Balances::free_balance(&Bank::account_id()), 5 * UNIT);
	});
}

#[test]
fn activation_migration_marks_batches_unbacked_when_sudo_cannot_seed() {
	new_test_ext().execute_with(|| {
		let authority = account(10);
		let user = account(12);
		Credits::add_authority(RuntimeOrigin::root(), authority.clone()).expect("add authority");

		let batch_1 = pallet_marketplace::NextBatchId::<Runtime>::get();
		Marketplace::deposit(
			RuntimeOrigin::signed(authority.clone()),
			user.clone(),
			5 * UNIT,
			3 * UNIT,
			false,
			None,
		)
		.expect("pre-upgrade deposit");
		let batch_2 = pallet_marketplace::NextBatchId::<Runtime>::get();
		Marketplace::deposit(
			RuntimeOrigin::signed(authority),
			user.clone(),
			5 * UNIT,
			2 * UNIT,
			false,
			None,
		)
		.expect("pre-upgrade deposit 2");
		let _ = pallet_marketplace::UnbackedBatchAlpha::<Runtime>::clear(u32::MAX, None);
		// No sudo key: the seed cannot be transferred.

		hippius_mainnet_runtime::migrations::ActivateMinerPaymentBank::<Runtime>::on_runtime_upgrade();

		// Whitelisting still happens; the un-seedable backing is not counted
		// as owed (nothing reached the bank) — instead every batch is marked
		// unbacked so later releases keep the ledger conservative.
		assert!(pallet_bank::WhitelistedRequesters::<Runtime>::contains_key(Arion::account_id()));
		assert_eq!(pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(), 0);
		assert_eq!(Balances::free_balance(&Bank::account_id()), 0);
		assert_eq!(pallet_marketplace::UnbackedBatchAlpha::<Runtime>::get(batch_1), 3 * UNIT);
		assert_eq!(pallet_marketplace::UnbackedBatchAlpha::<Runtime>::get(batch_2), 2 * UNIT);
	});
}

#[test]
fn stats_for_unclaimed_uid_do_not_accrue() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));

		// Uid 99 has no registered claimant: storing its stats must not open
		// an accrual nothing can ever settle or forfeit.
		submit_stats(99, 100 * GIB);
		assert!(!MinerAccruals::<Runtime>::contains_key(99));

		// Once a child claims the uid, accrual starts from that point.
		register_miner(&family, &child, 99);
		submit_stats(99, 100 * GIB);
		assert!(MinerAccruals::<Runtime>::contains_key(99));
	});
}

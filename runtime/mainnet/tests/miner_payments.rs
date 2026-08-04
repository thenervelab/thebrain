//! End-to-end tests for the Arion miner payment flow, run against the real
//! runtime: stats accrual → settlement → bank withdrawal → transfer → staking
//! bond, plus the marketplace deposit → bank revenue routing.

use frame_support::traits::{Currency, Hooks, OnRuntimeUpgrade};
use hippius_mainnet_runtime::{
	AccountId, Arion, Balances, Hippocampus, BlockNumber, Credits, Marketplace, Runtime, RuntimeEvent,
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
	let _ = Balances::deposit_creating(&Hippocampus::account_id(), bank_funds);
	Hippocampus::add_requester(RuntimeOrigin::signed(admin()), Arion::account_id())
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
		// Hippocampus accounting.
		assert_eq!(pallet_hippocampus::TotalPaidOut::<Runtime>::get(), expected);
		assert_eq!(
			pallet_hippocampus::TotalPaidByRequester::<Runtime>::get(Arion::account_id()),
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
		let _ = Balances::deposit_creating(&Hippocampus::account_id(), total_due);
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

		// Hippocampus paid out everything it could (down to its ED)...
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), 500);
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

		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), 3 * UNIT);
		assert_eq!(Balances::free_balance(&sudo), 97 * UNIT);
		assert_eq!(
			pallet_hippocampus::TotalDeposited::<Runtime>::get(
				pallet_hippocampus::DepositType::MarketplaceRevenue
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
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), 3 * UNIT);
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
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), 3 * UNIT);

		// The marketplace pallet account distributes from the bank — whitelist it.
		Hippocampus::add_requester(RuntimeOrigin::signed(admin()), Marketplace::account_id())
			.expect("whitelist marketplace");

		// Consume 2 of the 5 credits → releases 2×3/5 = 1.2 alpha, stays in bank
		// (no distribution to pots anymore)
		Marketplace::consume_credits(
			user.clone(),
			2 * UNIT,
		)
		.expect("consume credits");

		// Alpha stays in bank, pots get nothing
		assert_eq!(Balances::free_balance(&ranking_pot), 0);
		assert_eq!(Balances::free_balance(&marketplace_pot), 0);
		// Sudo was debited exactly once, at deposit time.
		assert_eq!(Balances::free_balance(&sudo), 97 * UNIT);
		// Bank holds the full amount released
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), 3 * UNIT);
		// No payment was made (distribution removed)
		assert_eq!(
			pallet_hippocampus::TotalPaidByRequester::<Runtime>::get(Marketplace::account_id()),
			0
		);
	});
}

#[test]
fn failed_deposit_routing_leaves_alpha_unbacked_and_out_of_the_ledger() {
	new_test_ext().execute_with(|| {
		let authority = account(10);
		let sudo = account(11); // set but unfunded: deposit routing fails
		let user = account(12);
		let funder = account(30);
		Credits::add_authority(RuntimeOrigin::root(), authority.clone()).expect("add authority");
		pallet_marketplace::SudoKey::<Runtime>::put(Some(sudo.clone()));
		let _ = Balances::deposit_creating(&sudo, 1_000);
		let _ = Balances::deposit_creating(&funder, 100 * UNIT);

		// Routing fails (sudo unfunded): the deposit still succeeds, but the
		// backing never reaches the bank so it must not enter the ledger.
		let batch_id = pallet_marketplace::NextBatchId::<Runtime>::get();
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
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), 0);
		assert_eq!(pallet_marketplace::UnbackedBatchAlpha::<Runtime>::get(batch_id), 3 * UNIT);

		// A grant arrives that is genuinely the miner budget.
		Hippocampus::deposit(
			RuntimeOrigin::signed(funder),
			20 * UNIT,
			pallet_hippocampus::DepositType::Grant,
		)
		.expect("grant");

		// Consuming unbacked alpha must not decrement the ledger — there was
		// never any backing behind it — and must not touch the grant.
		Marketplace::consume_credits(user.clone(), 2 * UNIT).expect("billing succeeds");
		assert_eq!(
			pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(),
			0,
			"unbacked release cannot drive the ledger negative"
		);
		assert_eq!(
			Balances::free_balance(&Hippocampus::account_id()),
			20 * UNIT,
			"the grant is untouched by an unbacked release"
		);
		assert!(
			pallet_marketplace::UnbackedBatchAlpha::<Runtime>::get(batch_id) < 3 * UNIT,
			"the unbacked marker is drawn down as the alpha is consumed"
		);
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
		Hippocampus::deposit(RuntimeOrigin::signed(funder), 2 * UNIT, pallet_hippocampus::DepositType::Grant)
			.expect("miner budget");
		Hippocampus::add_requester(RuntimeOrigin::signed(admin()), Marketplace::account_id())
			.expect("whitelist marketplace");

		// Miner due far above the bank balance: even a runaway due can only
		// take the miner budget, never the backing owed to the pots.
		enable_payments(1_000_000_000_000, UNIT / 20, 0);
		settle_at(SETTLEMENT_BLOCK);
		assert_eq!(Balances::free_balance(&family), 2 * UNIT - 500);
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), 3 * UNIT + 500);
		assert!(pallet_arion::FamilyArrears::<Runtime>::get(&family) > 0);

		// Alpha stays in bank, pots get nothing (distribution removed).
		Marketplace::consume_credits(
			user.clone(),
			2 * UNIT,
		)
		.expect("consume");
		assert_eq!(Balances::free_balance(&ranking_pot), 0);
		assert_eq!(Balances::free_balance(&marketplace_pot), 0);
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
		Hippocampus::add_requester(RuntimeOrigin::signed(admin()), Marketplace::account_id())
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
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), 3 * UNIT);
		assert_eq!(pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(), 3 * UNIT);

		// Chargeback: the reversed backing is refunded to sudo (minus the ED
		// the bank always keeps) and no longer counted as owed to the pots.
		Marketplace::chargeback(RuntimeOrigin::root(), batch_id).expect("chargeback");
		assert_eq!(Balances::free_balance(&sudo), 100 * UNIT - 500);
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), 500);
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
		Hippocampus::add_requester(RuntimeOrigin::signed(admin()), Marketplace::account_id())
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

		// Releasing batch B consumes only its own unbacked marker: batch A's
		// backing must stay in the ledger and in the bank.
		Marketplace::consume_credits(user_b.clone(), 5 * UNIT).expect("consume unbacked batch");
		assert_eq!(
			Balances::free_balance(&ranking_pot) + Balances::free_balance(&marketplace_pot),
			0,
			"pots receive nothing — the marketplace no longer distributes"
		);
		assert_eq!(
			pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(),
			3 * UNIT,
			"batch A's backing is untouched by batch B's unbacked release"
		);
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
		Hippocampus::add_requester(RuntimeOrigin::signed(admin()), Marketplace::account_id())
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
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), 500);

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
		Hippocampus::add_requester(RuntimeOrigin::signed(admin()), Marketplace::account_id())
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
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), 3 * UNIT);
		assert_eq!(Balances::free_balance(&sudo), 100 * UNIT - 3 * UNIT);

		// The pending refund is walled off from miner settlement exactly like
		// undistributed backing: a runaway miner due must not consume it.
		enable_payments(1_000_000_000_000, UNIT / 20, 0);
		settle_at(SETTLEMENT_BLOCK);
		assert_eq!(Balances::free_balance(&family), 0);
		assert!(FamilyArrears::<Runtime>::get(&family) > 0);
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), 3 * UNIT);
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
		assert!(pallet_hippocampus::WhitelistedRequesters::<Runtime>::contains_key(Arion::account_id()));
		assert!(pallet_hippocampus::WhitelistedRequesters::<Runtime>::contains_key(
			Marketplace::account_id()
		));
		// The outstanding backing (remaining + pending across batches) moved
		// from sudo to the bank and is counted as owed to the pots.
		assert_eq!(pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(), 5 * UNIT);
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), 5 * UNIT);
		assert_eq!(Balances::free_balance(&sudo), 95 * UNIT);

		// Idempotent: a second run (next runtime upgrade) must not seed again.
		hippius_mainnet_runtime::migrations::ActivateMinerPaymentBank::<Runtime>::on_runtime_upgrade();
		assert_eq!(pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(), 5 * UNIT);
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), 5 * UNIT);
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
		assert!(pallet_hippocampus::WhitelistedRequesters::<Runtime>::contains_key(Arion::account_id()));
		assert_eq!(pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(), 0);
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), 0);
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


#[test]
fn medium3_per_requester_cap_enforced() {
	new_test_ext().execute_with(|| {
		// Set up: fund bank and whitelist two requesters
		let requester_a = account(10);
		let requester_b = account(11);
		let recipient = account(20);

		let _ = Balances::deposit_creating(&Hippocampus::account_id(), 100 * UNIT);
		Hippocampus::add_requester(RuntimeOrigin::signed(admin()), requester_a.clone())
			.expect("whitelist A");
		Hippocampus::add_requester(RuntimeOrigin::signed(admin()), requester_b.clone())
			.expect("whitelist B");

		// Set cap for requester A at 30 UNIT
		Hippocampus::set_requester_cap(RuntimeOrigin::signed(admin()), requester_a.clone(), 30 * UNIT)
			.expect("set cap for A");

		// Request 50 UNIT for A (should be capped at 30)
		let paid_a = Hippocampus::request_payment(&requester_a, &recipient, 50 * UNIT)
			.expect("request_payment");
		assert_eq!(paid_a, 30 * UNIT, "A should be capped at 30 UNIT");

		// Request 50 UNIT for B (no cap, should get full amount)
		let paid_b = Hippocampus::request_payment(&requester_b, &recipient, 50 * UNIT)
			.expect("request_payment");
		assert_eq!(paid_b, 50 * UNIT, "B has no cap, should get full amount");

		// Remove cap for A
		Hippocampus::remove_requester_cap(RuntimeOrigin::signed(admin()), requester_a.clone())
			.expect("remove cap for A");

		// Now A should be able to request up to available
		let available = Hippocampus::available_for_payout();
		let paid_a_uncapped = Hippocampus::request_payment(&requester_a, &recipient, available)
			.expect("request_payment");
		assert_eq!(paid_a_uncapped, available, "A without cap should get full available");
	});
}

#[test]
fn low1_dust_byte_blocks_preserved() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		register_miner(&family, &child, 7);

		// One byte is worth less than a planck over a settlement interval, so
		// `tokens_for` floors to zero and the dust branch is taken. A HIGH price
		// would not work here: `tokens_for` rises with price and saturates, so
		// the branch would never be reached and this test would prove nothing.
		submit_stats(7, 1);

		let alpha_price = UNIT / 20;
		enable_payments(1, alpha_price, 100 * UNIT);

		let expected_bb = SETTLEMENT_BLOCK as u128 - 1;
		assert_eq!(
			tokens_for(expected_bb, 1, alpha_price),
			0,
			"precondition: this really does round to zero tokens"
		);

		settle_at(SETTLEMENT_BLOCK);

		// The dust is kept, not consumed: this is what fails if settlement goes
		// back to taking byte_blocks before checking whether tokens are zero.
		assert_eq!(Balances::free_balance(&family), 0, "nothing payable yet");
		assert_eq!(
			pallet_arion::MinerAccruals::<Runtime>::get(7).expect("entry").byte_blocks,
			expected_bb,
			"dust byte_blocks survive a zero-token settlement"
		);

		// Raising the price makes the preserved dust payable.
		Arion::set_miner_price(RuntimeOrigin::signed(admin()), 10_000_000_000_u128)
			.expect("set price");
		settle_at(SETTLEMENT_BLOCK * 2);

		assert!(Balances::free_balance(&family) > 0, "miner is paid with preserved dust");
	});
}

#[test]
fn low3_payment_released_event_only_when_paid() {
	new_test_ext().execute_with(|| {
		let requester = account(10);
		let recipient = account(20);

		let _ = Balances::deposit_creating(&Hippocampus::account_id(), 10 * UNIT);
		Hippocampus::add_requester(RuntimeOrigin::signed(admin()), requester.clone())
			.expect("whitelist");

		// Request 0 amount - should not emit event
		System::reset_events();
		let _ = Hippocampus::request_payment(&requester, &recipient, 0);
		let events = System::events();
		let zero_paid_count = events.iter()
			.filter(|e| matches!(&e.event, RuntimeEvent::Hippocampus(pallet_hippocampus::Event::PaymentReleased { .. })))
			.count();
		assert_eq!(zero_paid_count, 0, "no event when amount is zero");

		// Request amount but cap prevents payment - should not emit event
		Hippocampus::set_requester_cap(RuntimeOrigin::signed(admin()), requester.clone(), 0)
			.expect("set cap to 0");
		System::reset_events();
		let _ = Hippocampus::request_payment(&requester, &recipient, 5 * UNIT);
		let events = System::events();
		let cap_zero_count = events.iter()
			.filter(|e| matches!(&e.event, RuntimeEvent::Hippocampus(pallet_hippocampus::Event::PaymentReleased { .. })))
			.count();
		assert_eq!(cap_zero_count, 0, "no event when cap is zero");

		// Request with valid cap - should emit event
		Hippocampus::set_requester_cap(RuntimeOrigin::signed(admin()), requester.clone(), 5 * UNIT)
			.expect("set cap to 5");
		System::reset_events();
		let _ = Hippocampus::request_payment(&requester, &recipient, 5 * UNIT);
		let events = System::events();
		let valid_count = events.iter()
			.filter(|e| matches!(&e.event, RuntimeEvent::Hippocampus(pallet_hippocampus::Event::PaymentReleased { .. })))
			.count();
		assert_eq!(valid_count, 1, "event emitted when paid > 0");
	});
}


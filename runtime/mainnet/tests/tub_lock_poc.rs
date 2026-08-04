//! Regression coverage for the `TotalUndistributedBacking` (TUB) release path.
//!
//! After `distribute_alpha` was deleted, TUB was still incremented at deposit
//! but no longer decremented on consume — that wall starved miner settlement.
//! Faiz's fix (`4c19627`) decrements TUB by `take_backed_portion`'s return on
//! every release site. These tests lock that invariant: earned marketplace
//! revenue becomes miner budget; chargebacks still free TUB; rankings pots
//! stay intentionally unfunded.
//!
//! Everything here runs against the real runtime: real extrinsics, real
//! `on_initialize` hooks, real balances.

use frame_support::traits::{Currency, Hooks};
use hippius_mainnet_runtime::{
	AccountId, Arion, ArionPayoutSource, Balances, BlockNumber, Credits, Hippocampus, Marketplace,
	Runtime, RuntimeOrigin, System,
};
use pallet_arion::{
	ChildMinerUid, ChildRegistration, ChildRegistrations, ChildStatus, FamilyArrears, MinerStats,
	MinerStatsUpdate, PayoutSource,
};
use pallet_marketplace::{NextBatchId, PendingSudoRefunds, TotalUndistributedBacking};
use sp_core::crypto::Ss58Codec;
use sp_runtime::{AccountId32, BuildStorage};

const UNIT: u128 = 1_000_000_000_000_000_000; // 18 decimals
const GIB: u128 = 1 << 30;
/// Must match `ArionSettlementInterval` in the runtime.
const SETTLEMENT_BLOCK: BlockNumber = 14_400;
/// Must match `BlocksPerEra` (rankings distribution period).
const RANKING_ERA: BlockNumber = 3_600;
/// The bank's existential deposit — it can never pay this last chunk out.
const ED: u128 = 500;

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

/// What the arion settlement hook is actually allowed to spend.
fn miner_budget() -> u128 {
	<ArionPayoutSource as PayoutSource<AccountId, u128>>::available()
}

fn bank() -> u128 {
	Balances::free_balance(&Hippocampus::account_id())
}

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

fn settle_at(n: BlockNumber) {
	System::set_block_number(n);
	Arion::on_initialize(n);
}

fn tokens_for(byte_blocks: u128, price: u128, token_price: u128) -> u128 {
	payment_math::tokens_for(
		payment_math::ByteBlocks::new(byte_blocks),
		payment_math::UsdPerGibBlock::new(price),
		payment_math::Usd::new(token_price),
	)
	.get()
}

/// Marketplace credit-sale authority, funded sudo key, and both pallet accounts
/// whitelisted as bank requesters. Returns `(authority, sudo, user)`.
fn setup_marketplace() -> (AccountId, AccountId, AccountId) {
	let (authority, sudo, user) = (account(10), account(11), account(12));
	Credits::add_authority(RuntimeOrigin::root(), authority.clone()).expect("add authority");
	pallet_marketplace::SudoKey::<Runtime>::put(Some(sudo.clone()));
	let _ = Balances::deposit_creating(&sudo, 1_000 * UNIT);
	Hippocampus::add_requester(RuntimeOrigin::signed(admin()), Marketplace::account_id())
		.expect("whitelist marketplace");
	Hippocampus::add_requester(RuntimeOrigin::signed(admin()), Arion::account_id())
		.expect("whitelist arion");
	(authority, sudo, user)
}

/// Sell `credits` worth of credits backed by `alpha`; the backing is routed to
/// the bank from the sudo account. Returns the new batch id.
fn sell_credits(
	authority: &AccountId,
	user: &AccountId,
	credits: u128,
	alpha: u128,
	frozen: bool,
) -> u64 {
	let batch_id = NextBatchId::<Runtime>::get();
	Marketplace::deposit(
		RuntimeOrigin::signed(authority.clone()),
		user.clone(),
		credits,
		alpha,
		frozen,
		None,
	)
	.expect("marketplace deposit");
	batch_id
}

/// Enable miner settlement: price feeds set, arion already whitelisted.
fn enable_payments(price_usd_per_gb_block: u128, alpha_price: u128) {
	Arion::set_miner_price(RuntimeOrigin::signed(admin()), price_usd_per_gb_block)
		.expect("set price");
	pallet_credits::AlphaPrice::<Runtime>::put(alpha_price);
}

/// Money the bank kept over a cycle that the miner budget cannot see. Zero for
/// a correctly-accounted cycle; positive means funds are walled off forever.
fn stuck(bank_delta: i128, budget_delta: i128) -> i128 {
	bank_delta - budget_delta
}

// ---------------------------------------------------------------------------
// 1. Consuming a backed deposit leaves TUB untouched: the bank keeps the money
//    and the miner budget never sees it.
// ---------------------------------------------------------------------------

#[test]
fn consumed_backing_releases_tub_and_pays_miners() {
	new_test_ext().execute_with(|| {
		let (authority, sudo, user) = setup_marketplace();
		let (family, child) = (account(1), account(2));
		register_miner(&family, &child, 7);
		submit_stats(7, 100 * GIB);

		let sudo_before = Balances::free_balance(&sudo);
		assert_eq!(bank(), 0);
		assert_eq!(miner_budget(), 0);

		// Sell 5 credits backed by 3 alpha. The backing lands in the bank.
		let batch_id = sell_credits(&authority, &user, 5 * UNIT, 3 * UNIT, false);
		assert_eq!(bank(), 3 * UNIT, "backing reached the bank");
		assert_eq!(TotalUndistributedBacking::<Runtime>::get(), 3 * UNIT);
		assert_eq!(Balances::free_balance(&sudo), sudo_before - 3 * UNIT);
		assert_eq!(
			pallet_marketplace::UnbackedBatchAlpha::<Runtime>::get(batch_id),
			0,
			"fully backed deposit"
		);
		// Unconsumed deposit is still "owed" (refundable) — not miner budget yet.
		assert_eq!(miner_budget(), 0, "unearned backing stays walled off");

		// The user spends every credit: revenue is earned, TUB must drop.
		Marketplace::consume_credits(user.clone(), 5 * UNIT).expect("consume credits");

		let batch = pallet_marketplace::Batches::<Runtime>::get(batch_id).expect("batch");
		assert_eq!(batch.remaining_alpha, 0, "all alpha released");
		assert_eq!(batch.remaining_credits, 0, "all credits consumed");
		assert_eq!(pallet_credits::AlphaBalances::<Runtime>::get(&user), 0);

		assert_eq!(
			TotalUndistributedBacking::<Runtime>::get(),
			0,
			"TUB decremented by backed portion on release"
		);
		assert_eq!(bank(), 3 * UNIT, "bank still holds the revenue");
		assert_eq!(PendingSudoRefunds::<Runtime>::get(), 0, "nothing owed back to sudo");
		assert_eq!(
			miner_budget(),
			3 * UNIT - ED,
			"earned revenue is now available to arion settlement"
		);

		let price = 10_000_000_000_u128;
		let alpha_price = UNIT / 20;
		enable_payments(price, alpha_price);
		settle_at(SETTLEMENT_BLOCK);

		let due = tokens_for(100 * GIB * (SETTLEMENT_BLOCK as u128 - 1), price, alpha_price);
		assert!(due > 0 && due < 3 * UNIT, "due {} must be affordable from a 3 UNIT bank", due);
		assert_eq!(Balances::free_balance(&family), due, "miner paid from marketplace revenue");
		assert_eq!(FamilyArrears::<Runtime>::get(&family), 0);
	});
}

// ---------------------------------------------------------------------------
// 2. Manual TUB -= is redundant now that release decrements TUB itself.
// ---------------------------------------------------------------------------

#[test]
fn release_path_alone_restores_miner_budget() {
	new_test_ext().execute_with(|| {
		let (authority, _sudo, user) = setup_marketplace();
		let (family, child) = (account(1), account(2));
		register_miner(&family, &child, 7);
		submit_stats(7, 100 * GIB);

		sell_credits(&authority, &user, 5 * UNIT, 3 * UNIT, false);
		Marketplace::consume_credits(user.clone(), 5 * UNIT).expect("consume credits");
		assert_eq!(
			miner_budget(),
			3 * UNIT - ED,
			"release path alone unlocks the budget — no manual TUB patch needed"
		);

		let price = 10_000_000_000_u128;
		let alpha_price = UNIT / 20;
		enable_payments(price, alpha_price);
		settle_at(SETTLEMENT_BLOCK);

		let due = tokens_for(100 * GIB * (SETTLEMENT_BLOCK as u128 - 1), price, alpha_price);
		assert_eq!(Balances::free_balance(&family), due, "paid in full");
		assert_eq!(FamilyArrears::<Runtime>::get(&family), 0);
	});
}

// ---------------------------------------------------------------------------
// 3. Marketplace revenue accumulates into miner budget, not a permanent wall.
// ---------------------------------------------------------------------------

#[test]
fn marketplace_revenue_feeds_miner_budget_across_cycles() {
	new_test_ext().execute_with(|| {
		let (authority, _sudo, user) = setup_marketplace();
		let funder = account(30);
		let (family, child) = (account(1), account(2));
		let _ = Balances::deposit_creating(&funder, 1_000 * UNIT);
		register_miner(&family, &child, 7);
		submit_stats(7, 100 * GIB);

		Hippocampus::deposit(
			RuntimeOrigin::signed(funder),
			5 * UNIT,
			pallet_hippocampus::DepositType::Grant,
		)
		.expect("miner budget grant");

		// High price: each settlement drains the entire available budget.
		enable_payments(1_000_000_000_000_u128, UNIT / 20);

		let mut budgets = Vec::new();
		for cycle in 1..=4u64 {
			sell_credits(&authority, &user, 5 * UNIT, 3 * UNIT, false);
			Marketplace::consume_credits(user.clone(), 5 * UNIT).expect("consume credits");

			let budget_before = miner_budget();
			settle_at(SETTLEMENT_BLOCK * cycle);
			budgets.push(budget_before);
		}

		// Cycle 1: grant + first 3 UNIT of revenue (minus ED).
		// Cycles 2–4: each release adds another 3 UNIT of spendable budget.
		assert_eq!(
			budgets,
			vec![8 * UNIT - ED, 3 * UNIT, 3 * UNIT, 3 * UNIT],
			"each revenue cycle raises miner budget by the earned alpha"
		);
		assert_eq!(
			Balances::free_balance(&family),
			5 * UNIT - ED + 12 * UNIT,
			"grant + all marketplace revenue paid out"
		);
		assert_eq!(TotalUndistributedBacking::<Runtime>::get(), 0, "no stranded TUB");
		// Price is intentionally extreme so due >> budget; arrears are fine —
		// the point is every planck of earned revenue was still paid out.
		assert!(
			FamilyArrears::<Runtime>::get(&family) > 0,
			"high price leaves unpaid due as arrears"
		);
	});
}

// ---------------------------------------------------------------------------
// 4. Chargeback also frees TUB (refund path); neither consume nor chargeback
//    strands bank balance relative to miner budget.
// ---------------------------------------------------------------------------

#[test]
fn chargeback_and_consume_both_free_tub_without_stranding() {
	new_test_ext().execute_with(|| {
		let (authority, sudo, user) = setup_marketplace();
		let funder = account(30);
		let _ = Balances::deposit_creating(&funder, 1_000 * UNIT);
		// Pre-fund so the bank can refund in full without hitting its ED.
		Hippocampus::deposit(
			RuntimeOrigin::signed(funder),
			5 * UNIT,
			pallet_hippocampus::DepositType::Grant,
		)
		.expect("grant");

		// --- deposit then CONSUME: bank grows, budget grows by the same ----
		let bank_0 = bank() as i128;
		let budget_0 = miner_budget() as i128;
		sell_credits(&authority, &user, 5 * UNIT, 3 * UNIT, false);
		Marketplace::consume_credits(user.clone(), 5 * UNIT).expect("consume credits");
		let stuck_consume = stuck(bank() as i128 - bank_0, miner_budget() as i128 - budget_0);
		assert_eq!(stuck_consume, 0, "consume must not strand bank vs miner budget");

		// --- deposit then CHARGE BACK --------------------------------------
		let bank_1 = bank() as i128;
		let budget_1 = miner_budget() as i128;
		let tub_1 = TotalUndistributedBacking::<Runtime>::get();
		let sudo_1 = Balances::free_balance(&sudo);

		let batch_id = sell_credits(&authority, &user, 5 * UNIT, 3 * UNIT, true);
		assert_eq!(TotalUndistributedBacking::<Runtime>::get(), tub_1 + 3 * UNIT);
		Marketplace::chargeback(RuntimeOrigin::root(), batch_id).expect("chargeback");

		assert_eq!(
			TotalUndistributedBacking::<Runtime>::get(),
			tub_1,
			"chargeback releases the backing from the owed ledger"
		);
		assert_eq!(PendingSudoRefunds::<Runtime>::get(), 0, "refund delivered in full");
		assert_eq!(Balances::free_balance(&sudo), sudo_1, "sudo made whole: out 3, back 3");

		let stuck_chargeback = stuck(bank() as i128 - bank_1, miner_budget() as i128 - budget_1);
		assert_eq!(stuck_chargeback, 0, "chargeback strands nothing");
	});
}

// ---------------------------------------------------------------------------
// 5. Nothing funds the ranking / marketplace pots any more.
// ---------------------------------------------------------------------------

#[test]
fn ranking_and_marketplace_pots_receive_nothing_and_distribute_nothing() {
	new_test_ext().execute_with(|| {
		let (authority, _sudo, user) = setup_marketplace();
		let storage_pot = pallet_rankings::Pallet::<Runtime>::account_id();
		let compute_pot =
			pallet_rankings::Pallet::<Runtime, pallet_rankings::Instance2>::account_id();
		let validator_pot =
			pallet_rankings::Pallet::<Runtime, pallet_rankings::Instance3>::account_id();
		let marketplace_pot = Marketplace::account_id();

		// Three full sale-and-spend cycles: 9 UNIT of revenue earned.
		for _ in 0..3 {
			sell_credits(&authority, &user, 5 * UNIT, 3 * UNIT, false);
			Marketplace::consume_credits(user.clone(), 5 * UNIT)
				.expect("consume credits");
		}
		assert_eq!(
			pallet_hippocampus::TotalDeposited::<Runtime>::get(
				pallet_hippocampus::DepositType::MarketplaceRevenue
			),
			9 * UNIT,
			"9 UNIT of revenue really moved"
		);

		// The pots that used to split it 70/30 got zero.
		assert_eq!(Balances::free_balance(&storage_pot), 0, "rankings Instance1 pot starved");
		assert_eq!(Balances::free_balance(&compute_pot), 0, "rankings Instance2 pot starved");
		assert_eq!(Balances::free_balance(&validator_pot), 0, "rankings Instance3 pot starved");
		assert_eq!(Balances::free_balance(&marketplace_pot), 0, "marketplace pot starved");
		assert_eq!(
			pallet_hippocampus::TotalPaidByRequester::<Runtime>::get(Marketplace::account_id()),
			0,
			"the marketplace never pulls from the bank outside chargebacks"
		);

		// The rankings era hook therefore distributes nothing: its whole payout
		// is `free_balance(pallet_account)`, which no code path funds.
		System::reset_events();
		System::set_block_number(RANKING_ERA);
		<pallet_rankings::Pallet<Runtime> as Hooks<BlockNumber>>::on_initialize(RANKING_ERA);
		<pallet_rankings::Pallet<Runtime, pallet_rankings::Instance2> as Hooks<BlockNumber>>::on_initialize(
			RANKING_ERA,
		);
		let rewards = System::events()
			.iter()
			.filter(|r| {
				matches!(
					&r.event,
					hippius_mainnet_runtime::RuntimeEvent::RankingStorage(
						pallet_rankings::Event::RewardDistributed { .. }
					) | hippius_mainnet_runtime::RuntimeEvent::RankingCompute(
						pallet_rankings::Event::RewardDistributed { .. }
					)
				)
			})
			.count();
		assert_eq!(rewards, 0, "no ranking rewards can be distributed from an empty pot");
	});
}

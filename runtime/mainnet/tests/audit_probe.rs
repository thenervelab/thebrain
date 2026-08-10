//! Audit probes. NOT part of the shipped suite — each test here documents a
//! behaviour the reviewed suite claims to cover but does not.

use frame_support::traits::{Currency, Hooks, OnRuntimeUpgrade};
use hippius_mainnet_runtime::{
	AccountId, Arion, Balances, BlockNumber, Credits, Hippocampus, Marketplace, Runtime,
	RuntimeOrigin, System,
};
use pallet_arion::{
	ChildMinerUid, ChildRegistration, ChildRegistrations, ChildStatus, MinerAccruals, MinerStats,
	MinerStatsUpdate,
};
use sp_core::crypto::Ss58Codec;
use sp_runtime::{AccountId32, BuildStorage};

const UNIT: u128 = 1_000_000_000_000_000_000;
const GIB: u128 = 1 << 30;
const SETTLEMENT_BLOCK: BlockNumber = 14_400;

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

/// `low1_dust_byte_blocks_preserved` sets `price = u128::MAX / 2` and comments
/// "tokens will be 0". `tokens_for` is monotonically increasing in price, so
/// that configuration saturates instead — the dust branch is never reached.
#[test]
fn probe_low1_price_choice_saturates_instead_of_zeroing() {
	let byte_blocks = 100 * GIB * (SETTLEMENT_BLOCK as u128 - 1);
	let tokens = payment_math::tokens_for(
		payment_math::ByteBlocks::new(byte_blocks),
		payment_math::UsdPerGibBlock::new(u128::MAX / 2),
		payment_math::Usd::new(UNIT / 20),
	)
	.get();
	assert_eq!(tokens, u128::MAX, "low1's price saturates; tokens.is_zero() is never true");
}

/// What a dust test that actually enters the `tokens.is_zero()` branch looks
/// like: a tiny byte count with a low price. Kills the "discard the dust"
/// mutation that `low1_dust_byte_blocks_preserved` survives.
#[test]
fn probe_dust_is_actually_preserved() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		register_miner(&family, &child, 7);
		submit_stats(7, 1); // one byte: below one planck of value

		Arion::set_miner_price(RuntimeOrigin::signed(admin()), 1).expect("set price");
		pallet_credits::AlphaPrice::<Runtime>::put(UNIT / 20);
		let _ = Balances::deposit_creating(&Hippocampus::account_id(), 100 * UNIT);
		Hippocampus::add_requester(RuntimeOrigin::signed(admin()), Arion::account_id())
			.expect("whitelist");

		let expected_bb = SETTLEMENT_BLOCK as u128 - 1;
		assert_eq!(
			payment_math::tokens_for(
				payment_math::ByteBlocks::new(expected_bb),
				payment_math::UsdPerGibBlock::new(1),
				payment_math::Usd::new(UNIT / 20),
			)
			.get(),
			0,
			"precondition: this configuration really does round to zero tokens"
		);

		settle_at(SETTLEMENT_BLOCK);

		assert_eq!(Balances::free_balance(&family), 0, "nothing payable");
		assert_eq!(
			MinerAccruals::<Runtime>::get(7).expect("entry").byte_blocks,
			expected_bb,
			"dust byte_blocks must survive the zero-token settlement"
		);
	});
}

fn settle_at(n: BlockNumber) {
	System::set_block_number(n);
	Arion::on_initialize(n);
}

/// `RequesterWithdrawalCap` caps each individual call, not lifetime spend, and
/// nothing consults `TotalPaidByRequester`. A capped requester still drains the
/// bank by calling repeatedly.
#[test]
fn probe_requester_cap_is_per_call_not_lifetime() {
	new_test_ext().execute_with(|| {
		let requester = account(10);
		let recipient = account(20);
		let _ = Balances::deposit_creating(&Hippocampus::account_id(), 100 * UNIT);
		Hippocampus::add_requester(RuntimeOrigin::signed(admin()), requester.clone())
			.expect("whitelist");
		Hippocampus::set_requester_cap(
			RuntimeOrigin::signed(admin()),
			requester.clone(),
			30 * UNIT,
		)
		.expect("cap at 30");

		for _ in 0..3 {
			Hippocampus::request_payment(&requester, &recipient, 50 * UNIT).expect("pay");
		}

		assert_eq!(
			pallet_hippocampus::TotalPaidByRequester::<Runtime>::get(&requester),
			90 * UNIT,
			"three calls at a 30 UNIT cap withdraw 90 UNIT: the cap is per-call"
		);
		assert_eq!(Balances::free_balance(&recipient), 90 * UNIT);
	});
}

/// `ActivateMinerPaymentBank` moves real funds from sudo, so it must run once.
/// It used to infer "already ran" from whitelist contents, which meant a normal
/// `remove_requester` made the next upgrade re-seed — double-charging sudo and
/// double-counting the backing ledger. The guard is now the storage version.
#[test]
fn activation_migration_is_one_shot_after_remove_requester() {
	new_test_ext().execute_with(|| {
		let authority = account(10);
		let sudo = account(11);
		let user = account(12);
		Credits::add_authority(RuntimeOrigin::root(), authority.clone()).expect("add authority");

		Marketplace::deposit(
			RuntimeOrigin::signed(authority),
			user,
			5 * UNIT,
			3 * UNIT,
			false,
			None,
		)
		.expect("pre-upgrade deposit");
		let _ = pallet_marketplace::UnbackedBatchAlpha::<Runtime>::clear(u32::MAX, None);
		pallet_marketplace::SudoKey::<Runtime>::put(Some(sudo.clone()));
		let _ = Balances::deposit_creating(&sudo, 100 * UNIT);

		hippius_mainnet_runtime::migrations::ActivateMinerPaymentBank::<Runtime>::on_runtime_upgrade();
		assert_eq!(pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(), 3 * UNIT);
		assert_eq!(Balances::free_balance(&Hippocampus::account_id()), 3 * UNIT);
		assert_eq!(Balances::free_balance(&sudo), 97 * UNIT);

		// A plausible ops action: pause the marketplace's ability to pull.
		Hippocampus::remove_requester(RuntimeOrigin::signed(admin()), Marketplace::account_id())
			.expect("admin un-whitelists one requester");

		// Next runtime upgrade re-runs the migration.
		hippius_mainnet_runtime::migrations::ActivateMinerPaymentBank::<Runtime>::on_runtime_upgrade();

		// The StorageVersion guard makes the migration one-shot, so removing a
		// requester no longer makes a later upgrade re-seed.
		assert_eq!(
			pallet_marketplace::TotalUndistributedBacking::<Runtime>::get(),
			3 * UNIT,
			"backing ledger not double-counted"
		);
		assert_eq!(
			Balances::free_balance(&Hippocampus::account_id()),
			3 * UNIT,
			"sudo not charged a second time"
		);
		assert_eq!(Balances::free_balance(&sudo), 97 * UNIT);
		// The guard is the storage version, not the whitelist: the removed
		// requester stays removed rather than being silently re-added.
		assert!(
			!pallet_hippocampus::WhitelistedRequesters::<Runtime>::contains_key(
				&Marketplace::account_id()
			),
			"a re-run must not undo a deliberate admin removal"
		);
	});
}

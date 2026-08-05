//! Adversarial tests: open-source miners / families must not be able to claim
//! funds they did not earn.
//!
//! Threat model (unprivileged on-chain actor):
//! - controls one or more family + child accounts
//! - can call any signed extrinsic
//! - cannot forge admin/stats/map authority origins
//! - cannot write pallet storage directly
//!
//! Trust assumptions deliberately *out of scope* here (document, don't pretend):
//! - compromised `StatsAuthorityOrigin` can inflate `shard_data_bytes`
//! - compromised `MapAuthorityOrigin` can rebind uids arbitrarily
//! - compromised `ArionAdminOrigin` can set any miner price
//! Those are operator/key-compromise risks, not miner self-service theft.

use frame_support::{assert_err, assert_noop, traits::{Currency, Hooks}};
use hippius_mainnet_runtime::{
	AccountId, Arion, Balances, BlockNumber, Hippocampus, ProxyType, Runtime, RuntimeOrigin,
	System,
};
use pallet_arion::{
	ChildMinerUid, ChildRegistration, ChildRegistrations, ChildStatus, CrushParams, FamilyArrears,
	MinerAccruals, MinerRecord, MinerStats, MinerStatsUpdate, MinerUidToChild,
};
use parity_scale_codec::Encode;
use sp_core::{crypto::Ss58Codec, ed25519, Pair};
use sp_runtime::{AccountId32, BuildStorage, DispatchError};

const UNIT: u128 = 1_000_000_000_000_000_000;
const GIB: u128 = 1 << 30;
const ED: u128 = 500;
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

fn make_family(family: &AccountId, child: &AccountId) {
	let _ = Balances::deposit_creating(family, 10_000 * UNIT);
	pallet_registration::OwnerToNode::<Runtime>::insert(family, vec![b"node".to_vec()]);
	pallet_proxy::Pallet::<Runtime>::add_proxy(
		RuntimeOrigin::signed(family.clone()),
		child.clone().into(),
		ProxyType::Any,
		0,
	)
	.expect("add_proxy");
}

fn node_sig(node: &ed25519::Pair, family: &AccountId, child: &AccountId, nonce: u64) -> [u8; 64] {
	let msg = (b"ARION_NODE_REG_V1", family, child, &node.public().0, nonce).encode();
	node.sign(&msg).0
}

fn register(family: &AccountId, child: &AccountId, node: &ed25519::Pair) {
	Arion::register_child(
		RuntimeOrigin::signed(family.clone()),
		family.clone(),
		child.clone(),
		node.public().0,
		node_sig(node, family, child, 0),
	)
	.expect("register_child");
}

fn publish_crush_map(epoch: u64, family: &AccountId, node_id: [u8; 32], uid: u32) {
	let record = MinerRecord {
		uid,
		node_id,
		weight: 1,
		family_id: family.clone(),
		endpoint: Default::default(),
		http_addr: Default::default(),
	};
	Arion::submit_crush_map(
		RuntimeOrigin::signed(admin()),
		epoch,
		CrushParams { pg_count: 16_384, ec_k: 4, ec_m: 2 },
		vec![record].try_into().expect("bounded"),
	)
	.expect("submit_crush_map");
}

/// Direct storage registration — for settlement isolation tests that do not
/// need the full proxy/signature path.
fn register_miner_storage(family: &AccountId, child: &AccountId, uid: u32) {
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
	if uid != 0 {
		ChildMinerUid::<Runtime>::insert(child, uid);
		MinerUidToChild::<Runtime>::insert(uid, child.clone());
	}
}

fn submit_stats(uid: u32, bytes: u128) {
	let bucket: u32 = System::block_number().try_into().unwrap_or(u32::MAX);
	Arion::submit_miner_stats(
		RuntimeOrigin::signed(admin()),
		bucket,
		vec![MinerStatsUpdate {
			uid,
			stats: MinerStats { shard_data_bytes: bytes, ..Default::default() },
		}]
		.try_into()
		.expect("bounded"),
		None,
	)
	.expect("stats");
}

fn enable_payments(bank_funds: u128) {
	Arion::set_miner_price(RuntimeOrigin::signed(admin()), 10_000_000_000).expect("price");
	pallet_credits::AlphaPrice::<Runtime>::put(UNIT / 20);
	let _ = Balances::deposit_creating(&Hippocampus::account_id(), bank_funds);
	let _ = Hippocampus::add_requester(RuntimeOrigin::signed(admin()), Arion::account_id());
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

// ── Privilege: miners cannot drive the money inputs ──────────────────────────

#[test]
fn adversary_cannot_submit_miner_stats() {
	new_test_ext().execute_with(|| {
		let attacker = account(9);
		let updates = vec![MinerStatsUpdate {
			uid: 1,
			stats: MinerStats { shard_data_bytes: u128::MAX, ..Default::default() },
		}];
		assert_noop!(
			Arion::submit_miner_stats(
				RuntimeOrigin::signed(attacker),
				1,
				updates.try_into().expect("bounded"),
				None,
			),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn adversary_cannot_set_miner_price() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Arion::set_miner_price(RuntimeOrigin::signed(account(9)), u128::MAX),
			DispatchError::BadOrigin
		);
		// Price remains unset → settlement pays nothing even with huge stats.
		register_miner_storage(&account(1), &account(2), 7);
		submit_stats(7, u128::MAX);
		let _ = Balances::deposit_creating(&Hippocampus::account_id(), 1000 * UNIT);
		let _ = Hippocampus::add_requester(RuntimeOrigin::signed(admin()), Arion::account_id());
		settle_at(SETTLEMENT_BLOCK);
		assert_eq!(Balances::free_balance(&account(1)), 0);
	});
}

#[test]
fn adversary_cannot_submit_crush_map() {
	new_test_ext().execute_with(|| {
		let attacker = account(9);
		let record = MinerRecord {
			uid: 99,
			node_id: [1u8; 32],
			weight: 1,
			family_id: attacker.clone(),
			endpoint: Default::default(),
			http_addr: Default::default(),
		};
		assert_noop!(
			Arion::submit_crush_map(
				RuntimeOrigin::signed(attacker),
				1,
				CrushParams { pg_count: 16_384, ec_k: 4, ec_m: 2 },
				vec![record].try_into().expect("bounded"),
			),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn adversary_cannot_pull_bank_as_requester() {
	new_test_ext().execute_with(|| {
		let attacker = account(9);
		let dest = account(10);
		let _ = Balances::deposit_creating(&Hippocampus::account_id(), 100 * UNIT);
		// Even with a funded bank, a random account is not a whitelisted requester.
		assert_err!(
			Hippocampus::request_payment(&attacker, &dest, 50 * UNIT),
			pallet_hippocampus::Error::<Runtime>::RequesterNotWhitelisted
		);
		assert_eq!(Balances::free_balance(&dest), 0);
	});
}

// ── Registration / identity theft ────────────────────────────────────────────

#[test]
fn adversary_cannot_register_under_another_family() {
	new_test_ext().execute_with(|| {
		let (victim_family, child) = (account(1), account(2));
		let attacker = account(9);
		let node = ed25519::Pair::from_seed(&[11u8; 32]);
		make_family(&victim_family, &child);
		// Attacker signs the extrinsic but names victim as family.
		assert_noop!(
			Arion::register_child(
				RuntimeOrigin::signed(attacker),
				victim_family.clone(),
				child.clone(),
				node.public().0,
				node_sig(&node, &victim_family, &child, 0),
			),
			DispatchError::BadOrigin
		);
	});
}

#[test]
fn adversary_cannot_register_with_forged_node_signature() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		let real_node = ed25519::Pair::from_seed(&[11u8; 32]);
		let impostor = ed25519::Pair::from_seed(&[99u8; 32]);
		make_family(&family, &child);
		// Sign with a different key than the claimed node_id.
		assert_noop!(
			Arion::register_child(
				RuntimeOrigin::signed(family.clone()),
				family,
				child,
				real_node.public().0,
				node_sig(&impostor, &account(1), &account(2), 0),
			),
			pallet_arion::Error::<Runtime>::InvalidNodeSignature
		);
	});
}

#[test]
fn adversary_cannot_deregister_another_familys_child() {
	new_test_ext().execute_with(|| {
		let (victim_family, victim_child) = (account(1), account(2));
		let attacker = account(9);
		let node = ed25519::Pair::from_seed(&[11u8; 32]);
		make_family(&victim_family, &victim_child);
		register(&victim_family, &victim_child, &node);

		assert_noop!(
			Arion::deregister_child(RuntimeOrigin::signed(attacker), victim_child.clone()),
			DispatchError::BadOrigin
		);
		// Still active and payable path intact.
		assert_eq!(
			ChildRegistrations::<Runtime>::get(&victim_child).unwrap().status,
			ChildStatus::Active
		);
	});
}

#[test]
fn adversary_cannot_bind_uid_by_registration_argument() {
	// Regression of the closed squatting hole: register_child has no uid arg.
	// Declaring someone else's uid is not expressible; map alone binds.
	new_test_ext().execute_with(|| {
		let (victim_family, victim_child) = (account(1), account(2));
		let (attacker_family, attacker_child) = (account(3), account(4));
		let victim_node = ed25519::Pair::from_seed(&[11u8; 32]);
		let attacker_node = ed25519::Pair::from_seed(&[99u8; 32]);
		make_family(&victim_family, &victim_child);
		make_family(&attacker_family, &attacker_child);

		publish_crush_map(1, &victim_family, victim_node.public().0, 4_242);
		register(&attacker_family, &attacker_child, &attacker_node);

		assert!(ChildMinerUid::<Runtime>::get(&attacker_child).is_none());
		assert!(MinerUidToChild::<Runtime>::get(4_242).is_none());

		register(&victim_family, &victim_child, &victim_node);
		assert_eq!(MinerUidToChild::<Runtime>::get(4_242), Some(victim_child));
	});
}

// ── Settlement entitlement ───────────────────────────────────────────────────

/// Payout is always to the *family* stash from `ChildRegistration.family`,
/// never to the child node account — so a compromised child key cannot redirect
/// payment by being the extrinsic signer (settlement is a hook anyway).
#[test]
fn settlement_pays_family_not_child_account() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		register_miner_storage(&family, &child, 7);
		submit_stats(7, 100 * GIB);
		enable_payments(100 * UNIT);
		settle_at(SETTLEMENT_BLOCK);

		assert!(Balances::free_balance(&family) > 0, "family receives payout");
		assert_eq!(Balances::free_balance(&child), 0, "child account never receives payout");
	});
}

/// Stats for a uid with no bound child never open accrual. A miner who later
/// binds that uid cannot retroactively claim the unbound window.
#[test]
fn late_binder_cannot_claim_unbound_historical_stats() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));

		// Authority reports huge storage under uid 7, but nobody holds it yet.
		submit_stats(7, 1_000 * GIB);
		System::set_block_number(5_000);
		submit_stats(7, 1_000 * GIB);
		assert!(
			!MinerAccruals::<Runtime>::contains_key(7),
			"unbound uid must not accrue"
		);

		// Attacker binds after the fact.
		register_miner_storage(&family, &child, 7);
		// First stats tick after bind opens accrual at last_block=now, bb=0.
		submit_stats(7, 1_000 * GIB);
		let acc = MinerAccruals::<Runtime>::get(7).expect("opened");
		assert_eq!(acc.byte_blocks, 0, "no retroactive integrate of unbound window");

		enable_payments(1_000 * UNIT);
		// Settle at 14400: only integrates from bind/stats last_block (5000) → 14400.
		settle_at(SETTLEMENT_BLOCK);
		let price = 10_000_000_000_u128;
		let alpha = UNIT / 20;
		let expected = tokens_for(1_000 * GIB * (SETTLEMENT_BLOCK as u128 - 5_000), price, alpha);
		let full_if_retro = tokens_for(1_000 * GIB * (SETTLEMENT_BLOCK as u128 - 1), price, alpha);
		assert_eq!(Balances::free_balance(&family), expected);
		assert!(expected < full_if_retro, "must not be paid for the unbound era");
	});
}

/// After the map moves a uid, the displaced family is not paid for past work
/// (forfeited) nor for future bytes reported under that uid.
#[test]
fn displaced_family_cannot_collect_after_uid_reassign() {
	new_test_ext().execute_with(|| {
		let (fam_a, child_a) = (account(1), account(2));
		let (fam_b, child_b) = (account(3), account(4));
		let node_a = ed25519::Pair::from_seed(&[11u8; 32]);
		let node_b = ed25519::Pair::from_seed(&[99u8; 32]);
		make_family(&fam_a, &child_a);
		make_family(&fam_b, &child_b);
		register(&fam_a, &child_a, &node_a);
		register(&fam_b, &child_b, &node_b);

		publish_crush_map(1, &fam_a, node_a.public().0, 42);
		assert_eq!(MinerUidToChild::<Runtime>::get(42), Some(child_a.clone()));

		// A accrues unpaid work.
		submit_stats(42, 200 * GIB);
		System::set_block_number(100);
		submit_stats(42, 200 * GIB);
		assert!(MinerAccruals::<Runtime>::get(42).unwrap().byte_blocks > 0);

		// Map recycles the uid to B; A's accrual is forfeited.
		publish_crush_map(2, &fam_b, node_b.public().0, 42);
		assert_eq!(MinerUidToChild::<Runtime>::get(42), Some(child_b.clone()));
		assert!(ChildMinerUid::<Runtime>::get(&child_a).is_none());
		assert!(
			MinerAccruals::<Runtime>::get(42).map(|a| a.byte_blocks).unwrap_or(0) == 0
				|| !MinerAccruals::<Runtime>::contains_key(42)
				|| MinerAccruals::<Runtime>::get(42).unwrap().byte_blocks == 0,
			"displaced accrual must not remain payable under the uid"
		);

		// Fresh work under B only.
		submit_stats(42, 50 * GIB);
		enable_payments(100 * UNIT);
		let before_a = Balances::free_balance(&fam_a);
		let before_b = Balances::free_balance(&fam_b);
		settle_at(SETTLEMENT_BLOCK);

		assert_eq!(
			Balances::free_balance(&fam_a),
			before_a,
			"displaced family must not be paid"
		);
		assert_eq!(FamilyArrears::<Runtime>::get(&fam_a), 0);

		let price = 10_000_000_000_u128;
		let alpha = UNIT / 20;
		// B: submit_stats at block 100 sets last_block=100, bb=0; settle
		// integrates 50 GiB × (14400−100).
		let expected_b = tokens_for(50 * GIB * (SETTLEMENT_BLOCK as u128 - 100), price, alpha);
		assert_eq!(Balances::free_balance(&fam_b) - before_b, expected_b);
		// A must not hold arrears for the forfeited work either.
		assert_eq!(FamilyArrears::<Runtime>::get(&fam_a), 0);
		// B is not paid as if it inherited A's larger byte rate.
		let if_inherited_a_rate =
			tokens_for(200 * GIB * (SETTLEMENT_BLOCK as u128 - 100), price, alpha);
		assert!(expected_b < if_inherited_a_rate);
	});
}

/// Real deregister path: family forfeits unpaid accrual and is not paid later.
#[test]
fn deregistered_family_cannot_collect_at_next_settlement() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		let node = ed25519::Pair::from_seed(&[11u8; 32]);
		make_family(&family, &child);
		register(&family, &child, &node);
		publish_crush_map(1, &family, node.public().0, 7);

		submit_stats(7, 100 * GIB);
		System::set_block_number(50);
		submit_stats(7, 100 * GIB);
		assert!(MinerAccruals::<Runtime>::get(7).unwrap().byte_blocks > 0);

		Arion::deregister_child(RuntimeOrigin::signed(family.clone()), child.clone())
			.expect("deregister");
		assert!(MinerAccruals::<Runtime>::get(7).is_none());
		assert!(MinerUidToChild::<Runtime>::get(7).is_none());

		enable_payments(100 * UNIT);
		let before = Balances::free_balance(&family);
		settle_at(SETTLEMENT_BLOCK);
		assert_eq!(
			Balances::free_balance(&family),
			before,
			"deregistered family must not receive a settlement payout"
		);
		assert_eq!(FamilyArrears::<Runtime>::get(&family), 0);
	});
}

/// `claim_unbonded` only releases the registration deposit — it is not a
/// backdoor to reclaim forfeited miner payment accrual.
#[test]
fn claim_unbonded_does_not_pay_forfeited_accrual() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		let node = ed25519::Pair::from_seed(&[11u8; 32]);
		make_family(&family, &child);
		// Lockup off by default in tests → deposit 0, unbonding_end = now.
		register(&family, &child, &node);
		publish_crush_map(1, &family, node.public().0, 7);
		submit_stats(7, 100 * GIB);
		System::set_block_number(20);
		submit_stats(7, 100 * GIB);

		Arion::deregister_child(RuntimeOrigin::signed(family.clone()), child.clone())
			.expect("deregister");
		let before = Balances::free_balance(&family);

		Arion::claim_unbonded(RuntimeOrigin::signed(family.clone()), child).expect("claim");
		assert_eq!(
			Balances::free_balance(&family),
			before,
			"claim_unbonded must not mint miner-payment tokens"
		);

		enable_payments(100 * UNIT);
		settle_at(SETTLEMENT_BLOCK);
		assert_eq!(Balances::free_balance(&family), before);
	});
}

/// Uid 0 is never paid (sentinel / unbound).
#[test]
fn uid_zero_is_never_paid() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		// Active registration with uid 0 binding forced (malformed state).
		ChildRegistrations::<Runtime>::insert(
			&child,
			ChildRegistration {
				family: family.clone(),
				node_id: [0u8; 32],
				status: ChildStatus::Active,
				deposit: 0u128,
				unbonding_end: 0u32.into(),
			},
		);
		ChildMinerUid::<Runtime>::insert(&child, 0u32);
		// Reverse index for 0 would be toxic; leave it empty like production.
		submit_stats(0, 100 * GIB);
		// Even if stats wrote something, settlement skips uid==0.
		enable_payments(100 * UNIT);
		settle_at(SETTLEMENT_BLOCK);
		assert_eq!(Balances::free_balance(&family), 0);
	});
}

/// Under shortfall, no family can receive more than its own due (pro-rata cap).
#[test]
fn no_family_can_be_paid_more_than_its_due() {
	new_test_ext().execute_with(|| {
		let miners = [
			(account(1), account(2), 1u32, 300 * GIB),
			(account(3), account(4), 2u32, 100 * GIB),
			(account(5), account(6), 3u32, 50 * GIB),
		];
		for (f, c, uid, bytes) in &miners {
			register_miner_storage(f, c, *uid);
			submit_stats(*uid, *bytes);
		}
		let price = 10_000_000_000_u128;
		let alpha = UNIT / 20;
		let elapsed = SETTLEMENT_BLOCK as u128 - 1;
		let dues: Vec<u128> = miners
			.iter()
			.map(|(_, _, _, b)| tokens_for(*b * elapsed, price, alpha))
			.collect();
		let total: u128 = dues.iter().sum();
		// Severe shortfall.
		enable_payments(total / 10 + ED);
		settle_at(SETTLEMENT_BLOCK);

		for (i, (f, _, _, _)) in miners.iter().enumerate() {
			let paid = Balances::free_balance(f);
			let arrears = FamilyArrears::<Runtime>::get(f);
			assert!(paid <= dues[i], "family overpaid vs due");
			assert_eq!(paid + arrears, dues[i], "conservation: paid+arrears=due");
		}
	});
}

/// A family with zero byte-blocks cannot ride another family's settlement:
/// empty workers stay at 0 while a real worker is paid.
#[test]
fn zero_work_family_cannot_piggyback_on_others_settlement() {
	new_test_ext().execute_with(|| {
		let (worker_f, worker_c) = (account(1), account(2));
		let (idle_f, idle_c) = (account(3), account(4));
		register_miner_storage(&worker_f, &worker_c, 1);
		register_miner_storage(&idle_f, &idle_c, 2);
		submit_stats(1, 100 * GIB);
		submit_stats(2, 0);

		enable_payments(100 * UNIT);
		settle_at(SETTLEMENT_BLOCK);

		assert!(Balances::free_balance(&worker_f) > 0);
		assert_eq!(Balances::free_balance(&idle_f), 0);
		assert_eq!(FamilyArrears::<Runtime>::get(&idle_f), 0);
	});
}

/// Miner budget cannot cross the TUB wall even with absurd dues (runaway stats
/// from a *hypothetical* authority bug still cannot steal pot backing).
#[test]
fn runaway_due_cannot_steal_marketplace_backing() {
	new_test_ext().execute_with(|| {
		use hippius_mainnet_runtime::{Credits, Marketplace};

		let authority = account(10);
		let sudo = account(11);
		let user = account(12);
		Credits::add_authority(RuntimeOrigin::root(), authority.clone()).expect("auth");
		pallet_marketplace::SudoKey::<Runtime>::put(Some(sudo.clone()));
		let _ = Balances::deposit_creating(&sudo, 100 * UNIT);

		Marketplace::deposit(
			RuntimeOrigin::signed(authority),
			user,
			5 * UNIT,
			4 * UNIT, // TUB = 4 UNIT
			false,
			None,
		)
		.expect("deposit");

		let (family, child) = (account(1), account(2));
		register_miner_storage(&family, &child, 7);
		submit_stats(7, u128::MAX);
		// No miner grant: only TUB sits in the bank.
		Arion::set_miner_price(RuntimeOrigin::signed(admin()), u128::MAX).expect("price");
		pallet_credits::AlphaPrice::<Runtime>::put(1);
		let _ = Hippocampus::add_requester(RuntimeOrigin::signed(admin()), Arion::account_id());

		let tub = pallet_marketplace::TotalUndistributedBacking::<Runtime>::get();
		assert_eq!(tub, 4 * UNIT);
		let bank_before = Balances::free_balance(&Hippocampus::account_id());

		settle_at(SETTLEMENT_BLOCK);

		// Miner pool was free - ED - TUB ≈ 0 → nothing payable to miners.
		assert_eq!(Balances::free_balance(&family), 0);
		assert_eq!(
			Balances::free_balance(&Hippocampus::account_id()),
			bank_before,
			"TUB-backed funds must remain in the bank"
		);
		assert!(FamilyArrears::<Runtime>::get(&family) > 0, "due deferred, not stolen");
	});
}

// ── Residual gaps: cooldown, caps, weight isolation ─────────────────────────

/// Unregister cooldown blocks immediate re-register of the same child / node
/// (anti register-yoyo). Not theft, but DoS / churn protection.
#[test]
fn unregister_cooldown_blocks_immediate_reregister() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		let node = ed25519::Pair::from_seed(&[11u8; 32]);
		make_family(&family, &child);
		register(&family, &child, &node);
		Arion::deregister_child(RuntimeOrigin::signed(family.clone()), child.clone())
			.expect("deregister");
		Arion::claim_unbonded(RuntimeOrigin::signed(family.clone()), child.clone())
			.expect("claim");

		// Same block / before UnregisterCooldownBlocks (7200): child still cooling down.
		assert_noop!(
			Arion::register_child(
				RuntimeOrigin::signed(family.clone()),
				family.clone(),
				child.clone(),
				node.public().0,
				node_sig(&node, &family, &child, 1), // nonce advanced after first reg
			),
			pallet_arion::Error::<Runtime>::ChildInCooldown
		);

		// Past cooldown: allowed (may still hit NodeId cooldown with same node).
		System::set_block_number(7200 + 1);
		// Fresh nonce after failed attempt unchanged; original reg consumed nonce 0 → 1.
		// claim removed registration; NodeIdNonce stays at 1.
		let node2 = ed25519::Pair::from_seed(&[33u8; 32]);
		Arion::register_child(
			RuntimeOrigin::signed(family.clone()),
			family.clone(),
			child,
			node2.public().0,
			node_sig(&node2, &family, &account(2), 0),
		)
		.expect("reregister after cooldown with new node");
	});
}

/// Per-family child cap is enforced — spam children cannot grow without bound.
/// Uses the real `register_child` extrinsic with a small temporary max so we
/// do not hit proxy `MaxProxies` before arion's own cap.
#[test]
fn max_children_per_family_is_enforced() {
	new_test_ext().execute_with(|| {
		// Seed FamilyActiveChildren at the mainnet cap so the next real register
		// trips TooManyChildrenInFamily without needing 35 proxies.
		let family = account(1);
		let child = account(2);
		let node = ed25519::Pair::from_seed(&[11u8; 32]);
		make_family(&family, &child);
		pallet_arion::FamilyActiveChildren::<Runtime>::insert(&family, 35u32);
		// FamilyChildren index must also look full for try_push path — register
		// hits fam_count check before try_push.
		assert_noop!(
			Arion::register_child(
				RuntimeOrigin::signed(family.clone()),
				family.clone(),
				child,
				node.public().0,
				node_sig(&node, &family, &account(2), 0),
			),
			pallet_arion::Error::<Runtime>::TooManyChildrenInFamily
		);
	});
}

/// Miner USD settlement is driven only by byte-blocks × price, not by
/// `FamilyWeight` (emissions path). Huge weight + zero storage ⇒ no pay;
/// zero weight + real storage ⇒ pay.
#[test]
fn family_weight_does_not_affect_miner_usd_settlement() {
	new_test_ext().execute_with(|| {
		let (heavy_f, heavy_c) = (account(1), account(2));
		let (light_f, light_c) = (account(3), account(4));
		register_miner_storage(&heavy_f, &heavy_c, 1);
		register_miner_storage(&light_f, &light_c, 2);

		// Weight path (emissions): heavy family has max weight, light has zero.
		pallet_arion::FamilyWeight::<Runtime>::insert(&heavy_f, u16::MAX);
		pallet_arion::FamilyWeight::<Runtime>::insert(&light_f, 0u16);

		// Storage path (USD payments): only light stores data.
		submit_stats(1, 0);
		submit_stats(2, 100 * GIB);

		enable_payments(100 * UNIT);
		let before_h = Balances::free_balance(&heavy_f);
		let before_l = Balances::free_balance(&light_f);
		settle_at(SETTLEMENT_BLOCK);

		assert_eq!(
			Balances::free_balance(&heavy_f),
			before_h,
			"max FamilyWeight + zero bytes must not earn miner USD pay"
		);
		assert!(
			Balances::free_balance(&light_f) > before_l,
			"zero FamilyWeight + real bytes must still earn miner USD pay"
		);
	});
}

/// Re-binding the same child after cooldown must not resurrect forfeited
/// arrears or double-count old work (fresh registration, fresh uid path).
#[test]
fn reregister_after_forfeit_starts_clean() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		let node = ed25519::Pair::from_seed(&[11u8; 32]);
		make_family(&family, &child);
		register(&family, &child, &node);
		publish_crush_map(1, &family, node.public().0, 7);
		submit_stats(7, 100 * GIB);
		System::set_block_number(30);
		submit_stats(7, 100 * GIB);

		Arion::deregister_child(RuntimeOrigin::signed(family.clone()), child.clone())
			.expect("deregister");
		// claim to fully remove registration record
		Arion::claim_unbonded(RuntimeOrigin::signed(family.clone()), child.clone())
			.expect("claim");
		assert!(ChildRegistrations::<Runtime>::get(&child).is_none());
		assert_eq!(FamilyArrears::<Runtime>::get(&family), 0);

		// Cooldown may block immediate re-register; advance past cooldown.
		// UnregisterCooldownBlocks is a runtime constant — jump far enough.
		System::set_block_number(1_000_000);
		// New node key (old node_id still in cooldown).
		let node2 = ed25519::Pair::from_seed(&[22u8; 32]);
		// Proxy still valid for child.
		register(&family, &child, &node2);
		publish_crush_map(2, &family, node2.public().0, 8);

		submit_stats(8, 10 * GIB);
		enable_payments(100 * UNIT);
		// Next settlement boundary after 1_000_000: ceil to multiple of 14400.
		let settle_n = ((1_000_000 / SETTLEMENT_BLOCK) + 1) * SETTLEMENT_BLOCK;
		let stats_at = System::block_number() as u128;
		let before = Balances::free_balance(&family);
		settle_at(settle_n);

		// Only paid for post-reregister work under uid 8 — not the forfeited uid 7 era.
		let paid = Balances::free_balance(&family).saturating_sub(before);
		let price = 10_000_000_000_u128;
		let alpha = UNIT / 20;
		let expected = tokens_for(10 * GIB * (settle_n as u128 - stats_at), price, alpha);
		assert_eq!(paid, expected);
	});
}

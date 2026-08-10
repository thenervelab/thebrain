//! Does the new authority-bound `miner_uid` survive the real onboarding order?
//!
//! `register_child` now takes the uid from `EpochMiners[CurrentEpoch]` instead
//! of from the caller — which closes the squatting hole. The question these
//! tests answer is what happens for a miner that is *not yet* in the crush map
//! when it registers, because that is the order the stack actually enforces:
//! the validator refuses to admit a node to the cluster map until it is
//! registered on-chain, so the crush map can only ever contain nodes that
//! already registered.

use frame_support::traits::{Currency, Hooks};
use hippius_mainnet_runtime::{
	AccountId, Arion, Balances, BlockNumber, Hippocampus, ProxyType, Runtime, RuntimeEvent,
	RuntimeOrigin, System,
};
use pallet_arion::{
	ChildMinerUid, CrushParams, MinerRecord, MinerStats, MinerStatsUpdate, MinerUidToChild,
};
use parity_scale_codec::Encode;
use sp_core::{crypto::Ss58Codec, ed25519, Pair};
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

/// Publish a crush map containing `node_id` under `uid`, as the validator's
/// chain-submitter does once it admits the node to the cluster.
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

fn enable_payments() {
	Arion::set_miner_price(RuntimeOrigin::signed(admin()), 10_000_000_000).expect("price");
	pallet_credits::AlphaPrice::<Runtime>::put(UNIT / 20);
	let _ = Balances::deposit_creating(&Hippocampus::account_id(), 1_000 * UNIT);
	Hippocampus::add_requester(RuntimeOrigin::signed(admin()), Arion::account_id())
		.expect("whitelist arion");
}

/// THE REAL ORDER: register first (the validator will not admit an unregistered
/// node), crush map afterwards. Publishing the map is what binds the uid, so a
/// miner onboarded the only way the stack allows is payable.
#[test]
fn register_before_crush_map_still_binds_uid_and_pays() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		let node = ed25519::Pair::from_seed(&[11u8; 32]);
		make_family(&family, &child);

		// EpochMiners is empty: this node cannot be in the crush map yet,
		// because the validator gates admission on on-chain registration.
		register(&family, &child, &node);
		assert!(
			ChildMinerUid::<Runtime>::get(&child).is_none(),
			"nothing to bind yet — the map does not contain the node"
		);

		// The validator now admits the node and the authority publishes the map.
		System::reset_events();
		publish_crush_map(1, &family, node.public().0, 4_242);
		assert_eq!(
			ChildMinerUid::<Runtime>::get(&child),
			Some(4_242),
			"publishing the crush map binds the authority-assigned uid"
		);
		assert_eq!(MinerUidToChild::<Runtime>::get(4_242), Some(child.clone()));
		// Ops alert on this event's absence to spot unpayable miners, so it has
		// to actually fire.
		let bound = System::events().iter().any(|r| {
			matches!(
				&r.event,
				RuntimeEvent::Arion(pallet_arion::Event::MinerUidBound { child: c, uid })
					if *c == child && *uid == 4_242
			)
		});
		assert!(bound, "MinerUidBound event expected");

		let before = Balances::free_balance(&family);
		submit_stats(4_242, 100 * GIB);
		System::set_block_number(SETTLEMENT_BLOCK);
		submit_stats(4_242, 100 * GIB);
		enable_payments();
		Arion::on_initialize(SETTLEMENT_BLOCK);

		assert!(
			Balances::free_balance(&family) > before,
			"a fully-onboarded, storing miner is paid"
		);
	});
}

/// THE UPGRADE PATH THAT MATTERS. Miners already on the live chain registered
/// through tooling that never supplied a uid, so their `ChildMinerUid` is unset
/// and the v0->v1 migration (which builds the reverse index from that map) finds
/// nothing. They must become payable from the next published map alone, without
/// re-registering — otherwise the release strands every existing miner.
#[test]
fn legacy_miner_with_no_uid_is_bound_by_the_next_crush_map() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		let node = ed25519::Pair::from_seed(&[11u8; 32]);
		let node_id = node.public().0;

		// Pre-upgrade state: registered, node indexed, NO uid — exactly what the
		// documented 4-arg registration flow leaves behind.
		pallet_arion::ChildRegistrations::<Runtime>::insert(
			&child,
			pallet_arion::ChildRegistration {
				family: family.clone(),
				node_id,
				status: pallet_arion::ChildStatus::Active,
				deposit: 0u128,
				unbonding_end: 0u32.into(),
			},
		);
		pallet_arion::NodeIdToChild::<Runtime>::insert(node_id, &child);
		assert!(ChildMinerUid::<Runtime>::get(&child).is_none(), "legacy: no uid on chain");

		// The upgrade runs; the reverse-index migration has nothing to work with.
		<pallet_arion::Pallet<Runtime> as Hooks<BlockNumber>>::on_runtime_upgrade();
		assert!(MinerUidToChild::<Runtime>::get(4_242).is_none());

		// The next authority-published map binds them, with no operator action.
		publish_crush_map(1, &family, node_id, 4_242);
		assert_eq!(ChildMinerUid::<Runtime>::get(&child), Some(4_242));
		assert_eq!(MinerUidToChild::<Runtime>::get(4_242), Some(child.clone()));

		let before = Balances::free_balance(&family);
		submit_stats(4_242, 100 * GIB);
		System::set_block_number(SETTLEMENT_BLOCK);
		submit_stats(4_242, 100 * GIB);
		enable_payments();
		Arion::on_initialize(SETTLEMENT_BLOCK);
		assert!(Balances::free_balance(&family) > before, "legacy miner is paid");
	});
}

/// The upgrade must not depend on a future map publication. Crush-map
/// publication is event-driven — the validator bumps the epoch only on cluster
/// churn, and weight-driven bumps are off by default — so a stable cluster can
/// go indefinitely without publishing. The migration therefore binds from the
/// map already on-chain, and every existing miner is payable immediately.
#[test]
fn migration_binds_legacy_miners_from_the_already_published_map() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		let node = ed25519::Pair::from_seed(&[11u8; 32]);
		let node_id = node.public().0;

		// A map was published before the upgrade, and the miner registered
		// against it with tooling that never supplied a uid.
		publish_crush_map(1, &family, node_id, 4_242);
		pallet_arion::ChildRegistrations::<Runtime>::insert(
			&child,
			pallet_arion::ChildRegistration {
				family: family.clone(),
				node_id,
				status: pallet_arion::ChildStatus::Active,
				deposit: 0u128,
				unbonding_end: 0u32.into(),
			},
		);
		pallet_arion::NodeIdToChild::<Runtime>::insert(node_id, &child);
		ChildMinerUid::<Runtime>::remove(&child);
		MinerUidToChild::<Runtime>::remove(4_242);

		// The upgrade alone binds them — no new map, no operator action.
		<pallet_arion::Pallet<Runtime> as Hooks<BlockNumber>>::on_runtime_upgrade();
		assert_eq!(
			ChildMinerUid::<Runtime>::get(&child),
			Some(4_242),
			"migration binds from the map already on-chain"
		);
		assert_eq!(MinerUidToChild::<Runtime>::get(4_242), Some(child.clone()));

		let before = Balances::free_balance(&family);
		submit_stats(4_242, 100 * GIB);
		System::set_block_number(SETTLEMENT_BLOCK);
		submit_stats(4_242, 100 * GIB);
		enable_payments();
		Arion::on_initialize(SETTLEMENT_BLOCK);
		assert!(Balances::free_balance(&family) > before, "payable straight after upgrade");
	});
}

/// A deregistered child's node cannot be re-bound by a stale map still listing
/// it: deregistration removes the `NodeIdToChild` entry the binding resolves.
#[test]
fn deregistered_child_is_not_rebound_by_a_stale_crush_map() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		let node = ed25519::Pair::from_seed(&[11u8; 32]);
		let node_id = node.public().0;
		make_family(&family, &child);
		register(&family, &child, &node);
		publish_crush_map(1, &family, node_id, 4_242);
		assert_eq!(ChildMinerUid::<Runtime>::get(&child), Some(4_242));

		Arion::deregister_child(RuntimeOrigin::signed(family.clone()), child.clone())
			.expect("deregister");
		assert!(ChildMinerUid::<Runtime>::get(&child).is_none());

		// The validator has not dropped the node from its map yet.
		publish_crush_map(2, &family, node_id, 4_242);
		assert!(
			ChildMinerUid::<Runtime>::get(&child).is_none(),
			"a departed child is not resurrected by a stale map"
		);
		assert!(MinerUidToChild::<Runtime>::get(4_242).is_none());
	});
}

/// POSITIVE CONTROL: if the node somehow IS already in the crush map when it
/// registers, the binding works and the miner is paid. This is what the fix was
/// written for — it just is not the order the stack produces.
#[test]
fn control_crush_map_before_register_binds_uid_and_pays() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		let node = ed25519::Pair::from_seed(&[11u8; 32]);
		make_family(&family, &child);

		publish_crush_map(1, &family, node.public().0, 4_242);
		register(&family, &child, &node);

		assert_eq!(
			ChildMinerUid::<Runtime>::get(&child),
			Some(4_242),
			"uid bound from the authority-signed map"
		);
		assert_eq!(MinerUidToChild::<Runtime>::get(4_242), Some(child.clone()));

		submit_stats(4_242, 100 * GIB);
		System::set_block_number(SETTLEMENT_BLOCK);
		submit_stats(4_242, 100 * GIB);
		enable_payments();
		Arion::on_initialize(SETTLEMENT_BLOCK);

		assert!(
			Balances::free_balance(&family) > 10_000 * UNIT,
			"miner is paid when the uid was bound"
		);
	});
}

/// The squatting hole really is closed: a uid can no longer be chosen by the
/// caller at all, so declaring someone else's uid is not expressible.
#[test]
fn attacker_cannot_bind_a_uid_belonging_to_another_node() {
	new_test_ext().execute_with(|| {
		let (victim_family, victim_child) = (account(1), account(2));
		let (attacker_family, attacker_child) = (account(3), account(4));
		let victim_node = ed25519::Pair::from_seed(&[11u8; 32]);
		let attacker_node = ed25519::Pair::from_seed(&[99u8; 32]);
		make_family(&victim_family, &victim_child);
		make_family(&attacker_family, &attacker_child);

		// The map binds uid 4242 to the VICTIM's node.
		publish_crush_map(1, &victim_family, victim_node.public().0, 4_242);

		// The attacker registers its own node. There is no argument through
		// which it could claim 4242, and the map does not list its node.
		register(&attacker_family, &attacker_child, &attacker_node);

		assert!(
			ChildMinerUid::<Runtime>::get(&attacker_child).is_none(),
			"attacker binds no uid at all"
		);
		assert!(
			MinerUidToChild::<Runtime>::get(4_242).is_none(),
			"the victim's uid is not claimable by the attacker"
		);

		// And the victim can still claim it by registering.
		register(&victim_family, &victim_child, &victim_node);
		assert_eq!(MinerUidToChild::<Runtime>::get(4_242), Some(victim_child));
	});
}

/// The map is authoritative, because accrual follows the uid rather than the
/// binding: `submit_miner_stats` writes `MinerStatsByUid[uid]` from the
/// authority's view of who holds it. Leaving a stale child bound would pay it
/// for the new node's bytes, so a reassignment must actually move — and the
/// displaced child's unpaid accrual is forfeited explicitly, not stranded.
#[test]
fn crush_map_reassigns_a_uid_and_forfeits_the_displaced_accrual() {
	new_test_ext().execute_with(|| {
		let (family_a, child_a) = (account(1), account(2));
		let (family_b, child_b) = (account(3), account(4));
		let node_a = ed25519::Pair::from_seed(&[11u8; 32]);
		let node_b = ed25519::Pair::from_seed(&[99u8; 32]);
		make_family(&family_a, &child_a);
		make_family(&family_b, &child_b);
		register(&family_a, &child_a, &node_a);
		register(&family_b, &child_b, &node_b);

		publish_crush_map(1, &family_a, node_a.public().0, 4_242);
		assert_eq!(MinerUidToChild::<Runtime>::get(4_242), Some(child_a.clone()));

		// A leaves the fleet without deregistering and the authority recycles
		// its uid to B. Give A some unpaid accrual first.
		submit_stats(4_242, 100 * GIB);
		System::set_block_number(100);
		System::reset_events();
		publish_crush_map(2, &family_b, node_b.public().0, 4_242);

		assert_eq!(
			MinerUidToChild::<Runtime>::get(4_242),
			Some(child_b.clone()),
			"the map wins: the uid moves to the node the authority says owns it"
		);
		assert!(
			ChildMinerUid::<Runtime>::get(&child_a).is_none(),
			"the stale holder is unbound, so it cannot be paid for B's bytes"
		);

		let reassigned = System::events().iter().any(|r| {
			matches!(
				&r.event,
				RuntimeEvent::Arion(pallet_arion::Event::MinerUidReassigned { uid, from, to })
					if *uid == 4_242 && *from == child_a && *to == child_b
			)
		});
		assert!(reassigned, "MinerUidReassigned event expected");

		let forfeited = System::events().iter().any(|r| {
			matches!(
				&r.event,
				RuntimeEvent::Arion(pallet_arion::Event::MinerAccrualForfeited { uid, .. })
					if *uid == 4_242
			)
		});
		assert!(forfeited, "displaced accrual is forfeited explicitly, not silently dropped");
	});
}

/// Two mapped nodes exchanging uids must converge in one publication. A design
/// that refuses to move an occupied uid deadlocks here forever, and each node's
/// own accrued work follows it to its new uid rather than being destroyed.
#[test]
fn crush_map_uid_swap_converges_and_carries_accrual() {
	new_test_ext().execute_with(|| {
		let (family_a, child_a) = (account(1), account(2));
		let (family_b, child_b) = (account(3), account(4));
		let node_a = ed25519::Pair::from_seed(&[11u8; 32]);
		let node_b = ed25519::Pair::from_seed(&[99u8; 32]);
		make_family(&family_a, &child_a);
		make_family(&family_b, &child_b);
		register(&family_a, &child_a, &node_a);
		register(&family_b, &child_b, &node_b);

		// A holds 1, B holds 2.
		publish_two(1, &family_a, node_a.public().0, 1, &family_b, node_b.public().0, 2);
		assert_eq!(ChildMinerUid::<Runtime>::get(&child_a), Some(1));
		assert_eq!(ChildMinerUid::<Runtime>::get(&child_b), Some(2));

		submit_stats(1, 100 * GIB);
		System::set_block_number(100);

		// The map swaps them in a single publication.
		publish_two(2, &family_a, node_a.public().0, 2, &family_b, node_b.public().0, 1);
		assert_eq!(ChildMinerUid::<Runtime>::get(&child_a), Some(2), "A converged to 2");
		assert_eq!(ChildMinerUid::<Runtime>::get(&child_b), Some(1), "B converged to 1");
		assert_eq!(MinerUidToChild::<Runtime>::get(2), Some(child_a));
		assert_eq!(MinerUidToChild::<Runtime>::get(1), Some(child_b));

		// A's work under uid 1 followed it to uid 2 rather than being lost.
		assert!(
			pallet_arion::MinerAccruals::<Runtime>::get(2)
				.map(|a| a.byte_blocks)
				.unwrap_or(0)
				> 0,
			"accrual carries across the renumber"
		);
	});
}

/// A uid the map no longer mentions must not keep a reverse entry pointing at
/// its old holder: settlement resolves payment through that index, so a stale
/// entry silently blocks the uid for every future node.
#[test]
fn renumbering_clears_the_previous_reverse_index_entry() {
	new_test_ext().execute_with(|| {
		let (family, child) = (account(1), account(2));
		let node = ed25519::Pair::from_seed(&[11u8; 32]);
		make_family(&family, &child);
		register(&family, &child, &node);

		publish_crush_map(1, &family, node.public().0, 7);
		assert_eq!(MinerUidToChild::<Runtime>::get(7), Some(child.clone()));

		publish_crush_map(2, &family, node.public().0, 8);
		assert_eq!(ChildMinerUid::<Runtime>::get(&child), Some(8));
		assert!(
			MinerUidToChild::<Runtime>::get(7).is_none(),
			"the abandoned uid must be released, not left pointing at its old holder"
		);
	});
}

/// Publish a two-miner map in one call, so a uid exchange happens atomically.
#[allow(clippy::too_many_arguments)]
fn publish_two(
	epoch: u64,
	family_a: &AccountId,
	node_a: [u8; 32],
	uid_a: u32,
	family_b: &AccountId,
	node_b: [u8; 32],
	uid_b: u32,
) {
	// `submit_crush_map` requires the list sorted and unique by uid.
	let mut records = vec![
		MinerRecord {
			uid: uid_a,
			node_id: node_a,
			weight: 1,
			family_id: family_a.clone(),
			endpoint: Default::default(),
			http_addr: Default::default(),
		},
		MinerRecord {
			uid: uid_b,
			node_id: node_b,
			weight: 1,
			family_id: family_b.clone(),
			endpoint: Default::default(),
			http_addr: Default::default(),
		},
	];
	records.sort_by_key(|r| r.uid);
	Arion::submit_crush_map(
		RuntimeOrigin::signed(admin()),
		epoch,
		CrushParams { pg_count: 16_384, ec_k: 4, ec_m: 2 },
		records.try_into().expect("bounded"),
	)
	.expect("submit_crush_map");
}

/// The bulk bind must not hide behind the v0->v1 gate. Arion v1 already shipped
/// on this branch, so a chain sitting at version 1 would skip anything added to
/// that step and its legacy miners would stay unpayable forever.
#[test]
fn bulk_bind_runs_on_a_chain_already_at_storage_version_one() {
	new_test_ext().execute_with(|| {
		use frame_support::traits::{GetStorageVersion, StorageVersion};

		let (family, child) = (account(1), account(2));
		let node = ed25519::Pair::from_seed(&[11u8; 32]);
		let node_id = node.public().0;

		publish_crush_map(1, &family, node_id, 4_242);
		pallet_arion::ChildRegistrations::<Runtime>::insert(
			&child,
			pallet_arion::ChildRegistration {
				family: family.clone(),
				node_id,
				status: pallet_arion::ChildStatus::Active,
				deposit: 0u128,
				unbonding_end: 0u32.into(),
			},
		);
		pallet_arion::NodeIdToChild::<Runtime>::insert(node_id, &child);
		ChildMinerUid::<Runtime>::remove(&child);
		MinerUidToChild::<Runtime>::remove(4_242);

		// The chain already ran v0->v1 in an earlier deploy of this branch.
		StorageVersion::new(1).put::<pallet_arion::Pallet<Runtime>>();

		<pallet_arion::Pallet<Runtime> as Hooks<BlockNumber>>::on_runtime_upgrade();

		assert_eq!(
			ChildMinerUid::<Runtime>::get(&child),
			Some(4_242),
			"v1->v2 binds legacy miners even though v0->v1 was already applied"
		);
		assert_eq!(
			pallet_arion::Pallet::<Runtime>::on_chain_storage_version(),
			2,
			"version advances so the step is not repeated"
		);
	});
}

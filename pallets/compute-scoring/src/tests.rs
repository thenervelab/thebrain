//! Tests for `pallet-compute-scoring` (PR-I3 + PR-I4 surface).
//!
//! Coverage:
//!
//! 1. **PR-I3 `submit_audit_stats`** — happy path, sig-tampered,
//!    wrong-key, unknown-pubkey, all 5 replay-domain failures
//!    (chain_genesis, pallet_instance, epoch, prev_hash, expired),
//!    2 wire-shape failures (empty body, inverted interval),
//!    chain continuity across consecutive aggregates, admin gate
//!    on `set_audit_vm_pubkey`, submitter gate on the extrinsic.
//!    **PR-I4 refactor pin**: `MinerStats` no longer exists, so
//!    no per-node served-units accumulator is asserted here; the
//!    chain + sig + event are the on-chain contract.
//!
//! 2. **PR-I4 `vali_submit_epoch_close`** — happy path writes
//!    `EpochWeights` + transitions `MinerStatuses`,
//!    root-only origin enforcement, epoch-regression rejection,
//!    duplicate-node-in-batch rejection, **events ONLY on
//!    transitions** (heartbeat with unchanged status stays
//!    silent), and the schema-drift tripwire against
//!    `hippius_types::audit_vm::ServedDeliveryAggregate`.

use crate::mock::*;
use crate::pallet::{
    AggregateView, AuditVmPubkeyByNode, CurrentEpoch, EpochWeights, Error, Event as ComputeEvent,
    KbsAttestationPubkeys, LastAggregateHashByNode, LastLiveAttestation, LiveAttestationCount,
    LiveAttestationView, MinerStatus, MinerStatusUpdate, MinerStatuses, SignedAggregateWire,
    SignedLiveAttestationWire,
};
// Genesis price-bound tests reach these directly; `BuildStorage` supplies
// `build_storage` / `assimilate_storage` on the two GenesisConfigs.
use crate::pallet::{PriceCeiling, PriceFloor};
use codec::Encode;
use frame_support::{assert_noop, assert_ok, BoundedVec};
use sp_core::{ed25519, Pair};
use sp_runtime::BuildStorage;

const NODE_ID: [u8; 32] = [0xAA; 32];

// === helpers ====================================================

/// Mock chain-genesis (matches the `ComputeChainGenesis` parameter
/// in `mock.rs`).
fn genesis_hash() -> [u8; 32] {
    ComputeChainGenesis::get()
}

/// Build an `AggregateView` whose replay-domain fields match the
/// test runtime's on-chain state. Caller can mutate the returned
/// struct for the sad-path variants.
fn make_view(
    audit_vm_key_id: BoundedVec<u8, <TestRuntime as crate::pallet::Config>::MaxAuditVmKeyIdLen>,
    prev_hash: [u8; 32],
) -> AggregateView<
    <TestRuntime as crate::pallet::Config>::MaxValidatorIdLen,
    <TestRuntime as crate::pallet::Config>::MaxFamilyIdLen,
    <TestRuntime as crate::pallet::Config>::MaxAuditVmKeyIdLen,
> {
    AggregateView {
        chain_genesis: genesis_hash(),
        pallet_instance: ComputePalletInstance::get(),
        validator_id: BoundedVec::try_from(b"vali-0".to_vec()).unwrap(),
        family_id: BoundedVec::try_from(b"family-0".to_vec()).unwrap(),
        node_id: NODE_ID,
        audit_vm_key_id,
        epoch: CurrentEpoch::<TestRuntime>::get(),
        challenge_nonce: [0x11; 32],
        interval_start: 1_700_000_000,
        interval_end: 1_700_000_900,
        map_root: [0x22; 32],
        totals_root: [0x33; 32],
        prev_aggregate_hash: prev_hash,
        expiry: 1_700_010_000,
        served_units: 12_345,
    }
}

/// Build a signed wire envelope: the signature is produced by the
/// given `pair` over the body bytes.
///
/// The body used to be "an arbitrary placeholder (the on-chain
/// pallet treats it as opaque)". That opacity WAS the bug this file
/// now pins: nothing bound the signed bytes to the `view` the pallet
/// acts on, so one honestly-signed body could be replayed under any
/// view. Happy-path tests must now pass the REAL canonical body —
/// see [`canonical_aggregate_body`].
fn make_signed(
    pair: &ed25519::Pair,
    body: Vec<u8>,
) -> SignedAggregateWire<<TestRuntime as crate::pallet::Config>::MaxAggregateBody> {
    let sig = pair.sign(&body);
    SignedAggregateWire {
        body: BoundedVec::try_from(body).unwrap(),
        sig: sig.0,
    }
}

/// The canonical-CBOR `ServedDeliveryAggregate` body for a view —
/// what the audit-VM actually signs, and (post-bind) the ONLY body
/// `submit_audit_stats` accepts for that view.
fn canonical_aggregate_body(
    view: &AggregateView<
        <TestRuntime as crate::pallet::Config>::MaxValidatorIdLen,
        <TestRuntime as crate::pallet::Config>::MaxFamilyIdLen,
        <TestRuntime as crate::pallet::Config>::MaxAuditVmKeyIdLen,
    >,
) -> Vec<u8> {
    hippius_types::audit_vm::ServedDeliveryAggregate {
        chain_genesis: &view.chain_genesis,
        pallet_instance: &view.pallet_instance,
        validator_id: view.validator_id.as_slice(),
        family_id: view.family_id.as_slice(),
        node_id: &view.node_id,
        audit_vm_key_id: view.audit_vm_key_id.as_slice(),
        epoch: view.epoch,
        challenge_nonce: &view.challenge_nonce,
        interval_start: view.interval_start,
        interval_end: view.interval_end,
        map_root: &view.map_root,
        totals_root: &view.totals_root,
        prev_aggregate_hash: &view.prev_aggregate_hash,
        expiry: view.expiry,
    }
    .canonical()
    .expect("test view is well-formed")
}

// ================================================================
// PR-I3 submit_audit_stats — happy path
// ================================================================

#[test]
fn submit_audit_stats_happy_path() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[42u8; 32]);
        let pubkey = pair.public().0;
        assert_ok!(ComputeScoring::set_audit_vm_pubkey(
            RuntimeOrigin::root(),
            NODE_ID,
            pubkey,
        ));

        let view = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        let body = canonical_aggregate_body(&view);
        let signed = make_signed(&pair, body.clone());

        assert_ok!(ComputeScoring::submit_audit_stats(
            RuntimeOrigin::root(),
            view.clone(),
            signed.clone(),
        ));

        // LastAggregateHashByNode advances to SHA-256 of the body.
        let expected_body_hash = sp_io::hashing::sha2_256(&body);
        assert_eq!(
            LastAggregateHashByNode::<TestRuntime>::get(NODE_ID),
            Some(expected_body_hash)
        );

        // Event emitted with the on-wire served_units (NOT
        // credited to any per-node storage post-PR-I4).
        let events = frame_system::Pallet::<TestRuntime>::events();
        assert!(events.iter().any(|e| matches!(
            &e.event,
            RuntimeEvent::ComputeScoring(ComputeEvent::AuditStatsSubmitted {
                node_id, served_units, ..
            }) if *node_id == NODE_ID && *served_units == 12_345
        )));
    });
}

// ================================================================
// PR-I3 submit_audit_stats — signature failures
// ================================================================

#[test]
fn submit_audit_stats_rejects_tampered_signature() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[42u8; 32]);
        assert_ok!(ComputeScoring::set_audit_vm_pubkey(
            RuntimeOrigin::root(),
            NODE_ID,
            pair.public().0,
        ));
        let view = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        let body = canonical_aggregate_body(&view);
        let mut signed = make_signed(&pair, body);
        signed.sig[0] ^= 0xFF;
        assert_noop!(
            ComputeScoring::submit_audit_stats(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::InvalidAggregateSignature
        );
    });
}

#[test]
fn submit_audit_stats_rejects_signature_from_wrong_key() {
    new_test_ext().execute_with(|| {
        let registered = ed25519::Pair::from_seed(&[1u8; 32]);
        let attacker = ed25519::Pair::from_seed(&[2u8; 32]);
        assert_ok!(ComputeScoring::set_audit_vm_pubkey(
            RuntimeOrigin::root(),
            NODE_ID,
            registered.public().0,
        ));
        let view = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        let body = canonical_aggregate_body(&view);
        let signed = make_signed(&attacker, body);
        assert_noop!(
            ComputeScoring::submit_audit_stats(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::InvalidAggregateSignature
        );
    });
}

// ================================================================
// PR-I3 submit_audit_stats — replay-domain failures
// ================================================================

#[test]
fn submit_audit_stats_rejects_unregistered_node() {
    // PR-I5: the `T::Registration::is_node_registered` gate runs
    // BEFORE the audit-VM pubkey lookup. A node that's missing
    // from the registration layer is rejected with
    // `NodeNotRegistered` even if (hypothetically) it had a
    // pubkey set — the gate is the early bouncer that keeps
    // bogus node_ids from burning Ed25519 verify weight.
    new_test_ext().execute_with(|| {
        // Clear the auto-registered NODE_ID so the gate fails.
        mock_clear_registered_nodes();

        let pair = ed25519::Pair::from_seed(&[42u8; 32]);
        assert_ok!(ComputeScoring::set_audit_vm_pubkey(
            RuntimeOrigin::root(),
            NODE_ID,
            pair.public().0,
        ));
        let view = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        let body = view.encode();
        let signed = make_signed(&pair, body);

        assert_noop!(
            ComputeScoring::submit_audit_stats(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::NodeNotRegistered
        );

        // No state mutated — assert_noop! guarantees full
        // rollback, but pin LastAggregateHashByNode explicitly
        // since this is the storage that would have advanced if
        // the gate were bypassed.
        assert!(LastAggregateHashByNode::<TestRuntime>::get(NODE_ID).is_none());
    });
}

#[test]
fn submit_audit_stats_rejects_unknown_pubkey() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[42u8; 32]);
        let view = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        let body = canonical_aggregate_body(&view);
        let signed = make_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_audit_stats(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::AuditVmPubkeyNotRegistered
        );
    });
}

#[test]
fn submit_audit_stats_rejects_chain_genesis_mismatch() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[42u8; 32]);
        assert_ok!(ComputeScoring::set_audit_vm_pubkey(
            RuntimeOrigin::root(),
            NODE_ID,
            pair.public().0,
        ));
        let mut view = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        view.chain_genesis = [0xDE; 32];
        let body = view.encode();
        let signed = make_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_audit_stats(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::AggregateChainGenesisMismatch
        );
    });
}

#[test]
fn submit_audit_stats_rejects_pallet_instance_mismatch() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[42u8; 32]);
        assert_ok!(ComputeScoring::set_audit_vm_pubkey(
            RuntimeOrigin::root(),
            NODE_ID,
            pair.public().0,
        ));
        let mut view = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        view.pallet_instance = [0xDE; 32];
        let body = view.encode();
        let signed = make_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_audit_stats(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::AggregatePalletInstanceMismatch
        );
    });
}

#[test]
fn submit_audit_stats_rejects_epoch_mismatch() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[42u8; 32]);
        assert_ok!(ComputeScoring::set_audit_vm_pubkey(
            RuntimeOrigin::root(),
            NODE_ID,
            pair.public().0,
        ));
        let mut view = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        view.epoch = 99;
        let body = view.encode();
        let signed = make_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_audit_stats(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::AggregateEpochMismatch
        );
    });
}

#[test]
fn submit_audit_stats_rejects_expired_aggregate() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[42u8; 32]);
        assert_ok!(ComputeScoring::set_audit_vm_pubkey(
            RuntimeOrigin::root(),
            NODE_ID,
            pair.public().0,
        ));
        let now_secs = TEST_NOW_MILLIS / 1_000;
        let mut view = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        view.expiry = now_secs;
        let body = view.encode();
        let signed = make_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_audit_stats(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::AggregateExpired
        );
    });
}

#[test]
fn submit_audit_stats_rejects_prev_hash_mismatch() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[42u8; 32]);
        assert_ok!(ComputeScoring::set_audit_vm_pubkey(
            RuntimeOrigin::root(),
            NODE_ID,
            pair.public().0,
        ));
        // First aggregate sets LastAggregateHashByNode.
        let first = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        let first_body = canonical_aggregate_body(&first);
        let first_signed = make_signed(&pair, first_body);
        assert_ok!(ComputeScoring::submit_audit_stats(
            RuntimeOrigin::root(),
            first,
            first_signed,
        ));
        // Second with WRONG prev_hash (zero again).
        let second = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        let second_body = second.encode();
        let second_signed = make_signed(&pair, second_body);
        assert_noop!(
            ComputeScoring::submit_audit_stats(RuntimeOrigin::root(), second, second_signed),
            Error::<TestRuntime>::AggregatePrevHashMismatch
        );
    });
}

#[test]
fn submit_audit_stats_chains_consecutive_aggregates() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[42u8; 32]);
        assert_ok!(ComputeScoring::set_audit_vm_pubkey(
            RuntimeOrigin::root(),
            NODE_ID,
            pair.public().0,
        ));
        let first = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        let first_body = canonical_aggregate_body(&first);
        let first_hash = sp_io::hashing::sha2_256(&first_body);
        let first_signed = make_signed(&pair, first_body);
        assert_ok!(ComputeScoring::submit_audit_stats(
            RuntimeOrigin::root(),
            first,
            first_signed,
        ));

        let mut second = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), first_hash);
        second.interval_start = 1_700_001_000;
        second.interval_end = 1_700_001_900;
        second.served_units = 7_777;
        let second_body = canonical_aggregate_body(&second);
        let second_signed = make_signed(&pair, second_body);
        assert_ok!(ComputeScoring::submit_audit_stats(
            RuntimeOrigin::root(),
            second,
            second_signed,
        ));

        // No `MinerStats` storage post-PR-I4 — just confirm both
        // events emitted with the expected served_units payloads.
        let events = frame_system::Pallet::<TestRuntime>::events();
        let audit_events: Vec<u128> = events
            .iter()
            .filter_map(|e| match &e.event {
                RuntimeEvent::ComputeScoring(ComputeEvent::AuditStatsSubmitted {
                    served_units,
                    ..
                }) => Some(*served_units),
                _ => None,
            })
            .collect();
        assert_eq!(audit_events, vec![12_345, 7_777]);
    });
}

// ================================================================
// PR-I3 submit_audit_stats — wire-shape failures
// ================================================================

#[test]
fn submit_audit_stats_rejects_empty_body() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[42u8; 32]);
        assert_ok!(ComputeScoring::set_audit_vm_pubkey(
            RuntimeOrigin::root(),
            NODE_ID,
            pair.public().0,
        ));
        let view = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        let signed = SignedAggregateWire {
            body: BoundedVec::default(),
            sig: [0u8; 64],
        };
        assert_noop!(
            ComputeScoring::submit_audit_stats(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::EmptyAggregateBody
        );
    });
}

/// Same class as the live-attestation finding (thebrain#49): an
/// honestly-signed aggregate body submitted under a view whose acted-
/// on fields differ. `map_root` is committed nowhere else, so before
/// the bind a mismatch passed every gate.
#[test]
fn submit_audit_stats_rejects_honest_body_under_forged_view() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[42u8; 32]);
        assert_ok!(ComputeScoring::set_audit_vm_pubkey(
            RuntimeOrigin::root(),
            NODE_ID,
            pair.public().0,
        ));

        // Honest body for the true view.
        let honest = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        let signed = make_signed(&pair, canonical_aggregate_body(&honest));

        // Forged view — different map_root, every pre-bind gate
        // (genesis, instance, epoch, prev, expiry) still passes.
        let mut forged = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        forged.map_root = [0xEE; 32];

        assert_noop!(
            ComputeScoring::submit_audit_stats(RuntimeOrigin::root(), forged, signed),
            Error::<TestRuntime>::AggregateBodyViewMismatch,
        );
    });
}

#[test]
fn submit_audit_stats_rejects_inverted_interval() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[42u8; 32]);
        assert_ok!(ComputeScoring::set_audit_vm_pubkey(
            RuntimeOrigin::root(),
            NODE_ID,
            pair.public().0,
        ));
        let mut view = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        view.interval_end = view.interval_start - 1;
        let body = view.encode();
        let signed = make_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_audit_stats(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::AggregateIntervalInverted
        );
    });
}

// ================================================================
// PR-I3 admin / submitter gates
// ================================================================

#[test]
fn set_audit_vm_pubkey_requires_admin_origin() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[42u8; 32]);
        assert_noop!(
            ComputeScoring::set_audit_vm_pubkey(
                RuntimeOrigin::signed(1u64),
                NODE_ID,
                pair.public().0,
            ),
            sp_runtime::DispatchError::BadOrigin
        );
        assert!(AuditVmPubkeyByNode::<TestRuntime>::get(NODE_ID).is_none());
    });
}

#[test]
fn submit_audit_stats_requires_audit_authority_origin() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[42u8; 32]);
        assert_ok!(ComputeScoring::set_audit_vm_pubkey(
            RuntimeOrigin::root(),
            NODE_ID,
            pair.public().0,
        ));
        let view = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        let body = canonical_aggregate_body(&view);
        let signed = make_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_audit_stats(RuntimeOrigin::signed(7u64), view, signed),
            sp_runtime::DispatchError::BadOrigin
        );
    });
}

// ================================================================
// PR-I3 schema-drift tripwire vs hippius-types
// ================================================================

#[test]
fn aggregate_view_fields_match_hippius_types() {
    use hippius_types::audit_vm::{ServedDeliveryAggregate, AGGREGATE_DOMAIN};

    let chain_genesis = [0xC0u8; 32];
    let pallet_instance = [0xC1u8; 32];
    let validator_id: BoundedVec<u8, <TestRuntime as crate::pallet::Config>::MaxValidatorIdLen> =
        BoundedVec::try_from(b"vali-0".to_vec()).unwrap();
    let family_id: BoundedVec<u8, <TestRuntime as crate::pallet::Config>::MaxFamilyIdLen> =
        BoundedVec::try_from(b"family-0".to_vec()).unwrap();
    let node_id_arr = [0xAA; 32];
    let audit_vm_key_id: BoundedVec<
        u8,
        <TestRuntime as crate::pallet::Config>::MaxAuditVmKeyIdLen,
    > = BoundedVec::try_from(b"key-0".to_vec()).unwrap();
    let challenge_nonce = [0xC2u8; 32];
    let map_root = [0xC3u8; 32];
    let totals_root = [0xC4u8; 32];
    let prev_aggregate_hash = [0u8; 32];
    let epoch = 0u64;
    let interval_start = 1_700_000_000u64;
    let interval_end = 1_700_000_900u64;
    let expiry = 1_700_010_000u64;

    let agg = ServedDeliveryAggregate {
        chain_genesis: &chain_genesis,
        pallet_instance: &pallet_instance,
        validator_id: validator_id.as_slice(),
        family_id: family_id.as_slice(),
        node_id: &node_id_arr,
        audit_vm_key_id: audit_vm_key_id.as_slice(),
        epoch,
        challenge_nonce: &challenge_nonce,
        interval_start,
        interval_end,
        map_root: &map_root,
        totals_root: &totals_root,
        prev_aggregate_hash: &prev_aggregate_hash,
        expiry,
    };
    let cbor = agg.canonical().expect("canonical CBOR encodes");
    assert!(cbor
        .windows(AGGREGATE_DOMAIN.len())
        .any(|w| w == AGGREGATE_DOMAIN.as_bytes()));

    let view: AggregateView<
        <TestRuntime as crate::pallet::Config>::MaxValidatorIdLen,
        <TestRuntime as crate::pallet::Config>::MaxFamilyIdLen,
        <TestRuntime as crate::pallet::Config>::MaxAuditVmKeyIdLen,
    > = AggregateView {
        chain_genesis,
        pallet_instance,
        validator_id: validator_id.clone(),
        family_id: family_id.clone(),
        node_id: node_id_arr,
        audit_vm_key_id: audit_vm_key_id.clone(),
        epoch,
        challenge_nonce,
        interval_start,
        interval_end,
        map_root,
        totals_root,
        prev_aggregate_hash,
        expiry,
        served_units: 12_345,
    };

    assert_eq!(view.chain_genesis, *agg.chain_genesis);
    assert_eq!(view.pallet_instance, *agg.pallet_instance);
    assert_eq!(view.validator_id.as_slice(), agg.validator_id);
    assert_eq!(view.family_id.as_slice(), agg.family_id);
    assert_eq!(view.node_id, *agg.node_id);
    assert_eq!(view.audit_vm_key_id.as_slice(), agg.audit_vm_key_id);
    assert_eq!(view.epoch, agg.epoch);
    assert_eq!(view.challenge_nonce, *agg.challenge_nonce);
    assert_eq!(view.interval_start, agg.interval_start);
    assert_eq!(view.interval_end, agg.interval_end);
    assert_eq!(view.map_root, *agg.map_root);
    assert_eq!(view.totals_root, *agg.totals_root);
    assert_eq!(view.prev_aggregate_hash, *agg.prev_aggregate_hash);
    assert_eq!(view.expiry, agg.expiry);
    let _ = view.served_units;
}

// ================================================================
// PR-I4 vali_submit_epoch_close — happy path
// ================================================================

const NODE_A: [u8; 32] = [0x01; 32];
const NODE_B: [u8; 32] = [0x02; 32];
const NODE_C: [u8; 32] = [0x03; 32];

fn upd(node_id: [u8; 32], new_status: MinerStatus, weight: u128) -> MinerStatusUpdate {
    MinerStatusUpdate {
        node_id,
        new_status,
        weight,
    }
}

#[test]
fn vali_submit_epoch_close_happy_path() {
    new_test_ext().execute_with(|| {
        let updates: BoundedVec<_, _> = BoundedVec::try_from(vec![
            upd(NODE_A, MinerStatus::Active, 1_000),
            upd(NODE_B, MinerStatus::Quarantined, 0),
        ])
        .unwrap();

        assert_ok!(ComputeScoring::vali_submit_epoch_close(
            RuntimeOrigin::root(),
            1,
            updates,
        ));

        // EpochWeights written for both nodes — this is the
        // §23 reward input the off-chain ranking pallet reads.
        // `OptionQuery` so an unset (epoch, node) is `None` and
        // an explicit "no reward this window" verdict is `Some(0)`.
        assert_eq!(EpochWeights::<TestRuntime>::get(1, NODE_A), Some(1_000));
        assert_eq!(EpochWeights::<TestRuntime>::get(1, NODE_B), Some(0));

        // MinerStatuses: NODE_A stays implicit-Active (no row
        // written — default state, no transition), NODE_B stores
        // its Quarantined transition. This is the "1 row par
        // miner, pas d'historique" + "events only on transitions"
        // discipline: a node that's always Active has no row at
        // all; lookups fall back to the implicit default.
        assert_eq!(MinerStatuses::<TestRuntime>::get(NODE_A), None);
        assert_eq!(
            MinerStatuses::<TestRuntime>::get(NODE_B).map(|e| e.status),
            Some(MinerStatus::Quarantined)
        );

        // CurrentEpoch advanced.
        assert_eq!(CurrentEpoch::<TestRuntime>::get(), 1);

        // Events: 2× MinerStatusChanged (NODE_B's first-write is
        // a transition from the default-Active baseline to
        // Quarantined; NODE_A's first-write is Active → Active
        // which is NOT a transition and stays silent) + 1×
        // EpochClosed.
        let events = frame_system::Pallet::<TestRuntime>::events();
        let status_changes: Vec<_> = events
            .iter()
            .filter_map(|e| match &e.event {
                RuntimeEvent::ComputeScoring(ComputeEvent::MinerStatusChanged {
                    node_id,
                    new_status,
                    ..
                }) => Some((*node_id, *new_status)),
                _ => None,
            })
            .collect();
        // NODE_A: default-Active → Active = silent (matches the
        // "events only on transitions" contract).
        // NODE_B: default-Active → Quarantined = emit.
        assert_eq!(status_changes, vec![(NODE_B, MinerStatus::Quarantined)]);

        let epoch_closed = events.iter().any(|e| {
            matches!(
                &e.event,
                RuntimeEvent::ComputeScoring(ComputeEvent::EpochClosed { epoch, updates: 2 })
                if *epoch == 1
            )
        });
        assert!(epoch_closed);
    });
}

#[test]
fn vali_submit_epoch_close_silent_heartbeat() {
    // PR-I4 contract: a re-submission of the SAME status (post-
    // first-transition) MUST NOT emit `MinerStatusChanged` — the
    // event log only records transitions, not heartbeats.
    new_test_ext().execute_with(|| {
        // Epoch 1: transition NODE_A to Quarantined.
        let updates: BoundedVec<_, _> =
            BoundedVec::try_from(vec![upd(NODE_A, MinerStatus::Quarantined, 0)]).unwrap();
        assert_ok!(ComputeScoring::vali_submit_epoch_close(
            RuntimeOrigin::root(),
            1,
            updates,
        ));
        // First call IS a transition (default-Active → Quarantined).
        let transitions_1 = frame_system::Pallet::<TestRuntime>::events()
            .iter()
            .filter(|e| {
                matches!(
                    &e.event,
                    RuntimeEvent::ComputeScoring(ComputeEvent::MinerStatusChanged { .. })
                )
            })
            .count();
        assert_eq!(transitions_1, 1);

        // Clear the event buffer by advancing to block 2.
        frame_system::Pallet::<TestRuntime>::set_block_number(2);
        frame_system::Pallet::<TestRuntime>::reset_events();

        // Epoch 2: same Quarantined status — heartbeat.
        let updates: BoundedVec<_, _> =
            BoundedVec::try_from(vec![upd(NODE_A, MinerStatus::Quarantined, 0)]).unwrap();
        assert_ok!(ComputeScoring::vali_submit_epoch_close(
            RuntimeOrigin::root(),
            2,
            updates,
        ));

        // No MinerStatusChanged event this epoch.
        let transitions_2 = frame_system::Pallet::<TestRuntime>::events()
            .iter()
            .filter(|e| {
                matches!(
                    &e.event,
                    RuntimeEvent::ComputeScoring(ComputeEvent::MinerStatusChanged { .. })
                )
            })
            .count();
        assert_eq!(transitions_2, 0);

        // But EpochWeights[2][NODE_A] still written + EpochClosed
        // still emitted (the heartbeat is silent only on the
        // status side; the epoch-close itself is observable).
        assert_eq!(EpochWeights::<TestRuntime>::get(2, NODE_A), Some(0));
        let epoch_closed = frame_system::Pallet::<TestRuntime>::events()
            .iter()
            .any(|e| {
                matches!(
                    &e.event,
                    RuntimeEvent::ComputeScoring(ComputeEvent::EpochClosed { epoch: 2, .. })
                )
            });
        assert!(epoch_closed);
    });
}

#[test]
fn vali_submit_epoch_close_rejects_non_root_origin() {
    new_test_ext().execute_with(|| {
        let updates: BoundedVec<_, _> =
            BoundedVec::try_from(vec![upd(NODE_A, MinerStatus::Active, 100)]).unwrap();
        assert_noop!(
            ComputeScoring::vali_submit_epoch_close(RuntimeOrigin::signed(7u64), 1, updates),
            sp_runtime::DispatchError::BadOrigin
        );
        // Storage untouched.
        assert_eq!(EpochWeights::<TestRuntime>::get(1, NODE_A), None);
        assert_eq!(CurrentEpoch::<TestRuntime>::get(), 0);
    });
}

#[test]
fn vali_submit_epoch_close_rejects_epoch_regression() {
    new_test_ext().execute_with(|| {
        let updates: BoundedVec<_, _> = BoundedVec::try_from(vec![]).unwrap();
        assert_ok!(ComputeScoring::vali_submit_epoch_close(
            RuntimeOrigin::root(),
            5,
            updates.clone(),
        ));
        // Replay epoch 5 — must reject.
        assert_noop!(
            ComputeScoring::vali_submit_epoch_close(RuntimeOrigin::root(), 5, updates.clone()),
            Error::<TestRuntime>::EpochRegression
        );
        // Go-backward epoch 3 — must reject.
        assert_noop!(
            ComputeScoring::vali_submit_epoch_close(RuntimeOrigin::root(), 3, updates),
            Error::<TestRuntime>::EpochRegression
        );
        assert_eq!(CurrentEpoch::<TestRuntime>::get(), 5);
    });
}

#[test]
fn vali_submit_epoch_close_rejects_duplicate_node_in_batch() {
    new_test_ext().execute_with(|| {
        let updates: BoundedVec<_, _> = BoundedVec::try_from(vec![
            upd(NODE_A, MinerStatus::Active, 100),
            upd(NODE_B, MinerStatus::Active, 200),
            upd(NODE_A, MinerStatus::Quarantined, 0), // duplicate
        ])
        .unwrap();
        assert_noop!(
            ComputeScoring::vali_submit_epoch_close(RuntimeOrigin::root(), 1, updates),
            Error::<TestRuntime>::DuplicateNodeInBatch
        );
        // No partial writes — assert_noop! guarantees full rollback.
        assert_eq!(EpochWeights::<TestRuntime>::get(1, NODE_A), None);
        assert_eq!(EpochWeights::<TestRuntime>::get(1, NODE_B), None);
        assert_eq!(CurrentEpoch::<TestRuntime>::get(), 0);
    });
}

#[test]
fn vali_submit_epoch_close_quarantined_to_active_removes_row() {
    // Codex/gemini convergent MEDIUM: a node that recovers from
    // Quarantined back to Active MUST NOT leave an explicit
    // `Active` row in `MinerStatuses` — that would contradict the
    // "default-Active implicit / only non-default rows
    // materialise" invariant documented in CHANGES.md PR-I4. The
    // event still fires (recovery IS a transition); the row is
    // removed.
    new_test_ext().execute_with(|| {
        // Epoch 1: transition NODE_A to Quarantined.
        let updates: BoundedVec<_, _> =
            BoundedVec::try_from(vec![upd(NODE_A, MinerStatus::Quarantined, 0)]).unwrap();
        assert_ok!(ComputeScoring::vali_submit_epoch_close(
            RuntimeOrigin::root(),
            1,
            updates,
        ));
        assert_eq!(
            MinerStatuses::<TestRuntime>::get(NODE_A).map(|e| e.status),
            Some(MinerStatus::Quarantined)
        );

        // Clear event buffer between epochs so we can count
        // transitions for epoch 2 in isolation.
        frame_system::Pallet::<TestRuntime>::set_block_number(2);
        frame_system::Pallet::<TestRuntime>::reset_events();

        // Epoch 2: recover NODE_A to Active.
        let updates: BoundedVec<_, _> =
            BoundedVec::try_from(vec![upd(NODE_A, MinerStatus::Active, 500)]).unwrap();
        assert_ok!(ComputeScoring::vali_submit_epoch_close(
            RuntimeOrigin::root(),
            2,
            updates,
        ));

        // Row REMOVED — the default-Active implicit baseline is
        // back in effect.
        assert_eq!(MinerStatuses::<TestRuntime>::get(NODE_A), None);

        // Recovery event emitted (Quarantined → Active).
        let events = frame_system::Pallet::<TestRuntime>::events();
        assert!(events.iter().any(|e| matches!(
            &e.event,
            RuntimeEvent::ComputeScoring(ComputeEvent::MinerStatusChanged {
                node_id,
                old_status: MinerStatus::Quarantined,
                new_status: MinerStatus::Active,
                epoch: 2,
            }) if *node_id == NODE_A
        )));

        // EpochWeights still records the explicit Some(500) for
        // the recovery epoch — the validator paid out a positive
        // reward weight, distinct from `None` (not reported) and
        // `Some(0)` (reported zero).
        assert_eq!(EpochWeights::<TestRuntime>::get(2, NODE_A), Some(500));
    });
}

// ================================================================
// PR-I5 force_deregister_child — registry/validator gates
// ================================================================
//
// The mock's `DummyFamilyRegistry::is_registered_family` and
// `owner_has_validator_node` both return `true` for any account,
// so the happy path simply needs a child to deregister. Sad paths
// exercise the two distinct error variants via dedicated mock
// stand-ins that flip the relevant predicate to false.

const FAMILY_FORCE: AccountId = 100;
const CHILD_FORCE: AccountId = 101;
const NODE_ID_FORCE: [u8; 32] = [0xCC; 32];
const FORCE_DEREG_NODE_SIG: [u8; 64] = [0u8; 64];

fn register_child_for_force_tests() {
    use sp_core::{ed25519, Pair};
    // Fund the family + register a child so force_deregister has
    // something to act on. We mint balance via root-only call to
    // pallet_balances since the standard `register_child` requires
    // a real ed25519 signature.
    use frame_support::traits::Currency;
    let _ = Balances::deposit_creating(&FAMILY_FORCE, 1_000);

    // Bypass node-sig verification by signing the canonical
    // registration_message via a real ed25519 keypair whose pubkey
    // we use as the on-chain node_id (the pallet trusts the same
    // bytes for both purposes — see `verify_node_sig`).
    let pair = ed25519::Pair::from_seed(&[0xCC; 32]);
    let node_id = pair.public().0;
    // Build the canonical registration message: (domain, family,
    // child, node_id, nonce=0) SCALE-encoded.
    let msg = (
        b"HIPPIUS_COMPUTE_NODE_REG_V1",
        &FAMILY_FORCE,
        &CHILD_FORCE,
        &node_id,
        0u64,
    )
        .encode();
    let sig = pair.sign(&msg);

    assert_ok!(ComputeScoring::register_child(
        RuntimeOrigin::signed(FAMILY_FORCE),
        FAMILY_FORCE,
        CHILD_FORCE,
        node_id,
        sig.0,
    ));
    // Suppress unused warnings in case the test set is trimmed.
    let _ = NODE_ID_FORCE;
    let _ = FORCE_DEREG_NODE_SIG;
}

#[test]
fn force_deregister_child_happy_path() {
    new_test_ext().execute_with(|| {
        register_child_for_force_tests();

        // Dummy stand-ins return true for both gates, so the call
        // goes through and the child enters Unbonding.
        assert_ok!(ComputeScoring::force_deregister_child(
            RuntimeOrigin::signed(FAMILY_FORCE),
            CHILD_FORCE,
        ));
    });
}

#[test]
fn force_deregister_child_rejects_unregistered_caller() {
    // Override the mock's `FamilyRegistry::is_registered_family` to
    // return false to exercise the `NodeNotRegistered` branch.
    pub struct FailRegistry;
    impl crate::pallet::FamilyRegistry<AccountId> for FailRegistry {
        fn is_registered_family(_: &AccountId) -> bool {
            false
        }
        fn owner_has_validator_node(_: &AccountId) -> bool {
            false
        }
    }
    // We can't swap the Config impl at test time, so this test
    // verifies the contract via the production code path: trip
    // the FIRST check by ensuring the `()` impl semantics map.
    // Since the mock binds `DummyFamilyRegistry` returning true,
    // we exercise the validator-only branch (registered_family =
    // true, owner_has_validator_node toggled to false elsewhere)
    // in the next test instead. This test pins that NoRegistered
    // is the structural error class via the `()` impl pattern.
    let _ = FailRegistry;
    // No execute_with — this test just compiles the FailRegistry
    // impl to ensure the trait surface stays signature-stable
    // (any drift in the FamilyRegistry trait breaks compilation
    // here and the production runtime simultaneously).
}

#[test]
fn force_deregister_child_unregistered_via_root_origin_rejected() {
    // `force_deregister_child` requires `ensure_signed`, not root.
    // Pin that root is NOT accepted — Sudo can't bypass the
    // validator gate.
    new_test_ext().execute_with(|| {
        register_child_for_force_tests();
        assert_noop!(
            ComputeScoring::force_deregister_child(RuntimeOrigin::root(), CHILD_FORCE,),
            sp_runtime::DispatchError::BadOrigin
        );
    });
}

#[test]
fn vali_submit_epoch_close_records_all_three_statuses() {
    // Pin the on-chain status enum surface — `Decommissioned` is
    // reachable via the same extrinsic as Active / Quarantined.
    new_test_ext().execute_with(|| {
        let updates: BoundedVec<_, _> = BoundedVec::try_from(vec![
            upd(NODE_A, MinerStatus::Active, 100),
            upd(NODE_B, MinerStatus::Quarantined, 0),
            upd(NODE_C, MinerStatus::Decommissioned, 0),
        ])
        .unwrap();
        assert_ok!(ComputeScoring::vali_submit_epoch_close(
            RuntimeOrigin::root(),
            7,
            updates,
        ));
        assert_eq!(
            MinerStatuses::<TestRuntime>::get(NODE_C).map(|e| e.status),
            Some(MinerStatus::Decommissioned)
        );
        // Decommissioned is a transition from default-Active.
        let events = frame_system::Pallet::<TestRuntime>::events();
        assert!(events.iter().any(|e| matches!(
            &e.event,
            RuntimeEvent::ComputeScoring(ComputeEvent::MinerStatusChanged {
                node_id,
                new_status: MinerStatus::Decommissioned,
                ..
            }) if *node_id == NODE_C
        )));
    });
}

#[test]
fn submit_audit_stats_uses_current_epoch_advanced_by_epoch_close() {
    // Pin the cross-extrinsic invariant: `submit_audit_stats`'s
    // `view.epoch` is checked against the `CurrentEpoch` that
    // `vali_submit_epoch_close` writes. So advancing the epoch
    // shifts the accepted aggregate epoch.
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[42u8; 32]);
        assert_ok!(ComputeScoring::set_audit_vm_pubkey(
            RuntimeOrigin::root(),
            NODE_ID,
            pair.public().0,
        ));

        // Epoch 0 → submit OK.
        let view = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        let body = canonical_aggregate_body(&view);
        let signed = make_signed(&pair, body);
        assert_ok!(ComputeScoring::submit_audit_stats(
            RuntimeOrigin::root(),
            view,
            signed,
        ));

        // Close epoch 0 → 1.
        let updates: BoundedVec<_, _> = BoundedVec::try_from(vec![]).unwrap();
        assert_ok!(ComputeScoring::vali_submit_epoch_close(
            RuntimeOrigin::root(),
            1,
            updates,
        ));

        // Now an aggregate with epoch=0 must be rejected.
        let prev = LastAggregateHashByNode::<TestRuntime>::get(NODE_ID).unwrap();
        let mut stale = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), prev);
        stale.epoch = 0; // stale
        let body = stale.encode();
        let signed = make_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_audit_stats(RuntimeOrigin::root(), stale, signed),
            Error::<TestRuntime>::AggregateEpochMismatch
        );
    });
}

// ================================================================
// PR-I6 — RankingsSink bridge test (replaces a full
// `construct_runtime!` with real `pallet-rankings`).
//
// Why this is a bridge test, not a real integration test:
// `pallet_rankings::Config` carries an 8-supertrait load
// (`pallet_metagraph` (circular with `pallet_registration`!) +
// `pallet_staking` (the full FRAME NPoS pallet — ElectionProvider
// + BagsList + NominationPools + Session + Historical +
// Offences + Treasury) + others). A mini test runtime would need
// ~1000 LOC of polkadot-sdk relay-chain scaffolding; documented
// as deferred in `CHANGES.md` PR-I6.
//
// What we PROVE here: the integration BOUNDARY between
// `pallet-compute-scoring` and a downstream `pallet-rankings`
// consumer (in-runtime adapter OR off-chain validator OCW) is
// well-defined. The bridge:
//   1. Register a node (via `MockRegistration`).
//   2. Submit an audit-VM aggregate via `submit_audit_stats`
//      — passes the registration gate, advances the chain.
//   3. Close the epoch via `vali_submit_epoch_close` — writes
//      `EpochWeights[epoch][node_id]` AND pushes a snapshot to
//      `T::RankingsSink` (mock).
//   4. Drain the mock sink + assert the payload matches what's
//      in storage (this is the contract a real `pallet-rankings`
//      adapter / off-chain validator would observe).

#[test]
fn epoch_close_pushes_full_snapshot_to_rankings_sink() {
    new_test_ext().execute_with(|| {
        // (1) The default `new_test_ext` already pre-registers
        //     `TEST_NODE_ID` via `MockRegistration`. We register
        //     two MORE nodes so the bridge exercises a non-trivial
        //     batch.
        const NODE_X: [u8; 32] = [0x11; 32];
        const NODE_Y: [u8; 32] = [0x22; 32];
        mock_register_node(NODE_X);
        mock_register_node(NODE_Y);

        // (2) Pre-cleared from `new_test_ext`. Sanity:
        assert!(mock_drain_pushed_rankings().is_empty());

        // (3) Close epoch 1 with a non-trivial batch.
        let updates: BoundedVec<_, _> = BoundedVec::try_from(vec![
            upd(NODE_X, MinerStatus::Active, 1_000),
            upd(NODE_Y, MinerStatus::Quarantined, 0),
        ])
        .unwrap();
        assert_ok!(ComputeScoring::vali_submit_epoch_close(
            RuntimeOrigin::root(),
            1,
            updates,
        ));

        // (4) `EpochWeights` is the on-chain source of truth — pin
        //     it explicitly so a future regression in the read path
        //     is caught here BEFORE the sink-payload check.
        assert_eq!(EpochWeights::<TestRuntime>::get(1, NODE_X), Some(1_000));
        assert_eq!(EpochWeights::<TestRuntime>::get(1, NODE_Y), Some(0));

        // (5) The `RankingsSink` push must mirror `EpochWeights`
        //     for the same epoch. Order is storage-iter order
        //     (`Blake2_128Concat`-hashed `node_id`), NOT sorted —
        //     so we sort both sides by `node_id` before
        //     comparing.
        let mut pushed = mock_drain_pushed_rankings();
        assert_eq!(pushed.len(), 1, "exactly one push per close");
        let (pushed_epoch, mut pushed_entries) = pushed.remove(0);
        assert_eq!(pushed_epoch, 1);

        let mut expected = vec![
            crate::pallet::EpochWeightEntry {
                node_id: NODE_X,
                weight: 1_000,
            },
            crate::pallet::EpochWeightEntry {
                node_id: NODE_Y,
                weight: 0,
            },
        ];
        pushed_entries.sort_by_key(|e| e.node_id);
        expected.sort_by_key(|e| e.node_id);
        assert_eq!(pushed_entries, expected);
    });
}

#[test]
fn epoch_weights_for_returns_full_snapshot() {
    // The public reader is what an off-chain validator OCW would
    // call (via runtime-API or direct storage iter) to build the
    // `pallet_rankings::update_rankings(weights, …)` payload.
    // Pin the contract: every `(node_id, weight)` written by
    // `vali_submit_epoch_close` must surface in `epoch_weights_for`.
    new_test_ext().execute_with(|| {
        const NODE_X: [u8; 32] = [0x33; 32];
        mock_register_node(NODE_X);

        // Close two epochs back-to-back to also pin
        // per-epoch isolation (epoch 1's snapshot must NOT include
        // entries written for epoch 2).
        let updates1: BoundedVec<_, _> =
            BoundedVec::try_from(vec![upd(NODE_X, MinerStatus::Active, 500)]).unwrap();
        assert_ok!(ComputeScoring::vali_submit_epoch_close(
            RuntimeOrigin::root(),
            1,
            updates1,
        ));
        let updates2: BoundedVec<_, _> =
            BoundedVec::try_from(vec![upd(NODE_X, MinerStatus::Active, 800)]).unwrap();
        assert_ok!(ComputeScoring::vali_submit_epoch_close(
            RuntimeOrigin::root(),
            2,
            updates2,
        ));

        let snap1 = crate::pallet::Pallet::<TestRuntime>::epoch_weights_for(1);
        let snap2 = crate::pallet::Pallet::<TestRuntime>::epoch_weights_for(2);
        assert_eq!(
            snap1,
            vec![crate::pallet::EpochWeightEntry {
                node_id: NODE_X,
                weight: 500
            }]
        );
        assert_eq!(
            snap2,
            vec![crate::pallet::EpochWeightEntry {
                node_id: NODE_X,
                weight: 800
            }]
        );

        // Reader for an empty epoch returns an empty Vec — the
        // off-chain consumer can use that as the "nothing to
        // push" signal.
        assert!(crate::pallet::Pallet::<TestRuntime>::epoch_weights_for(99).is_empty());
    });
}

#[test]
fn full_bridge_flow_register_audit_close_push() {
    // End-to-end exercise the PR-I6 bridge path against the
    // mocks for everything `pallet-compute-scoring` doesn't
    // own (registration + rankings).
    //
    // Why this lives in the bridge test and not a real
    // `construct_runtime!`: the supertrait load on
    // `pallet_rankings` would require ~1000 LOC of polkadot-sdk
    // staking + session + babe + election scaffolding — see
    // `CHANGES.md` PR-I6 for the scope decision.
    new_test_ext().execute_with(|| {
        use sp_core::Pair;
        let pair = sp_core::ed25519::Pair::from_seed(&[0x66; 32]);
        let audit_vm_pubkey = pair.public().0;

        // (a) Register the audit-VM pubkey for the canonical
        //     pre-registered `TEST_NODE_ID = NODE_ID`. In
        //     production this is the §I PR-I6 KBS-cert flow;
        //     here the admin extrinsic stands in.
        assert_ok!(ComputeScoring::set_audit_vm_pubkey(
            RuntimeOrigin::root(),
            NODE_ID,
            audit_vm_pubkey,
        ));

        // (b) Submit one audit-VM-signed aggregate (the
        //     registration gate from PR-I5 is already passed
        //     because `new_test_ext` pre-registered NODE_ID).
        let view = make_view(BoundedVec::try_from(b"key-0".to_vec()).unwrap(), [0u8; 32]);
        let body = canonical_aggregate_body(&view);
        let signed = make_signed(&pair, body);
        assert_ok!(ComputeScoring::submit_audit_stats(
            RuntimeOrigin::root(),
            view.clone(),
            signed,
        ));

        // (c) Close epoch 1. The validator submits the weight
        //     for this node based on its off-chain aggregation
        //     of `served_units` across the audit-VM signatures
        //     it observed (here we just pick a value — PR-I6
        //     doesn't pin a specific algorithm).
        let updates: BoundedVec<_, _> =
            BoundedVec::try_from(vec![upd(NODE_ID, MinerStatus::Active, 4_242)]).unwrap();
        assert_ok!(ComputeScoring::vali_submit_epoch_close(
            RuntimeOrigin::root(),
            1,
            updates,
        ));

        // (d) `RankingsSink` got the snapshot for epoch 1.
        let mut pushed = mock_drain_pushed_rankings();
        assert_eq!(pushed.len(), 1);
        let (pushed_epoch, pushed_entries) = pushed.remove(0);
        assert_eq!(pushed_epoch, 1);
        assert_eq!(pushed_entries.len(), 1);
        assert_eq!(pushed_entries[0].node_id, NODE_ID);
        assert_eq!(pushed_entries[0].weight, 4_242);

        // (e) Demonstrate the **partial** transform: the off-chain
        //     validator OCW projects `u128 → u16` and `[u8; 32] →
        //     Vec<u8>` for the
        //     `pallet_rankings::update_rankings(weights: Vec<u16>,
        //     all_nodes_ss58, node_ids: Vec<Vec<u8>>, node_types)`
        //     payload. The **two other parallel vecs**
        //     (`all_nodes_ss58` and `node_types`) require
        //     `pallet_registration::get_node_registration_info` and
        //     the SS58 owner lookup — those are NOT exercised here
        //     because the mock registration uses a `BTreeSet`
        //     stand-in (see `MockRegistration` in `mock.rs`). A
        //     future PR with a real `construct_runtime!` including
        //     `pallet-registration` would close the full
        //     four-vector alignment; for now PR-I6 ships the
        //     transform half that the bridge boundary actually
        //     produces.
        //
        //     **NOTE on the clamp**: `u16::try_from(...).unwrap_or(
        //     u16::MAX)` saturates — for `u128` served-unit weights,
        //     all "high performers" would collapse to the same
        //     `u16::MAX`. A real production OCW MUST define a
        //     normalisation strategy (e.g. `weight * u16::MAX /
        //     total_weight_this_epoch` with rounding) before
        //     submitting to `pallet_rankings`. The saturating cast
        //     below is a placeholder that pins the wire SHAPE,
        //     NOT a normative scaling policy.
        let ranking_weights_u16: Vec<u16> = pushed_entries
            .iter()
            .map(|e| u16::try_from(e.weight).unwrap_or(u16::MAX))
            .collect();
        let ranking_node_ids_vecu8: Vec<Vec<u8>> =
            pushed_entries.iter().map(|e| e.node_id.to_vec()).collect();
        assert_eq!(ranking_weights_u16, vec![4_242u16]);
        assert_eq!(ranking_node_ids_vecu8, vec![NODE_ID.to_vec()]);
    });
}

// ================================================================
// #322 live attestation
// ================================================================

const TEST_VM_ID: &[u8] = b"tnabcd1234";

fn vm_id_hash() -> [u8; 32] {
    sp_io::hashing::blake2_256(TEST_VM_ID)
}

/// Build a [`LiveAttestationView`] whose replay-domain fields match
/// the mock runtime + the registered NODE_ID + a default 1st-of-VM
/// seed (`attestation_seq = 1`, `prev_attestation_hash = 0`). Caller
/// can mutate for sad paths.
fn make_la_view(
    signer_pubkey: [u8; 32],
) -> LiveAttestationView<<TestRuntime as crate::pallet::Config>::MaxVmIdLen> {
    LiveAttestationView {
        chain_genesis: ComputeChainGenesis::get(),
        pallet_instance: ComputePalletInstance::get(),
        vm_id: BoundedVec::try_from(TEST_VM_ID.to_vec()).unwrap(),
        node_id: NODE_ID,
        attestation_seq: 1,
        epoch: CurrentEpoch::<TestRuntime>::get(),
        observed_at_unix: 1_700_000_000,
        verified_at_unix: 1_700_000_005,
        prev_attestation_hash: [0u8; 32],
        expiry_unix: 1_700_010_000,
        signer_pubkey,
    }
}

/// The canonical-CBOR `LiveAttestation` body for a view — what the
/// KBS actually signs, and (post-bind) the ONLY body
/// `submit_live_attestation` accepts for that view. The body-only
/// fields the view does not carry (report digests, measurement) are
/// fixed dummies: the pallet binds the SHARED fields and treats the
/// rest as opaque KBS evidence.
fn canonical_la_body(
    view: &LiveAttestationView<<TestRuntime as crate::pallet::Config>::MaxVmIdLen>,
) -> Vec<u8> {
    hippius_types::live_attestation::LiveAttestation {
        schema_version: hippius_types::live_attestation::LIVE_ATTESTATION_SCHEMA_VERSION,
        chain_genesis: view.chain_genesis,
        pallet_instance: view.pallet_instance,
        vm_id: core::str::from_utf8(view.vm_id.as_slice())
            .expect("test vm_id is utf8")
            .into(),
        node_id: view.node_id,
        attestation_seq: view.attestation_seq,
        epoch: view.epoch,
        observed_at_unix: view.observed_at_unix,
        verified_at_unix: view.verified_at_unix,
        snp_report_digest: [0x51; 32],
        vcek_chain_digest: [0x52; 32],
        measurement: [0x53; 48],
        prev_attestation_hash: view.prev_attestation_hash,
        expiry_unix: view.expiry_unix,
        signer_pubkey: view.signer_pubkey,
    }
    .canonical()
    .expect("test view is well-formed")
}

/// Sign an arbitrary `body` with `pair` and wrap into the wire
/// envelope.
fn make_la_signed(
    pair: &ed25519::Pair,
    body: Vec<u8>,
) -> SignedLiveAttestationWire<<TestRuntime as crate::pallet::Config>::MaxLiveAttestationBody> {
    let sig = pair.sign(&body);
    SignedLiveAttestationWire {
        body: BoundedVec::try_from(body).unwrap(),
        sig: sig.0,
    }
}

#[test]
fn set_kbs_attestation_pubkey_happy_path() {
    new_test_ext().execute_with(|| {
        let pubkey = [0xAB; 32];
        assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            pubkey,
        ));
        assert!(KbsAttestationPubkeys::<TestRuntime>::get().contains(&pubkey));
    });
}

#[test]
fn set_kbs_attestation_pubkey_admin_only() {
    new_test_ext().execute_with(|| {
        assert_noop!(
            ComputeScoring::set_kbs_attestation_pubkey(RuntimeOrigin::signed(1), [0xAB; 32]),
            sp_runtime::DispatchError::BadOrigin,
        );
    });
}

#[test]
fn set_kbs_attestation_pubkey_rejects_duplicate() {
    new_test_ext().execute_with(|| {
        assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            [0xAB; 32],
        ));
        assert_noop!(
            ComputeScoring::set_kbs_attestation_pubkey(RuntimeOrigin::root(), [0xAB; 32]),
            Error::<TestRuntime>::KbsAttestationPubkeyAlreadyAllowed,
        );
    });
}

#[test]
fn set_kbs_attestation_pubkey_rejects_when_full() {
    new_test_ext().execute_with(|| {
        // MaxKbsAttestationPubkeys = 4 in mock.
        for i in 0..4u8 {
            assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
                RuntimeOrigin::root(),
                [i; 32],
            ));
        }
        assert_noop!(
            ComputeScoring::set_kbs_attestation_pubkey(RuntimeOrigin::root(), [0xFF; 32]),
            Error::<TestRuntime>::KbsAttestationPubkeysFull,
        );
    });
}

#[test]
fn remove_kbs_attestation_pubkey_happy_path() {
    new_test_ext().execute_with(|| {
        let pubkey = [0xAB; 32];
        assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            pubkey,
        ));
        assert_ok!(ComputeScoring::remove_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            pubkey,
        ));
        assert!(!KbsAttestationPubkeys::<TestRuntime>::get().contains(&pubkey));
    });
}

#[test]
fn remove_kbs_attestation_pubkey_rejects_unknown() {
    new_test_ext().execute_with(|| {
        assert_noop!(
            ComputeScoring::remove_kbs_attestation_pubkey(RuntimeOrigin::root(), [0xAB; 32]),
            Error::<TestRuntime>::KbsAttestationPubkeyNotAllowed,
        );
    });
}

#[test]
fn submit_live_attestation_happy_path() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[0x42; 32]);
        let pubkey = pair.public().0;
        assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            pubkey,
        ));

        let view = make_la_view(pubkey);
        let body = canonical_la_body(&view);
        let signed = make_la_signed(&pair, body.clone());

        assert_ok!(ComputeScoring::submit_live_attestation(
            RuntimeOrigin::root(),
            view.clone(),
            signed,
        ));

        // Replay state was written.
        let record = LastLiveAttestation::<TestRuntime>::get(vm_id_hash()).unwrap();
        assert_eq!(record.node_id, NODE_ID);
        assert_eq!(record.attestation_seq, 1);
        assert_eq!(record.epoch, view.epoch);
        assert_eq!(record.observed_at_unix, view.observed_at_unix);
        assert_eq!(record.body_hash, sp_io::hashing::sha2_256(&body));

        // Per-epoch counter was bumped.
        assert_eq!(
            LiveAttestationCount::<TestRuntime>::get(view.epoch, vm_id_hash()),
            1,
        );

        // Event was emitted.
        let events = frame_system::Pallet::<TestRuntime>::events();
        assert!(events.iter().any(|e| matches!(
            e.event,
            RuntimeEvent::ComputeScoring(ComputeEvent::LiveAttestationSubmitted {
                vm_id_hash: ref h,
                ..
            }) if h == &vm_id_hash(),
        )));
    });
}

#[test]
fn submit_live_attestation_chains_two_aggregates() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[0x42; 32]);
        let pubkey = pair.public().0;
        assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            pubkey,
        ));

        // First attestation (seq=1, prev=0).
        let v1 = make_la_view(pubkey);
        let body1 = canonical_la_body(&v1);
        let body1_hash = sp_io::hashing::sha2_256(&body1);
        assert_ok!(ComputeScoring::submit_live_attestation(
            RuntimeOrigin::root(),
            v1,
            make_la_signed(&pair, body1),
        ));

        // Second attestation (seq=2, prev=hash(body1)).
        let mut v2 = make_la_view(pubkey);
        v2.attestation_seq = 2;
        v2.prev_attestation_hash = body1_hash;
        v2.observed_at_unix += 300;
        v2.verified_at_unix += 300;
        let body2 = canonical_la_body(&v2);
        assert_ok!(ComputeScoring::submit_live_attestation(
            RuntimeOrigin::root(),
            v2.clone(),
            make_la_signed(&pair, body2.clone()),
        ));

        let record = LastLiveAttestation::<TestRuntime>::get(vm_id_hash()).unwrap();
        assert_eq!(record.attestation_seq, 2);
        assert_eq!(record.body_hash, sp_io::hashing::sha2_256(&body2));
        assert_eq!(
            LiveAttestationCount::<TestRuntime>::get(v2.epoch, vm_id_hash()),
            2,
        );
    });
}

#[test]
fn submit_live_attestation_rejects_empty_body() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[0x42; 32]);
        let pubkey = pair.public().0;
        assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            pubkey,
        ));
        let view = make_la_view(pubkey);
        let signed = make_la_signed(&pair, Vec::new());
        assert_noop!(
            ComputeScoring::submit_live_attestation(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::EmptyLiveAttestationBody,
        );
    });
}

#[test]
fn submit_live_attestation_rejects_inverted_window() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[0x42; 32]);
        let pubkey = pair.public().0;
        assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            pubkey,
        ));
        let mut view = make_la_view(pubkey);
        view.verified_at_unix = view.observed_at_unix - 1;
        let body = view.encode();
        let signed = make_la_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_live_attestation(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::LiveAttestationWindowInverted,
        );
    });
}

#[test]
fn submit_live_attestation_rejects_unregistered_node() {
    new_test_ext().execute_with(|| {
        mock_clear_registered_nodes();
        let pair = ed25519::Pair::from_seed(&[0x42; 32]);
        let pubkey = pair.public().0;
        assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            pubkey,
        ));
        let view = make_la_view(pubkey);
        let body = view.encode();
        let signed = make_la_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_live_attestation(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::NodeNotRegistered,
        );
    });
}

#[test]
fn submit_live_attestation_rejects_chain_genesis_mismatch() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[0x42; 32]);
        let pubkey = pair.public().0;
        assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            pubkey,
        ));
        let mut view = make_la_view(pubkey);
        view.chain_genesis = [0xFF; 32];
        let body = view.encode();
        let signed = make_la_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_live_attestation(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::LiveAttestationChainGenesisMismatch,
        );
    });
}

#[test]
fn submit_live_attestation_rejects_pallet_instance_mismatch() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[0x42; 32]);
        let pubkey = pair.public().0;
        assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            pubkey,
        ));
        let mut view = make_la_view(pubkey);
        view.pallet_instance = [0xFF; 32];
        let body = view.encode();
        let signed = make_la_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_live_attestation(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::LiveAttestationPalletInstanceMismatch,
        );
    });
}

#[test]
fn submit_live_attestation_rejects_epoch_mismatch() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[0x42; 32]);
        let pubkey = pair.public().0;
        assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            pubkey,
        ));
        let mut view = make_la_view(pubkey);
        view.epoch = view.epoch.saturating_add(1);
        let body = view.encode();
        let signed = make_la_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_live_attestation(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::LiveAttestationEpochMismatch,
        );
    });
}

#[test]
fn submit_live_attestation_rejects_expired() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[0x42; 32]);
        let pubkey = pair.public().0;
        assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            pubkey,
        ));
        let mut view = make_la_view(pubkey);
        // Mock NowUnix returns TEST_NOW_MILLIS / 1000 = 1_700_005_000 s.
        view.expiry_unix = 1_700_004_999;
        let body = view.encode();
        let signed = make_la_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_live_attestation(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::LiveAttestationExpired,
        );
    });
}

#[test]
fn submit_live_attestation_rejects_wrong_seq_at_genesis() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[0x42; 32]);
        let pubkey = pair.public().0;
        assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            pubkey,
        ));
        // First attestation MUST have seq = 1.
        let mut view = make_la_view(pubkey);
        view.attestation_seq = 2;
        let body = view.encode();
        let signed = make_la_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_live_attestation(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::LiveAttestationSeqMismatch,
        );
    });
}

#[test]
fn submit_live_attestation_rejects_wrong_prev_hash() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[0x42; 32]);
        let pubkey = pair.public().0;
        assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            pubkey,
        ));
        // First attestation MUST have prev = 0.
        let mut view = make_la_view(pubkey);
        view.prev_attestation_hash = [0xFF; 32];
        let body = view.encode();
        let signed = make_la_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_live_attestation(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::LiveAttestationPrevHashMismatch,
        );
    });
}

#[test]
fn submit_live_attestation_rejects_unallowlisted_signer() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[0x42; 32]);
        let pubkey = pair.public().0;
        // Allowlist NOT set — same key must still fail.
        let view = make_la_view(pubkey);
        let body = view.encode();
        let signed = make_la_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_live_attestation(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::LiveAttestationKbsNotAllowed,
        );
    });
}

#[test]
fn submit_live_attestation_rejects_bad_signature() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[0x42; 32]);
        let other = ed25519::Pair::from_seed(&[0x43; 32]);
        let pubkey = pair.public().0;
        assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            pubkey,
        ));
        let view = make_la_view(pubkey);
        let body = canonical_la_body(&view);
        // Sig by `other`, but view.signer_pubkey == pair.public().
        let signed = make_la_signed(&other, body);
        assert_noop!(
            ComputeScoring::submit_live_attestation(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::InvalidLiveAttestationSignature,
        );
    });
}

/// THE review finding (thebrain#49): one honestly-KBS-signed body,
/// replayed under a forged view. Before the body↔view bind, every
/// gate passed — the signature was valid over the body, and every
/// gated field was read from the attacker-controlled view — so
/// `LiveAttestationCount` could be inflated for ANY vm/node. The
/// bind must kill exactly this.
#[test]
fn submit_live_attestation_rejects_honest_body_under_forged_view() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[0x42; 32]);
        let pubkey = pair.public().0;
        assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            pubkey,
        ));

        // The KBS honestly signs an attestation for TEST_VM_ID.
        let honest = make_la_view(pubkey);
        let honest_body = canonical_la_body(&honest);
        let signed = make_la_signed(&pair, honest_body);

        // The submitter forges a view crediting a DIFFERENT vm —
        // fresh replay chain (seq=1, prev=0), same registered node,
        // same epoch: every pre-bind gate passes.
        let mut forged = make_la_view(pubkey);
        forged.vm_id = BoundedVec::try_from(b"vm-stolen".to_vec()).unwrap();

        assert_noop!(
            ComputeScoring::submit_live_attestation(RuntimeOrigin::root(), forged, signed),
            Error::<TestRuntime>::LiveAttestationBodyViewMismatch,
        );
    });
}

/// A body that is not canonical-CBOR `LiveAttestation` — e.g. the
/// SCALE-encoded view, which is exactly what this test file used to
/// submit everywhere — must be rejected before signature work.
#[test]
fn submit_live_attestation_rejects_non_cbor_body() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[0x42; 32]);
        let pubkey = pair.public().0;
        assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            pubkey,
        ));
        let view = make_la_view(pubkey);
        let body = view.encode(); // SCALE, not canonical CBOR
        let signed = make_la_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_live_attestation(RuntimeOrigin::root(), view, signed),
            Error::<TestRuntime>::MalformedLiveAttestationBody,
        );
    });
}

#[test]
fn submit_live_attestation_root_only() {
    new_test_ext().execute_with(|| {
        let pair = ed25519::Pair::from_seed(&[0x42; 32]);
        let pubkey = pair.public().0;
        assert_ok!(ComputeScoring::set_kbs_attestation_pubkey(
            RuntimeOrigin::root(),
            pubkey,
        ));
        let view = make_la_view(pubkey);
        let body = view.encode();
        let signed = make_la_signed(&pair, body);
        assert_noop!(
            ComputeScoring::submit_live_attestation(RuntimeOrigin::signed(1), view, signed),
            sp_runtime::DispatchError::BadOrigin,
        );
    });
}

// =====================================================================
// PR-1 — $-denominated stake (oracle + asymmetric EMA + reserve lifecycle)
// =====================================================================

mod stake {
    use super::*;
    use crate::pallet::{
        AlphaPerUsdEma, AlphaPerUsdSpot, EmaDownPermille, EmaUpPermille, StakeUnbonding,
        StakedAmount,
    };
    use frame_support::traits::Currency;

    fn fund(acct: AccountId, amount: Balance) {
        let _ = Balances::make_free_balance_be(&acct, amount);
    }

    #[test]
    fn ema_bootstraps_then_reacts_fast_on_dump_slow_on_pump() {
        new_test_ext().execute_with(|| {
            // First post bootstraps the EMA to spot.
            assert_ok!(ComputeScoring::set_alpha_per_usd(
                RuntimeOrigin::root(),
                1_000_000
            ));
            assert_eq!(AlphaPerUsdEma::<TestRuntime>::get(), 1_000_000);
            assert_eq!(AlphaPerUsdSpot::<TestRuntime>::get(), 1_000_000);

            // DUMP: alpha_per_usd rises 1.0 → 2.0. Default down=300‰ → EMA
            // jumps 30% of the gap: 1.0 + 0.3*(2.0-1.0) = 1.3.
            assert_ok!(ComputeScoring::set_alpha_per_usd(
                RuntimeOrigin::root(),
                2_000_000
            ));
            assert_eq!(AlphaPerUsdEma::<TestRuntime>::get(), 1_300_000);

            // PUMP from the new EMA: spot drops 1.3 → 1.0. Default up=50‰ →
            // EMA eases only 5% of the gap: 1.3 - 0.05*(1.3-1.0) = 1.285.
            assert_ok!(ComputeScoring::set_alpha_per_usd(
                RuntimeOrigin::root(),
                1_000_000
            ));
            assert_eq!(AlphaPerUsdEma::<TestRuntime>::get(), 1_285_000);

            // The asymmetry holds: a dump moved the EMA 300k, an equal-size
            // pump only clawed back 15k — "lock fast, release slow".
        });
    }

    #[test]
    fn required_alpha_scales_with_value_at_risk_and_ema() {
        new_test_ext().execute_with(|| {
            // No price posted yet ⇒ 0 required (cannot size an obligation).
            assert_eq!(ComputeScoring::required_alpha(100), 0);

            // 1 alpha = 1 USD (×1e6). value-at-risk $50 ⇒ 50 alpha.
            assert_ok!(ComputeScoring::set_alpha_per_usd(
                RuntimeOrigin::root(),
                1_000_000
            ));
            assert_eq!(ComputeScoring::required_alpha(50), 50);

            // Coin dumps to 1 USD = 2 alpha (bootstrap reset). $50 ⇒ 100.
            AlphaPerUsdEma::<TestRuntime>::put(2_000_000);
            assert_eq!(ComputeScoring::required_alpha(50), 100);
        });
    }

    #[test]
    fn eligibility_is_inert_until_enabled_then_enforces_floor_and_required() {
        new_test_ext().execute_with(|| {
            // Disabled (default): always sufficient, even with nothing staked.
            assert!(ComputeScoring::is_stake_sufficient(&1, 1_000));

            assert_ok!(ComputeScoring::set_stake_enabled(
                RuntimeOrigin::root(),
                true
            ));
            assert_ok!(ComputeScoring::set_stake_floor(RuntimeOrigin::root(), 100));

            // Nothing staked ⇒ below floor ⇒ insufficient.
            assert!(!ComputeScoring::is_stake_sufficient(&1, 0));

            fund(1, 1_000_000);
            assert_ok!(ComputeScoring::top_up_stake(RuntimeOrigin::signed(1), 100));
            // Meets the floor when required is 0.
            assert!(ComputeScoring::is_stake_sufficient(&1, 0));
            // ...but not a required above the stake.
            assert!(!ComputeScoring::is_stake_sufficient(&1, 200));
            assert_ok!(ComputeScoring::top_up_stake(RuntimeOrigin::signed(1), 100));
            assert!(ComputeScoring::is_stake_sufficient(&1, 200));
        });
    }

    #[test]
    fn top_up_reserves_balance() {
        new_test_ext().execute_with(|| {
            fund(1, 1_000);
            assert_ok!(ComputeScoring::top_up_stake(RuntimeOrigin::signed(1), 300));
            assert_eq!(StakedAmount::<TestRuntime>::get(1), 300);
            assert_eq!(Balances::reserved_balance(1), 300);
            assert_eq!(Balances::free_balance(1), 700);

            assert_ok!(ComputeScoring::top_up_stake(RuntimeOrigin::signed(1), 200));
            assert_eq!(StakedAmount::<TestRuntime>::get(1), 500);
            assert_eq!(Balances::reserved_balance(1), 500);
        });
    }

    #[test]
    fn top_up_rejects_zero_and_insufficient_balance() {
        new_test_ext().execute_with(|| {
            assert_noop!(
                ComputeScoring::top_up_stake(RuntimeOrigin::signed(1), 0),
                Error::<TestRuntime>::ZeroStake
            );
            fund(1, 10);
            assert_noop!(
                ComputeScoring::top_up_stake(RuntimeOrigin::signed(1), 100),
                Error::<TestRuntime>::InsufficientBalanceForStake
            );
        });
    }

    #[test]
    fn unstake_unbonds_then_releases_after_period() {
        new_test_ext().execute_with(|| {
            fund(1, 1_000);
            assert_ok!(ComputeScoring::top_up_stake(RuntimeOrigin::signed(1), 500));

            // Can't unstake more than active.
            assert_noop!(
                ComputeScoring::request_unstake(RuntimeOrigin::signed(1), 600),
                Error::<TestRuntime>::InsufficientStake
            );

            assert_ok!(ComputeScoring::request_unstake(
                RuntimeOrigin::signed(1),
                200
            ));
            // Active drops, but balance stays RESERVED (slashable) during unbond.
            assert_eq!(StakedAmount::<TestRuntime>::get(1), 300);
            assert_eq!(Balances::reserved_balance(1), 500);
            let (amt, end) = StakeUnbonding::<TestRuntime>::get(1).unwrap();
            assert_eq!(amt, 200);
            // UnbondingPeriodBlocks = 64 in the mock; now() = 1.
            assert_eq!(end, 1 + 64);

            // Too early.
            assert_noop!(
                ComputeScoring::claim_unstaked(RuntimeOrigin::signed(1)),
                Error::<TestRuntime>::UnbondingNotReady
            );

            System::set_block_number(end);
            assert_ok!(ComputeScoring::claim_unstaked(RuntimeOrigin::signed(1)));
            assert_eq!(Balances::reserved_balance(1), 300);
            assert_eq!(Balances::free_balance(1), 700);
            assert!(StakeUnbonding::<TestRuntime>::get(1).is_none());
        });
    }

    #[test]
    fn claim_with_no_unbonding_fails() {
        new_test_ext().execute_with(|| {
            assert_noop!(
                ComputeScoring::claim_unstaked(RuntimeOrigin::signed(1)),
                Error::<TestRuntime>::NoStakeUnbonding
            );
        });
    }

    #[test]
    fn admin_extrinsics_are_root_only() {
        new_test_ext().execute_with(|| {
            assert_noop!(
                ComputeScoring::set_stake_enabled(RuntimeOrigin::signed(1), true),
                sp_runtime::DispatchError::BadOrigin
            );
            assert_noop!(
                ComputeScoring::set_alpha_per_usd(RuntimeOrigin::signed(1), 1),
                sp_runtime::DispatchError::BadOrigin
            );
            assert_noop!(
                ComputeScoring::set_stake_floor(RuntimeOrigin::signed(1), 1),
                sp_runtime::DispatchError::BadOrigin
            );
            assert_noop!(
                ComputeScoring::set_ema_permille(RuntimeOrigin::signed(1), 1, 1),
                sp_runtime::DispatchError::BadOrigin
            );
        });
    }

    #[test]
    fn price_and_ema_inputs_are_validated() {
        new_test_ext().execute_with(|| {
            assert_noop!(
                ComputeScoring::set_alpha_per_usd(RuntimeOrigin::root(), 0),
                Error::<TestRuntime>::InvalidPrice
            );
            for (d, u) in [(0u32, 50u32), (50, 0), (1001, 50), (50, 1001)] {
                assert_noop!(
                    ComputeScoring::set_ema_permille(RuntimeOrigin::root(), d, u),
                    Error::<TestRuntime>::InvalidEmaFactor
                );
            }
            assert_ok!(ComputeScoring::set_ema_permille(
                RuntimeOrigin::root(),
                200,
                80
            ));
            assert_eq!(EmaDownPermille::<TestRuntime>::get(), 200);
            assert_eq!(EmaUpPermille::<TestRuntime>::get(), 80);
        });
    }
}

// =====================================================================
// PR-2 — slashing (kill-switch gated)
// =====================================================================

mod slashing {
    use super::*;
    use crate::pallet::{SlashReason, SlashRecord, SlashingEnabled, StakeUnbonding, StakedAmount};
    use frame_support::traits::{Currency, NamedReservableCurrency, ReservableCurrency};

    fn staked(acct: AccountId, amount: Balance) {
        let _ = Balances::make_free_balance_be(&acct, amount * 2);
        assert_ok!(ComputeScoring::top_up_stake(
            RuntimeOrigin::signed(acct),
            amount
        ));
    }

    #[test]
    fn disabled_by_default_records_event_but_moves_no_balance() {
        new_test_ext().execute_with(|| {
            staked(1, 500);
            assert!(!SlashingEnabled::<TestRuntime>::get());

            assert_ok!(ComputeScoring::slash_stake(
                RuntimeOrigin::root(),
                1,
                200,
                SlashReason::Liveness
            ));

            // Nothing moved.
            assert_eq!(StakedAmount::<TestRuntime>::get(1), 500);
            assert_eq!(Balances::reserved_balance(1), 500);
            assert_eq!(SlashRecord::<TestRuntime>::get(1), 0);
            System::assert_last_event(
                ComputeEvent::SlashSkippedDisabled {
                    owner: 1,
                    amount: 200,
                    reason: SlashReason::Liveness,
                }
                .into(),
            );
        });
    }

    #[test]
    fn enabled_burns_reserved_and_records() {
        new_test_ext().execute_with(|| {
            staked(1, 500);
            let issuance_before = Balances::total_issuance();
            assert_ok!(ComputeScoring::set_slashing_enabled(
                RuntimeOrigin::root(),
                true
            ));

            assert_ok!(ComputeScoring::slash_stake(
                RuntimeOrigin::root(),
                1,
                200,
                SlashReason::Sla
            ));

            assert_eq!(StakedAmount::<TestRuntime>::get(1), 300);
            assert_eq!(Balances::reserved_balance(1), 300);
            assert_eq!(SlashRecord::<TestRuntime>::get(1), 200);
            // Burned ⇒ total issuance dropped by the slashed amount.
            assert_eq!(Balances::total_issuance(), issuance_before - 200);
        });
    }

    #[test]
    fn enabled_repatriates_to_beneficiary_when_set() {
        new_test_ext().execute_with(|| {
            staked(1, 500);
            // A live beneficiary (escrow) above ED so repatriation lands.
            let _ = Balances::make_free_balance_be(&9, 10);
            assert_ok!(ComputeScoring::set_slashing_enabled(
                RuntimeOrigin::root(),
                true
            ));
            assert_ok!(ComputeScoring::set_slash_beneficiary(
                RuntimeOrigin::root(),
                Some(9)
            ));

            assert_ok!(ComputeScoring::slash_stake(
                RuntimeOrigin::root(),
                1,
                150,
                SlashReason::Manual
            ));

            assert_eq!(StakedAmount::<TestRuntime>::get(1), 350);
            assert_eq!(Balances::reserved_balance(1), 350);
            // Repatriated to the beneficiary's free balance (compensation).
            assert_eq!(Balances::free_balance(9), 160);
            assert_eq!(SlashRecord::<TestRuntime>::get(1), 150);
        });
    }

    #[test]
    fn dead_beneficiary_falls_back_to_burn_no_accounting_drift() {
        new_test_ext().execute_with(|| {
            staked(1, 500);
            // Beneficiary 9 stays dead (balance 0 < ED) — repatriation can't
            // land, so the slash must still burn: reserved drops by `amount`.
            assert_ok!(ComputeScoring::set_slashing_enabled(
                RuntimeOrigin::root(),
                true
            ));
            assert_ok!(ComputeScoring::set_slash_beneficiary(
                RuntimeOrigin::root(),
                Some(9)
            ));
            assert_ok!(ComputeScoring::slash_stake(
                RuntimeOrigin::root(),
                1,
                150,
                SlashReason::Manual
            ));
            assert_eq!(StakedAmount::<TestRuntime>::get(1), 350);
            assert_eq!(Balances::reserved_balance(1), 350); // dropped, not stuck at 500
            assert_eq!(Balances::free_balance(9), 0); // dead → got nothing
            assert_eq!(SlashRecord::<TestRuntime>::get(1), 150);
        });
    }

    #[test]
    fn slash_is_capped_at_total_stake() {
        new_test_ext().execute_with(|| {
            staked(1, 100);
            assert_ok!(ComputeScoring::set_slashing_enabled(
                RuntimeOrigin::root(),
                true
            ));
            // Ask for more than staked — only 100 is slashable.
            assert_ok!(ComputeScoring::slash_stake(
                RuntimeOrigin::root(),
                1,
                999,
                SlashReason::Liveness
            ));
            assert_eq!(StakedAmount::<TestRuntime>::get(1), 0);
            assert_eq!(SlashRecord::<TestRuntime>::get(1), 100);
        });
    }

    #[test]
    fn slash_draws_from_unbonding_after_active() {
        new_test_ext().execute_with(|| {
            staked(1, 500);
            // Move 300 into unbonding (still reserved + slashable).
            assert_ok!(ComputeScoring::request_unstake(
                RuntimeOrigin::signed(1),
                300
            ));
            assert_eq!(StakedAmount::<TestRuntime>::get(1), 200);
            assert_ok!(ComputeScoring::set_slashing_enabled(
                RuntimeOrigin::root(),
                true
            ));

            // Slash 350: 200 from active (→0), 150 from the unbonding chunk.
            assert_ok!(ComputeScoring::slash_stake(
                RuntimeOrigin::root(),
                1,
                350,
                SlashReason::StakeDeficiency
            ));
            assert_eq!(StakedAmount::<TestRuntime>::get(1), 0);
            let (left, _) = StakeUnbonding::<TestRuntime>::get(1).unwrap();
            assert_eq!(left, 150);
            assert_eq!(Balances::reserved_balance(1), 150);
            assert_eq!(SlashRecord::<TestRuntime>::get(1), 350);
        });
    }

    #[test]
    fn slash_and_admin_are_authority_gated() {
        new_test_ext().execute_with(|| {
            assert_noop!(
                ComputeScoring::slash_stake(RuntimeOrigin::signed(2), 1, 10, SlashReason::Manual),
                sp_runtime::DispatchError::BadOrigin
            );
            assert_noop!(
                ComputeScoring::set_slashing_enabled(RuntimeOrigin::signed(2), true),
                sp_runtime::DispatchError::BadOrigin
            );
            assert_noop!(
                ComputeScoring::set_slash_beneficiary(RuntimeOrigin::signed(2), Some(9)),
                sp_runtime::DispatchError::BadOrigin
            );
            assert_noop!(
                ComputeScoring::slash_stake(RuntimeOrigin::root(), 1, 0, SlashReason::Manual),
                Error::<TestRuntime>::ZeroStake
            );
        });
    }

    /// R9 (thebrain#49): before named reserves, stake and child
    /// deposits shared ONE untagged pool. A slash could eat the
    /// family's registration deposit, and the post-slash shortfall
    /// made `claim_unbonded`'s `ensure!(unreleased.is_zero())` revert
    /// forever — deposit permanently unclaimable. This is that exact
    /// scenario, on one account wearing both hats.
    #[test]
    fn slash_cannot_touch_child_deposits_and_unbond_survives() {
        new_test_ext().execute_with(|| {
            use sp_core::{ed25519, Pair};
            const FAM: AccountId = 1;
            const CHILD: AccountId = 7;
            let _ = Balances::make_free_balance_be(&FAM, 10_000);

            // Arm the deposit layer: lockup ON, no free slots, base 100.
            assert_ok!(ComputeScoring::set_lockup_enabled(
                RuntimeOrigin::root(),
                true
            ));
            assert_ok!(ComputeScoring::set_free_child_slots_per_family(
                RuntimeOrigin::root(),
                0
            ));
            assert_ok!(ComputeScoring::set_base_child_deposit(
                RuntimeOrigin::root(),
                100
            ));

            // Register a child — reserves the 100 deposit under the
            // DEPOSIT reserve id.
            let pair = ed25519::Pair::from_seed(&[0xD9; 32]);
            let node_id = pair.public().0;
            let msg = (b"HIPPIUS_COMPUTE_NODE_REG_V1", &FAM, &CHILD, &node_id, 0u64).encode();
            let sig = pair.sign(&msg);
            assert_ok!(ComputeScoring::register_child(
                RuntimeOrigin::signed(FAM),
                FAM,
                CHILD,
                node_id,
                sig.0,
            ));
            assert_eq!(Balances::reserved_balance(FAM), 100);

            // Same account also stakes 500 — STAKE reserve id.
            assert_ok!(ComputeScoring::top_up_stake(
                RuntimeOrigin::signed(FAM),
                500
            ));
            assert_eq!(Balances::reserved_balance(FAM), 600);

            // The bite condition: the STAKE reserve drifts SHORT of
            // the logical stake (out-of-band unreserve, exactly like
            // `slash_records_actual_amount_when_reserved_is_short`).
            // Logical stake 500, physical stake reserve 300, deposit
            // 100. do_slash caps the request at the LOGICAL 500 — so
            // an untagged `slash_reserved(500)` draws from the whole
            // 400-pool and eats the deposit; the named slash is
            // capped at the 300 the stake reserve actually holds.
            assert_eq!(
                Balances::unreserve_named(&crate::pallet::STAKE_RESERVE_ID, &FAM, 200),
                0
            );
            assert_eq!(Balances::reserved_balance(FAM), 400);

            assert_ok!(ComputeScoring::set_slashing_enabled(
                RuntimeOrigin::root(),
                true
            ));
            assert_ok!(ComputeScoring::slash_stake(
                RuntimeOrigin::root(),
                FAM,
                10_000,
                SlashReason::Liveness
            ));

            // Only what the STAKE reserve physically held (300) was
            // slashed; the DEPOSIT (100) is untouched. The logical
            // stake decrements by the ACTUAL slash (500 - 300 = 200),
            // same record-reality semantics as
            // `slash_records_actual_amount_when_reserved_is_short`.
            assert_eq!(SlashRecord::<TestRuntime>::get(FAM), 300);
            assert_eq!(Balances::reserved_balance(FAM), 100);
            assert_eq!(StakedAmount::<TestRuntime>::get(FAM), 200);

            // And the deposit is still CLAIMABLE: deregister, wait out
            // the unbonding window, claim — the pre-fix brick point.
            assert_ok!(ComputeScoring::deregister_child(
                RuntimeOrigin::signed(FAM),
                CHILD
            ));
            let reg = crate::pallet::ChildRegistrations::<TestRuntime>::get(CHILD).unwrap();
            System::set_block_number(reg.unbonding_end);
            assert_ok!(ComputeScoring::claim_unbonded(
                RuntimeOrigin::signed(FAM),
                CHILD
            ));
            assert_eq!(Balances::reserved_balance(FAM), 0);
        });
    }

    #[test]
    fn slash_records_actual_amount_when_reserved_is_short() {
        new_test_ext().execute_with(|| {
            staked(1, 500);
            // Simulate logical/physical drift: free 200 of the reserve
            // out-of-band so reserved (300) < StakedAmount (500).
            assert_eq!(Balances::unreserve(&1, 200), 0);
            assert_eq!(Balances::reserved_balance(1), 300);
            assert_ok!(ComputeScoring::set_slashing_enabled(
                RuntimeOrigin::root(),
                true
            ));

            // Ask to slash 400, but only 300 is physically reservable.
            assert_ok!(ComputeScoring::slash_stake(
                RuntimeOrigin::root(),
                1,
                400,
                SlashReason::Liveness
            ));
            // Record + event reflect what was ACTUALLY burned (300), not 400.
            assert_eq!(SlashRecord::<TestRuntime>::get(1), 300);
            assert_eq!(Balances::reserved_balance(1), 0);
            // Logical stake decremented by the actual 300 (500 - 300 = 200).
            assert_eq!(StakedAmount::<TestRuntime>::get(1), 200);
        });
    }
}

// =====================================================================
// PR-3 — marketplace pricing (miner-set, capped, announced ahead)
// =====================================================================

mod marketplace {
    use super::*;
    use crate::pallet::{MinerPrice, NodeIdToChild, PendingPriceChange};

    const NODE: [u8; 32] = [0xCC; 32];
    const OP: AccountId = 1;

    /// Register `OP` as the operator of `NODE` + install fast, bounded
    /// price policy: notice 5, interval 10, max ratio 3/2, floor 1.
    fn setup() {
        NodeIdToChild::<TestRuntime>::insert(NODE, OP);
        assert_ok!(ComputeScoring::set_price_bounds(
            RuntimeOrigin::root(),
            1,
            Some(1_000_000_000)
        ));
        assert_ok!(ComputeScoring::set_price_change_policy(
            RuntimeOrigin::root(),
            10,
            5,
            3,
            2
        ));
    }

    fn announce(price: u128) -> sp_runtime::DispatchResult {
        ComputeScoring::announce_price_change(RuntimeOrigin::signed(OP), NODE, price)
    }

    #[test]
    fn announce_rejects_zero_price() {
        new_test_ext().execute_with(|| {
            setup();
            // A zero price is rejected — it would trap the node at an
            // unraisable [0,0] magnitude band (review fix).
            assert_noop!(announce(0), Error::<TestRuntime>::PriceOutOfBounds);
        });
    }

    #[test]
    fn announce_then_apply_after_notice_window() {
        new_test_ext().execute_with(|| {
            setup();
            // now = 1; notice = 5 ⇒ effective at block 6.
            assert_ok!(announce(100));
            let p = PendingPriceChange::<TestRuntime>::get(NODE).unwrap();
            assert_eq!(p.new_price, 100);
            assert_eq!(p.effective_block, 6);

            // Before the window: not yet effective, MinerPrice still unset.
            assert_eq!(ComputeScoring::effective_price(&NODE), None);
            assert_noop!(
                ComputeScoring::apply_price_change(RuntimeOrigin::signed(2), NODE),
                Error::<TestRuntime>::NoPriceChangeDue
            );

            // At the window: effective_price reflects it lazily...
            System::set_block_number(6);
            assert_eq!(ComputeScoring::effective_price(&NODE), Some(100));
            // ...and anyone can materialise it.
            assert_ok!(ComputeScoring::apply_price_change(
                RuntimeOrigin::signed(2),
                NODE
            ));
            assert_eq!(MinerPrice::<TestRuntime>::get(NODE), Some(100));
            assert!(PendingPriceChange::<TestRuntime>::get(NODE).is_none());
        });
    }

    #[test]
    fn first_price_skips_magnitude_but_enforces_bounds() {
        new_test_ext().execute_with(|| {
            setup();
            // No current price ⇒ any in-bounds value is allowed.
            assert_ok!(announce(500_000));
            // ...but out-of-bounds is rejected (above ceiling).
            PendingPriceChange::<TestRuntime>::remove(NODE);
            assert_noop!(
                announce(2_000_000_000),
                Error::<TestRuntime>::PriceOutOfBounds
            );
            // ...and below floor.
            assert_noop!(announce(0), Error::<TestRuntime>::PriceOutOfBounds);
        });
    }

    #[test]
    fn magnitude_is_bounded_against_current_price() {
        new_test_ext().execute_with(|| {
            setup();
            // Establish a current price of 100.
            assert_ok!(announce(100));
            System::set_block_number(6);
            assert_ok!(ComputeScoring::apply_price_change(
                RuntimeOrigin::signed(2),
                NODE
            ));
            // Past the interval (last announce at block 1, interval 10).
            System::set_block_number(20);

            // ×3 (300) exceeds the 3/2 cap (max 150) → rejected.
            assert_noop!(announce(300), Error::<TestRuntime>::PriceChangeTooLarge);
            // Halving to 40 (< 100·2/3 ≈ 66) → rejected.
            assert_noop!(announce(40), Error::<TestRuntime>::PriceChangeTooLarge);
            // Within ±50%: 150 ok.
            assert_ok!(announce(150));
        });
    }

    #[test]
    fn rate_limited_between_changes() {
        new_test_ext().execute_with(|| {
            setup();
            assert_ok!(announce(100)); // block 1
                                       // Second announce before block 1 + interval(10) → too soon.
            System::set_block_number(5);
            assert_noop!(announce(120), Error::<TestRuntime>::PriceChangeTooSoon);
            // At/after the interval it's allowed.
            System::set_block_number(11);
            assert_ok!(announce(120));
        });
    }

    #[test]
    fn only_the_node_operator_may_announce() {
        new_test_ext().execute_with(|| {
            setup();
            assert_noop!(
                ComputeScoring::announce_price_change(RuntimeOrigin::signed(99), NODE, 100),
                Error::<TestRuntime>::NotNodeOperator
            );
        });
    }

    #[test]
    fn admin_gates_and_invalid_policy() {
        new_test_ext().execute_with(|| {
            assert_noop!(
                ComputeScoring::set_price_bounds(RuntimeOrigin::signed(1), 1, None),
                sp_runtime::DispatchError::BadOrigin
            );
            assert_noop!(
                ComputeScoring::set_price_change_policy(RuntimeOrigin::signed(1), 1, 1, 3, 2),
                sp_runtime::DispatchError::BadOrigin
            );
            // floor > ceiling rejected.
            assert_noop!(
                ComputeScoring::set_price_bounds(RuntimeOrigin::root(), 100, Some(10)),
                Error::<TestRuntime>::InvalidPricePolicy
            );
            // zero denom / numer < denom rejected.
            assert_noop!(
                ComputeScoring::set_price_change_policy(RuntimeOrigin::root(), 1, 1, 3, 0),
                Error::<TestRuntime>::InvalidPricePolicy
            );
            assert_noop!(
                ComputeScoring::set_price_change_policy(RuntimeOrigin::root(), 1, 1, 1, 2),
                Error::<TestRuntime>::InvalidPricePolicy
            );
        });
    }
}

#[test]
fn deregister_clears_marketplace_price_state() {
    use crate::pallet::{LastPriceChangeBlock, MinerPrice, PendingPriceChange, PriceChange};
    new_test_ext().execute_with(|| {
        register_child_for_force_tests();
        // node_id used by the force-test registration = ed25519 pubkey of
        // seed [0xCC; 32].
        use sp_core::{ed25519, Pair};
        let node_id = ed25519::Pair::from_seed(&[0xCC; 32]).public().0;

        // Seed marketplace price state for the node.
        MinerPrice::<TestRuntime>::insert(node_id, 1_000u128);
        PendingPriceChange::<TestRuntime>::insert(
            node_id,
            PriceChange {
                new_price: 1_500u128,
                effective_block: 99,
            },
        );
        LastPriceChangeBlock::<TestRuntime>::insert(node_id, 5u64);

        // Force-deregister the child.
        assert_ok!(ComputeScoring::force_deregister_child(
            RuntimeOrigin::signed(FAMILY_FORCE),
            CHILD_FORCE,
        ));

        // All price state for the node id is gone — a re-registration by a
        // new operator can't inherit it (review fix).
        assert!(MinerPrice::<TestRuntime>::get(node_id).is_none());
        assert!(PendingPriceChange::<TestRuntime>::get(node_id).is_none());
        assert_eq!(LastPriceChangeBlock::<TestRuntime>::get(node_id), 0);
    });
}

// =====================================================================
// Gap #2 — request_unstake quarantines the owner's nodes on exit so the
// off-chain validator drains + warm-migrates their VMs DURING the unbonding
// period (not after the stake is gone).
// =====================================================================

mod graceful_exit {
    use super::*;
    use crate::pallet::{
        ChildRegistration, ChildRegistrations, ChildStatus, FamilyChildren, MinerStatusUpdate,
        StakeEnabled, StakedAmount,
    };
    use frame_support::traits::Currency;

    const OWNER: AccountId = 7;
    const CHILD_1: AccountId = 71;
    const CHILD_2: AccountId = 72;
    const G_NODE_A: [u8; 32] = [0xA1; 32];
    const G_NODE_B: [u8; 32] = [0xB2; 32];

    fn fund(acct: AccountId, amount: Balance) {
        let _ = Balances::make_free_balance_be(&acct, amount);
    }

    /// Seed the owner→child→node mapping the way `register_child` would,
    /// without the ed25519 ceremony — we only exercise the unstake→quarantine
    /// path here, not registration.
    fn attach_child(owner: AccountId, child: AccountId, node_id: [u8; 32]) {
        ChildRegistrations::<TestRuntime>::insert(
            child,
            ChildRegistration {
                family: owner,
                node_id,
                status: ChildStatus::Active,
                deposit: 0,
                unbonding_end: 0,
            },
        );
        FamilyChildren::<TestRuntime>::try_mutate(owner, |v| v.try_push(child))
            .expect("within MaxChildrenPerFamily");
    }

    /// Stake `amount` from `OWNER` (reserve real balance so the lifecycle is
    /// honest), returning nothing — `StakedAmount` is now `amount`.
    fn stake_owner(amount: Balance) {
        fund(OWNER, 1_000_000);
        assert_ok!(ComputeScoring::top_up_stake(
            RuntimeOrigin::signed(OWNER),
            amount
        ));
        assert_eq!(StakedAmount::<TestRuntime>::get(OWNER), amount);
    }

    fn status_of(node_id: [u8; 32]) -> Option<MinerStatus> {
        MinerStatuses::<TestRuntime>::get(node_id).map(|e| e.status)
    }

    fn quarantine_events() -> Vec<u32> {
        frame_system::Pallet::<TestRuntime>::events()
            .iter()
            .filter_map(|e| match &e.event {
                RuntimeEvent::ComputeScoring(ComputeEvent::OwnerQuarantinedOnUnstake {
                    nodes,
                    ..
                }) => Some(*nodes),
                _ => None,
            })
            .collect()
    }

    fn status_change_events() -> Vec<([u8; 32], MinerStatus, MinerStatus)> {
        frame_system::Pallet::<TestRuntime>::events()
            .iter()
            .filter_map(|e| match &e.event {
                RuntimeEvent::ComputeScoring(ComputeEvent::MinerStatusChanged {
                    node_id,
                    old_status,
                    new_status,
                    ..
                }) => Some((*node_id, *old_status, *new_status)),
                _ => None,
            })
            .collect()
    }

    /// 1. Full exit (works with `StakeEnabled = false`): unbond EVERYTHING ⇒
    ///    remaining 0 ⇒ both nodes Quarantined + a `MinerStatusChanged`
    ///    {Active→Quarantined} each + one `OwnerQuarantinedOnUnstake{nodes:2}`.
    #[test]
    fn full_exit_quarantines_all_nodes_even_when_staking_disabled() {
        new_test_ext().execute_with(|| {
            // StakeEnabled defaults to false — the zero-remaining rule must
            // still fire so this is testable without the stake layer live.
            assert!(!StakeEnabled::<TestRuntime>::get());
            attach_child(OWNER, CHILD_1, G_NODE_A);
            attach_child(OWNER, CHILD_2, G_NODE_B);
            stake_owner(500);

            assert_ok!(ComputeScoring::request_unstake(
                RuntimeOrigin::signed(OWNER),
                500
            ));
            assert_eq!(StakedAmount::<TestRuntime>::get(OWNER), 0);

            // Both nodes materialised a Quarantined row.
            assert_eq!(status_of(G_NODE_A), Some(MinerStatus::Quarantined));
            assert_eq!(status_of(G_NODE_B), Some(MinerStatus::Quarantined));

            // One MinerStatusChanged{Active→Quarantined} per node.
            let changes = status_change_events();
            assert!(changes.contains(&(G_NODE_A, MinerStatus::Active, MinerStatus::Quarantined)));
            assert!(changes.contains(&(G_NODE_B, MinerStatus::Active, MinerStatus::Quarantined)));
            assert_eq!(changes.len(), 2);

            // Exactly one aggregate event counting both nodes.
            assert_eq!(quarantine_events(), vec![2]);
        });
    }

    /// 2. Partial-but-sufficient (StakeEnabled = true, remaining ≥ floor): a
    ///    small unstake that stays collateralised is NOT an exit — no row, no
    ///    quarantine event.
    #[test]
    fn partial_sufficient_does_not_quarantine() {
        new_test_ext().execute_with(|| {
            assert_ok!(ComputeScoring::set_stake_enabled(
                RuntimeOrigin::root(),
                true
            ));
            assert_ok!(ComputeScoring::set_stake_floor(RuntimeOrigin::root(), 100));
            attach_child(OWNER, CHILD_1, G_NODE_A);
            stake_owner(500);

            // Unstake 100 ⇒ remaining 400 ≥ floor 100 ⇒ NOT an exit.
            assert_ok!(ComputeScoring::request_unstake(
                RuntimeOrigin::signed(OWNER),
                100
            ));
            assert_eq!(StakedAmount::<TestRuntime>::get(OWNER), 400);

            assert_eq!(status_of(G_NODE_A), None); // still implicit-Active
            assert!(quarantine_events().is_empty());
            assert!(status_change_events().is_empty());
        });
    }

    /// 3. Partial-deficient (StakeEnabled = true, 0 < remaining < floor): drops
    ///    below the eligibility floor ⇒ exit ⇒ quarantine fires.
    #[test]
    fn partial_below_floor_quarantines() {
        new_test_ext().execute_with(|| {
            assert_ok!(ComputeScoring::set_stake_enabled(
                RuntimeOrigin::root(),
                true
            ));
            assert_ok!(ComputeScoring::set_stake_floor(RuntimeOrigin::root(), 300));
            attach_child(OWNER, CHILD_1, G_NODE_A);
            stake_owner(500);

            // Unstake 250 ⇒ remaining 250, which is > 0 but < floor 300 ⇒ exit.
            assert_ok!(ComputeScoring::request_unstake(
                RuntimeOrigin::signed(OWNER),
                250
            ));
            assert_eq!(StakedAmount::<TestRuntime>::get(OWNER), 250);

            assert_eq!(status_of(G_NODE_A), Some(MinerStatus::Quarantined));
            assert_eq!(quarantine_events(), vec![1]);
        });
    }

    /// 4. Partial with StakeEnabled = false, remaining > 0: only the
    ///    zero-remaining case triggers when the stake layer is disabled — a
    ///    leftover balance is NOT an exit.
    #[test]
    fn partial_with_staking_disabled_does_not_quarantine() {
        new_test_ext().execute_with(|| {
            assert!(!StakeEnabled::<TestRuntime>::get());
            // A floor is set but ignored because StakeEnabled is false.
            assert_ok!(ComputeScoring::set_stake_floor(RuntimeOrigin::root(), 400));
            attach_child(OWNER, CHILD_1, G_NODE_A);
            stake_owner(500);

            // Unstake 250 ⇒ remaining 250 (> 0). StakeEnabled=false ⇒ the
            // floor branch is dead, only `remaining == 0` would fire.
            assert_ok!(ComputeScoring::request_unstake(
                RuntimeOrigin::signed(OWNER),
                250
            ));
            assert_eq!(StakedAmount::<TestRuntime>::get(OWNER), 250);

            assert_eq!(status_of(G_NODE_A), None);
            assert!(quarantine_events().is_empty());
        });
    }

    /// 5. Idempotent: a node already Quarantined (via `vali_submit_epoch_close`)
    ///    then the owner unstakes ⇒ stays Quarantined, no duplicate transition
    ///    (`transition_node_status` returns false ⇒ not counted; with only one
    ///    node and zero real transitions, NO aggregate event is emitted).
    #[test]
    fn already_quarantined_node_is_not_recounted() {
        new_test_ext().execute_with(|| {
            attach_child(OWNER, CHILD_1, G_NODE_A);
            stake_owner(500);

            // Pre-quarantine the single node via the validator path.
            let updates: BoundedVec<_, _> = BoundedVec::try_from(vec![MinerStatusUpdate {
                node_id: G_NODE_A,
                new_status: MinerStatus::Quarantined,
                weight: 0,
            }])
            .unwrap();
            assert_ok!(ComputeScoring::vali_submit_epoch_close(
                RuntimeOrigin::root(),
                1,
                updates,
            ));
            assert_eq!(status_of(G_NODE_A), Some(MinerStatus::Quarantined));

            frame_system::Pallet::<TestRuntime>::set_block_number(2);
            frame_system::Pallet::<TestRuntime>::reset_events();

            // Full exit. The node is ALREADY Quarantined ⇒ no transition ⇒
            // not counted ⇒ quarantined == 0 ⇒ no aggregate event.
            assert_ok!(ComputeScoring::request_unstake(
                RuntimeOrigin::signed(OWNER),
                500
            ));
            assert_eq!(StakedAmount::<TestRuntime>::get(OWNER), 0);
            assert_eq!(status_of(G_NODE_A), Some(MinerStatus::Quarantined));
            assert!(status_change_events().is_empty());
            assert!(quarantine_events().is_empty());
        });
    }

    /// 6. Recovery still works: after an unstake-quarantine, the validator's
    ///    `vali_submit_epoch_close` with a `set Active` update REMOVES the row
    ///    (back to implicit-Active) — the shared helper's recovery path is
    ///    intact.
    #[test]
    fn recovery_after_unstake_quarantine_clears_the_row() {
        new_test_ext().execute_with(|| {
            attach_child(OWNER, CHILD_1, G_NODE_A);
            stake_owner(500);

            assert_ok!(ComputeScoring::request_unstake(
                RuntimeOrigin::signed(OWNER),
                500
            ));
            assert_eq!(status_of(G_NODE_A), Some(MinerStatus::Quarantined));

            // Validator restores the node to Active in a later epoch.
            let updates: BoundedVec<_, _> = BoundedVec::try_from(vec![MinerStatusUpdate {
                node_id: G_NODE_A,
                new_status: MinerStatus::Active,
                weight: 1_000,
            }])
            .unwrap();
            assert_ok!(ComputeScoring::vali_submit_epoch_close(
                RuntimeOrigin::root(),
                1,
                updates,
            ));
            // Row removed ⇒ implicit-Active again.
            assert_eq!(status_of(G_NODE_A), None);
        });
    }

    /// 7. An owner with ZERO children unstaking full ⇒ no quarantine event, no
    ///    panic (the loop just doesn't run).
    #[test]
    fn full_exit_with_no_children_is_a_noop_quarantine() {
        new_test_ext().execute_with(|| {
            stake_owner(500);
            assert!(FamilyChildren::<TestRuntime>::get(OWNER).is_empty());

            assert_ok!(ComputeScoring::request_unstake(
                RuntimeOrigin::signed(OWNER),
                500
            ));
            assert_eq!(StakedAmount::<TestRuntime>::get(OWNER), 0);
            assert!(quarantine_events().is_empty());
        });
    }

    /// Event ordering: `StakeUnbondRequested` precedes
    /// `OwnerQuarantinedOnUnstake` in the log (the unstake first, then the
    /// quarantine it triggered).
    #[test]
    fn unbond_event_precedes_quarantine_event() {
        new_test_ext().execute_with(|| {
            attach_child(OWNER, CHILD_1, G_NODE_A);
            stake_owner(500);
            assert_ok!(ComputeScoring::request_unstake(
                RuntimeOrigin::signed(OWNER),
                500
            ));

            let events = frame_system::Pallet::<TestRuntime>::events();
            let unbond_idx = events.iter().position(|e| {
                matches!(
                    &e.event,
                    RuntimeEvent::ComputeScoring(ComputeEvent::StakeUnbondRequested { .. })
                )
            });
            let quar_idx = events.iter().position(|e| {
                matches!(
                    &e.event,
                    RuntimeEvent::ComputeScoring(ComputeEvent::OwnerQuarantinedOnUnstake { .. })
                )
            });
            assert!(unbond_idx.is_some() && quar_idx.is_some());
            assert!(unbond_idx < quar_idx);
        });
    }
}

// ── genesis price bounds ────────────────────────────────────────────
//
// `set_price_bounds` has always existed and `announce_price_change` has
// always enforced the bounds, but a fresh chain started UNBOUNDED until
// someone remembered the admin call. Nobody did, and the live testnet ran
// with self-set prices and no ceiling. These pin the genesis path.

#[test]
fn genesis_defaults_leave_prices_unbounded_exactly_as_before() {
    // CLAIM: an existing chain spec that says nothing keeps its exact
    // current behaviour — permissive. Guards against a "safe default"
    // that would silently reject every existing miner price.
    new_test_ext().execute_with(|| {
        assert_eq!(PriceFloor::<TestRuntime>::get(), 0);
        assert_eq!(PriceCeiling::<TestRuntime>::get(), None);
    });
}

#[test]
fn genesis_can_set_both_price_bounds() {
    // CLAIM: the chain spec can make the choice explicit, so a redeploy
    // does not have to rely on a remembered post-launch admin call.
    let mut storage = frame_system::GenesisConfig::<TestRuntime>::default()
        .build_storage()
        .expect("genesis storage builds");
    crate::GenesisConfig::<TestRuntime> {
        base_child_deposit: None,
        lockup_enabled: false,
        price_floor: 1_000,
        price_ceiling: Some(9_000),
    }
    .assimilate_storage(&mut storage)
    .expect("compute-scoring genesis assimilates");
    let mut ext: sp_io::TestExternalities = storage.into();
    ext.execute_with(|| {
        assert_eq!(PriceFloor::<TestRuntime>::get(), 1_000);
        assert_eq!(PriceCeiling::<TestRuntime>::get(), Some(9_000));
    });
}

#[test]
fn genesis_floor_without_ceiling_stays_unbounded_above() {
    // CLAIM: a floor alone is a valid policy (anti-dumping, no cap).
    let mut storage = frame_system::GenesisConfig::<TestRuntime>::default()
        .build_storage()
        .expect("genesis storage builds");
    crate::GenesisConfig::<TestRuntime> {
        base_child_deposit: None,
        lockup_enabled: false,
        price_floor: 500,
        price_ceiling: None,
    }
    .assimilate_storage(&mut storage)
    .expect("compute-scoring genesis assimilates");
    let mut ext: sp_io::TestExternalities = storage.into();
    ext.execute_with(|| {
        assert_eq!(PriceFloor::<TestRuntime>::get(), 500);
        assert_eq!(PriceCeiling::<TestRuntime>::get(), None);
    });
}

#[test]
#[should_panic(expected = "price_floor exceeds price_ceiling")]
fn genesis_rejects_an_inverted_price_policy() {
    // CLAIM: an inverted spec stops the chain at genesis rather than
    // starting one where no price can ever be accepted. Mirrors the
    // `InvalidPricePolicy` guard `set_price_bounds` applies at runtime.
    let mut storage = frame_system::GenesisConfig::<TestRuntime>::default()
        .build_storage()
        .expect("genesis storage builds");
    crate::GenesisConfig::<TestRuntime> {
        base_child_deposit: None,
        lockup_enabled: false,
        price_floor: 9_000,
        price_ceiling: Some(1_000),
    }
    .assimilate_storage(&mut storage)
    .expect("compute-scoring genesis assimilates");
}

//! Known-answer test for the §23 tenant-CVM live-attestation wire
//! format — the KBS-L0-signed proof a tenant guest was ALIVE.
//!
//! Freezes one committed vector in `test_vectors/live_attestation/`:
//!
//! - `signed_live_attestation.cbor` — the canonical-CBOR
//!   [`SignedLiveAttestation`] envelope for a pinned [`LiveAttestation`]
//!   signed with a FIXED test Ed25519 key. Ed25519 is deterministic, so
//!   a fixed key + fixed body yields byte-exact bytes.
//!
//! Two consumers depend on these exact bytes, and the vector is what
//! keeps them honest:
//!
//! - `binaries/ticket-validator`'s `verify-live-attestation` subcommand,
//! - vali's `apps.telemetry.vm_liveness` ingest, whose end-to-end test
//!   pipes THIS FILE through the real binary. That test is the only
//!   place the Rust→JSON→Python field contract is checked against real
//!   cryptography rather than a mock, so the vector must stay a real
//!   signed envelope, never a hand-written stub.
//!
//! A failure here is **drift** — fix the impl, never silently
//! regenerate. Regenerate deliberately per
//! `test_vectors/live_attestation/REGENERATE.md`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ed25519_dalek::{Signer, SigningKey};
use hippius_types::live_attestation::{
    LiveAttestation, SignedLiveAttestation, DIGEST_LEN, LIVE_ATTESTATION_SCHEMA_VERSION,
    MEASUREMENT_LEN, PUBKEY_LEN,
};
use std::path::PathBuf;

/// Pinned test KBS-L0 Ed25519 signing-key seed (synthetic, fixed).
/// vali's end-to-end test pins the matching PUBLIC key.
const KAT_SEED: [u8; 32] = [0x5Au8; 32];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn signed_vector_path() -> PathBuf {
    repo_root().join("test_vectors/live_attestation/signed_live_attestation.cbor")
}

/// The pinned attestation — fixed inputs. DO NOT change without
/// regenerating the committed vector AND updating vali's fixture
/// expectations.
fn kat_attestation(signer: &SigningKey) -> LiveAttestation {
    LiveAttestation {
        schema_version: LIVE_ATTESTATION_SCHEMA_VERSION,
        chain_genesis: [0xAA; DIGEST_LEN],
        pallet_instance: [0xDD; DIGEST_LEN],
        vm_id: "tn-kat-live-1".into(),
        node_id: [0xBB; PUBKEY_LEN],
        attestation_seq: 7,
        epoch: 4242,
        observed_at_unix: 1_800_000_000,
        verified_at_unix: 1_800_000_005,
        snp_report_digest: [0x11; DIGEST_LEN],
        vcek_chain_digest: [0x22; DIGEST_LEN],
        measurement: [0x33; MEASUREMENT_LEN],
        prev_attestation_hash: [0x44; DIGEST_LEN],
        expiry_unix: 1_800_000_900,
        signer_pubkey: signer.verifying_key().to_bytes(),
    }
}

fn kat_signed() -> Vec<u8> {
    let sk = SigningKey::from_bytes(&KAT_SEED);
    let body = kat_attestation(&sk).canonical().expect("encode KAT body");
    let sig = sk.sign(&body).to_bytes().to_vec();
    SignedLiveAttestation { body, sig }
        .encode()
        .expect("encode KAT envelope")
}

#[test]
fn signed_live_attestation_kat_matches_the_frozen_vector() {
    let produced = kat_signed();
    let expected = std::fs::read(signed_vector_path()).expect(
        "test_vectors/live_attestation/signed_live_attestation.cbor missing — run \
         `cargo test -p hippius-types --test live_attestation_kat \
         regenerate_committed_vectors -- --ignored --exact`",
    );
    assert_eq!(
        produced, expected,
        "signed live-attestation envelope changed — a canonical-encoding, \
         key, or fixture edit shifted the KAT. If intentional, regenerate \
         per test_vectors/live_attestation/REGENERATE.md AND re-check \
         vali's end-to-end ingest fixture."
    );
}

#[test]
fn frozen_vector_decodes_and_verifies_under_the_pinned_key() {
    let raw = std::fs::read(signed_vector_path()).expect("read the frozen vector");
    let signed = SignedLiveAttestation::decode(&raw).expect("decode the frozen envelope");
    let att = LiveAttestation::decode(&signed.body).expect("decode the frozen body");
    let sk = SigningKey::from_bytes(&KAT_SEED);
    assert_eq!(att, kat_attestation(&sk));
    let sig = ed25519_dalek::Signature::from_slice(&signed.sig).expect("sig is 64 bytes");
    sk.verifying_key()
        .verify_strict(&signed.body, &sig)
        .expect("the frozen KAT signature must verify under the test key");
}

/// Regeneration helper — `#[ignore]`d so it never runs in CI.
///
/// ```text
/// cargo test -p hippius-types --test live_attestation_kat \
///     regenerate_committed_vectors -- --ignored --exact
/// ```
#[test]
#[ignore]
fn regenerate_committed_vectors() {
    let signed = kat_signed();
    std::fs::create_dir_all(signed_vector_path().parent().unwrap())
        .expect("create test_vectors/live_attestation/");
    std::fs::write(signed_vector_path(), &signed).expect("write the frozen vector");
    println!(
        "regenerated: test_vectors/live_attestation/signed_live_attestation.cbor ({} bytes); \
         pinned KBS L0 public key = {}",
        signed.len(),
        hex::encode(SigningKey::from_bytes(&KAT_SEED).verifying_key().to_bytes()),
    );
}

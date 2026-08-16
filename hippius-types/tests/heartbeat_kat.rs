//! Known-answer test for the §K miner heartbeat wire format (PR-MA-6).
//!
//! Freezes two committed vectors in `test_vectors/heartbeat/`:
//!
//! - `heartbeat_body.cbor` — the canonical-CBOR encoding of a pinned
//!   [`MinerHeartbeat`]. A fixed struct → a byte-exact body. Any
//!   canonical-encoding change in `hippius_types::heartbeat` shifts
//!   this and fails CI loudly.
//! - `signed_heartbeat.cbor` — the canonical-CBOR encoding of the
//!   [`SignedMinerHeartbeat`] envelope produced by signing that body
//!   with a FIXED test Ed25519 key. Ed25519 is deterministic, so a
//!   fixed key + fixed body yields a byte-exact signed envelope.
//!
//! A parallel implementation (TS / Python tooling, an audit tool)
//! MUST reproduce these exact bytes for the same inputs. A failure is
//! **drift** — fix the impl, never silently regenerate the vector.
//!
//! Regenerate deliberately — see `test_vectors/heartbeat/REGENERATE.md`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use ed25519_dalek::{Signer, SigningKey};
use hippius_types::heartbeat::{MinerHeartbeat, SignedMinerHeartbeat, DOMAIN, SCHEMA_VERSION};
use std::path::PathBuf;

// ── Pinned KAT input tuple ──────────────────────────────────────────

/// Pinned test Ed25519 signing-key seed (synthetic, fixed).
const KAT_SEED: [u8; 32] = [0x3Bu8; 32];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn body_vector_path() -> PathBuf {
    repo_root().join("test_vectors/heartbeat/heartbeat_body.cbor")
}

fn signed_vector_path() -> PathBuf {
    repo_root().join("test_vectors/heartbeat/signed_heartbeat.cbor")
}

/// The pinned heartbeat — fixed inputs. DO NOT change without bumping
/// `test_vectors` and regenerating the committed vectors.
fn kat_heartbeat() -> MinerHeartbeat {
    MinerHeartbeat {
        schema_version: SCHEMA_VERSION,
        miner_id: "miner-a".into(),
        timestamp_unix: 1_700_000_000,
        sequence: 42,
        vm_count_running: 3,
        vm_count_total: 5,
        cpu_load_1m_centi: 175,
        memory_total_mib: 262_144,
        memory_available_mib: 131_072,
        domain: DOMAIN.into(),
        // The KAT is the `v1` baseline — the flag is never present in a
        // `v1` body, so a `false` here leaves the frozen bytes unchanged.
        graceful_exit_requested: false,
    }
}

/// Sign the pinned body with the fixed test key — deterministic.
fn kat_signed() -> SignedMinerHeartbeat {
    let body = kat_heartbeat().canonical().expect("encode KAT body");
    let sk = SigningKey::from_bytes(&KAT_SEED);
    let sig = sk.sign(&body);
    SignedMinerHeartbeat {
        body,
        sig: sig.to_bytes().to_vec(),
    }
}

#[test]
fn heartbeat_body_kat_matches_the_frozen_vector() {
    let produced = kat_heartbeat().canonical().expect("encode KAT body");
    let expected = std::fs::read(body_vector_path()).expect(
        "test_vectors/heartbeat/heartbeat_body.cbor missing — run \
         `cargo test -p hippius-types --test heartbeat_kat \
         regenerate_committed_vectors -- --ignored --exact`",
    );
    assert_eq!(
        produced, expected,
        "heartbeat canonical-CBOR body changed — a canonical-encoding or \
         fixture edit shifted the KAT. If intentional, regenerate per \
         test_vectors/heartbeat/REGENERATE.md."
    );
}

#[test]
fn signed_heartbeat_kat_matches_the_frozen_vector() {
    let produced = kat_signed().canonical().expect("encode KAT envelope");
    let expected = std::fs::read(signed_vector_path()).expect(
        "test_vectors/heartbeat/signed_heartbeat.cbor missing — run \
         `cargo test -p hippius-types --test heartbeat_kat \
         regenerate_committed_vectors -- --ignored --exact`",
    );
    assert_eq!(
        produced, expected,
        "signed heartbeat envelope changed — a canonical-encoding, key, \
         or fixture edit shifted the KAT. If intentional, regenerate per \
         test_vectors/heartbeat/REGENERATE.md."
    );
}

#[test]
fn frozen_signed_vector_decodes_and_verifies() {
    // The committed envelope decodes to the pinned tuple and its
    // signature verifies under the fixed test key — independent of the
    // byte-exact check above (catches a fixture swap).
    let raw = std::fs::read(signed_vector_path()).expect("read signed_heartbeat.cbor");
    let signed: SignedMinerHeartbeat =
        ciborium::de::from_reader(raw.as_slice()).expect("decode the frozen envelope");
    assert_eq!(signed.body, kat_heartbeat().canonical().unwrap());
    let vk = SigningKey::from_bytes(&KAT_SEED).verifying_key();
    let sig = ed25519_dalek::Signature::from_slice(&signed.sig).expect("sig is 64 bytes");
    vk.verify_strict(&signed.body, &sig)
        .expect("the frozen KAT signature must verify under the test key");
}

/// Regeneration helper — `#[ignore]`d so it never runs in CI. Run it
/// deliberately to (re)write the committed KAT vectors after an
/// intentional change:
///
/// ```text
/// cargo test -p hippius-types --test heartbeat_kat \
///     regenerate_committed_vectors -- --ignored --exact
/// ```
#[test]
#[ignore]
fn regenerate_committed_vectors() {
    let body = kat_heartbeat().canonical().expect("encode KAT body");
    let signed = kat_signed().canonical().expect("encode KAT envelope");
    std::fs::write(body_vector_path(), &body).expect("write heartbeat_body.cbor");
    std::fs::write(signed_vector_path(), &signed).expect("write signed_heartbeat.cbor");
    println!(
        "regenerated: test_vectors/heartbeat/heartbeat_body.cbor ({} bytes), \
         signed_heartbeat.cbor ({} bytes)",
        body.len(),
        signed.len()
    );
}

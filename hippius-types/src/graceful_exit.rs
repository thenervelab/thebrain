//! Miner-agent **graceful-exit request** — the miner-self-service signal
//! that it wants to stop hosting so vali can WARM-migrate its VMs off
//! before it leaves (and, where staking is enabled, before its stake
//! unbonds). ARCHITECTURE.md §13/§25.
//!
//! This is a SEPARATE signed payload from the periodic
//! [`crate::heartbeat`] — deliberately so. The heartbeat is the
//! liveness-critical, high-frequency channel that the scheduler's
//! dispatchability gate keys off; adding fields to it (and bumping its
//! `SCHEMA_VERSION`) risks breaking a miner's liveness on a version
//! skew. A graceful-exit is a rare, one-shot, miner-initiated event, so
//! it gets its own low-frequency envelope + its own replay domain.
//!
//! The miner-agent signs a [`GracefulExitRequest`] body with the SAME
//! Ed25519 identity key it signs heartbeats with; vali verifies the
//! [`SignedGracefulExit`] envelope against the out-of-band-registered
//! miner key (the same trust anchor as the heartbeat), enforces a
//! monotonic `sequence` (replay defence) and a `±300 s` skew window,
//! then quarantines that miner in its own `MinerIdentity` registry ⇒
//! the §13/§25 auto-migration enrols a warm migration of its VMs.
//!
//! [`DOMAIN`] is **distinct** from every other signed-payload domain in
//! the stack (`HIPPIUS_MINER_HEARTBEAT_V1`, `RELEASE_DOMAIN`, …) so a
//! signature minted over a graceful-exit body can never be lifted into
//! another scheme; the domain tag is itself a signed field.

use alloc::string::String;
use alloc::vec::Vec;

use ciborium::value::Value;
use serde::{Deserialize, Serialize};

use crate::cbor::to_canonical_vec;

/// Replay-domain separator — the first field (by sort order) of every
/// signed graceful-exit body. Distinct from every other signed-payload
/// domain in the stack.
pub const DOMAIN: &str = "HIPPIUS_MINER_GRACEFUL_EXIT_V1";

/// Wire-format version. An unknown value fails closed at [`GracefulExitRequest::validate`].
pub const SCHEMA_VERSION: u8 = 1;

/// Length of an Ed25519 signature — the `sig` field of the envelope.
pub const SIGNATURE_LEN: usize = 64;

/// Max accepted `miner_id` length (matches the heartbeat bound).
pub const MAX_MINER_ID_LEN: usize = 64;

/// Errors raised encoding / validating a graceful-exit request.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GracefulExitError {
    /// `schema_version` was not [`SCHEMA_VERSION`].
    #[error("graceful-exit-schema-version")]
    SchemaVersion,
    /// `domain` was not [`DOMAIN`].
    #[error("graceful-exit-domain")]
    Domain,
    /// `miner_id` empty or over [`MAX_MINER_ID_LEN`].
    #[error("graceful-exit-miner-id")]
    MinerId,
    /// `sig` was not [`SIGNATURE_LEN`] bytes.
    #[error("graceful-exit-signature-length")]
    SignatureLength,
    /// Deterministic-CBOR encode failed.
    #[error("graceful-exit-encode")]
    Encode,
}

/// Result alias for this module.
pub type Result<T> = core::result::Result<T, GracefulExitError>;

/// The body the miner-agent signs (and vali re-derives + verifies) to
/// request a graceful exit. The fields pin the request to a single
/// miner at a single instant:
///
/// - `miner_id` — which registered miner (the `vali` registry key);
/// - `timestamp_unix` — the miner's wall-clock, anti-skew-checked;
/// - `sequence` — a per-miner monotonic counter (replay defence; vali
///   rejects any value ≤ the last accepted graceful-exit sequence);
/// - `domain` — the replay-context separator (also in the signed bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GracefulExitRequest {
    /// Wire-format version. MUST be [`SCHEMA_VERSION`].
    pub schema_version: u8,
    /// Operator-assigned miner identifier — the `vali` registry key.
    pub miner_id: String,
    /// The miner's wall-clock at build time, Unix seconds.
    pub timestamp_unix: i64,
    /// Per-miner monotonic counter (replay defence).
    pub sequence: u64,
    /// Replay-domain tag. MUST be [`DOMAIN`].
    pub domain: String,
}

impl GracefulExitRequest {
    /// Every semantic invariant of a well-formed request. Run by
    /// [`canonical`](Self::canonical) so a signature over an impossible
    /// value can never be produced; a decoder MUST run it too.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(GracefulExitError::SchemaVersion);
        }
        if self.domain != DOMAIN {
            return Err(GracefulExitError::Domain);
        }
        if self.miner_id.is_empty() || self.miner_id.len() > MAX_MINER_ID_LEN {
            return Err(GracefulExitError::MinerId);
        }
        Ok(())
    }

    /// Deterministic-CBOR encoding of the to-be-signed body — the
    /// signature preimage. Keys are emitted in sorted order (the
    /// `to_canonical_vec` invariant); `validate` runs first so an
    /// impossible body never reaches the signer.
    pub fn canonical(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let v = Value::Map(alloc::vec![
            (Value::Text("domain".into()), Value::Text(DOMAIN.into())),
            (
                Value::Text("miner_id".into()),
                Value::Text(self.miner_id.clone()),
            ),
            (
                Value::Text("schema_version".into()),
                Value::Integer(self.schema_version.into()),
            ),
            (
                Value::Text("sequence".into()),
                Value::Integer(self.sequence.into()),
            ),
            (
                Value::Text("timestamp_unix".into()),
                Value::Integer(self.timestamp_unix.into()),
            ),
        ]);
        to_canonical_vec(&v).map_err(|_| GracefulExitError::Encode)
    }
}

/// The `{body, sig}` envelope the miner-agent POSTs to vali. `body` is
/// the canonical [`GracefulExitRequest`] bytes; `sig` is the Ed25519
/// signature over them by the miner's identity key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedGracefulExit {
    #[serde(with = "serde_bytes")]
    pub body: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub sig: Vec<u8>,
}

impl SignedGracefulExit {
    /// Deterministic-CBOR encoding of the `{body, sig}` envelope — the
    /// exact bytes the miner-agent POSTs and vali's verifier ingests.
    pub fn canonical(&self) -> Result<Vec<u8>> {
        if self.sig.len() != SIGNATURE_LEN {
            return Err(GracefulExitError::SignatureLength);
        }
        let v = Value::Map(alloc::vec![
            (Value::Text("body".into()), Value::Bytes(self.body.clone())),
            (Value::Text("sig".into()), Value::Bytes(self.sig.clone())),
        ]);
        to_canonical_vec(&v).map_err(|_| GracefulExitError::Encode)
    }

    /// Extract `timestamp_unix` from the canonical-CBOR `body` without a
    /// full struct decode — the miner-agent's pusher uses it to drop a
    /// request that has aged past vali's skew window (a signed envelope
    /// cannot be re-signed without breaking the canonical invariant).
    pub fn timestamp_unix(&self) -> Option<i64> {
        let v: Value = ciborium::de::from_reader(self.body.as_slice()).ok()?;
        let entries = match v {
            Value::Map(e) => e,
            _ => return None,
        };
        for (k, val) in entries {
            if matches!(&k, Value::Text(t) if t == "timestamp_unix") {
                if let Value::Integer(i) = val {
                    let n: i128 = i.into();
                    return i64::try_from(n).ok();
                }
                return None;
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cbor::assert_canonical;

    fn sample() -> GracefulExitRequest {
        GracefulExitRequest {
            schema_version: SCHEMA_VERSION,
            miner_id: "miner-a".into(),
            timestamp_unix: 1_700_000_000,
            sequence: 7,
            domain: DOMAIN.into(),
        }
    }

    #[test]
    fn canonical_round_trips_and_is_canonical() {
        let body = sample().canonical().unwrap();
        assert_canonical(&body).unwrap();
        let env = SignedGracefulExit {
            body: body.clone(),
            sig: alloc::vec![9u8; SIGNATURE_LEN],
        };
        let enc = env.canonical().unwrap();
        assert_canonical(&enc).unwrap();
        assert_eq!(env.timestamp_unix(), Some(1_700_000_000));
    }

    #[test]
    fn bad_schema_version_fails_closed() {
        let mut r = sample();
        r.schema_version = 9;
        assert_eq!(r.validate(), Err(GracefulExitError::SchemaVersion));
        assert!(r.canonical().is_err());
    }

    #[test]
    fn bad_domain_and_miner_id_fail_closed() {
        let mut r = sample();
        r.domain = "WRONG".into();
        assert_eq!(r.validate(), Err(GracefulExitError::Domain));
        let mut r = sample();
        r.miner_id = String::new();
        assert_eq!(r.validate(), Err(GracefulExitError::MinerId));
    }

    #[test]
    fn envelope_rejects_wrong_sig_length() {
        let env = SignedGracefulExit {
            body: sample().canonical().unwrap(),
            sig: alloc::vec![0u8; 63],
        };
        assert_eq!(env.canonical(), Err(GracefulExitError::SignatureLength));
    }
}

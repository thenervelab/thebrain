//! Guest-signed end-of-life acknowledgement (ARCHITECTURE.md §24/§25).
//!
//! When a VM shuts down — voluntary EOL or as part of a migration drain
//! — the guest agent emits a single signed message proving that THIS
//! generation of THIS lease will not run again. The orchestrator (vali)
//! requires the ack before:
//! - committing `Destroyed{gen}` in §24 (no zombie that could re-attest);
//! - activating the destination guest in a §25 migration (no
//!   authorized split-brain — the KBS gives the secret to ≤1 host/gen).
//!
//! Wire is deterministic CBOR over a fixed schema, signed Ed25519 by
//! the guest's lifecycle key (provisioned via the §7 attested release,
//! same crypto profile as the rest of the protocol).

#[allow(unused_imports)]
// per-module slice of the alloc prelude — not every module needs every item
use alloc::{
    boxed::Box,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use crate::cbor::to_canonical_vec;
use crate::{HippiusTypesError, Result};
use ciborium::value::Value;
use serde::{Deserialize, Serialize};

/// Domain tag — distinct from `RELEASE_DOMAIN` / `DENIAL_DOMAIN` so a
/// stopped-ack signature can never be mis-bound to a release response
/// even with a key-confusion bug.
pub const STOPPED_DOMAIN: &str = "HIPPIUS_GUEST_STOPPED_V1";

/// The body the guest signs. Fields chosen to ALONE pin the message to
/// a single VM lifecycle event:
/// - `vm_id` — which VM,
/// - `lease_id` — which lease (anti-replay across re-leasings),
/// - `vm_generation` — which generation (anti-replay across §25
///   migrations and §24 destroy+re-create),
/// - `nonce` — single-use freshness from the orchestrator,
/// - `now_unix` — guest's view of stop time (orchestrator should sanity
///   check vs its own clock window; spec leaves the bound to ops).
///
/// `domain` is signed but redundant on the wire (it's also the CBOR
/// `domain` field; the encoded bytes include it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoppedAck<'a> {
    pub vm_id: &'a str,
    pub lease_id: &'a str,
    pub vm_generation: u64,
    pub nonce: &'a [u8; 32],
    pub now_unix: u64,
}

impl StoppedAck<'_> {
    /// Deterministic-CBOR encoding (stable across guest, orchestrator,
    /// and KBS). The orchestrator verifies the same bytes.
    pub fn canonical(&self) -> Result<Vec<u8>> {
        let v = Value::Map(vec![
            (
                Value::Text("domain".into()),
                Value::Text(STOPPED_DOMAIN.into()),
            ),
            (
                Value::Text("lease_id".into()),
                Value::Text(self.lease_id.into()),
            ),
            (
                Value::Text("nonce".into()),
                Value::Bytes(self.nonce.to_vec()),
            ),
            (
                Value::Text("now_unix".into()),
                Value::Integer(self.now_unix.into()),
            ),
            (
                Value::Text("vm_generation".into()),
                Value::Integer(self.vm_generation.into()),
            ),
            (Value::Text("vm_id".into()), Value::Text(self.vm_id.into())),
        ]);
        to_canonical_vec(&v).map_err(|e| HippiusTypesError::Cbor(format!("stopped ack: {e}")))
    }
}

/// Signed envelope. `body` is the canonical CBOR of [`StoppedAck`];
/// `sig` is Ed25519 over `body`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedStoppedAck {
    #[serde(with = "serde_bytes")]
    pub body: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub sig: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cbor::assert_canonical;

    #[test]
    fn ack_canonical_is_stable_and_canonical() {
        let n = [3u8; 32];
        let a = StoppedAck {
            vm_id: "abc",
            lease_id: "l1",
            vm_generation: 7,
            nonce: &n,
            now_unix: 1_000_000,
        };
        let b1 = a.canonical().unwrap();
        let b2 = a.canonical().unwrap();
        assert_eq!(b1, b2);
        assert_canonical(&b1).unwrap();
    }

    #[test]
    fn changing_generation_changes_signed_bytes() {
        let n = [3u8; 32];
        let a1 = StoppedAck {
            vm_id: "abc",
            lease_id: "l1",
            vm_generation: 7,
            nonce: &n,
            now_unix: 1_000_000,
        };
        let mut a2 = a1.clone();
        a2.vm_generation = 8;
        assert_ne!(a1.canonical().unwrap(), a2.canonical().unwrap());
    }
}

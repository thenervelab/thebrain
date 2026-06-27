//! Tenant-VM `ServedDeliveryReceipt` (ARCHITECTURE.md §23).
//!
//! The signed periodic attestation a tenant VM emits to claim "I served
//! work for this lease in this window". Spec §23:
//!
//! > A per-VM telemetry agent key is established inside the measured
//! > guest under the §7/§21 attested release. The guest signs periodic
//! > receipts over `{validator nonce/challenge, epoch, vm_id, lease_id,
//! > family_id, compute node_id, resource_class, monotonic_seq,
//! > observed_degradation, expiry}`. Reward only if the receipt digest
//! > is inside the Audit-VM-co-signed `ServedDeliveryAggregate` within
//! > its nonce window.
//!
//! `RECEIPT_DOMAIN` is distinct from every other signed payload in the
//! stack (release, denial, stopped-ack, audit-vm aggregate) so a
//! signature lifted from one scheme cannot replay into any other.

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
use sha2::{Digest, Sha256};

pub const RECEIPT_DOMAIN: &str = "HIPPIUS_TENANT_SERVED_RECEIPT_V1";

/// `observed_degradation` is a 0–10_000 hundredths-of-percent value the
/// tenant honestly reports about its own delivery. Host-claimed inputs
/// (§23) may only DECREASE or corroborate reward — they never raise it.
/// The validator clamps + uses this as one input to `f`.
pub const DEGRADATION_MAX_BPS: u32 = 10_000;

/// The body the tenant guest signs (and the validator re-derives).
///
/// Fields chosen to make a receipt unique within a (chain, validator,
/// epoch, family, node, vm, lease) window: a hostile party that lifts
/// the bytes cannot re-use them for a different VM, lease, or window
/// — every replay-domain piece is part of the signed body.
///
/// `validator_id` is bound (codex round-1 review): with only
/// `validator_nonce` for validator scoping, two validators that reused
/// the same fresh nonce could roll the same receipt into aggregates
/// under different `validator_id`s. Including `validator_id` in the
/// receipt body forces the tenant guest to commit to a specific
/// validator at sign time.
///
/// `monotonic_seq` is per-(vm_id, lease_id); the validator + Audit VM
/// detect any out-of-order or repeated sequence number within a window.
/// `observed_degradation` is in basis points (0–10_000) of "less than
/// full work served"; `canonical()` rejects values > [`DEGRADATION_MAX_BPS`].
///
/// `Debug` is implemented manually to print only field LENGTHS — the
/// schema otherwise leaks identifiers (`vm_id`, `lease_id`, nonce)
/// into operator logs by default. Use a redacted form intentionally
/// (codex+gemini Low).
#[derive(Clone)]
pub struct ServedDeliveryReceipt<'a> {
    pub validator_id: &'a [u8],
    pub validator_nonce: &'a [u8; 32],
    pub epoch: u64,
    pub vm_id: &'a str,
    pub lease_id: &'a str,
    pub family_id: &'a [u8],
    pub node_id: &'a [u8],
    pub resource_class: &'a str,
    pub monotonic_seq: u64,
    pub observed_degradation_bps: u32,
    pub period_start: u64,
    pub period_end: u64,
    pub expiry: u64,
}

impl core::fmt::Debug for ServedDeliveryReceipt<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ServedDeliveryReceipt")
            .field("validator_id_len", &self.validator_id.len())
            .field("validator_nonce_len", &self.validator_nonce.len())
            .field("epoch", &self.epoch)
            .field("vm_id_len", &self.vm_id.len())
            .field("lease_id_len", &self.lease_id.len())
            .field("family_id_len", &self.family_id.len())
            .field("node_id_len", &self.node_id.len())
            .field("resource_class_len", &self.resource_class.len())
            .field("monotonic_seq", &self.monotonic_seq)
            .field("observed_degradation_bps", &self.observed_degradation_bps)
            .field("period_start", &self.period_start)
            .field("period_end", &self.period_end)
            .field("expiry", &self.expiry)
            .finish()
    }
}

impl ServedDeliveryReceipt<'_> {
    /// Deterministic-CBOR encoding. Rejects malformed inputs at
    /// encode time so a signature over impossible values (inverted
    /// period, out-of-range degradation) cannot exist — fail closed.
    pub fn canonical(&self) -> Result<Vec<u8>> {
        if self.observed_degradation_bps > DEGRADATION_MAX_BPS {
            return Err(HippiusTypesError::Cbor(format!(
                "observed_degradation_bps out of range: {} > {}",
                self.observed_degradation_bps, DEGRADATION_MAX_BPS
            )));
        }
        if self.period_end < self.period_start {
            return Err(HippiusTypesError::Cbor(
                "receipt: period_end < period_start".into(),
            ));
        }
        if self.expiry < self.period_end {
            return Err(HippiusTypesError::Cbor(
                "receipt: expiry < period_end".into(),
            ));
        }
        let v = Value::Map(vec![
            (
                Value::Text("domain".into()),
                Value::Text(RECEIPT_DOMAIN.into()),
            ),
            (
                Value::Text("epoch".into()),
                Value::Integer(self.epoch.into()),
            ),
            (
                Value::Text("expiry".into()),
                Value::Integer(self.expiry.into()),
            ),
            (
                Value::Text("family_id".into()),
                Value::Bytes(self.family_id.to_vec()),
            ),
            (
                Value::Text("lease_id".into()),
                Value::Text(self.lease_id.into()),
            ),
            (
                Value::Text("monotonic_seq".into()),
                Value::Integer(self.monotonic_seq.into()),
            ),
            (
                Value::Text("node_id".into()),
                Value::Bytes(self.node_id.to_vec()),
            ),
            (
                Value::Text("observed_degradation_bps".into()),
                Value::Integer(u64::from(self.observed_degradation_bps).into()),
            ),
            (
                Value::Text("period_end".into()),
                Value::Integer(self.period_end.into()),
            ),
            (
                Value::Text("period_start".into()),
                Value::Integer(self.period_start.into()),
            ),
            (
                Value::Text("resource_class".into()),
                Value::Text(self.resource_class.into()),
            ),
            (
                Value::Text("validator_id".into()),
                Value::Bytes(self.validator_id.to_vec()),
            ),
            (
                Value::Text("validator_nonce".into()),
                Value::Bytes(self.validator_nonce.to_vec()),
            ),
            (Value::Text("vm_id".into()), Value::Text(self.vm_id.into())),
        ]);
        to_canonical_vec(&v).map_err(|e| HippiusTypesError::Cbor(format!("receipt: {e}")))
    }
}

/// Signed envelope. `body` is the canonical CBOR; `sig` is Ed25519
/// over `body` by the tenant telemetry key established via §7/§21.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedServedDeliveryReceipt {
    #[serde(with = "serde_bytes")]
    pub body: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub sig: Vec<u8>,
}

impl SignedServedDeliveryReceipt {
    /// Deterministic-CBOR encoding of the signed envelope itself — the
    /// `{body, sig}` map in canonical (RFC 8949 §4.2.1) form.
    ///
    /// `body` is already canonical (it is the inner receipt's
    /// `canonical()` output); this wraps it together with the
    /// signature so the whole envelope re-encodes to byte-identical
    /// bytes on every hop. PR-E2.3's tenant→host vsock frame is a
    /// length-prefixed blob of exactly these bytes.
    pub fn canonical(&self) -> Result<Vec<u8>> {
        let v = Value::Map(vec![
            (Value::Text("body".into()), Value::Bytes(self.body.clone())),
            (Value::Text("sig".into()), Value::Bytes(self.sig.clone())),
        ]);
        to_canonical_vec(&v).map_err(|e| HippiusTypesError::Cbor(format!("signed-receipt: {e}")))
    }
}

/// SHA-256 over the `body` of a signed receipt — the value that goes
/// into [`crate::audit_vm::ServedReceiptEntry::digest`] when the
/// Audit VM rolls receipts into a `ServedDeliveryAggregate`.
///
/// We digest the signed envelope's BODY (not body+sig) because the
/// receipt-set digest must be reproducible by any party that holds
/// the body (the body alone is what the validator stores + replays).
/// The signature is verified independently in the audit pipeline.
pub fn receipt_digest(signed: &SignedServedDeliveryReceipt) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(Sha256::digest(&signed.body).as_slice());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cbor::assert_canonical;

    fn base<'a>(nonce: &'a [u8; 32]) -> ServedDeliveryReceipt<'a> {
        ServedDeliveryReceipt {
            validator_id: b"validator-1",
            validator_nonce: nonce,
            epoch: 42,
            vm_id: "abc",
            lease_id: "lease-1",
            family_id: b"family-1",
            node_id: b"node-1",
            resource_class: "std",
            monotonic_seq: 1,
            observed_degradation_bps: 0,
            period_start: 1_000,
            period_end: 1_060,
            expiry: 2_000,
        }
    }

    #[test]
    fn canonical_is_stable_and_canonical() {
        let n = [3u8; 32];
        let r = base(&n);
        let a = r.canonical().unwrap();
        let b = r.canonical().unwrap();
        assert_eq!(a, b);
        assert_canonical(&a).unwrap();
    }

    #[test]
    fn degradation_out_of_range_rejected() {
        let n = [3u8; 32];
        let mut r = base(&n);
        r.observed_degradation_bps = DEGRADATION_MAX_BPS + 1;
        assert!(r.canonical().is_err());
    }

    #[test]
    fn period_inverted_rejected() {
        let n = [3u8; 32];
        let mut r = base(&n);
        r.period_start = 2_000;
        r.period_end = 1_000;
        assert!(r.canonical().is_err());
    }

    #[test]
    fn expiry_before_period_end_rejected() {
        let n = [3u8; 32];
        let mut r = base(&n);
        r.expiry = 100;
        r.period_end = 1_060;
        assert!(r.canonical().is_err());
    }

    #[test]
    fn changing_any_replay_field_changes_bytes() {
        let n = [3u8; 32];
        let r = base(&n);
        let base_bytes = r.canonical().unwrap();
        // Spot-check a representative field per category.
        let mut bumped = r.clone();
        bumped.epoch = 43;
        assert_ne!(base_bytes, bumped.canonical().unwrap());
        let mut bumped = r.clone();
        bumped.monotonic_seq = 2;
        assert_ne!(base_bytes, bumped.canonical().unwrap());
        let mut bumped = r.clone();
        bumped.resource_class = "high-mem";
        assert_ne!(base_bytes, bumped.canonical().unwrap());
        let mut bumped = r.clone();
        bumped.lease_id = "lease-2";
        assert_ne!(base_bytes, bumped.canonical().unwrap());
        let other = [9u8; 32];
        let mut bumped = r;
        bumped.validator_nonce = &other;
        assert_ne!(base_bytes, bumped.canonical().unwrap());
    }

    #[test]
    fn changing_validator_id_diverges() {
        // Two validators reusing the same fresh nonce would otherwise
        // be able to roll the same receipt into different aggregates.
        // Binding `validator_id` into the signed body kills that.
        let n = [3u8; 32];
        let mut r = base(&n);
        let bytes_a = r.canonical().unwrap();
        r.validator_id = b"validator-attacker";
        let bytes_b = r.canonical().unwrap();
        assert_ne!(bytes_a, bytes_b);
    }

    #[test]
    fn receipt_digest_is_stable() {
        let n = [3u8; 32];
        let body = base(&n).canonical().unwrap();
        let s = SignedServedDeliveryReceipt {
            body,
            sig: vec![0u8; 64],
        };
        let d1 = receipt_digest(&s);
        let d2 = receipt_digest(&s);
        assert_eq!(d1, d2);
        // Length sanity.
        assert_eq!(d1.len(), 32);
    }

    #[test]
    fn signed_receipt_canonical_is_stable_and_canonical() {
        let n = [3u8; 32];
        let body = base(&n).canonical().unwrap();
        let s = SignedServedDeliveryReceipt {
            body,
            sig: vec![7u8; 64],
        };
        let a = s.canonical().unwrap();
        let b = s.canonical().unwrap();
        // Deterministic — and genuinely canonical (keys byte-sorted:
        // `sig` encodes shorter than `body`, so the encoder reorders).
        assert_eq!(a, b);
        assert_canonical(&a).unwrap();
    }
}

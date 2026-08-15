//! KBS-issued telemetry-signer certificate (ARCHITECTURE.md §7/§21/§23).
//!
//! The §23 tenant telemetry-signer key is **established inside the
//! measured guest**: the guest generates the Ed25519 keypair, folds
//! `REPORT_DATA = nonce ‖ SHA-256(…signer_pubkey…)` (see
//! [`crate::report_data::tenant_telemetry`]) into its SNP attestation
//! report, and the KBS — once it has verified that report — issues
//! this certificate. The certificate is the KBS's signed statement
//! *"I verified a genuine measured tenant VM (`node_id`, `vm_id`)
//! controls this telemetry `signer_pubkey`"*. A receipt validator
//! (vali, §23) later trusts a `ServedDeliveryReceipt` only if its
//! signer matches a certificate like this.
//!
//! `TELEMETRY_CERT_DOMAIN` is distinct from every other signed payload
//! in the stack (release, denial, stopped-ack, served receipt, audit
//! aggregate, image provenance) so a signature minted for one scheme
//! can never be reinterpreted as another.
//!
//! Same shape as the other signed wire types here: a borrowed
//! [`TelemetryCert`] with a deterministic-CBOR [`canonical`](TelemetryCert::canonical)
//! encoder, wrapped by a [`SignedTelemetryCert`] `{body, sig}`
//! envelope. The verifier ([`hippius_guest`]) re-derives the canonical
//! body from the fields it independently knows and byte-compares
//! *before* checking the signature — a producer cannot smuggle bytes
//! the field checks never saw.

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

/// Replay-domain separator — the first field of every signed body.
pub const TELEMETRY_CERT_DOMAIN: &str = "HIPPIUS_TENANT_TELEMETRY_CERT_V1";

/// The only certificate schema version this build understands.
pub const TELEMETRY_CERT_SCHEMA_VERSION: u32 = 1;

/// Length of an Ed25519 public key / a SHA-256 nonce.
pub const PUBKEY_LEN: usize = 32;

/// Length of an SNP launch measurement (§20).
pub const MEASUREMENT_LEN: usize = 48;

/// The body the KBS signs to certify a guest-generated telemetry
/// signer key.
///
/// Every field is something both the KBS (from the verified SNP
/// report) and the guest (from its own state) compute independently —
/// so the guest can re-derive the exact body and reject a tampered
/// one. Borrowed (`&'a`) so the producer / verifier encode without
/// copying; `canonical()` is the single serialisation point.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelemetryCert<'a> {
    /// Schema version — MUST be [`TELEMETRY_CERT_SCHEMA_VERSION`].
    pub v: u32,
    /// Key id of the KBS response-signing key (the guest pins this).
    pub kbs_kid: &'a [u8],
    /// The single-use KBS nonce folded into `REPORT_DATA[0..32]`.
    pub kbs_nonce: &'a [u8; PUBKEY_LEN],
    /// The SNP launch measurement the KBS verified — the guest
    /// re-derives it from its own report and binds it here.
    pub measurement: &'a [u8; MEASUREMENT_LEN],
    /// The compute node the VM runs on.
    pub node_id: &'a [u8],
    /// The certified Ed25519 telemetry-signer public key.
    pub signer_pubkey: &'a [u8; PUBKEY_LEN],
    /// The VM this telemetry key belongs to.
    pub vm_id: &'a str,
}

impl TelemetryCert<'_> {
    /// Deterministic-CBOR encoding of the to-be-signed body. The KBS
    /// signs these exact bytes; the guest re-derives them.
    pub fn canonical(&self) -> Result<Vec<u8>> {
        // Source order is cosmetic — `to_canonical_vec` re-sorts to
        // the RFC 8949 §4.2.1 canonical key order.
        let v = Value::Map(vec![
            (
                Value::Text("domain".into()),
                Value::Text(TELEMETRY_CERT_DOMAIN.into()),
            ),
            (
                Value::Text("kbs_kid".into()),
                Value::Bytes(self.kbs_kid.to_vec()),
            ),
            (
                Value::Text("kbs_nonce".into()),
                Value::Bytes(self.kbs_nonce.to_vec()),
            ),
            (
                Value::Text("measurement".into()),
                Value::Bytes(self.measurement.to_vec()),
            ),
            (
                Value::Text("node_id".into()),
                Value::Bytes(self.node_id.to_vec()),
            ),
            (
                Value::Text("signer_pubkey".into()),
                Value::Bytes(self.signer_pubkey.to_vec()),
            ),
            (
                Value::Text("v".into()),
                Value::Integer(u64::from(self.v).into()),
            ),
            (Value::Text("vm_id".into()), Value::Text(self.vm_id.into())),
        ]);
        to_canonical_vec(&v).map_err(|e| HippiusTypesError::Cbor(format!("telemetry cert: {e}")))
    }
}

/// Signed envelope. `body` is the canonical CBOR of a [`TelemetryCert`];
/// `sig` is the KBS's Ed25519 signature over `body`.
///
/// `deny_unknown_fields` — the envelope is exactly `{body, sig}`; an
/// extra top-level key is a malformed response and is rejected at
/// decode (fail-closed parser hygiene).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedTelemetryCert {
    #[serde(with = "serde_bytes")]
    pub body: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub sig: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cbor::assert_canonical;

    fn sample<'a>(
        nonce: &'a [u8; 32],
        measurement: &'a [u8; 48],
        signer_pubkey: &'a [u8; 32],
    ) -> TelemetryCert<'a> {
        TelemetryCert {
            v: TELEMETRY_CERT_SCHEMA_VERSION,
            kbs_kid: b"kbs-kid-1",
            kbs_nonce: nonce,
            measurement,
            node_id: b"node-1",
            signer_pubkey,
            vm_id: "vm-1",
        }
    }

    #[test]
    fn canonical_is_stable_and_canonical() {
        let n = [3u8; 32];
        let m = [7u8; 48];
        let pk = [9u8; 32];
        let cert = sample(&n, &m, &pk);
        let a = cert.canonical().unwrap();
        let b = cert.canonical().unwrap();
        assert_eq!(a, b);
        assert_canonical(&a).unwrap();
    }

    #[test]
    fn changing_any_field_changes_the_signed_bytes() {
        let n = [3u8; 32];
        let m = [7u8; 48];
        let pk = [9u8; 32];
        let base = sample(&n, &m, &pk).canonical().unwrap();

        let mut bumped = sample(&n, &m, &pk);
        bumped.v = 2;
        assert_ne!(base, bumped.canonical().unwrap());

        let mut bumped = sample(&n, &m, &pk);
        bumped.kbs_kid = b"kbs-kid-2";
        assert_ne!(base, bumped.canonical().unwrap());

        let mut bumped = sample(&n, &m, &pk);
        bumped.node_id = b"node-2";
        assert_ne!(base, bumped.canonical().unwrap());

        let mut bumped = sample(&n, &m, &pk);
        bumped.vm_id = "vm-2";
        assert_ne!(base, bumped.canonical().unwrap());

        let other_nonce = [4u8; 32];
        let mut bumped = sample(&n, &m, &pk);
        bumped.kbs_nonce = &other_nonce;
        assert_ne!(base, bumped.canonical().unwrap());

        let other_meas = [8u8; 48];
        let mut bumped = sample(&n, &m, &pk);
        bumped.measurement = &other_meas;
        assert_ne!(base, bumped.canonical().unwrap());

        let other_pk = [0xAAu8; 32];
        let mut bumped = sample(&n, &m, &pk);
        bumped.signer_pubkey = &other_pk;
        assert_ne!(base, bumped.canonical().unwrap());
    }
}

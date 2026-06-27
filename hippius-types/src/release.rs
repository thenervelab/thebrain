//! Wire types for the KBS release contract (ARCHITECTURE.md §20). The
//! KBS produces these, the guest re-derives + verifies them.

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

pub const HPKE_SUITE_ID: u16 = 0x0001;
pub const HPKE_INFO: &[u8] = b"HIPPIUS_KBS_RELEASE_V1";
pub const RELEASE_DOMAIN: &str = "HIPPIUS_KBS_RELEASE_V1";
pub const DENIAL_DOMAIN: &str = "HIPPIUS_KBS_DENIAL_V1";

/// Per-secret release context. Bound as HPKE `info`+`aad` by the KBS;
/// carried (its fields) inside the signed `KbsResponse` so the guest
/// re-derives the identical context before decrypt.
///
/// `allowed_userdata_digest` is the ticket-pinned 32-byte SHA-256 over
/// the user-data plaintext (§6/§19/§20). Bundling it into the HPKE
/// info+aad creates a third independent binding (on top of the
/// signature check and the post-unwrap recompute), so a future code
/// change that accidentally relaxes either of those still cannot pair
/// a release wrapped for one digest with plaintext that hashes to
/// another — the AEAD tag mismatch fails the open.
#[derive(Debug, Clone)]
pub struct ReleaseContext<'a> {
    pub v: u32,
    pub ticket_id: &'a str,
    pub tenant_id: &'a str,
    pub vm_id: &'a str,
    pub vm_generation: u64,
    pub kbs_nonce: &'a [u8; 32],
    pub measurement: &'a [u8; 48],
    pub kbs_kid: &'a [u8],
    pub secret_type: &'a str,
    pub secret_path: &'a str,
    pub secret_version: u64,
    pub allowed_userdata_digest: &'a [u8; 32],
}

impl ReleaseContext<'_> {
    /// Deterministic-CBOR encoding (stable across KBS and guest).
    pub fn canonical(&self) -> Result<Vec<u8>> {
        let v = Value::Map(vec![
            (
                Value::Text("allowed_userdata_digest".into()),
                Value::Bytes(self.allowed_userdata_digest.to_vec()),
            ),
            (
                Value::Text("domain".into()),
                Value::Text(RELEASE_DOMAIN.into()),
            ),
            (
                Value::Text("hpke_suite_id".into()),
                Value::Integer(u64::from(HPKE_SUITE_ID).into()),
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
                Value::Text("secret_path".into()),
                Value::Text(self.secret_path.into()),
            ),
            (
                Value::Text("secret_type".into()),
                Value::Text(self.secret_type.into()),
            ),
            (
                Value::Text("secret_version".into()),
                Value::Integer(self.secret_version.into()),
            ),
            (
                Value::Text("tenant_id".into()),
                Value::Text(self.tenant_id.into()),
            ),
            (
                Value::Text("ticket_id".into()),
                Value::Text(self.ticket_id.into()),
            ),
            (
                Value::Text("v".into()),
                Value::Integer(u64::from(self.v).into()),
            ),
            (
                Value::Text("vm_generation".into()),
                Value::Integer(self.vm_generation.into()),
            ),
            (Value::Text("vm_id".into()), Value::Text(self.vm_id.into())),
        ]);
        to_canonical_vec(&v).map_err(|e| HippiusTypesError::Cbor(format!("release context: {e}")))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WrappedSecret {
    pub secret_type: String,
    pub secret_path: String,
    pub secret_version: u64,
    #[serde(with = "serde_bytes")]
    pub enc: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub ct: Vec<u8>,
}

/// The KBS-signed release response body (§20). The signature lives on
/// the enclosing [`SignedResponse`] and covers the canonical-CBOR
/// encoding of this struct.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct KbsResponse {
    pub domain: String,
    pub v: u32,
    pub ticket_id: String,
    pub tenant_id: String,
    pub vm_id: String,
    pub vm_generation: u64,
    #[serde(with = "serde_bytes")]
    pub kbs_nonce: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub measurement: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub kbs_kid: Vec<u8>,
    pub hpke_suite_id: u16,
    #[serde(with = "serde_bytes")]
    pub allowed_userdata_digest: Vec<u8>,
    pub luks: WrappedSecret,
    pub userdata: WrappedSecret,
    /// Phase 2A of audit follow-up Codex #2 — the per-`vm_id`
    /// monotonic counter the KBS committed for this release. On
    /// first boot the response carries `0` (no counter check ran);
    /// on subsequent boots — once the guest has opted in by submitting
    /// `ReleaseRequestBody::submitted_boot_counter` — this echoes
    /// the value the guest submitted, so the guest can persist it
    /// post-unlock for the next boot's comparison.
    ///
    /// `#[serde(default)]` so a pre-Phase-2A signed response (no
    /// field on the wire) decodes to `0` on a newer guest.
    /// Cryptographic note: the signature covers whatever bytes were
    /// canonicalised at sign time, so old-KBS-into-new-guest never
    /// trips a signature-verify failure — the field just defaults.
    #[serde(default)]
    pub boot_counter: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedResponse {
    #[serde(with = "serde_bytes")]
    pub body: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub sig: Vec<u8>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedDenial {
    #[serde(with = "serde_bytes")]
    pub body: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub sig: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_context_canonical_is_stable() {
        let nonce = [1u8; 32];
        let meas = [7u8; 48];
        let digest = [9u8; 32];
        let ctx = ReleaseContext {
            v: 1,
            ticket_id: "tk-1",
            tenant_id: "t",
            vm_id: "abc",
            vm_generation: 5,
            kbs_nonce: &nonce,
            measurement: &meas,
            kbs_kid: b"kbs-kid",
            secret_type: "luks",
            secret_path: "kbs/vm/abc/luks",
            secret_version: 3,
            allowed_userdata_digest: &digest,
        };
        let a = ctx.canonical().unwrap();
        let b = ctx.canonical().unwrap();
        assert_eq!(a, b); // determinism
        crate::cbor::assert_canonical(&a).unwrap();
    }

    #[test]
    fn different_secret_type_diverges() {
        let nonce = [1u8; 32];
        let meas = [7u8; 48];
        let digest = [9u8; 32];
        let mut ctx = ReleaseContext {
            v: 1,
            ticket_id: "tk-1",
            tenant_id: "t",
            vm_id: "abc",
            vm_generation: 5,
            kbs_nonce: &nonce,
            measurement: &meas,
            kbs_kid: b"kbs-kid",
            secret_type: "luks",
            secret_path: "kbs/vm/abc/luks",
            secret_version: 3,
            allowed_userdata_digest: &digest,
        };
        let a = ctx.canonical().unwrap();
        ctx.secret_type = "userdata";
        let b = ctx.canonical().unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn different_allowed_userdata_digest_diverges() {
        let nonce = [1u8; 32];
        let meas = [7u8; 48];
        let d1 = [9u8; 32];
        let d2 = [0xAAu8; 32];
        let mut ctx = ReleaseContext {
            v: 1,
            ticket_id: "tk-1",
            tenant_id: "t",
            vm_id: "abc",
            vm_generation: 5,
            kbs_nonce: &nonce,
            measurement: &meas,
            kbs_kid: b"kbs-kid",
            secret_type: "luks",
            secret_path: "kbs/vm/abc/luks",
            secret_version: 3,
            allowed_userdata_digest: &d1,
        };
        let a = ctx.canonical().unwrap();
        ctx.allowed_userdata_digest = &d2;
        let b = ctx.canonical().unwrap();
        assert_ne!(a, b);
    }
}

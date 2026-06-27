//! KBS-issued Audit-VM certificate (ARCHITECTURE.md §22 / §23).
//!
//! After the §7 attestation checks pass for an Audit-VM first-boot
//! request (the §20 `audit_vm` `REPORT_DATA` layout — see
//! [`crate::report_data::audit_vm`]), the KBS issues an **Audit-VM
//! certificate**: a §22-offline-root-signed statement binding the
//! Audit-VM's Ed25519 public key to its node identity and a validity
//! window. Validators accept Audit-VM signals (§23 liveness, canary
//! results, `ServedDeliveryAggregate` co-signatures) **only** under a
//! valid, unexpired cert.
//!
//! This module owns only the **wire format** — the canonical encoder,
//! the fail-closed decoder, and the [`SignedAuditVmCert`] envelope. The
//! Ed25519 sign / verify live with their callers so this crate stays
//! crypto-free and `no_std`:
//! - `kbs-core` (`audit_vm_cert::issue_cert`) signs with the §22 root;
//! - `hippius-guest` (`audit_vm_cert::verify_cert`) verifies inside the
//!   attested Audit VM.
//!
//! ## Domain separation
//!
//! [`AUDIT_VM_CERT_DOMAIN`] is **distinct** from every other signed
//! payload — including [`crate::provenance::PROVENANCE_DOMAIN`], which
//! is signed by the **same §22 root key**. A cert and an image-
//! provenance map therefore can never be confused: the domain tag is
//! the first field of the signed body, so a signature minted over one
//! cannot verify as the other (different body bytes). E3.1 scope: the
//! cert binds `{audit_vm_pubkey, node_id, platform_id, validity
//! window}`; the fuller §23 binding (chain genesis, launch measurement,
//! TCB floor, and the issuance `nonce`) is layered on by a later §E3
//! PR. Until the `nonce` field lands, first-boot replay protection
//! rests on the validity window plus the fact that the Audit-VM key is
//! freshly random each boot — a replayed in-window cert is for a
//! *different* key and fails `verify_cert`'s key-match check.
//!
//! ## Schema (`v1`) — fixed, fail-closed
//!
//! Every field is mandatory. Decode rejects a non-canonical body, a
//! non-text / duplicated map key, an unknown field, a wrong `domain`,
//! an unknown `schema_version`, and any semantically invalid value
//! (empty `node_id` / `platform_id`, an inverted validity window).

#[allow(unused_imports)]
// per-module slice of the alloc prelude — not every module needs every item
use alloc::{
    boxed::Box,
    collections::BTreeMap,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use crate::cbor::{assert_canonical, to_canonical_vec};
use crate::{HippiusTypesError, Result};
use ciborium::value::Value;

/// Replay-domain separator — the first field of every signed cert body.
/// Distinct from `PROVENANCE_DOMAIN` / `RELEASE_DOMAIN` / the §23
/// aggregate domain, even though the §22 root key signs both certs and
/// provenance maps.
pub const AUDIT_VM_CERT_DOMAIN: &str = "HIPPIUS_AUDIT_VM_CERT_V1";

/// The only `schema_version` this build understands.
pub const AUDIT_VM_CERT_SCHEMA_VERSION: u32 = 1;

/// Length of an Ed25519 public key (the Audit-VM key + the §22 root).
pub const PUBKEY_LEN: usize = 32;

/// Length of an Ed25519 signature.
pub const SIGNATURE_LEN: usize = 64;

/// One Audit-VM certificate body — the §22-root-signed statement.
///
/// Every field is signed (encoded into the body the §22 root key
/// signs). `signer_pubkey` is bound IN so the signature self-certifies
/// which root produced it — a verifier checks it equals its pinned §22
/// root before trusting the signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditVmCert {
    /// Wire-format version. MUST be [`AUDIT_VM_CERT_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The Audit-VM's Ed25519 public key — generated inside the
    /// attested Audit VM and bound into `REPORT_DATA[32..64]` at first
    /// attestation (§20 `audit_vm` layout).
    pub audit_vm_pubkey: [u8; PUBKEY_LEN],
    /// The compute `node_id` (§23 — the node's own ed25519 device key
    /// identity, distinct from the Audit-VM key).
    pub node_id: Vec<u8>,
    /// The SNP platform identity (`CHIP_ID` / VCEK digest, §23) the
    /// `node_id` is bound to — closes the 1-to-N proxy/clone hole.
    pub platform_id: Vec<u8>,
    /// Validity window start, Unix seconds (inclusive).
    pub not_before: u64,
    /// Validity window end, Unix seconds (inclusive).
    pub not_after: u64,
    /// The Ed25519 public key of the §22 root that signs this cert.
    pub signer_pubkey: [u8; PUBKEY_LEN],
}

impl AuditVmCert {
    /// Every semantic invariant of a well-formed cert. Run by BOTH
    /// [`canonical`](Self::canonical) and [`decode`](Self::decode) so a
    /// hand-crafted canonical body cannot skip the encode-time gates.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != AUDIT_VM_CERT_SCHEMA_VERSION {
            return Err(schema(format!(
                "unknown schema_version {} (want {AUDIT_VM_CERT_SCHEMA_VERSION})",
                self.schema_version
            )));
        }
        if self.node_id.is_empty() {
            return Err(schema("node_id must be non-empty".into()));
        }
        if self.platform_id.is_empty() {
            return Err(schema("platform_id must be non-empty".into()));
        }
        if self.not_before == 0 {
            return Err(schema("not_before must be non-zero".into()));
        }
        // A non-empty, non-inverted validity window. `not_after` is
        // inclusive, so `==` is a one-second window, still rejected:
        // a zero-or-negative-length window is never a useful cert.
        if self.not_after <= self.not_before {
            return Err(schema("not_after must be > not_before".into()));
        }
        Ok(())
    }

    /// `true` iff `now_unix` falls within `[not_before, not_after]`
    /// (both inclusive). Crypto-free — the caller still has to verify
    /// the signature; this is only the window arithmetic.
    pub fn covers(&self, now_unix: u64) -> bool {
        now_unix >= self.not_before && now_unix <= self.not_after
    }

    /// Deterministic-CBOR encoding of the to-be-signed body. Fails
    /// closed on any [`validate`](Self::validate) violation.
    pub fn canonical(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let v = Value::Map(vec![
            (
                Value::Text("audit_vm_pubkey".into()),
                Value::Bytes(self.audit_vm_pubkey.to_vec()),
            ),
            (
                Value::Text("domain".into()),
                Value::Text(AUDIT_VM_CERT_DOMAIN.into()),
            ),
            (
                Value::Text("node_id".into()),
                Value::Bytes(self.node_id.clone()),
            ),
            (
                Value::Text("not_after".into()),
                Value::Integer(self.not_after.into()),
            ),
            (
                Value::Text("not_before".into()),
                Value::Integer(self.not_before.into()),
            ),
            (
                Value::Text("platform_id".into()),
                Value::Bytes(self.platform_id.clone()),
            ),
            (
                Value::Text("schema_version".into()),
                Value::Integer(self.schema_version.into()),
            ),
            (
                Value::Text("signer_pubkey".into()),
                Value::Bytes(self.signer_pubkey.to_vec()),
            ),
        ]);
        to_canonical_vec(&v).map_err(|e| schema(format!("encode: {e}")))
    }

    /// Decode + validate a canonical-CBOR cert body.
    ///
    /// Fail-closed at every step: rejects a non-canonical encoding, a
    /// non-text or duplicated map key, an unknown field, a wrong
    /// `domain`, an unknown `schema_version`, a wrong-length byte
    /// field, and any semantically invalid value. This parser runs on
    /// hostile-origin bytes — it never trusts the wire shape.
    pub fn decode(body: &[u8]) -> Result<AuditVmCert> {
        // Canonical gate BEFORE the structural decode.
        assert_canonical(body).map_err(|e| schema(format!("body: {e}")))?;
        let value: Value =
            ciborium::de::from_reader(body).map_err(|e| schema(format!("decode: {e}")))?;
        let mut map = into_string_map(value)?;

        let domain = take_text(&mut map, "domain")?;
        if domain != AUDIT_VM_CERT_DOMAIN {
            return Err(schema(format!(
                "domain must be {AUDIT_VM_CERT_DOMAIN:?}, got {domain:?}"
            )));
        }
        let schema_version = take_u32(&mut map, "schema_version")?;
        if schema_version != AUDIT_VM_CERT_SCHEMA_VERSION {
            return Err(schema(format!(
                "unknown schema_version {schema_version} (want {AUDIT_VM_CERT_SCHEMA_VERSION})"
            )));
        }
        let audit_vm_pubkey = take_byte_array::<PUBKEY_LEN>(&mut map, "audit_vm_pubkey")?;
        let signer_pubkey = take_byte_array::<PUBKEY_LEN>(&mut map, "signer_pubkey")?;
        let node_id = take_bytes(&mut map, "node_id")?;
        let platform_id = take_bytes(&mut map, "platform_id")?;
        let not_before = take_u64(&mut map, "not_before")?;
        let not_after = take_u64(&mut map, "not_after")?;
        reject_leftover(&map, "audit-vm cert")?;

        let cert = AuditVmCert {
            schema_version,
            audit_vm_pubkey,
            node_id,
            platform_id,
            not_before,
            not_after,
            signer_pubkey,
        };
        // A hand-crafted canonical body must not decode if it carries a
        // value `canonical()` would never have emitted.
        cert.validate()?;
        Ok(cert)
    }
}

/// Signed Audit-VM cert envelope.
///
/// `body` is the canonical CBOR of an [`AuditVmCert`]; `sig` is the
/// 64-byte Ed25519 signature over `body` by the §22 root key. Both
/// [`encode`](Self::encode) and [`decode`](Self::decode) keep the outer
/// envelope canonical too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedAuditVmCert {
    pub body: Vec<u8>,
    pub sig: Vec<u8>,
}

impl SignedAuditVmCert {
    /// Canonical-CBOR encoding of the `{body, sig}` envelope.
    pub fn encode(&self) -> Result<Vec<u8>> {
        if self.sig.len() != SIGNATURE_LEN {
            return Err(schema(format!(
                "sig must be {SIGNATURE_LEN} bytes, got {}",
                self.sig.len()
            )));
        }
        let v = Value::Map(vec![
            (Value::Text("body".into()), Value::Bytes(self.body.clone())),
            (Value::Text("sig".into()), Value::Bytes(self.sig.clone())),
        ]);
        to_canonical_vec(&v).map_err(|e| schema(format!("envelope encode: {e}")))
    }

    /// Decode a canonical-CBOR signed-cert envelope. Rejects a
    /// non-canonical wrapper, unknown fields, and a wrong-length sig.
    pub fn decode(bytes: &[u8]) -> Result<SignedAuditVmCert> {
        assert_canonical(bytes).map_err(|e| schema(format!("envelope: {e}")))?;
        let value: Value = ciborium::de::from_reader(bytes)
            .map_err(|e| schema(format!("envelope decode: {e}")))?;
        let mut map = into_string_map(value)?;
        let body = take_bytes(&mut map, "body")?;
        let sig = take_bytes(&mut map, "sig")?;
        reject_leftover(&map, "signed cert envelope")?;
        if sig.len() != SIGNATURE_LEN {
            return Err(schema(format!(
                "sig must be {SIGNATURE_LEN} bytes, got {}",
                sig.len()
            )));
        }
        Ok(SignedAuditVmCert { body, sig })
    }
}

// ── decode helpers (module-local, mirroring `provenance.rs`) ─────────

fn schema(msg: String) -> HippiusTypesError {
    HippiusTypesError::AuditVmCertSchema(msg)
}

fn into_string_map(value: Value) -> Result<BTreeMap<String, Value>> {
    let entries = match value {
        Value::Map(entries) => entries,
        _ => return Err(schema("expected a CBOR map".into())),
    };
    let mut out = BTreeMap::new();
    for (k, v) in entries {
        match k {
            Value::Text(name) => {
                out.insert(name, v);
            }
            _ => return Err(schema("map key is not a text string".into())),
        }
    }
    Ok(out)
}

fn take(map: &mut BTreeMap<String, Value>, key: &str) -> Result<Value> {
    map.remove(key)
        .ok_or_else(|| schema(format!("missing field {key:?}")))
}

fn take_text(map: &mut BTreeMap<String, Value>, key: &str) -> Result<String> {
    match take(map, key)? {
        Value::Text(s) => Ok(s),
        _ => Err(schema(format!("field {key:?} is not a text string"))),
    }
}

fn take_bytes(map: &mut BTreeMap<String, Value>, key: &str) -> Result<Vec<u8>> {
    match take(map, key)? {
        Value::Bytes(b) => Ok(b),
        _ => Err(schema(format!("field {key:?} is not a byte string"))),
    }
}

fn take_byte_array<const N: usize>(
    map: &mut BTreeMap<String, Value>,
    key: &str,
) -> Result<[u8; N]> {
    let bytes = take_bytes(map, key)?;
    bytes.as_slice().try_into().map_err(|_| {
        schema(format!(
            "field {key:?} must be {N} bytes, got {}",
            bytes.len()
        ))
    })
}

fn take_u64(map: &mut BTreeMap<String, Value>, key: &str) -> Result<u64> {
    match take(map, key)? {
        Value::Integer(i) => {
            let n: i128 = i.into();
            u64::try_from(n).map_err(|_| schema(format!("field {key:?} out of u64 range")))
        }
        _ => Err(schema(format!("field {key:?} is not an integer"))),
    }
}

fn take_u32(map: &mut BTreeMap<String, Value>, key: &str) -> Result<u32> {
    let n = take_u64(map, key)?;
    u32::try_from(n).map_err(|_| schema(format!("field {key:?} out of u32 range")))
}

fn reject_leftover(map: &BTreeMap<String, Value>, what: &str) -> Result<()> {
    if let Some(key) = map.keys().next() {
        return Err(schema(format!("unknown field {key:?} in {what}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> AuditVmCert {
        AuditVmCert {
            schema_version: AUDIT_VM_CERT_SCHEMA_VERSION,
            audit_vm_pubkey: [0x11; PUBKEY_LEN],
            node_id: b"compute-node-1".to_vec(),
            platform_id: b"chip-id-abcdef".to_vec(),
            not_before: 1_700_000_000,
            not_after: 1_700_086_400,
            signer_pubkey: [0x22; PUBKEY_LEN],
        }
    }

    #[test]
    fn canonical_is_stable_and_canonical() {
        let c = sample();
        let a = c.canonical().unwrap();
        let b = c.canonical().unwrap();
        assert_eq!(a, b);
        assert_canonical(&a).unwrap();
    }

    #[test]
    fn canonical_round_trips_through_decode() {
        let c = sample();
        let body = c.canonical().unwrap();
        let decoded = AuditVmCert::decode(&body).unwrap();
        assert_eq!(c, decoded);
        assert_eq!(body, decoded.canonical().unwrap());
    }

    #[test]
    fn every_field_is_signed() {
        let base = sample().canonical().unwrap();
        let mutate: &[fn(&mut AuditVmCert)] = &[
            |c| c.audit_vm_pubkey[0] ^= 0xff,
            |c| c.node_id = b"other-node".to_vec(),
            |c| c.platform_id = b"other-chip".to_vec(),
            |c| c.not_before += 1,
            |c| c.not_after += 1,
            |c| c.signer_pubkey[0] ^= 0xff,
        ];
        for m in mutate {
            let mut c = sample();
            m(&mut c);
            assert_ne!(
                base,
                c.canonical().unwrap(),
                "a field escaped the signature"
            );
        }
    }

    #[test]
    fn covers_is_inclusive_on_both_bounds() {
        let c = sample();
        assert!(!c.covers(c.not_before - 1));
        assert!(c.covers(c.not_before));
        assert!(c.covers(c.not_after));
        assert!(!c.covers(c.not_after + 1));
    }

    #[test]
    fn inverted_window_rejected_at_encode() {
        let mut c = sample();
        c.not_after = c.not_before;
        assert!(c.canonical().is_err());
        c.not_after = c.not_before - 1;
        assert!(c.canonical().is_err());
    }

    #[test]
    fn empty_node_or_platform_rejected_at_encode() {
        let mut c = sample();
        c.node_id = Vec::new();
        assert!(c.canonical().is_err());
        let mut c = sample();
        c.platform_id = Vec::new();
        assert!(c.canonical().is_err());
    }

    #[test]
    fn decode_rejects_unknown_field() {
        let c = sample();
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(c.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        entries.push((Value::Text("rogue".into()), Value::Integer(1.into())));
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(AuditVmCert::decode(&body).is_err());
    }

    #[test]
    fn decode_rejects_wrong_domain() {
        let c = sample();
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(c.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        for (k, v) in &mut entries {
            if matches!(k, Value::Text(t) if t == "domain") {
                *v = Value::Text("HIPPIUS_IMAGE_PROVENANCE_V1".into());
            }
        }
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(AuditVmCert::decode(&body).is_err());
    }

    #[test]
    fn decode_rejects_non_canonical_body() {
        let v = Value::Map(vec![
            (Value::Text("zzz".into()), Value::Integer(1.into())),
            (
                Value::Text("domain".into()),
                Value::Text(AUDIT_VM_CERT_DOMAIN.into()),
            ),
        ]);
        let mut noncanon = Vec::new();
        ciborium::ser::into_writer(&v, &mut noncanon).unwrap();
        assert!(AuditVmCert::decode(&noncanon).is_err());
    }

    #[test]
    fn decode_rejects_short_pubkey() {
        let c = sample();
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(c.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        for (k, v) in &mut entries {
            if matches!(k, Value::Text(t) if t == "audit_vm_pubkey") {
                *v = Value::Bytes(vec![0u8; 31]);
            }
        }
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(AuditVmCert::decode(&body).is_err());
    }

    #[test]
    fn signed_envelope_round_trips() {
        let signed = SignedAuditVmCert {
            body: sample().canonical().unwrap(),
            sig: vec![0xAB; SIGNATURE_LEN],
        };
        let bytes = signed.encode().unwrap();
        assert_canonical(&bytes).unwrap();
        assert_eq!(signed, SignedAuditVmCert::decode(&bytes).unwrap());
    }

    #[test]
    fn signed_envelope_rejects_wrong_sig_length() {
        let signed = SignedAuditVmCert {
            body: sample().canonical().unwrap(),
            sig: vec![0u8; 63],
        };
        assert!(signed.encode().is_err());
    }
}

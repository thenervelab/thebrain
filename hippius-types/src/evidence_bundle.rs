//! KBS-issued attestation **evidence bundle** (issue #280 Phase 1).
//!
//! Per granted `process_release` call, KBS records — out-of-band of the
//! §15 hash-chained audit log — a `SignedEvidenceBundle`: the
//! cryptographic raw materials a paying tenant (or any third party
//! holding the KBS L0 verifying key) needs to verify the SEV-SNP chain
//! of custody themselves, end-to-end:
//!
//! * the raw SNP report bytes (1184 B for SEV-SNP — same bytes the
//!   guest minted in initramfs via `SNP_GET_REPORT`),
//! * the VCEK → ASK → ARK PEM chain the KBS used to verify that report
//!   against AMD's silicon root,
//! * the §22 allowlist epoch the release was admitted under + the
//!   SHA-256 of the signed manifest at that epoch,
//! * the L1-signed COSE OrderTicket bytes (already public — the
//!   placement assertion).
//!
//! The bundle is signed by the KBS L0 signing key (same key that signs
//! `KbsResponse` release envelopes — `kbs-core::trust_anchors`'s
//! pinned VK), so a verifier can chain trust through a key it already
//! trusts.
//!
//! This module owns only the **wire format** — the canonical encoder,
//! the fail-closed decoder, and the [`SignedEvidenceBundle`]
//! envelope. The Ed25519 sign / verify live with their callers so this
//! crate stays crypto-free and `no_std`:
//! - `kbs-core` signs at `process_release` grant time;
//! - a future tenant verifier (issue #280 Phase 3) verifies offline.
//!
//! ## Domain separation
//!
//! [`EVIDENCE_BUNDLE_DOMAIN`] is **distinct** from every other signed
//! payload in the stack — including the release `RELEASE_DOMAIN`,
//! `AUDIT_VM_CERT_DOMAIN`, `PROVENANCE_DOMAIN`. Even though the KBS L0
//! key signs both release envelopes and evidence bundles, the
//! domain-tag-first body layout means a release signature cannot
//! replay as a bundle signature (different first-field bytes ⇒
//! different signed bytes).
//!
//! ## Schema (`v1`) — fixed, fail-closed
//!
//! Every field is mandatory. Decode rejects a non-canonical body, a
//! non-text / duplicated map key, an unknown field, a wrong `domain`,
//! an unknown `schema_version`, a wrong-length byte field, and any
//! semantically invalid value (empty `vm_id` / `tenant_id` /
//! `ticket_id`, short SNP report, empty VCEK chain).

#[allow(unused_imports)]
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

/// Replay-domain separator — the first field of every signed bundle
/// body. Distinct from `RELEASE_DOMAIN` / `AUDIT_VM_CERT_DOMAIN` /
/// `PROVENANCE_DOMAIN`. Even when the same Ed25519 key signs over
/// multiple of these domains, the domain-tag-first layout makes a
/// signature minted for one impossible to replay as another (the
/// signed bytes differ from byte 0).
pub const EVIDENCE_BUNDLE_DOMAIN: &str = "HIPPIUS_EVIDENCE_BUNDLE_V1";

/// The only `schema_version` this build understands.
pub const EVIDENCE_BUNDLE_SCHEMA_VERSION: u32 = 1;

/// Ed25519 public-key length (32 bytes).
pub const PUBKEY_LEN: usize = 32;

/// Ed25519 signature length (64 bytes).
pub const SIGNATURE_LEN: usize = 64;

/// SEV-SNP launch-digest / measurement length (48 bytes — SHA-384).
pub const MEASUREMENT_LEN: usize = 48;

/// SHA-256 digest length (32 bytes).
pub const SHA256_LEN: usize = 32;

/// Minimum SNP report length we accept (full SEV-SNP report is 1184 B;
/// reject anything obviously short so a truncated bundle never claims
/// to attest hardware it doesn't actually carry).
pub const MIN_SNP_REPORT_LEN: usize = 1184;

/// Maximum SNP report length we accept (hard cap so a hostile body can
/// never claim a multi-MB report and exhaust memory at decode).
pub const MAX_SNP_REPORT_LEN: usize = 4096;

/// Maximum total bundle body size — cap on the canonical CBOR before
/// signature. Generous (bundle is typically ~6 KB with a 3 KB VCEK
/// chain) but bounded so a malicious decoder input can't OOM.
pub const MAX_BUNDLE_BODY_LEN: usize = 32 * 1024;

/// One evidence bundle body — the KBS-L0-signed statement.
///
/// Every field is signed (encoded into the body the KBS L0 key signs).
/// `kbs_signer_pubkey` is bound IN so the signature self-certifies
/// which KBS L0 key produced it — a verifier checks it equals its
/// pinned (or KID-resolved) L0 VK before trusting the signature.
///
/// **No secret bytes**: every field is already-public attestation
/// data. The SNP report's `REPORT_DATA` binds nonces + the guest's
/// X25519 pubkey, never the LUKS KEK; the VCEK chain is AMD-public;
/// the manifest digest commits to public allowlist bytes; the ticket
/// COSE bytes are L1-signed and already-public.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceBundle {
    /// Wire-format version. MUST be [`EVIDENCE_BUNDLE_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// The tenant CVM identifier (matches the OrderTicket's `vm_id`).
    pub vm_id: String,
    /// The tenant identifier (matches the OrderTicket's `tenant_id`).
    pub tenant_id: String,
    /// The OrderTicket's `ticket_id` — unique per release (the replay
    /// store enforces single-use), so it also keys this bundle's
    /// on-disk filename.
    pub ticket_id: String,
    /// Unix seconds when the release was granted by KBS.
    pub granted_at_unix: u64,
    /// SEV-SNP launch digest (= the SNP measurement) the guest
    /// attested. Same bytes the §22 allowlist admitted.
    pub measurement: [u8; MEASUREMENT_LEN],
    /// The §22 allowlist epoch at release time.
    pub allowlist_epoch: u64,
    /// SHA-256 of the signed §22 manifest bytes at `allowlist_epoch`
    /// — commits the bundle to the exact manifest the release was
    /// admitted under, without inlining the (potentially large)
    /// manifest. A verifier fetches the manifest from the §22 source
    /// (S3) and re-hashes to check.
    pub allowlist_manifest_digest: [u8; SHA256_LEN],
    /// The raw SNP report bytes the guest emitted (`req.raw_snp_report`
    /// verbatim — 1184 B for SEV-SNP).
    pub snp_report_bytes: Vec<u8>,
    /// VCEK → ASK → ARK certificate chain in PEM, in verification
    /// order. From the [`AttestationVerifier`] that just verified the
    /// report. A verifier checks the chain against AMD's pinned root.
    pub vcek_chain_pem: Vec<u8>,
    /// The L1-signed COSE_Sign1 OrderTicket bytes (`req.cose_ticket`
    /// verbatim).
    pub ticket_cose_bytes: Vec<u8>,
    /// The Ed25519 public key of the KBS L0 signer. A verifier checks
    /// this equals its pinned `PINNED_KBS_RESPONSE_VK` before trusting
    /// the signature.
    pub kbs_signer_pubkey: [u8; PUBKEY_LEN],
}

impl EvidenceBundle {
    /// Every semantic invariant of a well-formed bundle. Run by both
    /// [`canonical`](Self::canonical) and [`decode`](Self::decode) so
    /// a hand-crafted canonical body cannot skip the encode-time gates.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != EVIDENCE_BUNDLE_SCHEMA_VERSION {
            return Err(schema(format!(
                "unknown schema_version {} (want {EVIDENCE_BUNDLE_SCHEMA_VERSION})",
                self.schema_version
            )));
        }
        if self.vm_id.is_empty() {
            return Err(schema("vm_id must be non-empty".into()));
        }
        if self.tenant_id.is_empty() {
            return Err(schema("tenant_id must be non-empty".into()));
        }
        if self.ticket_id.is_empty() {
            return Err(schema("ticket_id must be non-empty".into()));
        }
        if self.granted_at_unix == 0 {
            return Err(schema("granted_at_unix must be non-zero".into()));
        }
        if self.snp_report_bytes.len() < MIN_SNP_REPORT_LEN {
            return Err(schema(format!(
                "snp_report_bytes must be >= {MIN_SNP_REPORT_LEN} bytes, got {}",
                self.snp_report_bytes.len()
            )));
        }
        if self.snp_report_bytes.len() > MAX_SNP_REPORT_LEN {
            return Err(schema(format!(
                "snp_report_bytes must be <= {MAX_SNP_REPORT_LEN} bytes, got {}",
                self.snp_report_bytes.len()
            )));
        }
        if self.vcek_chain_pem.is_empty() {
            return Err(schema("vcek_chain_pem must be non-empty".into()));
        }
        if self.ticket_cose_bytes.is_empty() {
            return Err(schema("ticket_cose_bytes must be non-empty".into()));
        }
        Ok(())
    }

    /// Deterministic-CBOR encoding of the to-be-signed body. Fails
    /// closed on any [`validate`](Self::validate) violation. Map keys
    /// in canonical (lexicographic byte) order — `to_canonical_vec`
    /// re-asserts.
    pub fn canonical(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let v = Value::Map(vec![
            (
                Value::Text("allowlist_epoch".into()),
                Value::Integer(self.allowlist_epoch.into()),
            ),
            (
                Value::Text("allowlist_manifest_digest".into()),
                Value::Bytes(self.allowlist_manifest_digest.to_vec()),
            ),
            (
                Value::Text("domain".into()),
                Value::Text(EVIDENCE_BUNDLE_DOMAIN.into()),
            ),
            (
                Value::Text("granted_at_unix".into()),
                Value::Integer(self.granted_at_unix.into()),
            ),
            (
                Value::Text("kbs_signer_pubkey".into()),
                Value::Bytes(self.kbs_signer_pubkey.to_vec()),
            ),
            (
                Value::Text("measurement".into()),
                Value::Bytes(self.measurement.to_vec()),
            ),
            (
                Value::Text("schema_version".into()),
                Value::Integer(self.schema_version.into()),
            ),
            (
                Value::Text("snp_report_bytes".into()),
                Value::Bytes(self.snp_report_bytes.clone()),
            ),
            (
                Value::Text("tenant_id".into()),
                Value::Text(self.tenant_id.clone()),
            ),
            (
                Value::Text("ticket_cose_bytes".into()),
                Value::Bytes(self.ticket_cose_bytes.clone()),
            ),
            (
                Value::Text("ticket_id".into()),
                Value::Text(self.ticket_id.clone()),
            ),
            (
                Value::Text("vcek_chain_pem".into()),
                Value::Bytes(self.vcek_chain_pem.clone()),
            ),
            (Value::Text("vm_id".into()), Value::Text(self.vm_id.clone())),
        ]);
        let bytes = to_canonical_vec(&v).map_err(|e| schema(format!("encode: {e}")))?;
        if bytes.len() > MAX_BUNDLE_BODY_LEN {
            return Err(schema(format!(
                "canonical body must be <= {MAX_BUNDLE_BODY_LEN} bytes, got {}",
                bytes.len()
            )));
        }
        Ok(bytes)
    }

    /// Decode + validate a canonical-CBOR bundle body.
    ///
    /// Fail-closed at every step: rejects a non-canonical encoding, a
    /// non-text or duplicated map key, an unknown field, a wrong
    /// `domain`, an unknown `schema_version`, a wrong-length byte
    /// field, and any semantically invalid value. This parser runs on
    /// hostile-origin bytes (a verifier reads from disk / S3 / the
    /// network) — it never trusts the wire shape.
    pub fn decode(body: &[u8]) -> Result<EvidenceBundle> {
        if body.len() > MAX_BUNDLE_BODY_LEN {
            return Err(schema(format!(
                "body must be <= {MAX_BUNDLE_BODY_LEN} bytes, got {}",
                body.len()
            )));
        }
        // Canonical gate BEFORE the structural decode.
        assert_canonical(body).map_err(|e| schema(format!("body: {e}")))?;
        let value: Value =
            ciborium::de::from_reader(body).map_err(|e| schema(format!("decode: {e}")))?;
        let mut map = into_string_map(value)?;

        let domain = take_text(&mut map, "domain")?;
        if domain != EVIDENCE_BUNDLE_DOMAIN {
            return Err(schema(format!(
                "domain must be {EVIDENCE_BUNDLE_DOMAIN:?}, got {domain:?}"
            )));
        }
        let schema_version = take_u32(&mut map, "schema_version")?;
        if schema_version != EVIDENCE_BUNDLE_SCHEMA_VERSION {
            return Err(schema(format!(
                "unknown schema_version {schema_version} (want {EVIDENCE_BUNDLE_SCHEMA_VERSION})"
            )));
        }
        let vm_id = take_text(&mut map, "vm_id")?;
        let tenant_id = take_text(&mut map, "tenant_id")?;
        let ticket_id = take_text(&mut map, "ticket_id")?;
        let granted_at_unix = take_u64(&mut map, "granted_at_unix")?;
        let measurement = take_byte_array::<MEASUREMENT_LEN>(&mut map, "measurement")?;
        let allowlist_epoch = take_u64(&mut map, "allowlist_epoch")?;
        let allowlist_manifest_digest =
            take_byte_array::<SHA256_LEN>(&mut map, "allowlist_manifest_digest")?;
        let snp_report_bytes = take_bytes(&mut map, "snp_report_bytes")?;
        let vcek_chain_pem = take_bytes(&mut map, "vcek_chain_pem")?;
        let ticket_cose_bytes = take_bytes(&mut map, "ticket_cose_bytes")?;
        let kbs_signer_pubkey = take_byte_array::<PUBKEY_LEN>(&mut map, "kbs_signer_pubkey")?;
        reject_leftover(&map, "evidence bundle")?;

        let bundle = EvidenceBundle {
            schema_version,
            vm_id,
            tenant_id,
            ticket_id,
            granted_at_unix,
            measurement,
            allowlist_epoch,
            allowlist_manifest_digest,
            snp_report_bytes,
            vcek_chain_pem,
            ticket_cose_bytes,
            kbs_signer_pubkey,
        };
        // A hand-crafted canonical body must not decode if it carries a
        // value `canonical()` would never have emitted.
        bundle.validate()?;
        Ok(bundle)
    }
}

/// Signed evidence bundle envelope.
///
/// `body` is the canonical CBOR of an [`EvidenceBundle`]; `sig` is the
/// 64-byte Ed25519 signature over `body` by the KBS L0 key. Both
/// [`encode`](Self::encode) and [`decode`](Self::decode) keep the outer
/// envelope canonical too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedEvidenceBundle {
    pub body: Vec<u8>,
    pub sig: Vec<u8>,
}

impl SignedEvidenceBundle {
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

    /// Decode a canonical-CBOR signed-bundle envelope. Rejects a
    /// non-canonical wrapper, unknown fields, and a wrong-length sig.
    pub fn decode(bytes: &[u8]) -> Result<SignedEvidenceBundle> {
        assert_canonical(bytes).map_err(|e| schema(format!("envelope: {e}")))?;
        let value: Value = ciborium::de::from_reader(bytes)
            .map_err(|e| schema(format!("envelope decode: {e}")))?;
        let mut map = into_string_map(value)?;
        let body = take_bytes(&mut map, "body")?;
        let sig = take_bytes(&mut map, "sig")?;
        reject_leftover(&map, "signed evidence envelope")?;
        if sig.len() != SIGNATURE_LEN {
            return Err(schema(format!(
                "sig must be {SIGNATURE_LEN} bytes, got {}",
                sig.len()
            )));
        }
        Ok(SignedEvidenceBundle { body, sig })
    }
}

// ── decode helpers (module-local, mirroring `audit_vm_cert.rs`) ──────

fn schema(msg: String) -> HippiusTypesError {
    HippiusTypesError::EvidenceBundleSchema(msg)
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
                if out.insert(name.clone(), v).is_some() {
                    return Err(schema(format!("duplicate map key {name:?}")));
                }
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

    fn sample() -> EvidenceBundle {
        EvidenceBundle {
            schema_version: EVIDENCE_BUNDLE_SCHEMA_VERSION,
            vm_id: "tenant-cvm-01".into(),
            tenant_id: "tenant-alice".into(),
            ticket_id: "ticket-0xabcdef".into(),
            granted_at_unix: 1_780_000_000,
            measurement: [0x33; MEASUREMENT_LEN],
            allowlist_epoch: 42,
            allowlist_manifest_digest: [0x44; SHA256_LEN],
            snp_report_bytes: vec![0x55; MIN_SNP_REPORT_LEN],
            vcek_chain_pem: b"-----BEGIN CERTIFICATE-----\n...\n-----END CERTIFICATE-----\n"
                .to_vec(),
            ticket_cose_bytes: b"cose-ticket-bytes".to_vec(),
            kbs_signer_pubkey: [0x66; PUBKEY_LEN],
        }
    }

    #[test]
    fn canonical_is_stable_and_canonical() {
        let b = sample();
        let a = b.canonical().unwrap();
        let c = b.canonical().unwrap();
        assert_eq!(a, c);
        assert_canonical(&a).unwrap();
    }

    #[test]
    fn canonical_round_trips_through_decode() {
        let b = sample();
        let body = b.canonical().unwrap();
        let decoded = EvidenceBundle::decode(&body).unwrap();
        assert_eq!(b, decoded);
        assert_eq!(body, decoded.canonical().unwrap());
    }

    #[test]
    fn signed_envelope_round_trips() {
        let b = sample();
        let body = b.canonical().unwrap();
        let signed = SignedEvidenceBundle {
            body,
            sig: vec![0x77; SIGNATURE_LEN],
        };
        let bytes = signed.encode().unwrap();
        let decoded = SignedEvidenceBundle::decode(&bytes).unwrap();
        assert_eq!(signed, decoded);
        // Body round-trips back into a structured EvidenceBundle too.
        let b2 = EvidenceBundle::decode(&decoded.body).unwrap();
        assert_eq!(b, b2);
    }

    #[test]
    fn every_field_is_signed() {
        let base = sample().canonical().unwrap();
        let mutate: &[fn(&mut EvidenceBundle)] = &[
            |b| b.vm_id = "other-vm".into(),
            |b| b.tenant_id = "other-tenant".into(),
            |b| b.ticket_id = "other-ticket".into(),
            |b| b.granted_at_unix += 1,
            |b| b.measurement[0] ^= 0xff,
            |b| b.allowlist_epoch += 1,
            |b| b.allowlist_manifest_digest[0] ^= 0xff,
            |b| b.snp_report_bytes[0] ^= 0xff,
            |b| b.vcek_chain_pem.push(b'X'),
            |b| b.ticket_cose_bytes.push(b'X'),
            |b| b.kbs_signer_pubkey[0] ^= 0xff,
        ];
        for m in mutate {
            let mut b = sample();
            m(&mut b);
            assert_ne!(
                base,
                b.canonical().unwrap(),
                "a field escaped the signature"
            );
        }
    }

    #[test]
    fn empty_string_fields_rejected_at_encode() {
        let mutate: &[fn(&mut EvidenceBundle)] = &[
            |b| b.vm_id = String::new(),
            |b| b.tenant_id = String::new(),
            |b| b.ticket_id = String::new(),
        ];
        for m in mutate {
            let mut b = sample();
            m(&mut b);
            assert!(b.canonical().is_err());
        }
    }

    #[test]
    fn short_or_oversize_snp_report_rejected_at_encode() {
        let mut b = sample();
        b.snp_report_bytes = vec![0u8; MIN_SNP_REPORT_LEN - 1];
        assert!(b.canonical().is_err());
        let mut b = sample();
        b.snp_report_bytes = vec![0u8; MAX_SNP_REPORT_LEN + 1];
        assert!(b.canonical().is_err());
    }

    #[test]
    fn decode_rejects_unknown_field() {
        let b = sample();
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(b.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        entries.push((Value::Text("rogue".into()), Value::Integer(1.into())));
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(EvidenceBundle::decode(&body).is_err());
    }

    #[test]
    fn decode_rejects_wrong_domain() {
        let b = sample();
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(b.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        for (k, v) in &mut entries {
            if matches!(k, Value::Text(t) if t == "domain") {
                *v = Value::Text("HIPPIUS_AUDIT_VM_CERT_V1".into());
            }
        }
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(EvidenceBundle::decode(&body).is_err());
    }

    #[test]
    fn decode_rejects_non_canonical_body() {
        // Wrong key order in the OUTER map — canonical requires
        // lexicographic.
        let v = Value::Map(vec![
            (Value::Text("zzz".into()), Value::Integer(1.into())),
            (
                Value::Text("domain".into()),
                Value::Text(EVIDENCE_BUNDLE_DOMAIN.into()),
            ),
        ]);
        let mut noncanon = Vec::new();
        ciborium::ser::into_writer(&v, &mut noncanon).unwrap();
        assert!(EvidenceBundle::decode(&noncanon).is_err());
    }

    #[test]
    fn decode_rejects_short_measurement() {
        let b = sample();
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(b.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        for (k, v) in &mut entries {
            if matches!(k, Value::Text(t) if t == "measurement") {
                *v = Value::Bytes(vec![0u8; MEASUREMENT_LEN - 1]);
            }
        }
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(EvidenceBundle::decode(&body).is_err());
    }

    #[test]
    fn signed_envelope_rejects_wrong_length_sig() {
        let body = sample().canonical().unwrap();
        let signed = SignedEvidenceBundle {
            body,
            sig: vec![0u8; SIGNATURE_LEN - 1],
        };
        assert!(signed.encode().is_err());
    }
}

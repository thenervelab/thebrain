//! Signed image-provenance map (ARCHITECTURE.md §11 / §22).
//!
//! The Packer factory (`binaries/image-provenance`, PR-F4) builds one
//! `ProvenanceMap` per measured UKI: it binds the SEV-SNP launch
//! measurement the §22 offline allowlist gates secret release on to
//! **every input** that produced it (the verity-anchored rootfs, the
//! kernel / initrd / cmdline / OVMF fingerprints, the pinned launch
//! config) plus the content-addressed S3 location the image is
//! published at.
//!
//! The map is signed by the **§22 offline allowlist root key**
//! (Ed25519). This module owns only the *wire format* — the canonical
//! encoder, the fail-closed decoder, and the [`SignedProvenance`]
//! envelope. The Ed25519 sign / verify themselves live with their
//! callers (`binaries/image-provenance` for PR-F4; the miner-side
//! fetch tool for PR-F5) so this crate stays crypto-free and `no_std`.
//!
//! `PROVENANCE_DOMAIN` is distinct from every other signed payload in
//! the stack (release, denial, stopped-ack, served receipt, audit-VM
//! aggregate) so a signature lifted from one scheme cannot replay into
//! another — the domain tag is the first field of the signed body.
//!
//! ## Schema (`v1`) — fixed, fail-closed
//!
//! Provenance describes a **production** measured image, so the schema
//! is fixed: every field is mandatory and `measurement_kind` MUST be
//! [`MEASUREMENT_KIND_SNP`]. A `uki_sha384` placeholder build has no
//! launch-digest trust value and is rejected at encode time — there is
//! deliberately no optional-field path that could emit a provenance
//! map with an absent anchor. Decode rejects unknown fields, a wrong
//! `domain`, an unknown `schema_version`, and any non-canonical body.

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

/// Replay-domain separator — the first field of every signed body.
pub const PROVENANCE_DOMAIN: &str = "HIPPIUS_IMAGE_PROVENANCE_V1";

/// The only `schema_version` this build understands. Decode fails
/// closed on any other value (§22 "explicit schema `v`").
pub const PROVENANCE_SCHEMA_VERSION: u32 = 1;

/// The only accepted `measurement_kind`. A provenance map exists to
/// anchor the §22 allowlist, which gates exclusively on the real
/// SEV-SNP launch digest; the `uki_sha384` placeholder is refused.
pub const MEASUREMENT_KIND_SNP: &str = "snp_launch_digest_v1";

/// Length of a SEV-SNP launch digest (`SHA-384`, 48 bytes).
pub const LAUNCH_MEASUREMENT_LEN: usize = 48;

/// Length of every SHA-256 fingerprint / Ed25519 public key field.
pub const SHA256_LEN: usize = 32;

/// Pinned SEV-SNP launch parameters that feed the launch digest. Their
/// values do not change the digest's *trust* meaning but a verifier
/// auditing a provenance map needs them recorded — the digest is a
/// function of this config, so an operator must be able to see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnpLaunchConfig {
    pub vcpus: u32,
    pub vcpu_type: String,
    pub guest_features: String,
}

/// One measured image's provenance: launch measurement ⇒ all inputs.
///
/// Every field is signed (it is encoded into the body the §22 root key
/// signs). The S3 `version_id` is deliberately **not** here: it is
/// assigned by the object store at upload time, is not a deterministic
/// function of the artifact, and would make the signed body
/// un-reproducible. Integrity is fully anchored by `artifact_sha256`
/// and the content-addressed `s3_key`; `version_id` is recorded
/// unsigned in the publish receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenanceMap {
    /// Wire-format version. MUST be [`PROVENANCE_SCHEMA_VERSION`].
    pub schema_version: u32,
    /// MUST be [`MEASUREMENT_KIND_SNP`].
    pub measurement_kind: String,
    /// The 48-byte SEV-SNP launch digest — the §22 allowlist anchor.
    pub launch_measurement: [u8; LAUNCH_MEASUREMENT_LEN],
    /// SHA-256 of the published artifact (the signed UKI PE binary).
    pub artifact_sha256: [u8; SHA256_LEN],
    /// dm-verity root hash of the rootfs the UKI's cmdline pins.
    pub verity_root_hash: [u8; SHA256_LEN],
    /// SHA-256 of the kernel image fed to the launch digest.
    pub kernel_sha256: [u8; SHA256_LEN],
    /// SHA-256 of the initrd fed to the launch digest.
    pub initrd_sha256: [u8; SHA256_LEN],
    /// SHA-256 of the kernel cmdline fed to the launch digest.
    pub cmdline_sha256: [u8; SHA256_LEN],
    /// SHA-256 of the pinned OVMF firmware fed to the launch digest.
    pub ovmf_sha256: [u8; SHA256_LEN],
    /// Pinned launch parameters the digest is computed under.
    pub snp_launch_config: SnpLaunchConfig,
    /// The S3 bucket the §22 signer declares this image belongs in.
    /// Not content-derived — it is the signer's *intent*; the publish
    /// path refuses to push the image to any other bucket.
    pub s3_bucket: String,
    /// Content-addressed S3 key — `images/<artifact_sha256-hex>/<name>`.
    pub s3_key: String,
    /// Unix seconds the provenance was built (informational).
    pub built_at_unix: u64,
    /// The Ed25519 public key of the §22 root that signs this map.
    /// Bound into the signed body so the signature self-certifies
    /// which key produced it (a verifier checks this equals its
    /// compiled-in root before trusting the signature).
    pub signer_pubkey: [u8; SHA256_LEN],
}

impl ProvenanceMap {
    /// Every semantic invariant of a well-formed provenance map.
    ///
    /// Run by BOTH [`canonical`](Self::canonical) (so a signature over
    /// an impossible map cannot be produced) AND
    /// [`decode`](Self::decode) (so a hand-crafted canonical body that
    /// skips these checks cannot verify) — the two paths must reject
    /// exactly the same set of maps.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != PROVENANCE_SCHEMA_VERSION {
            return Err(schema(format!(
                "unknown schema_version {} (want {PROVENANCE_SCHEMA_VERSION})",
                self.schema_version
            )));
        }
        if self.measurement_kind != MEASUREMENT_KIND_SNP {
            return Err(schema(format!(
                "measurement_kind must be {MEASUREMENT_KIND_SNP:?}, got {:?}",
                self.measurement_kind
            )));
        }
        if self.s3_bucket.is_empty() || self.s3_key.is_empty() {
            return Err(schema("s3_bucket / s3_key must be non-empty".into()));
        }
        if self.built_at_unix == 0 {
            return Err(schema("built_at_unix must be non-zero".into()));
        }
        if self.snp_launch_config.vcpus == 0 {
            return Err(schema("snp_launch_config.vcpus must be >= 1".into()));
        }
        if self.snp_launch_config.vcpu_type.is_empty()
            || self.snp_launch_config.guest_features.is_empty()
        {
            return Err(schema(
                "snp_launch_config vcpu_type / guest_features must be non-empty".into(),
            ));
        }
        Ok(())
    }

    /// Deterministic-CBOR encoding of the to-be-signed body. Fails
    /// closed on any [`validate`](Self::validate) violation.
    pub fn canonical(&self) -> Result<Vec<u8>> {
        self.validate()?;

        let snp_cfg = Value::Map(vec![
            (
                Value::Text("guest_features".into()),
                Value::Text(self.snp_launch_config.guest_features.clone()),
            ),
            (
                Value::Text("vcpu_type".into()),
                Value::Text(self.snp_launch_config.vcpu_type.clone()),
            ),
            (
                Value::Text("vcpus".into()),
                Value::Integer(self.snp_launch_config.vcpus.into()),
            ),
        ]);

        let v = Value::Map(vec![
            (
                Value::Text("artifact_sha256".into()),
                Value::Bytes(self.artifact_sha256.to_vec()),
            ),
            (
                Value::Text("built_at_unix".into()),
                Value::Integer(self.built_at_unix.into()),
            ),
            (
                Value::Text("cmdline_sha256".into()),
                Value::Bytes(self.cmdline_sha256.to_vec()),
            ),
            (
                Value::Text("domain".into()),
                Value::Text(PROVENANCE_DOMAIN.into()),
            ),
            (
                Value::Text("initrd_sha256".into()),
                Value::Bytes(self.initrd_sha256.to_vec()),
            ),
            (
                Value::Text("kernel_sha256".into()),
                Value::Bytes(self.kernel_sha256.to_vec()),
            ),
            (
                Value::Text("launch_measurement".into()),
                Value::Bytes(self.launch_measurement.to_vec()),
            ),
            (
                Value::Text("measurement_kind".into()),
                Value::Text(self.measurement_kind.clone()),
            ),
            (
                Value::Text("ovmf_sha256".into()),
                Value::Bytes(self.ovmf_sha256.to_vec()),
            ),
            (
                Value::Text("s3_bucket".into()),
                Value::Text(self.s3_bucket.clone()),
            ),
            (
                Value::Text("s3_key".into()),
                Value::Text(self.s3_key.clone()),
            ),
            (
                Value::Text("schema_version".into()),
                Value::Integer(self.schema_version.into()),
            ),
            (
                Value::Text("signer_pubkey".into()),
                Value::Bytes(self.signer_pubkey.to_vec()),
            ),
            (Value::Text("snp_launch_config".into()), snp_cfg),
            (
                Value::Text("verity_root_hash".into()),
                Value::Bytes(self.verity_root_hash.to_vec()),
            ),
        ]);
        to_canonical_vec(&v).map_err(|e| schema(format!("encode: {e}")))
    }

    /// Decode + validate a canonical-CBOR provenance body.
    ///
    /// Fail-closed at every step: rejects a non-canonical encoding, a
    /// non-text or duplicated map key, an unknown field, a wrong
    /// `domain`, an unknown `schema_version` / `measurement_kind`, and
    /// any byte field whose length is wrong. This is the parser the
    /// §22 signer and the miner fetch tool run on hostile-origin
    /// bytes — it never trusts the wire shape.
    pub fn decode(body: &[u8]) -> Result<ProvenanceMap> {
        // Canonical gate BEFORE the structural decode — a
        // non-deterministic wrapper never reaches the field logic.
        assert_canonical(body).map_err(|e| schema(format!("body: {e}")))?;
        let value: Value =
            ciborium::de::from_reader(body).map_err(|e| schema(format!("decode: {e}")))?;
        let mut map = into_string_map(value)?;

        let domain = take_text(&mut map, "domain")?;
        if domain != PROVENANCE_DOMAIN {
            return Err(schema(format!(
                "domain must be {PROVENANCE_DOMAIN:?}, got {domain:?}"
            )));
        }
        let schema_version = take_u32(&mut map, "schema_version")?;
        if schema_version != PROVENANCE_SCHEMA_VERSION {
            return Err(schema(format!(
                "unknown schema_version {schema_version} (want {PROVENANCE_SCHEMA_VERSION})"
            )));
        }
        let measurement_kind = take_text(&mut map, "measurement_kind")?;
        if measurement_kind != MEASUREMENT_KIND_SNP {
            return Err(schema(format!(
                "measurement_kind must be {MEASUREMENT_KIND_SNP:?}, got {measurement_kind:?}"
            )));
        }

        let launch_measurement =
            take_byte_array::<LAUNCH_MEASUREMENT_LEN>(&mut map, "launch_measurement")?;
        let artifact_sha256 = take_byte_array::<SHA256_LEN>(&mut map, "artifact_sha256")?;
        let verity_root_hash = take_byte_array::<SHA256_LEN>(&mut map, "verity_root_hash")?;
        let kernel_sha256 = take_byte_array::<SHA256_LEN>(&mut map, "kernel_sha256")?;
        let initrd_sha256 = take_byte_array::<SHA256_LEN>(&mut map, "initrd_sha256")?;
        let cmdline_sha256 = take_byte_array::<SHA256_LEN>(&mut map, "cmdline_sha256")?;
        let ovmf_sha256 = take_byte_array::<SHA256_LEN>(&mut map, "ovmf_sha256")?;
        let signer_pubkey = take_byte_array::<SHA256_LEN>(&mut map, "signer_pubkey")?;
        let s3_bucket = take_text(&mut map, "s3_bucket")?;
        let s3_key = take_text(&mut map, "s3_key")?;
        let built_at_unix = take_u64(&mut map, "built_at_unix")?;

        let mut snp = into_string_map(take(&mut map, "snp_launch_config")?)?;
        let snp_launch_config = SnpLaunchConfig {
            vcpus: take_u32(&mut snp, "vcpus")?,
            vcpu_type: take_text(&mut snp, "vcpu_type")?,
            guest_features: take_text(&mut snp, "guest_features")?,
        };
        reject_leftover(&snp, "snp_launch_config")?;
        reject_leftover(&map, "provenance map")?;

        let provenance = ProvenanceMap {
            schema_version,
            measurement_kind,
            launch_measurement,
            artifact_sha256,
            verity_root_hash,
            kernel_sha256,
            initrd_sha256,
            cmdline_sha256,
            ovmf_sha256,
            snp_launch_config,
            s3_bucket,
            s3_key,
            built_at_unix,
            signer_pubkey,
        };
        // Apply the same semantic invariants `canonical()` enforces —
        // a hand-crafted canonical body must not verify if it carries
        // a value the encoder would never have produced.
        provenance.validate()?;
        Ok(provenance)
    }
}

/// Signed provenance envelope — the bytes of a `provenance.cbor` file.
///
/// `body` is the canonical CBOR of a [`ProvenanceMap`]; `sig` is the
/// 64-byte Ed25519 signature over `body` by the §22 root key. Both
/// [`encode`](Self::encode) and [`decode`](Self::decode) keep the
/// outer envelope canonical too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedProvenance {
    pub body: Vec<u8>,
    pub sig: Vec<u8>,
}

/// Length of an Ed25519 signature.
pub const SIGNATURE_LEN: usize = 64;

impl SignedProvenance {
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

    /// Decode a canonical-CBOR `provenance.cbor` envelope. Rejects a
    /// non-canonical wrapper, unknown fields, and a wrong-length sig.
    pub fn decode(bytes: &[u8]) -> Result<SignedProvenance> {
        assert_canonical(bytes).map_err(|e| schema(format!("envelope: {e}")))?;
        let value: Value = ciborium::de::from_reader(bytes)
            .map_err(|e| schema(format!("envelope decode: {e}")))?;
        let mut map = into_string_map(value)?;
        let body = take_bytes(&mut map, "body")?;
        let sig = take_bytes(&mut map, "sig")?;
        reject_leftover(&map, "signed envelope")?;
        if sig.len() != SIGNATURE_LEN {
            return Err(schema(format!(
                "sig must be {SIGNATURE_LEN} bytes, got {}",
                sig.len()
            )));
        }
        Ok(SignedProvenance { body, sig })
    }
}

// ── decode helpers ──────────────────────────────────────────────────

fn schema(msg: String) -> HippiusTypesError {
    HippiusTypesError::ProvenanceSchema(msg)
}

/// Convert a CBOR map into a string-keyed map, rejecting a non-map
/// value or any non-text key. (`assert_canonical` has already rejected
/// duplicate keys, so an insertion collision cannot occur here.)
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
    let arr: [u8; N] = bytes.as_slice().try_into().map_err(|_| {
        schema(format!(
            "field {key:?} must be {N} bytes, got {}",
            bytes.len()
        ))
    })?;
    Ok(arr)
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

    fn sample() -> ProvenanceMap {
        ProvenanceMap {
            schema_version: PROVENANCE_SCHEMA_VERSION,
            measurement_kind: MEASUREMENT_KIND_SNP.to_string(),
            launch_measurement: [0x11; LAUNCH_MEASUREMENT_LEN],
            artifact_sha256: [0x22; SHA256_LEN],
            verity_root_hash: [0x33; SHA256_LEN],
            kernel_sha256: [0x44; SHA256_LEN],
            initrd_sha256: [0x55; SHA256_LEN],
            cmdline_sha256: [0x66; SHA256_LEN],
            ovmf_sha256: [0x77; SHA256_LEN],
            snp_launch_config: SnpLaunchConfig {
                vcpus: 1,
                vcpu_type: "EpycV4".to_string(),
                guest_features: "0x1".to_string(),
            },
            s3_bucket: "hippius-compute-images".to_string(),
            s3_key: "images/2222.../kbs.uki".to_string(),
            built_at_unix: 1_700_000_000,
            signer_pubkey: [0x88; SHA256_LEN],
        }
    }

    #[test]
    fn canonical_is_stable_and_canonical() {
        let p = sample();
        let a = p.canonical().unwrap();
        let b = p.canonical().unwrap();
        assert_eq!(a, b);
        assert_canonical(&a).unwrap();
    }

    #[test]
    fn canonical_round_trips_through_decode() {
        let p = sample();
        let body = p.canonical().unwrap();
        let decoded = ProvenanceMap::decode(&body).unwrap();
        assert_eq!(p, decoded);
        // And re-encoding the decoded value is byte-identical.
        assert_eq!(body, decoded.canonical().unwrap());
    }

    #[test]
    fn every_field_is_signed() {
        // Mutating any field changes the canonical bytes — nothing in
        // the struct rides outside the signature.
        let base = sample().canonical().unwrap();
        let mutate: &[fn(&mut ProvenanceMap)] = &[
            |p| p.launch_measurement[0] ^= 0xff,
            |p| p.artifact_sha256[0] ^= 0xff,
            |p| p.verity_root_hash[0] ^= 0xff,
            |p| p.kernel_sha256[0] ^= 0xff,
            |p| p.initrd_sha256[0] ^= 0xff,
            |p| p.cmdline_sha256[0] ^= 0xff,
            |p| p.ovmf_sha256[0] ^= 0xff,
            |p| p.signer_pubkey[0] ^= 0xff,
            |p| p.snp_launch_config.vcpus += 1,
            |p| p.snp_launch_config.vcpu_type = "EpycMilan".to_string(),
            |p| p.snp_launch_config.guest_features = "0x3".to_string(),
            |p| p.s3_bucket = "other-bucket".to_string(),
            |p| p.s3_key = "images/other/kbs.uki".to_string(),
            |p| p.built_at_unix += 1,
        ];
        for m in mutate {
            let mut p = sample();
            m(&mut p);
            assert_ne!(
                base,
                p.canonical().unwrap(),
                "a field escaped the signature"
            );
        }
    }

    #[test]
    fn non_snp_measurement_kind_rejected_at_encode() {
        let mut p = sample();
        p.measurement_kind = "uki_sha384".to_string();
        assert!(p.canonical().is_err());
    }

    #[test]
    fn empty_s3_location_rejected_at_encode() {
        let mut p = sample();
        p.s3_key = String::new();
        assert!(p.canonical().is_err());
    }

    #[test]
    fn zero_vcpus_rejected_at_encode() {
        let mut p = sample();
        p.snp_launch_config.vcpus = 0;
        assert!(p.canonical().is_err());
    }

    #[test]
    fn decode_rejects_unknown_top_level_field() {
        let p = sample();
        // Re-encode with an extra key — must be rejected.
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(p.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        entries.push((Value::Text("rogue".into()), Value::Integer(1.into())));
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(ProvenanceMap::decode(&body).is_err());
    }

    #[test]
    fn decode_rejects_unknown_nested_field() {
        let p = sample();
        let mut top = match ciborium::de::from_reader::<Value, _>(p.canonical().unwrap().as_slice())
            .unwrap()
        {
            Value::Map(e) => e,
            _ => unreachable!(),
        };
        for (k, v) in &mut top {
            if matches!(k, Value::Text(t) if t == "snp_launch_config") {
                if let Value::Map(inner) = v {
                    inner.push((Value::Text("rogue".into()), Value::Integer(9.into())));
                }
            }
        }
        let body = to_canonical_vec(&Value::Map(top)).unwrap();
        assert!(ProvenanceMap::decode(&body).is_err());
    }

    #[test]
    fn decode_rejects_wrong_domain() {
        let p = sample();
        let mut top = match ciborium::de::from_reader::<Value, _>(p.canonical().unwrap().as_slice())
            .unwrap()
        {
            Value::Map(e) => e,
            _ => unreachable!(),
        };
        for (k, v) in &mut top {
            if matches!(k, Value::Text(t) if t == "domain") {
                *v = Value::Text("HIPPIUS_KBS_RELEASE_V1".into());
            }
        }
        let body = to_canonical_vec(&Value::Map(top)).unwrap();
        assert!(ProvenanceMap::decode(&body).is_err());
    }

    #[test]
    fn decode_rejects_non_canonical_body() {
        // A correctly-shaped map serialized in non-canonical key order.
        let v = Value::Map(vec![
            (Value::Text("zzz".into()), Value::Integer(1.into())),
            (
                Value::Text("domain".into()),
                Value::Text(PROVENANCE_DOMAIN.into()),
            ),
        ]);
        let mut noncanon = Vec::new();
        ciborium::ser::into_writer(&v, &mut noncanon).unwrap();
        assert!(ProvenanceMap::decode(&noncanon).is_err());
    }

    #[test]
    fn decode_rejects_short_measurement() {
        let p = sample();
        let mut top = match ciborium::de::from_reader::<Value, _>(p.canonical().unwrap().as_slice())
            .unwrap()
        {
            Value::Map(e) => e,
            _ => unreachable!(),
        };
        for (k, v) in &mut top {
            if matches!(k, Value::Text(t) if t == "launch_measurement") {
                *v = Value::Bytes(vec![0u8; 47]); // one byte short
            }
        }
        let body = to_canonical_vec(&Value::Map(top)).unwrap();
        assert!(ProvenanceMap::decode(&body).is_err());
    }

    #[test]
    fn decode_rejects_a_semantically_invalid_body() {
        // A body that is perfectly canonical CBOR but carries a value
        // `canonical()` would never emit (here: an empty `s3_bucket`)
        // must be rejected by `decode()` too — `canonical` and
        // `decode` reject exactly the same set of maps, so a
        // hand-crafted signed body cannot skip the encode-time gates.
        let p = sample();
        let mut top = match ciborium::de::from_reader::<Value, _>(p.canonical().unwrap().as_slice())
            .unwrap()
        {
            Value::Map(e) => e,
            _ => unreachable!(),
        };
        for (k, v) in &mut top {
            if matches!(k, Value::Text(t) if t == "s3_bucket") {
                *v = Value::Text(String::new());
            }
        }
        let body = to_canonical_vec(&Value::Map(top)).unwrap();
        assert_canonical(&body).unwrap(); // the body IS canonical …
        assert!(ProvenanceMap::decode(&body).is_err()); // … but still rejected.
    }

    #[test]
    fn signed_envelope_round_trips() {
        let signed = SignedProvenance {
            body: sample().canonical().unwrap(),
            sig: vec![0xABu8; SIGNATURE_LEN],
        };
        let bytes = signed.encode().unwrap();
        assert_canonical(&bytes).unwrap();
        assert_eq!(signed, SignedProvenance::decode(&bytes).unwrap());
    }

    #[test]
    fn signed_envelope_rejects_wrong_sig_length() {
        let signed = SignedProvenance {
            body: sample().canonical().unwrap(),
            sig: vec![0u8; 63],
        };
        assert!(signed.encode().is_err());
    }

    #[test]
    fn signed_envelope_decode_rejects_unknown_field() {
        let v = Value::Map(vec![
            (Value::Text("body".into()), Value::Bytes(vec![1, 2, 3])),
            (Value::Text("rogue".into()), Value::Integer(1.into())),
            (
                Value::Text("sig".into()),
                Value::Bytes(vec![0u8; SIGNATURE_LEN]),
            ),
        ]);
        let bytes = to_canonical_vec(&v).unwrap();
        assert!(SignedProvenance::decode(&bytes).is_err());
    }
}

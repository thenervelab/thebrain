//! Blackbox **host-attestor** wire formats — bare-metal SEV-SNP host
//! enrollment + liveness beacon (blackbox host-attestor chantier PR-1).
//!
//! The "blackbox" in the tenant CVM proves *a guest is alive*
//! ([`crate::live_attestation`]). The **host attestor** is the
//! complementary trust anchor for the *bare-metal host itself*: a small
//! agent running on the SNP host (outside every tenant CVM) that
//!
//! 1. **enrols** with the KBS once per boot — it generates an Ed25519
//!    signer keypair, folds a [`crate::report_data::host_attestor`]
//!    binding (nonce ‖ SHA-256(canonical-CBOR{domain, node_id,
//!    attestor_pubkey})) into its *platform* SNP report, and ships a
//!    [`HostEnrollment`]. The KBS re-verifies the report against AMD's
//!    silicon root + the platform allowlist and, on success, issues an
//!    L0 enrollment cert (domain [`HOST_ATTESTOR_CERT_DOMAIN`]) binding
//!    the now-trusted `attestor_pubkey` to the `node_id`;
//! 2. **beats** — after enrollment the attestor emits periodic
//!    [`SignedHostBeacon`]s signed by that enrolled key, letting the
//!    control plane cheaply confirm the host is still the same
//!    measured, allowlisted platform without a fresh full SNP
//!    verification on every beat.
//!
//! This module owns only the **wire format** — canonical encoder,
//! fail-closed decoder, and the [`SignedHostBeacon`] envelope. The
//! Ed25519 / HKDF crypto lives with the callers so this crate stays
//! crypto-free and `no_std`:
//! - the attestor derives its signer key with HKDF under
//!   [`HOST_ATTESTOR_KEY_DOMAIN`] (defined here for the agent, unused by
//!   this crate);
//! - the KBS signs the L0 enrollment cert under
//!   [`HOST_ATTESTOR_CERT_DOMAIN`];
//! - the enrolled attestor signs each beacon body under
//!   [`HOST_ATTESTOR_BEACON_DOMAIN`].
//!
//! ## Domain separation
//!
//! The four host-attestor domains are **byte-disjoint** from every
//! other domain tag in the stack (see the `distinct_from_*` tests in
//! this module and in [`crate::report_data`]). A signature or hash
//! minted under one scheme cannot verify as another: the beacon body
//! leads with its `domain` field, the enrollment REPORT_DATA binds the
//! report domain, and the HKDF / cert domains never share a preimage.
//!
//! ## Ships inert
//!
//! Nothing consumes these types yet — this is the frozen wire contract
//! only. The agent, KBS enrollment endpoint, and beacon verifier land
//! in later PRs.
//!
//! ## Schema (`v1`) — fixed, fail-closed
//!
//! Every field is mandatory. Decode rejects a non-canonical body,
//! trailing bytes, a non-text / duplicated map key, an unknown field, a
//! wrong `domain`, an unknown `schema_version`, a wrong-length byte
//! field, and any semantically invalid value.

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

// ── frozen domain constants (byte-exact; do NOT re-invent) ──────────

/// HKDF `info` for deriving the attestor's per-boot Ed25519 signer key.
/// Defined here for the agent; unused by this crate. Byte-disjoint from
/// every other domain so a key derived under it can never coincide with
/// a signed-body / cert / report-data binding.
pub const HOST_ATTESTOR_KEY_DOMAIN: &str = "HIPPIUS_HOST_ATTESTOR_KEY_V1";

/// Replay-domain separator bound into the enrollment `REPORT_DATA`
/// preimage. Re-exported from [`crate::report_data`] so both the report
/// layout and this module name the exact same bytes.
pub use crate::report_data::HOST_ATTESTOR_REPORT_DOMAIN;

/// Replay-domain separator — the first field of every signed
/// [`HostAliveBeacon`] body. Distinct from every other domain so a
/// beacon signature can never replay as a release / cert / live
/// attestation, and vice-versa.
pub const HOST_ATTESTOR_BEACON_DOMAIN: &str = "HIPPIUS_HOST_ATTESTOR_BEACON_V1";

/// Domain tag for the KBS L0 **enrollment cert** the KBS signs after
/// verifying a [`HostEnrollment`]. Defined here for the KBS + verifier;
/// the cert wire type lands in a later PR.
pub const HOST_ATTESTOR_CERT_DOMAIN: &str = "HIPPIUS_HOST_ATTESTOR_CERT_V1";

/// The only `schema_version` this build understands.
pub const HOST_ATTESTOR_SCHEMA_VERSION: u16 = 1;

// ── fixed byte-field lengths ────────────────────────────────────────

/// Raw SEV-SNP attestation report length.
pub const SNP_REPORT_LEN: usize = 1184;

/// Ed25519 public-key length (the attestor signer + the on-chain
/// `signer_pubkey`).
pub const PUBKEY_LEN: usize = 32;

/// Ed25519 signature length.
pub const SIGNATURE_LEN: usize = 64;

/// SHA-256 output length — `chain_genesis`, `pallet_instance`, `nonce`.
pub const DIGEST_LEN: usize = 32;

/// SEV-SNP launch-measurement length (3x SHA-384 — mirrors
/// [`crate::live_attestation::MEASUREMENT_LEN`]).
pub const MEASUREMENT_LEN: usize = 48;

/// SEV-SNP `CHIP_ID` length.
pub const CHIP_ID_LEN: usize = 64;

/// Platform TCB triple carried in a beacon — the SNP-reported TCB
/// versions the KBS pins across the platform's lifetime. Each is the
/// opaque 8-byte little-endian TCB version as an integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlatformTcb {
    /// `reported_tcb` from the platform SNP report.
    pub reported: u64,
    /// `committed_tcb` from the platform SNP report.
    pub committed: u64,
    /// `current_tcb` from the platform SNP report.
    pub current: u64,
}

impl PlatformTcb {
    fn to_value(self) -> Value {
        // Source key order is cosmetic — the parent body is re-sorted by
        // `to_canonical_vec` to RFC 8949 §4.2.1 canonical order.
        Value::Map(vec![
            (
                Value::Text("committed".into()),
                Value::Integer(self.committed.into()),
            ),
            (
                Value::Text("current".into()),
                Value::Integer(self.current.into()),
            ),
            (
                Value::Text("reported".into()),
                Value::Integer(self.reported.into()),
            ),
        ])
    }

    fn from_value(value: Value) -> Result<Self> {
        let mut map = into_string_map(value)?;
        let committed = take_u64(&mut map, "committed")?;
        let current = take_u64(&mut map, "current")?;
        let reported = take_u64(&mut map, "reported")?;
        reject_leftover(&map, "platform_tcb")?;
        Ok(PlatformTcb {
            reported,
            committed,
            current,
        })
    }
}

/// One host-attestor **enrollment** — the once-per-boot message the
/// attestor ships to the KBS. `snp_report` is the raw platform SNP
/// report whose `REPORT_DATA` binds `signer_pubkey` + `node_id` under
/// [`crate::report_data::host_attestor`]; the KBS re-verifies it before
/// issuing the L0 enrollment cert. This is a container — the SNP report
/// *is* the attestation, so the struct carries no signature of its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostEnrollment {
    /// Wire-format version. MUST be [`HOST_ATTESTOR_SCHEMA_VERSION`].
    pub schema_version: u16,
    /// The raw 1184-byte platform SEV-SNP report. Its `REPORT_DATA`
    /// binds `signer_pubkey` + `node_id` under the host-attestor report
    /// domain.
    pub snp_report: [u8; SNP_REPORT_LEN],
    /// The attestor's per-boot Ed25519 signer public key — the same key
    /// bound into the report `REPORT_DATA` and later used to sign every
    /// [`HostAliveBeacon`].
    pub signer_pubkey: [u8; PUBKEY_LEN],
    /// The host's persistent node identity (matches the miner-agent
    /// `node_id`). Bound into the report so the KBS credits the right
    /// host.
    pub node_id: String,
    /// Opaque per-boot identifier — a fresh enrollment per boot, so a
    /// stale enrollment from a previous boot cannot be replayed.
    pub boot_id: String,
    /// When the attestor issued this enrollment, Unix seconds.
    pub issued_at_unix: u64,
}

impl HostEnrollment {
    /// Every semantic invariant of a well-formed enrollment. Run by
    /// BOTH [`canonical`](Self::canonical) and [`decode`](Self::decode).
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != HOST_ATTESTOR_SCHEMA_VERSION {
            return Err(schema(format!(
                "unknown schema_version {} (want {HOST_ATTESTOR_SCHEMA_VERSION})",
                self.schema_version
            )));
        }
        if self.node_id.is_empty() {
            return Err(schema("node_id must be non-empty".into()));
        }
        if self.boot_id.is_empty() {
            return Err(schema("boot_id must be non-empty".into()));
        }
        if self.issued_at_unix == 0 {
            return Err(schema("issued_at_unix must be non-zero".into()));
        }
        Ok(())
    }

    /// Deterministic-CBOR encoding. Fails closed on any
    /// [`validate`](Self::validate) violation. Field ordering:
    /// alphabetic on text keys (RFC 8949 §4.2.1).
    pub fn canonical(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let v = Value::Map(vec![
            (
                Value::Text("boot_id".into()),
                Value::Text(self.boot_id.clone()),
            ),
            (
                Value::Text("issued_at_unix".into()),
                Value::Integer(self.issued_at_unix.into()),
            ),
            (
                Value::Text("node_id".into()),
                Value::Text(self.node_id.clone()),
            ),
            (
                Value::Text("schema_version".into()),
                Value::Integer(self.schema_version.into()),
            ),
            (
                Value::Text("signer_pubkey".into()),
                Value::Bytes(self.signer_pubkey.to_vec()),
            ),
            (
                Value::Text("snp_report".into()),
                Value::Bytes(self.snp_report.to_vec()),
            ),
        ]);
        to_canonical_vec(&v).map_err(|e| schema(format!("encode: {e}")))
    }

    /// Decode + validate a canonical-CBOR enrollment. Hostile-origin
    /// parser — never trusts the wire shape. Rejects: non-canonical
    /// encoding (incl. trailing bytes), non-text / duplicated map key,
    /// unknown field, unknown `schema_version`, wrong-length byte field,
    /// or any semantically invalid value.
    pub fn decode(body: &[u8]) -> Result<HostEnrollment> {
        assert_canonical(body).map_err(|e| schema(format!("body: {e}")))?;
        let value: Value =
            ciborium::de::from_reader(body).map_err(|e| schema(format!("decode: {e}")))?;
        let mut map = into_string_map(value)?;

        let schema_version = take_u16(&mut map, "schema_version")?;
        if schema_version != HOST_ATTESTOR_SCHEMA_VERSION {
            return Err(schema(format!(
                "unknown schema_version {schema_version} (want {HOST_ATTESTOR_SCHEMA_VERSION})"
            )));
        }
        let snp_report = take_byte_array::<SNP_REPORT_LEN>(&mut map, "snp_report")?;
        let signer_pubkey = take_byte_array::<PUBKEY_LEN>(&mut map, "signer_pubkey")?;
        let node_id = take_text(&mut map, "node_id")?;
        let boot_id = take_text(&mut map, "boot_id")?;
        let issued_at_unix = take_u64(&mut map, "issued_at_unix")?;
        reject_leftover(&map, "host enrollment")?;

        let enrollment = HostEnrollment {
            schema_version,
            snp_report,
            signer_pubkey,
            node_id,
            boot_id,
            issued_at_unix,
        };
        enrollment.validate()?;
        Ok(enrollment)
    }
}

/// One host **liveness beacon** body — the attestor-signed statement
/// "this measured, allowlisted host (`node_id` / `chip_id` /
/// `measurement`) is alive at `observed_at_unix`". Signed by the
/// enrolled attestor key under [`HOST_ATTESTOR_BEACON_DOMAIN`]; the
/// verifier checks `signer_pubkey` equals the key the KBS enrolled for
/// `node_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostAliveBeacon {
    /// Wire-format version. MUST be [`HOST_ATTESTOR_SCHEMA_VERSION`].
    pub schema_version: u16,
    /// SHA-256 of the substrate compute-chain genesis hash — replay
    /// domain across forks.
    pub chain_genesis: [u8; DIGEST_LEN],
    /// Compute-pallet instance discriminator — replay domain across
    /// multiple deployments of the same pallet on one chain.
    pub pallet_instance: [u8; DIGEST_LEN],
    /// The platform `CHIP_ID` the KBS pinned at enrollment. Binds the
    /// beacon to one physical CPU.
    pub chip_id: [u8; CHIP_ID_LEN],
    /// The platform launch `measurement` the KBS allowlisted — a host
    /// re-launched on a non-allowlisted image cannot emit a beacon that
    /// verifies against the enrolled key + measurement.
    pub measurement: [u8; MEASUREMENT_LEN],
    /// The host's persistent node identity (matches
    /// [`HostEnrollment::node_id`]).
    pub node_id: String,
    /// Opaque per-boot identifier (matches [`HostEnrollment::boot_id`]).
    /// A beacon whose `boot_id` no longer matches the current enrollment
    /// is stale.
    pub boot_id: String,
    /// Monotonic per-`(node_id, boot_id)` counter — the first beacon
    /// after enrollment uses `1` and each subsequent beacon increments.
    pub seq: u64,
    /// When the attestor produced this beacon, Unix seconds.
    pub observed_at_unix: u64,
    /// Platform TCB triple the KBS pinned at enrollment.
    pub platform_tcb: PlatformTcb,
    /// SEV-SNP guest-`policy` the platform runs under.
    pub policy: u64,
    /// Single-use freshness nonce (KBS- or vali-minted) folded into the
    /// beacon so a captured beacon cannot be replayed after its window.
    pub nonce: [u8; DIGEST_LEN],
    /// The enrolled attestor Ed25519 public key signing this body.
    /// Bound IN so a verifier checks it equals the KBS-enrolled key for
    /// `node_id` before trusting the signature.
    pub signer_pubkey: [u8; PUBKEY_LEN],
    /// Hard expiry, Unix seconds. A verifier rejects a beacon whose
    /// `expiry <= now`. `> observed_at_unix`.
    pub expiry_unix: u64,
}

impl HostAliveBeacon {
    /// Every semantic invariant of a well-formed beacon. Run by BOTH
    /// [`canonical`](Self::canonical) and [`decode`](Self::decode).
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != HOST_ATTESTOR_SCHEMA_VERSION {
            return Err(schema(format!(
                "unknown schema_version {} (want {HOST_ATTESTOR_SCHEMA_VERSION})",
                self.schema_version
            )));
        }
        if self.node_id.is_empty() {
            return Err(schema("node_id must be non-empty".into()));
        }
        if self.boot_id.is_empty() {
            return Err(schema("boot_id must be non-empty".into()));
        }
        if self.seq == 0 {
            return Err(schema("seq must be > 0".into()));
        }
        if self.observed_at_unix == 0 {
            return Err(schema("observed_at_unix must be non-zero".into()));
        }
        if self.expiry_unix <= self.observed_at_unix {
            return Err(schema(
                "expiry_unix must be strictly after observed_at_unix".into(),
            ));
        }
        Ok(())
    }

    /// Deterministic-CBOR encoding of the to-be-signed body. Fails
    /// closed on any [`validate`](Self::validate) violation. Field
    /// ordering: alphabetic on text keys (RFC 8949 §4.2.1).
    pub fn canonical(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let v = Value::Map(vec![
            (
                Value::Text("boot_id".into()),
                Value::Text(self.boot_id.clone()),
            ),
            (
                Value::Text("chain_genesis".into()),
                Value::Bytes(self.chain_genesis.to_vec()),
            ),
            (
                Value::Text("chip_id".into()),
                Value::Bytes(self.chip_id.to_vec()),
            ),
            (
                Value::Text("domain".into()),
                Value::Text(HOST_ATTESTOR_BEACON_DOMAIN.into()),
            ),
            (
                Value::Text("expiry_unix".into()),
                Value::Integer(self.expiry_unix.into()),
            ),
            (
                Value::Text("measurement".into()),
                Value::Bytes(self.measurement.to_vec()),
            ),
            (
                Value::Text("node_id".into()),
                Value::Text(self.node_id.clone()),
            ),
            (
                Value::Text("nonce".into()),
                Value::Bytes(self.nonce.to_vec()),
            ),
            (
                Value::Text("observed_at_unix".into()),
                Value::Integer(self.observed_at_unix.into()),
            ),
            (
                Value::Text("pallet_instance".into()),
                Value::Bytes(self.pallet_instance.to_vec()),
            ),
            (
                Value::Text("platform_tcb".into()),
                self.platform_tcb.to_value(),
            ),
            (
                Value::Text("policy".into()),
                Value::Integer(self.policy.into()),
            ),
            (
                Value::Text("schema_version".into()),
                Value::Integer(self.schema_version.into()),
            ),
            (Value::Text("seq".into()), Value::Integer(self.seq.into())),
            (
                Value::Text("signer_pubkey".into()),
                Value::Bytes(self.signer_pubkey.to_vec()),
            ),
        ]);
        to_canonical_vec(&v).map_err(|e| schema(format!("encode: {e}")))
    }

    /// Decode + validate a canonical-CBOR beacon body. Hostile-origin
    /// parser. Rejects: non-canonical encoding (incl. trailing bytes),
    /// non-text / duplicated map key, unknown field, wrong `domain`,
    /// unknown `schema_version`, wrong-length byte field, or any
    /// semantically invalid value.
    pub fn decode(body: &[u8]) -> Result<HostAliveBeacon> {
        assert_canonical(body).map_err(|e| schema(format!("body: {e}")))?;
        let value: Value =
            ciborium::de::from_reader(body).map_err(|e| schema(format!("decode: {e}")))?;
        let mut map = into_string_map(value)?;

        let domain = take_text(&mut map, "domain")?;
        if domain != HOST_ATTESTOR_BEACON_DOMAIN {
            return Err(schema(format!(
                "domain must be {HOST_ATTESTOR_BEACON_DOMAIN:?}, got {domain:?}"
            )));
        }
        let schema_version = take_u16(&mut map, "schema_version")?;
        if schema_version != HOST_ATTESTOR_SCHEMA_VERSION {
            return Err(schema(format!(
                "unknown schema_version {schema_version} (want {HOST_ATTESTOR_SCHEMA_VERSION})"
            )));
        }
        let chain_genesis = take_byte_array::<DIGEST_LEN>(&mut map, "chain_genesis")?;
        let pallet_instance = take_byte_array::<DIGEST_LEN>(&mut map, "pallet_instance")?;
        let chip_id = take_byte_array::<CHIP_ID_LEN>(&mut map, "chip_id")?;
        let measurement = take_byte_array::<MEASUREMENT_LEN>(&mut map, "measurement")?;
        let node_id = take_text(&mut map, "node_id")?;
        let boot_id = take_text(&mut map, "boot_id")?;
        let seq = take_u64(&mut map, "seq")?;
        let observed_at_unix = take_u64(&mut map, "observed_at_unix")?;
        let platform_tcb = PlatformTcb::from_value(take(&mut map, "platform_tcb")?)?;
        let policy = take_u64(&mut map, "policy")?;
        let nonce = take_byte_array::<DIGEST_LEN>(&mut map, "nonce")?;
        let signer_pubkey = take_byte_array::<PUBKEY_LEN>(&mut map, "signer_pubkey")?;
        let expiry_unix = take_u64(&mut map, "expiry_unix")?;
        reject_leftover(&map, "host alive beacon")?;

        let beacon = HostAliveBeacon {
            schema_version,
            chain_genesis,
            pallet_instance,
            chip_id,
            measurement,
            node_id,
            boot_id,
            seq,
            observed_at_unix,
            platform_tcb,
            policy,
            nonce,
            signer_pubkey,
            expiry_unix,
        };
        beacon.validate()?;
        Ok(beacon)
    }
}

/// Signed host-beacon envelope.
///
/// `body` is the canonical CBOR of a [`HostAliveBeacon`]; `sig` is the
/// 64-byte Ed25519 signature over `body` by the enrolled attestor key.
/// Both [`encode`](Self::encode) and [`decode`](Self::decode) keep the
/// outer envelope canonical too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedHostBeacon {
    pub body: Vec<u8>,
    pub sig: [u8; SIGNATURE_LEN],
}

impl SignedHostBeacon {
    /// Canonical-CBOR encoding of the `{body, sig}` envelope.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let v = Value::Map(vec![
            (Value::Text("body".into()), Value::Bytes(self.body.clone())),
            (Value::Text("sig".into()), Value::Bytes(self.sig.to_vec())),
        ]);
        to_canonical_vec(&v).map_err(|e| schema(format!("envelope encode: {e}")))
    }

    /// Decode a canonical-CBOR signed envelope. Rejects a non-canonical
    /// wrapper (incl. trailing bytes), unknown fields, and a
    /// wrong-length sig.
    pub fn decode(bytes: &[u8]) -> Result<SignedHostBeacon> {
        assert_canonical(bytes).map_err(|e| schema(format!("envelope: {e}")))?;
        let value: Value = ciborium::de::from_reader(bytes)
            .map_err(|e| schema(format!("envelope decode: {e}")))?;
        let mut map = into_string_map(value)?;
        let body = take_bytes(&mut map, "body")?;
        let sig = take_byte_array::<SIGNATURE_LEN>(&mut map, "sig")?;
        reject_leftover(&map, "signed host-beacon envelope")?;
        Ok(SignedHostBeacon { body, sig })
    }
}

/// The KBS L0 **enrollment certificate** — the KBS's signed statement
/// *"I cryptographically verified a genuine SEV-SNP platform report
/// (measured, allowlisted as a host-attestor, on physical `chip_id`)
/// whose `REPORT_DATA` bound `attestor_pubkey` + `node_id` under the
/// vali-minted single-use `nonce`; trust this `attestor_pubkey` for
/// this `node_id` until `expiry_unix`."*
///
/// The KBS signs this body under [`HOST_ATTESTOR_CERT_DOMAIN`] with its
/// L0 signing key. The validator (a later PR) pins the cert, then
/// trusts the enrolled attestor's [`SignedHostBeacon`]s against
/// `attestor_pubkey` / `chip_id` / `measurement` — without a fresh full
/// SNP verification per beat.
///
/// ## What is (and is NOT) attested
///
/// Every field here is derived from the **AMD-signed** platform report
/// the KBS verified: `chip_id`, `measurement`, and `tcb` come straight
/// from the verified report; `attestor_pubkey` + `node_id` are the
/// values the KBS recomputed into the report's `REPORT_DATA` binding
/// (see [`crate::report_data::host_attestor`]) and byte-matched — so
/// they are attested too. The enrollment's `boot_id` / `issued_at_unix`
/// are **NOT** in `REPORT_DATA` and are therefore NOT attested; they
/// never enter this cert. Anti-replay rests solely on the single-use
/// `nonce`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostAttestorCert {
    /// Wire-format version. MUST be [`HOST_ATTESTOR_SCHEMA_VERSION`].
    pub schema_version: u16,
    /// The attested host node identity — recomputed into the report
    /// `REPORT_DATA` binding + byte-matched by the KBS, never taken
    /// blindly from a self-declared field.
    pub node_id: String,
    /// The AMD-signed platform `CHIP_ID` from the verified report. The
    /// anti-relay linchpin: a later PR cross-checks this against the
    /// on-chain `CHIP_ID` so a host cannot relay another platform's
    /// beacons.
    pub chip_id: [u8; CHIP_ID_LEN],
    /// The now-trusted attestor Ed25519 signer public key — the same
    /// key the report `REPORT_DATA` bound. Every subsequent
    /// [`SignedHostBeacon`] must verify against this key.
    pub attestor_pubkey: [u8; PUBKEY_LEN],
    /// The platform launch `measurement` from the verified report —
    /// allowlisted with the host-attestor class.
    pub measurement: [u8; MEASUREMENT_LEN],
    /// The platform `reported_tcb` (comparable integer) from the
    /// verified report.
    pub tcb: u64,
    /// The vali/KBS-minted single-use nonce bound into the report
    /// `REPORT_DATA[0..32]`. Anchors this cert to one enrollment.
    pub nonce: [u8; DIGEST_LEN],
    /// Hard expiry, Unix seconds. A verifier rejects a cert whose
    /// `expiry_unix <= now`.
    pub expiry_unix: u64,
}

impl HostAttestorCert {
    /// Every semantic invariant of a well-formed cert. Run by BOTH
    /// [`canonical`](Self::canonical) and [`decode`](Self::decode).
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != HOST_ATTESTOR_SCHEMA_VERSION {
            return Err(schema(format!(
                "unknown schema_version {} (want {HOST_ATTESTOR_SCHEMA_VERSION})",
                self.schema_version
            )));
        }
        if self.node_id.is_empty() {
            return Err(schema("node_id must be non-empty".into()));
        }
        if self.expiry_unix == 0 {
            return Err(schema("expiry_unix must be non-zero".into()));
        }
        Ok(())
    }

    /// Deterministic-CBOR encoding of the to-be-signed body. Fails
    /// closed on any [`validate`](Self::validate) violation. Field
    /// ordering: alphabetic on text keys (RFC 8949 §4.2.1). Leads with
    /// its `domain` field so a cert signature can never replay as a
    /// beacon / release / live-attestation body.
    pub fn canonical(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let v = Value::Map(vec![
            (
                Value::Text("attestor_pubkey".into()),
                Value::Bytes(self.attestor_pubkey.to_vec()),
            ),
            (
                Value::Text("chip_id".into()),
                Value::Bytes(self.chip_id.to_vec()),
            ),
            (
                Value::Text("domain".into()),
                Value::Text(HOST_ATTESTOR_CERT_DOMAIN.into()),
            ),
            (
                Value::Text("expiry_unix".into()),
                Value::Integer(self.expiry_unix.into()),
            ),
            (
                Value::Text("measurement".into()),
                Value::Bytes(self.measurement.to_vec()),
            ),
            (
                Value::Text("node_id".into()),
                Value::Text(self.node_id.clone()),
            ),
            (
                Value::Text("nonce".into()),
                Value::Bytes(self.nonce.to_vec()),
            ),
            (
                Value::Text("schema_version".into()),
                Value::Integer(self.schema_version.into()),
            ),
            (Value::Text("tcb".into()), Value::Integer(self.tcb.into())),
        ]);
        to_canonical_vec(&v).map_err(|e| schema(format!("encode: {e}")))
    }

    /// Decode + validate a canonical-CBOR cert body. Hostile-origin
    /// parser. Rejects: non-canonical encoding (incl. trailing bytes),
    /// non-text / duplicated map key, unknown field, wrong `domain`,
    /// unknown `schema_version`, wrong-length byte field, or any
    /// semantically invalid value.
    pub fn decode(body: &[u8]) -> Result<HostAttestorCert> {
        assert_canonical(body).map_err(|e| schema(format!("body: {e}")))?;
        let value: Value =
            ciborium::de::from_reader(body).map_err(|e| schema(format!("decode: {e}")))?;
        let mut map = into_string_map(value)?;

        let domain = take_text(&mut map, "domain")?;
        if domain != HOST_ATTESTOR_CERT_DOMAIN {
            return Err(schema(format!(
                "domain must be {HOST_ATTESTOR_CERT_DOMAIN:?}, got {domain:?}"
            )));
        }
        let schema_version = take_u16(&mut map, "schema_version")?;
        if schema_version != HOST_ATTESTOR_SCHEMA_VERSION {
            return Err(schema(format!(
                "unknown schema_version {schema_version} (want {HOST_ATTESTOR_SCHEMA_VERSION})"
            )));
        }
        let chip_id = take_byte_array::<CHIP_ID_LEN>(&mut map, "chip_id")?;
        let attestor_pubkey = take_byte_array::<PUBKEY_LEN>(&mut map, "attestor_pubkey")?;
        let measurement = take_byte_array::<MEASUREMENT_LEN>(&mut map, "measurement")?;
        let node_id = take_text(&mut map, "node_id")?;
        let tcb = take_u64(&mut map, "tcb")?;
        let nonce = take_byte_array::<DIGEST_LEN>(&mut map, "nonce")?;
        let expiry_unix = take_u64(&mut map, "expiry_unix")?;
        reject_leftover(&map, "host attestor cert")?;

        let cert = HostAttestorCert {
            schema_version,
            node_id,
            chip_id,
            attestor_pubkey,
            measurement,
            tcb,
            nonce,
            expiry_unix,
        };
        cert.validate()?;
        Ok(cert)
    }
}

/// Signed host-attestor-cert envelope.
///
/// `body` is the canonical CBOR of a [`HostAttestorCert`]; `sig` is the
/// 64-byte Ed25519 signature over `body` by the KBS L0 key. Both
/// [`encode`](Self::encode) and [`decode`](Self::decode) keep the outer
/// envelope canonical too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedHostAttestorCert {
    pub body: Vec<u8>,
    pub sig: [u8; SIGNATURE_LEN],
}

impl SignedHostAttestorCert {
    /// Canonical-CBOR encoding of the `{body, sig}` envelope.
    pub fn encode(&self) -> Result<Vec<u8>> {
        let v = Value::Map(vec![
            (Value::Text("body".into()), Value::Bytes(self.body.clone())),
            (Value::Text("sig".into()), Value::Bytes(self.sig.to_vec())),
        ]);
        to_canonical_vec(&v).map_err(|e| schema(format!("envelope encode: {e}")))
    }

    /// Decode a canonical-CBOR signed envelope. Rejects a non-canonical
    /// wrapper (incl. trailing bytes), unknown fields, and a
    /// wrong-length sig.
    pub fn decode(bytes: &[u8]) -> Result<SignedHostAttestorCert> {
        assert_canonical(bytes).map_err(|e| schema(format!("envelope: {e}")))?;
        let value: Value = ciborium::de::from_reader(bytes)
            .map_err(|e| schema(format!("envelope decode: {e}")))?;
        let mut map = into_string_map(value)?;
        let body = take_bytes(&mut map, "body")?;
        let sig = take_byte_array::<SIGNATURE_LEN>(&mut map, "sig")?;
        reject_leftover(&map, "signed host-attestor-cert envelope")?;
        Ok(SignedHostAttestorCert { body, sig })
    }
}

// ── decode helpers (module-local, mirroring `live_attestation.rs`) ──

fn schema(msg: String) -> HippiusTypesError {
    HippiusTypesError::HostAttestorSchema(msg)
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
    let len = bytes.len();
    bytes
        .try_into()
        .map_err(|_| schema(format!("field {key:?} must be {N} bytes, got {len}")))
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

fn take_u16(map: &mut BTreeMap<String, Value>, key: &str) -> Result<u16> {
    let n = take_u64(map, key)?;
    u16::try_from(n).map_err(|_| schema(format!("field {key:?} out of u16 range")))
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

    /// Frozen canonical-CBOR of [`sample_beacon`] — the exact
    /// to-be-signed bytes. Regenerated only on a deliberate schema bump.
    const BEACON_BODY_HEX_KAT: &str = "af6373657107656e6f6e63655820111111111111111111111111111111111111111111111111111111111111111166646f6d61696e781f484950504955535f484f53545f4154544553544f525f424541434f4e5f563166706f6c6963791a0003000067626f6f745f696468626f6f742d61626367636869705f6964584033333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333676e6f64655f69646b6e6f64652d686f73742d316b6578706972795f756e69781a6b49d5846b6d6561737572656d656e7458304444444444444444444444444444444444444444444444444444444444444444444444444444444444444444444444446c706c6174666f726d5f746362a36763757272656e741b070800000000000b687265706f727465641b070800000000000b69636f6d6d69747465641b070800000000000a6d636861696e5f67656e657369735820aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa6d7369676e65725f7075626b6579582022222222222222222222222222222222222222222222222222222222222222226e736368656d615f76657273696f6e016f70616c6c65745f696e7374616e63655820dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd706f627365727665645f61745f756e69781a6b49d200";

    /// Frozen [`SignedHostBeacon`] envelope over [`BEACON_BODY_HEX_KAT`]
    /// with a fixed placeholder `sig = [0xAB; 64]` — the sig-over-body
    /// wire vector.
    const SIGNED_BEACON_HEX_KAT: &str = "a2637369675840abababababababababababababababababababababababababababababababababababababababababababababababababababababababababababababababab64626f647959021caf6373657107656e6f6e63655820111111111111111111111111111111111111111111111111111111111111111166646f6d61696e781f484950504955535f484f53545f4154544553544f525f424541434f4e5f563166706f6c6963791a0003000067626f6f745f696468626f6f742d61626367636869705f6964584033333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333333676e6f64655f69646b6e6f64652d686f73742d316b6578706972795f756e69781a6b49d5846b6d6561737572656d656e7458304444444444444444444444444444444444444444444444444444444444444444444444444444444444444444444444446c706c6174666f726d5f746362a36763757272656e741b070800000000000b687265706f727465641b070800000000000b69636f6d6d69747465641b070800000000000a6d636861696e5f67656e657369735820aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa6d7369676e65725f7075626b6579582022222222222222222222222222222222222222222222222222222222222222226e736368656d615f76657273696f6e016f70616c6c65745f696e7374616e63655820dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd706f627365727665645f61745f756e69781a6b49d200";

    fn sample_enrollment() -> HostEnrollment {
        HostEnrollment {
            schema_version: HOST_ATTESTOR_SCHEMA_VERSION,
            snp_report: [0x5A; SNP_REPORT_LEN],
            signer_pubkey: [0x22; PUBKEY_LEN],
            node_id: "node-host-1".into(),
            boot_id: "boot-abc".into(),
            issued_at_unix: 1_800_000_000,
        }
    }

    fn sample_beacon() -> HostAliveBeacon {
        HostAliveBeacon {
            schema_version: HOST_ATTESTOR_SCHEMA_VERSION,
            chain_genesis: [0xAA; DIGEST_LEN],
            pallet_instance: [0xDD; DIGEST_LEN],
            chip_id: [0x33; CHIP_ID_LEN],
            measurement: [0x44; MEASUREMENT_LEN],
            node_id: "node-host-1".into(),
            boot_id: "boot-abc".into(),
            seq: 7,
            observed_at_unix: 1_800_000_000,
            platform_tcb: PlatformTcb {
                reported: 0x0708_0000_0000_000B,
                committed: 0x0708_0000_0000_000A,
                current: 0x0708_0000_0000_000B,
            },
            policy: 0x30000,
            nonce: [0x11; DIGEST_LEN],
            signer_pubkey: [0x22; PUBKEY_LEN],
            expiry_unix: 1_800_000_900,
        }
    }

    fn to_hex(bytes: &[u8]) -> String {
        let mut hex = String::new();
        for b in bytes {
            hex.push_str(&format!("{b:02x}"));
        }
        hex
    }

    // ── enrollment round-trip / fail-closed ────────────────────────

    #[test]
    fn enrollment_round_trips_through_decode() {
        let a = sample_enrollment();
        let body = a.canonical().unwrap();
        let decoded = HostEnrollment::decode(&body).unwrap();
        assert_eq!(a, decoded);
        assert_eq!(body, decoded.canonical().unwrap());
    }

    #[test]
    fn enrollment_canonical_is_canonical() {
        let body = sample_enrollment().canonical().unwrap();
        assert_canonical(&body).unwrap();
    }

    #[test]
    fn enrollment_every_field_is_bound() {
        let base = sample_enrollment().canonical().unwrap();
        let mutate: &[fn(&mut HostEnrollment)] = &[
            |a| a.snp_report[0] ^= 0xFF,
            |a| a.signer_pubkey[0] ^= 0xFF,
            |a| a.node_id = "other-node".into(),
            |a| a.boot_id = "other-boot".into(),
            |a| a.issued_at_unix += 1,
        ];
        for m in mutate {
            let mut a = sample_enrollment();
            m(&mut a);
            assert_ne!(base, a.canonical().unwrap(), "a field escaped the encoding");
        }
    }

    #[test]
    fn enrollment_rejects_empty_node_id_at_encode() {
        let mut a = sample_enrollment();
        a.node_id = String::new();
        assert!(a.canonical().is_err());
    }

    #[test]
    fn enrollment_rejects_empty_boot_id_at_encode() {
        let mut a = sample_enrollment();
        a.boot_id = String::new();
        assert!(a.canonical().is_err());
    }

    #[test]
    fn enrollment_rejects_zero_issued_at_at_encode() {
        let mut a = sample_enrollment();
        a.issued_at_unix = 0;
        assert!(a.canonical().is_err());
    }

    #[test]
    fn enrollment_rejects_unknown_schema_version_at_encode() {
        let mut a = sample_enrollment();
        a.schema_version = 99;
        assert!(a.canonical().is_err());
    }

    #[test]
    fn enrollment_decode_rejects_unknown_field() {
        let a = sample_enrollment();
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(a.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        entries.push((Value::Text("rogue".into()), Value::Integer(1.into())));
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(HostEnrollment::decode(&body).is_err());
    }

    #[test]
    fn enrollment_decode_rejects_short_signer_pubkey() {
        let a = sample_enrollment();
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(a.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        for (k, v) in &mut entries {
            if matches!(k, Value::Text(t) if t == "signer_pubkey") {
                *v = Value::Bytes(vec![0u8; 31]);
            }
        }
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(HostEnrollment::decode(&body).is_err());
    }

    #[test]
    fn enrollment_decode_rejects_trailing_bytes() {
        let mut body = sample_enrollment().canonical().unwrap();
        body.push(0x00);
        assert!(HostEnrollment::decode(&body).is_err());
    }

    #[test]
    fn enrollment_decode_rejects_non_canonical_body() {
        // Two keys in the wrong (non-sorted) order — canonical form
        // sorts them, so the raw bytes differ ⇒ rejected.
        let v = Value::Map(vec![
            (Value::Text("zzz".into()), Value::Integer(1.into())),
            (
                Value::Text("schema_version".into()),
                Value::Integer(1.into()),
            ),
        ]);
        let mut noncanon = Vec::new();
        ciborium::ser::into_writer(&v, &mut noncanon).unwrap();
        assert!(HostEnrollment::decode(&noncanon).is_err());
    }

    // ── beacon round-trip / fail-closed ────────────────────────────

    #[test]
    fn beacon_round_trips_through_decode() {
        let a = sample_beacon();
        let body = a.canonical().unwrap();
        let decoded = HostAliveBeacon::decode(&body).unwrap();
        assert_eq!(a, decoded);
        assert_eq!(body, decoded.canonical().unwrap());
    }

    #[test]
    fn beacon_canonical_is_canonical() {
        let body = sample_beacon().canonical().unwrap();
        assert_canonical(&body).unwrap();
    }

    #[test]
    fn beacon_every_field_is_signed() {
        let base = sample_beacon().canonical().unwrap();
        let mutate: &[fn(&mut HostAliveBeacon)] = &[
            |a| a.chain_genesis[0] ^= 0xFF,
            |a| a.pallet_instance[0] ^= 0xFF,
            |a| a.chip_id[0] ^= 0xFF,
            |a| a.measurement[0] ^= 0xFF,
            |a| a.node_id = "other-node".into(),
            |a| a.boot_id = "other-boot".into(),
            |a| a.seq += 1,
            |a| a.observed_at_unix += 1,
            |a| a.platform_tcb.reported += 1,
            |a| a.platform_tcb.committed += 1,
            |a| a.platform_tcb.current += 1,
            |a| a.policy += 1,
            |a| a.nonce[0] ^= 0xFF,
            |a| a.signer_pubkey[0] ^= 0xFF,
            |a| a.expiry_unix += 1,
        ];
        for m in mutate {
            let mut a = sample_beacon();
            m(&mut a);
            assert_ne!(
                base,
                a.canonical().unwrap(),
                "a field escaped the signature"
            );
        }
    }

    #[test]
    fn beacon_rejects_zero_seq_at_encode() {
        let mut a = sample_beacon();
        a.seq = 0;
        assert!(a.canonical().is_err());
    }

    #[test]
    fn beacon_rejects_non_strict_expiry_at_encode() {
        let mut a = sample_beacon();
        a.expiry_unix = a.observed_at_unix;
        assert!(a.canonical().is_err());
        a.expiry_unix = a.observed_at_unix - 1;
        assert!(a.canonical().is_err());
    }

    #[test]
    fn beacon_rejects_empty_node_id_at_encode() {
        let mut a = sample_beacon();
        a.node_id = String::new();
        assert!(a.canonical().is_err());
    }

    #[test]
    fn beacon_decode_rejects_wrong_domain() {
        let a = sample_beacon();
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(a.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        for (k, v) in &mut entries {
            if matches!(k, Value::Text(t) if t == "domain") {
                *v = Value::Text("HIPPIUS_LIVE_ATTESTATION_V1".into());
            }
        }
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(HostAliveBeacon::decode(&body).is_err());
    }

    #[test]
    fn beacon_decode_rejects_unknown_field() {
        let a = sample_beacon();
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(a.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        entries.push((Value::Text("rogue".into()), Value::Integer(1.into())));
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(HostAliveBeacon::decode(&body).is_err());
    }

    #[test]
    fn beacon_decode_rejects_short_measurement() {
        let a = sample_beacon();
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(a.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        for (k, v) in &mut entries {
            if matches!(k, Value::Text(t) if t == "measurement") {
                *v = Value::Bytes(vec![0u8; 47]);
            }
        }
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(HostAliveBeacon::decode(&body).is_err());
    }

    #[test]
    fn beacon_decode_rejects_trailing_bytes() {
        let mut body = sample_beacon().canonical().unwrap();
        body.push(0x00);
        assert!(HostAliveBeacon::decode(&body).is_err());
    }

    #[test]
    fn beacon_decode_rejects_malformed_platform_tcb() {
        // platform_tcb with an extra unknown key ⇒ rejected.
        let a = sample_beacon();
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(a.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        for (k, v) in &mut entries {
            if matches!(k, Value::Text(t) if t == "platform_tcb") {
                *v = Value::Map(vec![
                    (Value::Text("committed".into()), Value::Integer(1.into())),
                    (Value::Text("current".into()), Value::Integer(1.into())),
                    (Value::Text("reported".into()), Value::Integer(1.into())),
                    (Value::Text("rogue".into()), Value::Integer(1.into())),
                ]);
            }
        }
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(HostAliveBeacon::decode(&body).is_err());
    }

    // ── signed envelope ────────────────────────────────────────────

    #[test]
    fn signed_beacon_round_trips() {
        let signed = SignedHostBeacon {
            body: sample_beacon().canonical().unwrap(),
            sig: [0xAB; SIGNATURE_LEN],
        };
        let bytes = signed.encode().unwrap();
        assert_canonical(&bytes).unwrap();
        assert_eq!(signed, SignedHostBeacon::decode(&bytes).unwrap());
    }

    #[test]
    fn signed_beacon_decode_rejects_short_sig() {
        let signed = SignedHostBeacon {
            body: sample_beacon().canonical().unwrap(),
            sig: [0xAB; SIGNATURE_LEN],
        };
        let bytes = signed.encode().unwrap();
        let mut entries = match ciborium::de::from_reader::<Value, _>(bytes.as_slice()).unwrap() {
            Value::Map(e) => e,
            _ => unreachable!(),
        };
        for (k, v) in &mut entries {
            if matches!(k, Value::Text(t) if t == "sig") {
                *v = Value::Bytes(vec![0u8; 63]);
            }
        }
        let bad = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(SignedHostBeacon::decode(&bad).is_err());
    }

    #[test]
    fn signed_beacon_decode_rejects_unknown_field() {
        let signed = SignedHostBeacon {
            body: sample_beacon().canonical().unwrap(),
            sig: [0xAB; SIGNATURE_LEN],
        };
        let bytes = signed.encode().unwrap();
        let mut entries = match ciborium::de::from_reader::<Value, _>(bytes.as_slice()).unwrap() {
            Value::Map(e) => e,
            _ => unreachable!(),
        };
        entries.push((Value::Text("extra".into()), Value::Integer(1.into())));
        let bad = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(SignedHostBeacon::decode(&bad).is_err());
    }

    // ── enrollment cert round-trip / fail-closed ──────────────────

    fn sample_cert() -> HostAttestorCert {
        HostAttestorCert {
            schema_version: HOST_ATTESTOR_SCHEMA_VERSION,
            node_id: "node-host-1".into(),
            chip_id: [0x33; CHIP_ID_LEN],
            attestor_pubkey: [0x22; PUBKEY_LEN],
            measurement: [0x44; MEASUREMENT_LEN],
            tcb: 0x0708_0000_0000_000B,
            nonce: [0x11; DIGEST_LEN],
            expiry_unix: 1_800_000_900,
        }
    }

    #[test]
    fn cert_round_trips_through_decode() {
        let a = sample_cert();
        let body = a.canonical().unwrap();
        let decoded = HostAttestorCert::decode(&body).unwrap();
        assert_eq!(a, decoded);
        assert_eq!(body, decoded.canonical().unwrap());
    }

    #[test]
    fn cert_canonical_is_canonical() {
        let body = sample_cert().canonical().unwrap();
        assert_canonical(&body).unwrap();
    }

    #[test]
    fn cert_every_field_is_signed() {
        let base = sample_cert().canonical().unwrap();
        let mutate: &[fn(&mut HostAttestorCert)] = &[
            |a| a.node_id = "other-node".into(),
            |a| a.chip_id[0] ^= 0xFF,
            |a| a.attestor_pubkey[0] ^= 0xFF,
            |a| a.measurement[0] ^= 0xFF,
            |a| a.tcb += 1,
            |a| a.nonce[0] ^= 0xFF,
            |a| a.expiry_unix += 1,
        ];
        for m in mutate {
            let mut a = sample_cert();
            m(&mut a);
            assert_ne!(
                base,
                a.canonical().unwrap(),
                "a field escaped the signature"
            );
        }
    }

    #[test]
    fn cert_rejects_empty_node_id_at_encode() {
        let mut a = sample_cert();
        a.node_id = String::new();
        assert!(a.canonical().is_err());
    }

    #[test]
    fn cert_rejects_zero_expiry_at_encode() {
        let mut a = sample_cert();
        a.expiry_unix = 0;
        assert!(a.canonical().is_err());
    }

    #[test]
    fn cert_decode_rejects_wrong_domain() {
        let a = sample_cert();
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(a.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        for (k, v) in &mut entries {
            if matches!(k, Value::Text(t) if t == "domain") {
                *v = Value::Text(HOST_ATTESTOR_BEACON_DOMAIN.into());
            }
        }
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(HostAttestorCert::decode(&body).is_err());
    }

    #[test]
    fn cert_decode_rejects_unknown_field() {
        let a = sample_cert();
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(a.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        entries.push((Value::Text("rogue".into()), Value::Integer(1.into())));
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(HostAttestorCert::decode(&body).is_err());
    }

    #[test]
    fn cert_decode_rejects_short_chip_id() {
        let a = sample_cert();
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(a.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        for (k, v) in &mut entries {
            if matches!(k, Value::Text(t) if t == "chip_id") {
                *v = Value::Bytes(vec![0u8; 63]);
            }
        }
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(HostAttestorCert::decode(&body).is_err());
    }

    #[test]
    fn cert_decode_rejects_trailing_bytes() {
        let mut body = sample_cert().canonical().unwrap();
        body.push(0x00);
        assert!(HostAttestorCert::decode(&body).is_err());
    }

    #[test]
    fn signed_cert_round_trips() {
        let signed = SignedHostAttestorCert {
            body: sample_cert().canonical().unwrap(),
            sig: [0xAB; SIGNATURE_LEN],
        };
        let bytes = signed.encode().unwrap();
        assert_canonical(&bytes).unwrap();
        assert_eq!(signed, SignedHostAttestorCert::decode(&bytes).unwrap());
    }

    #[test]
    fn signed_cert_decode_rejects_short_sig() {
        let signed = SignedHostAttestorCert {
            body: sample_cert().canonical().unwrap(),
            sig: [0xAB; SIGNATURE_LEN],
        };
        let bytes = signed.encode().unwrap();
        let mut entries = match ciborium::de::from_reader::<Value, _>(bytes.as_slice()).unwrap() {
            Value::Map(e) => e,
            _ => unreachable!(),
        };
        for (k, v) in &mut entries {
            if matches!(k, Value::Text(t) if t == "sig") {
                *v = Value::Bytes(vec![0u8; 63]);
            }
        }
        let bad = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(SignedHostAttestorCert::decode(&bad).is_err());
    }

    // ── domain separation ──────────────────────────────────────────

    #[test]
    fn four_host_attestor_domains_are_pairwise_distinct() {
        let domains = [
            HOST_ATTESTOR_KEY_DOMAIN,
            HOST_ATTESTOR_REPORT_DOMAIN,
            HOST_ATTESTOR_BEACON_DOMAIN,
            HOST_ATTESTOR_CERT_DOMAIN,
        ];
        for (i, a) in domains.iter().enumerate() {
            for (j, b) in domains.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "host-attestor domains {i} and {j} collide");
                }
            }
        }
    }

    #[test]
    fn host_attestor_domains_disjoint_from_existing_tags() {
        // Byte-disjoint from every other domain tag in the stack — a
        // signature / hash / key minted under any of these cannot
        // cross-replay as a host-attestor payload, and vice-versa.
        let existing = [
            crate::report_data::TENANT_TELEMETRY_DOMAIN,
            crate::report_data::LIVE_ATTESTATION_REPORT_DOMAIN,
            crate::live_attestation::LIVE_ATTESTATION_DOMAIN,
            crate::graceful_exit::DOMAIN,
            crate::vm_progress::DOMAIN,
            crate::heartbeat::DOMAIN,
            crate::audit_vm::AGGREGATE_DOMAIN,
            crate::audit_vm_cert::AUDIT_VM_CERT_DOMAIN,
            crate::evidence_bundle::EVIDENCE_BUNDLE_DOMAIN,
            crate::release::RELEASE_DOMAIN,
            crate::release::DENIAL_DOMAIN,
            crate::served_receipt::RECEIPT_DOMAIN,
            crate::stopped::STOPPED_DOMAIN,
            crate::telemetry_cert::TELEMETRY_CERT_DOMAIN,
            crate::provenance::PROVENANCE_DOMAIN,
            crate::digest::USERDATA_DIGEST_DOMAIN,
        ];
        let ours = [
            HOST_ATTESTOR_KEY_DOMAIN,
            HOST_ATTESTOR_REPORT_DOMAIN,
            HOST_ATTESTOR_BEACON_DOMAIN,
            HOST_ATTESTOR_CERT_DOMAIN,
        ];
        for o in ours {
            for e in existing {
                assert_ne!(
                    o, e,
                    "host-attestor domain {o:?} collides with existing {e:?}"
                );
            }
        }
        // Also disjoint from the byte-slice audit_vm report domain.
        for o in ours {
            assert_ne!(o.as_bytes(), crate::report_data::AUDIT_VM_DOMAIN);
        }
    }

    #[test]
    fn beacon_domain_binds_the_signed_body() {
        // A body encoded under the beacon domain cannot decode after its
        // domain field is swapped for a foreign tag — cross-domain
        // replay of a signed beacon fails closed.
        let body = sample_beacon().canonical().unwrap();
        let mut entries = match ciborium::de::from_reader::<Value, _>(body.as_slice()).unwrap() {
            Value::Map(e) => e,
            _ => unreachable!(),
        };
        for (k, v) in &mut entries {
            if matches!(k, Value::Text(t) if t == "domain") {
                *v = Value::Text(HOST_ATTESTOR_CERT_DOMAIN.into());
            }
        }
        let swapped = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(HostAliveBeacon::decode(&swapped).is_err());
    }

    // ── known-answer vectors (frozen) ──────────────────────────────

    /// Frozen canonical-CBOR KAT for the enrollment body. A canonical
    /// encoding change or field re-ordering shifts these bytes ⇒ CI
    /// fails loudly.
    #[test]
    fn enrollment_canonical_known_answer() {
        let body = sample_enrollment().canonical().unwrap();
        // The 1184-byte snp_report dominates the length; assert the full
        // digest of the canonical bytes to pin every byte cheaply.
        assert_eq!(body.len(), 1320);
        assert_eq!(
            sha256_hex(&body),
            "986e278af5ec61ecec863c26a3a051f2b2baf02315f7c8f9d848a79db8ac34c1"
        );
    }

    /// Frozen canonical-CBOR KAT for the beacon body + a fixed
    /// sig-over-body envelope vector (sig = `[0xAB; 64]`). Pins the
    /// exact to-be-signed bytes AND the signed-envelope wire bytes.
    #[test]
    fn beacon_known_answer_and_signed_vector() {
        let body = sample_beacon().canonical().unwrap();
        assert_eq!(to_hex(&body), BEACON_BODY_HEX_KAT);

        let signed = SignedHostBeacon {
            body,
            sig: [0xAB; SIGNATURE_LEN],
        };
        assert_eq!(to_hex(&signed.encode().unwrap()), SIGNED_BEACON_HEX_KAT);
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        to_hex(&Sha256::digest(bytes))
    }
}

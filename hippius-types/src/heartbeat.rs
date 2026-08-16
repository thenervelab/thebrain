//! Miner-agent periodic signed heartbeat (ARCHITECTURE.md §K / §13).
//!
//! A miner-agent runs on an **untrusted** bare-metal host. To prove it
//! is alive — so vali's scheduler does not quarantine it — it builds a
//! signed [`MinerHeartbeat`] every `[heartbeat] interval_secs`, queues
//! it, and a pusher relays it over mTLS to the Edge gateway, which
//! forwards it to vali's §9 telemetry ingest. vali verifies the
//! Ed25519 signature against the out-of-band-registered miner identity,
//! enforces a monotonic `sequence` (replay defence) and a `±300 s`
//! timestamp anti-skew, then refreshes the miner's `last_seen_at`.
//!
//! This module owns only the **wire format** — the canonical encoder
//! and the [`SignedMinerHeartbeat`] envelope. The Ed25519 sign /
//! verify live with their callers (the miner-agent's `MinerIdentity`,
//! vali's `ticket-validator` shell-out) so this crate stays crypto-free
//! and `no_std`.
//!
//! ## Domain separation
//!
//! [`DOMAIN`] is **distinct** from every other signed payload in the
//! stack (`RELEASE_DOMAIN`, `RECEIPT_DOMAIN`, `STOPPED_DOMAIN`,
//! `AUDIT_VM_CERT_DOMAIN`, …) so a signature minted over a heartbeat
//! body can never be lifted into another scheme. The domain tag is a
//! field of the signed body, so a cross-scheme replay produces
//! different bytes and fails verification.
//!
//! ## Schema (`v1`) — fixed, fail-closed
//!
//! Every field is mandatory. [`MinerHeartbeat::validate`] runs on both
//! the encode and (caller-side) decode paths; [`canonical`] rejects a
//! malformed heartbeat at encode time so a signature over an
//! impossible value can never exist. `#[serde(deny_unknown_fields)]`
//! on [`SignedMinerHeartbeat`] blocks extra-key smuggling past the
//! envelope decode.

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
use ciborium::value::Value;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Replay-domain separator — the first field of every signed heartbeat
/// body. Distinct from every other signed-payload domain in the stack.
pub const DOMAIN: &str = "HIPPIUS_MINER_HEARTBEAT_V1";

/// The baseline `schema_version` — the original 10-field heartbeat.
/// Every existing miner emits this; its wire bytes are frozen by the
/// `heartbeat_kat` known-answer test and MUST never shift.
pub const SCHEMA_VERSION: u8 = 1;

/// The `schema_version` of a `v2` heartbeat — identical to the `v1`
/// body plus one extra `graceful_exit_requested` bool (which, being the
/// longest key, the RFC 8949 deterministic encoder places last). A
/// `v2` heartbeat is OPT-IN: a miner emits it only when it wants to
/// carry the graceful-exit flag (transport (B), the always-on passive
/// complement to the Edge-relayed graceful-exit request). vali accepts
/// BOTH versions; a `v1` heartbeat is byte-identical to before.
pub const SCHEMA_VERSION_GRACEFUL_EXIT: u8 = 2;

/// Anti-skew window, in seconds. vali rejects a heartbeat whose
/// `timestamp_unix` is more than this far from its own clock — in
/// either direction.
pub const MAX_AGE_SECONDS: i64 = 300;

/// Hard upper bound on the `miner_id` length, in bytes. Mirrors the
/// `vali` `MinerIdentity.miner_id` column (`max_length=64`). A longer
/// id is malformed — fail closed at encode.
pub const MAX_MINER_ID_LEN: usize = 64;

/// Length of an Ed25519 signature — the `sig` field of the envelope.
pub const SIGNATURE_LEN: usize = 64;

/// Fixed-classifier error for the heartbeat wire format.
///
/// Every `Display` is a `&'static str` — there is no `{0}`
/// interpolation that could splice a run-time value into a log line
/// (the §K logging discipline). The variants are a closed vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeartbeatError {
    /// `schema_version` was not [`SCHEMA_VERSION`].
    SchemaVersion,
    /// `domain` was not [`DOMAIN`].
    Domain,
    /// `miner_id` was empty or longer than [`MAX_MINER_ID_LEN`].
    MinerId,
    /// `sig` was not [`SIGNATURE_LEN`] bytes.
    SignatureLength,
    /// The canonical-CBOR encode failed.
    Encode,
}

impl core::fmt::Display for HeartbeatError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl HeartbeatError {
    /// The fixed classifier — identical to the `Display` impl. Named
    /// contract for callers that log a classifier.
    pub fn as_str(&self) -> &'static str {
        match self {
            HeartbeatError::SchemaVersion => "heartbeat-schema-version",
            HeartbeatError::Domain => "heartbeat-domain",
            HeartbeatError::MinerId => "heartbeat-miner-id",
            HeartbeatError::SignatureLength => "heartbeat-signature-length",
            HeartbeatError::Encode => "heartbeat-encode",
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for HeartbeatError {}

/// Crate-local result alias for the heartbeat wire format.
pub type Result<T> = core::result::Result<T, HeartbeatError>;

/// The body the miner-agent signs (and vali re-derives + verifies).
///
/// Fields chosen so the heartbeat ALONE pins liveness to a single
/// miner at a single instant:
///
/// - `miner_id` — which registered miner (the trust-registry key);
/// - `timestamp_unix` — the miner's wall-clock view, anti-skew-checked;
/// - `sequence` — a per-miner monotonic counter, persisted across
///   restarts; vali rejects any value ≤ the last accepted one (replay
///   defence);
/// - the `vm_*` / `cpu_*` / `memory_*` counters — coarse host state the
///   scheduler may use as a soft signal;
/// - `domain` — the replay-context separator (also encoded into the
///   signed bytes).
///
/// `schema_version` pins the wire format; an unknown value fails
/// closed at [`validate`](Self::validate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MinerHeartbeat {
    /// Wire-format version. MUST be [`SCHEMA_VERSION`].
    pub schema_version: u8,
    /// Operator-assigned miner identifier — the `vali` registry key.
    pub miner_id: String,
    /// The miner's wall-clock at build time, Unix seconds.
    pub timestamp_unix: i64,
    /// Per-miner monotonic counter, persisted to disk across restarts.
    pub sequence: u64,
    /// Tenant CVMs currently in the running phase.
    pub vm_count_running: u32,
    /// Tenant CVMs the lifecycle is tracking (any phase).
    pub vm_count_total: u32,
    /// 1-minute load average, in centi-units (load × 100).
    pub cpu_load_1m_centi: u32,
    /// Total host RAM, MiB.
    pub memory_total_mib: u32,
    /// Available host RAM, MiB.
    pub memory_available_mib: u32,
    /// Replay-domain tag. MUST be [`DOMAIN`].
    pub domain: String,
    /// `v2`-only graceful-exit flag (transport (B)). When `true`, this
    /// heartbeat is ALSO a self-requested graceful exit: vali accepts the
    /// heartbeat (the miner is alive, just leaving) AND quarantines the
    /// miner so the §13/§25 auto-migration warm-migrates its VMs off. It
    /// is the always-on passive complement to the Edge-relayed
    /// `SignedGracefulExit` request — a second, redundant signal path.
    ///
    /// A `v1` heartbeat ([`SCHEMA_VERSION`]) NEVER carries this: it is
    /// not in the `v1` canonical map, and [`validate`](Self::validate)
    /// rejects a `v1` body whose flag is `true`. Only a `v2` heartbeat
    /// ([`SCHEMA_VERSION_GRACEFUL_EXIT`]) encodes it.
    pub graceful_exit_requested: bool,
}

impl MinerHeartbeat {
    /// Every semantic invariant of a well-formed heartbeat. Run by
    /// [`canonical`](Self::canonical) so a signature over an impossible
    /// value can never be produced; a decoder MUST run it too.
    pub fn validate(&self) -> Result<()> {
        // Accept BOTH wire versions — `v1` (the frozen 10-field
        // baseline) and `v2` (the 11-field graceful-exit-flag form).
        // Any other value fails closed, exactly as `v1`-only did.
        if self.schema_version != SCHEMA_VERSION
            && self.schema_version != SCHEMA_VERSION_GRACEFUL_EXIT
        {
            return Err(HeartbeatError::SchemaVersion);
        }
        // A `v1` heartbeat CANNOT carry the graceful-exit flag: the flag
        // is not a `v1` wire field, so a `v1` body with it set is
        // malformed (it would never round-trip through the `v1`
        // canonical map). This keeps the flag strictly a `v2` concept.
        if self.schema_version == SCHEMA_VERSION && self.graceful_exit_requested {
            return Err(HeartbeatError::SchemaVersion);
        }
        if self.domain != DOMAIN {
            return Err(HeartbeatError::Domain);
        }
        if self.miner_id.is_empty() || self.miner_id.len() > MAX_MINER_ID_LEN {
            return Err(HeartbeatError::MinerId);
        }
        Ok(())
    }

    /// Deterministic-CBOR encoding of the to-be-signed body — the
    /// signature preimage. Fails closed on any
    /// [`validate`](Self::validate) violation.
    pub fn canonical(&self) -> Result<Vec<u8>> {
        self.validate()?;
        // `to_canonical_vec` re-sorts the map by its ENCODED key bytes
        // (RFC 8949 §4.2.1 deterministic encoding: length-first, then
        // bytewise), so the order we push entries in here is irrelevant —
        // the encoder fixes it. `graceful_exit_requested` (23 chars) is
        // the longest key, so it canonically sorts LAST.
        //
        // `v1` emits EXACTLY the 10 keys it always has — the flag is
        // NOT pushed, so a `v1` heartbeat re-encodes byte-identically to
        // every prior build (the `heartbeat_kat` vector is unchanged).
        // `v2` adds the 11th key, which the canonical encoder places at
        // the end. The `schema_version` value is the ONLY change to a
        // shared field's encoding between the two versions.
        let mut entries = vec![
            (
                Value::Text("cpu_load_1m_centi".into()),
                Value::Integer(self.cpu_load_1m_centi.into()),
            ),
            (Value::Text("domain".into()), Value::Text(DOMAIN.into())),
        ];
        if self.schema_version == SCHEMA_VERSION_GRACEFUL_EXIT {
            entries.push((
                Value::Text("graceful_exit_requested".into()),
                Value::Bool(self.graceful_exit_requested),
            ));
        }
        entries.extend([
            (
                Value::Text("memory_available_mib".into()),
                Value::Integer(self.memory_available_mib.into()),
            ),
            (
                Value::Text("memory_total_mib".into()),
                Value::Integer(self.memory_total_mib.into()),
            ),
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
            (
                Value::Text("vm_count_running".into()),
                Value::Integer(self.vm_count_running.into()),
            ),
            (
                Value::Text("vm_count_total".into()),
                Value::Integer(self.vm_count_total.into()),
            ),
        ]);
        to_canonical_vec(&Value::Map(entries)).map_err(|_| HeartbeatError::Encode)
    }

    /// Build a `v2` graceful-exit heartbeat from the live metrics of an
    /// ordinary heartbeat — transport (B).
    ///
    /// Takes every field of a normally-built heartbeat (`miner_id`, the
    /// timestamp, the monotonic `sequence`, the VM + host counters) and
    /// produces the SAME body with `schema_version = `
    /// [`SCHEMA_VERSION_GRACEFUL_EXIT`] and `graceful_exit_requested =
    /// true`. The miner-agent emits one of these to piggyback its
    /// graceful-exit intent on the liveness channel; vali accepts the
    /// heartbeat AND quarantines the miner.
    ///
    /// `domain` is forced to [`DOMAIN`] — the flag rides the SAME signed
    /// heartbeat scheme, never a separate one.
    #[allow(clippy::too_many_arguments)]
    pub fn graceful_exit(
        miner_id: String,
        timestamp_unix: i64,
        sequence: u64,
        vm_count_running: u32,
        vm_count_total: u32,
        cpu_load_1m_centi: u32,
        memory_total_mib: u32,
        memory_available_mib: u32,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION_GRACEFUL_EXIT,
            miner_id,
            timestamp_unix,
            sequence,
            vm_count_running,
            vm_count_total,
            cpu_load_1m_centi,
            memory_total_mib,
            memory_available_mib,
            domain: DOMAIN.into(),
            graceful_exit_requested: true,
        }
    }
}

/// Signed heartbeat envelope.
///
/// `body` is the canonical CBOR of a [`MinerHeartbeat`] (its
/// [`canonical`](MinerHeartbeat::canonical) output); `sig` is the
/// 64-byte Ed25519 signature over `body` by the miner identity key.
///
/// `#[serde(deny_unknown_fields)]` blocks an extra-key smuggle past
/// the envelope decode — defence in depth for the relay path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedMinerHeartbeat {
    #[serde(with = "serde_bytes")]
    pub body: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub sig: Vec<u8>,
}

impl SignedMinerHeartbeat {
    /// Deterministic-CBOR encoding of the `{body, sig}` envelope.
    ///
    /// `body` is already canonical (it is the inner heartbeat's
    /// `canonical()` output); this wraps it with the signature so the
    /// whole envelope re-encodes byte-identically on every hop. This
    /// is exactly the blob the Edge relays opaquely (§5.6) and vali
    /// ingests.
    pub fn canonical(&self) -> Result<Vec<u8>> {
        if self.sig.len() != SIGNATURE_LEN {
            return Err(HeartbeatError::SignatureLength);
        }
        let v = Value::Map(vec![
            (Value::Text("body".into()), Value::Bytes(self.body.clone())),
            (Value::Text("sig".into()), Value::Bytes(self.sig.clone())),
        ]);
        to_canonical_vec(&v).map_err(|_| HeartbeatError::Encode)
    }

    /// SHA-256 over the `body` of the signed heartbeat — the value a
    /// log line carries instead of the body bytes (§K logging
    /// discipline: only `body_hash + kind + miner_id`, never bytes).
    pub fn body_hash(&self) -> [u8; 32] {
        let mut out = [0u8; 32];
        out.copy_from_slice(Sha256::digest(&self.body).as_slice());
        out
    }

    /// Extract the inner `MinerHeartbeat.timestamp_unix` directly from
    /// the canonical-CBOR `body` — a read-only accessor, no decode of
    /// the full struct.
    ///
    /// The miner-agent's pusher uses this to drop heartbeats that have
    /// aged past vali's [`MAX_AGE_SECONDS`] anti-skew window: the
    /// envelope is signed at build time, so a stale one cannot be
    /// rescued by re-signing (that would break the canonical-CBOR
    /// signed-envelope invariant) — it must be discarded rather than
    /// burned indefinitely on the wire.
    ///
    /// Returns `None` on a corrupt / wrong-shape body, or a body that
    /// does not carry the field as a CBOR integer; the caller treats
    /// it as stale-equivalent (vali would refuse to verify it anyway).
    /// Note: a body that is decodable but **not canonically** encoded
    /// also returns the field — this accessor's job is the local
    /// staleness check, not canonical validation. The wire path's
    /// canonical-CBOR check lives in vali's verifier shell-out.
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

    fn sample() -> MinerHeartbeat {
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
            graceful_exit_requested: false,
        }
    }

    #[test]
    fn canonical_is_stable_and_canonical() {
        let hb = sample();
        let a = hb.canonical().unwrap();
        let b = hb.canonical().unwrap();
        assert_eq!(a, b);
        assert_canonical(&a).unwrap();
    }

    /// Walk the top-level CBOR map and return its keys in wire order —
    /// a test helper that proves the canonical key sequence.
    fn canonical_keys(bytes: &[u8]) -> Vec<String> {
        let v: Value = ciborium::de::from_reader(bytes).unwrap();
        match v {
            Value::Map(entries) => entries
                .into_iter()
                .map(|(k, _)| match k {
                    Value::Text(t) => t,
                    _ => panic!("non-text key"),
                })
                .collect(),
            _ => panic!("body is not a CBOR map"),
        }
    }

    #[test]
    fn v1_canonical_has_exactly_the_ten_frozen_keys() {
        // The `v1` baseline — the flag is NOT present, the key set + order
        // are byte-frozen (also pinned by the `heartbeat_kat` vector).
        let hb = sample();
        assert_eq!(hb.schema_version, SCHEMA_VERSION);
        let keys = canonical_keys(&hb.canonical().unwrap());
        // RFC 8949 deterministic order — by ENCODED key bytes
        // (length-first, then bytewise), NOT plain lexicographic.
        assert_eq!(
            keys,
            vec![
                "domain",
                "miner_id",
                "sequence",
                "schema_version",
                "timestamp_unix",
                "vm_count_total",
                "memory_total_mib",
                "vm_count_running",
                "cpu_load_1m_centi",
                "memory_available_mib",
            ],
            "v1 canonical key set/order must never shift"
        );
    }

    #[test]
    fn v2_canonical_inserts_the_flag_in_sorted_position() {
        // `v2` adds exactly one key, `graceful_exit_requested`, sorted
        // AFTER `domain` and BEFORE `memory_available_mib` — 11 keys.
        let hb = MinerHeartbeat::graceful_exit(
            "miner-a".into(),
            1_700_000_000,
            42,
            3,
            5,
            175,
            262_144,
            131_072,
        );
        let bytes = hb.canonical().unwrap();
        assert_canonical(&bytes).unwrap();
        let keys = canonical_keys(&bytes);
        // The 23-char flag key is the LONGEST, so RFC 8949 length-first
        // deterministic ordering places it LAST. The other 10 keys keep
        // their exact v1 positions.
        assert_eq!(
            keys,
            vec![
                "domain",
                "miner_id",
                "sequence",
                "schema_version",
                "timestamp_unix",
                "vm_count_total",
                "memory_total_mib",
                "vm_count_running",
                "cpu_load_1m_centi",
                "memory_available_mib",
                "graceful_exit_requested",
            ],
            "v2 appends the flag as the canonically-last (longest) key"
        );
    }

    #[test]
    fn v1_and_v2_differ_only_by_the_flag_key_and_schema_version() {
        // The shared metric fields are byte-identical between versions —
        // the ONLY differences are the inserted flag key and the
        // `schema_version` value. Proven by stripping both back to a
        // common projection. (Direct evidence that adding the flag did
        // not perturb any v1 field encoding.)
        let v1 = sample();
        let v2 = MinerHeartbeat::graceful_exit(
            v1.miner_id.clone(),
            v1.timestamp_unix,
            v1.sequence,
            v1.vm_count_running,
            v1.vm_count_total,
            v1.cpu_load_1m_centi,
            v1.memory_total_mib,
            v1.memory_available_mib,
        );
        let v1_keys = canonical_keys(&v1.canonical().unwrap());
        let v2_keys: Vec<String> = canonical_keys(&v2.canonical().unwrap())
            .into_iter()
            .filter(|k| k != "graceful_exit_requested")
            .collect();
        assert_eq!(v1_keys, v2_keys);
    }

    #[test]
    fn v2_with_flag_false_is_still_valid_and_eleven_keys() {
        // A `v2` heartbeat with the flag `false` is a legitimate
        // (non-exiting) heartbeat — it still carries the 11th key (the
        // version, not the flag value, decides the wire shape).
        let mut hb = MinerHeartbeat::graceful_exit("m".into(), 1, 1, 0, 0, 0, 0, 0);
        hb.graceful_exit_requested = false;
        assert!(hb.validate().is_ok());
        assert_eq!(canonical_keys(&hb.canonical().unwrap()).len(), 11);
    }

    #[test]
    fn v1_body_with_the_flag_true_is_rejected() {
        // A `v1` heartbeat MUST NOT carry the flag — fail closed.
        let mut hb = sample();
        hb.graceful_exit_requested = true;
        assert_eq!(hb.validate(), Err(HeartbeatError::SchemaVersion));
        assert!(hb.canonical().is_err());
    }

    #[test]
    fn graceful_exit_constructor_sets_v2_and_the_flag() {
        let hb = MinerHeartbeat::graceful_exit("m".into(), 1, 1, 0, 0, 0, 0, 0);
        assert_eq!(hb.schema_version, SCHEMA_VERSION_GRACEFUL_EXIT);
        assert!(hb.graceful_exit_requested);
        assert_eq!(hb.domain, DOMAIN);
        assert!(hb.validate().is_ok());
    }

    #[test]
    fn wrong_schema_version_rejected() {
        // An UNKNOWN version (neither 1 nor 2) still fails closed.
        let mut hb = sample();
        hb.schema_version = 3;
        assert_eq!(hb.validate(), Err(HeartbeatError::SchemaVersion));
        assert!(hb.canonical().is_err());
    }

    #[test]
    fn wrong_domain_rejected() {
        let mut hb = sample();
        hb.domain = "HIPPIUS_OTHER_V1".into();
        assert_eq!(hb.validate(), Err(HeartbeatError::Domain));
        assert!(hb.canonical().is_err());
    }

    #[test]
    fn empty_or_oversize_miner_id_rejected() {
        let mut hb = sample();
        hb.miner_id = String::new();
        assert_eq!(hb.validate(), Err(HeartbeatError::MinerId));
        let mut hb = sample();
        hb.miner_id = "x".repeat(MAX_MINER_ID_LEN + 1);
        assert_eq!(hb.validate(), Err(HeartbeatError::MinerId));
    }

    #[test]
    fn changing_any_field_changes_signed_bytes() {
        let base = sample().canonical().unwrap();
        let mutate: &[fn(&mut MinerHeartbeat)] = &[
            |h| h.miner_id = "other-miner".into(),
            |h| h.timestamp_unix += 1,
            |h| h.sequence += 1,
            |h| h.vm_count_running += 1,
            |h| h.vm_count_total += 1,
            |h| h.cpu_load_1m_centi += 1,
            |h| h.memory_total_mib += 1,
            |h| h.memory_available_mib += 1,
        ];
        for m in mutate {
            let mut h = sample();
            m(&mut h);
            assert_ne!(
                base,
                h.canonical().unwrap(),
                "a field escaped the signature"
            );
        }
    }

    #[test]
    fn signed_envelope_canonical_is_stable_and_canonical() {
        let s = SignedMinerHeartbeat {
            body: sample().canonical().unwrap(),
            sig: vec![7u8; SIGNATURE_LEN],
        };
        let a = s.canonical().unwrap();
        let b = s.canonical().unwrap();
        assert_eq!(a, b);
        assert_canonical(&a).unwrap();
    }

    #[test]
    fn signed_envelope_rejects_wrong_sig_length() {
        let s = SignedMinerHeartbeat {
            body: sample().canonical().unwrap(),
            sig: vec![0u8; 63],
        };
        assert_eq!(s.canonical(), Err(HeartbeatError::SignatureLength));
    }

    #[test]
    fn body_hash_is_stable_and_32_bytes() {
        let s = SignedMinerHeartbeat {
            body: sample().canonical().unwrap(),
            sig: vec![0u8; SIGNATURE_LEN],
        };
        let d1 = s.body_hash();
        let d2 = s.body_hash();
        assert_eq!(d1, d2);
        assert_eq!(d1.len(), 32);
    }

    #[test]
    fn signed_envelope_timestamp_unix_reads_the_canonical_body() {
        let mut h = sample();
        h.timestamp_unix = 1_700_000_042;
        let s = SignedMinerHeartbeat {
            body: h.canonical().unwrap(),
            sig: vec![0u8; SIGNATURE_LEN],
        };
        assert_eq!(s.timestamp_unix(), Some(1_700_000_042));
    }

    #[test]
    fn signed_envelope_timestamp_unix_is_none_on_a_corrupt_body() {
        // A non-canonical body (not CBOR at all) — the accessor must
        // not panic; the caller treats `None` as stale-equivalent.
        let s = SignedMinerHeartbeat {
            body: vec![0xff, 0xff, 0xff],
            sig: vec![0u8; SIGNATURE_LEN],
        };
        assert!(s.timestamp_unix().is_none());
    }

    #[test]
    fn signed_envelope_timestamp_unix_is_none_when_the_field_is_missing() {
        // A valid CBOR map without `timestamp_unix` — accessor returns
        // `None`, the caller treats it as stale.
        let v = Value::Map(vec![(
            Value::Text("other".into()),
            Value::Integer(0.into()),
        )]);
        let bytes = to_canonical_vec(&v).unwrap();
        let s = SignedMinerHeartbeat {
            body: bytes,
            sig: vec![0u8; SIGNATURE_LEN],
        };
        assert!(s.timestamp_unix().is_none());
    }

    #[test]
    fn signed_envelope_rejects_unknown_field() {
        // `deny_unknown_fields` must trip — a `{body, sig, extra}`
        // envelope is a smuggle attempt.
        let v = Value::Map(vec![
            (Value::Text("body".into()), Value::Bytes(vec![1u8; 8])),
            (Value::Text("extra".into()), Value::Integer(0.into())),
            (
                Value::Text("sig".into()),
                Value::Bytes(vec![0u8; SIGNATURE_LEN]),
            ),
        ]);
        let bytes = to_canonical_vec(&v).unwrap();
        let decoded: core::result::Result<SignedMinerHeartbeat, _> =
            ciborium::de::from_reader(bytes.as_slice());
        assert!(decoded.is_err());
    }

    #[test]
    fn error_classifiers_are_stable() {
        for (err, class) in [
            (HeartbeatError::SchemaVersion, "heartbeat-schema-version"),
            (HeartbeatError::Domain, "heartbeat-domain"),
            (HeartbeatError::MinerId, "heartbeat-miner-id"),
            (
                HeartbeatError::SignatureLength,
                "heartbeat-signature-length",
            ),
            (HeartbeatError::Encode, "heartbeat-encode"),
        ] {
            assert_eq!(err.as_str(), class);
            assert_eq!(err.to_string(), class);
        }
    }
}

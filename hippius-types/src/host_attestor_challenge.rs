//! Blackbox host-attestor **nonce-challenge** wire (blackbox host-attestor
//! chantier PR-10).
//!
//! The enrollment `REPORT_DATA[0..32]` MUST be a **vali-chosen, single-use,
//! freshness-bounded nonce** — never a guest-generated value — so a guest
//! cannot pre-generate an enrollment SNP report while attested and replay
//! it later from a paused / migrated / rehosted VM. This module is the
//! wire the fresh nonce travels on.
//!
//! ## Direction — a guest-initiated *pull* (not a host→guest reverse-dial)
//!
//! The attestor already dials the host over `AF_VSOCK` (the enrollment /
//! beacon pusher connects to `VMADDR_CID_HOST`). PR-10 reuses that shape:
//! the guest **opens** a connection to the miner-agent on
//! [`CHALLENGE_PORT`] and the response rides back on the SAME socket. The
//! guest therefore needs no CID of its own for the reply — exactly the
//! [`crate::kbs_vsock`] request/response pattern.
//!
//! ```text
//!  guest --HostChallengeRequest{pk}-->  miner-agent
//!  miner-agent --POST /v1/edge/host-attestor-challenge--> Edge --> vali
//!  vali: node_id from the Edge peer-id, pk from the request,
//!        mint CSPRNG 32-byte nonce bound to {node_id, pk}, TTL, unspent
//!  vali --nonce--> Edge --> miner-agent --HostChallengeResponse{nonce}--> guest
//!  guest: REPORT_DATA[0..32] = nonce  ->  enrol
//! ```
//!
//! ## Wire
//!
//! One request → one response per connection, each a **canonical-CBOR**
//! body with a `u32` big-endian length prefix (the same framing as
//! [`crate::ticket_vsock`] / [`crate::kbs_vsock`]). Canonical CBOR is
//! load-bearing: the request crosses the Edge §10 wire gate, which runs
//! `assert_canonical` before its typed decode.
//!
//! - guest → host: [`HostChallengeRequest`] `{schema_version, signer_pubkey}`
//! - host → guest: [`HostChallengeResponse`] `{schema_version, nonce,
//!   expiry_unix}`
//!
//! ## Ships inert
//!
//! Nothing arms this yet — this is the frozen wire contract + the port /
//! size constants both ends agree on. The channel is wired in this PR but
//! stays behind default-off flags across vali / miner / agent.

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
use crate::host_attestor::DIGEST_LEN;
/// Ed25519 public-key length — re-exported so both ends name the same
/// constant without reaching into [`crate::host_attestor`].
pub use crate::host_attestor::PUBKEY_LEN;
use crate::{HippiusTypesError, Result};
use ciborium::value::Value;

/// AF_VSOCK port the miner-agent binds for the host-attestor nonce
/// challenge. Distinct from [`crate::ticket_vsock::PORT`] (`0x4849`),
/// [`crate::kbs_vsock::PORT`] (`0x4B42`), and the guest→host relay port
/// (`5000`). `0x4843` spells "HC" (Host Challenge).
pub const CHALLENGE_PORT: u32 = 0x4843;

/// AF_VSOCK port the miner-agent binds for the host-attestor enrollment
/// and liveness-beacon UP relay (blackbox host-attestor chantier
/// PR-10b-S2a). The attestor guest's fire-and-forget pusher
/// (`agent-host-attestor::vsock_pusher`) dials the host on this port and
/// streams `GuestFrame{kind: "host-enroll" | "host-beacon", body}` frames;
/// the miner-agent's `host_relay` listener relays each UP to the Edge.
///
/// Distinct from every sibling vsock channel — the tenant relay
/// (`5000`), the nonce challenge ([`CHALLENGE_PORT`] = `0x4843`), the
/// ticket ([`crate::ticket_vsock::PORT`] = `0x4849`), and the KBS proxy
/// ([`crate::kbs_vsock::PORT`] = `0x4B42`). `0x4841` spells "HA" (Host
/// Attestor).
pub const ENROLL_BEACON_PORT: u32 = 0x4841;

/// The well-known host context-id the guest dials (`VMADDR_CID_HOST`).
pub const HOST_CID: u32 = 2;

/// Freshness-nonce length — the vali-minted single-use nonce that becomes
/// `REPORT_DATA[0..32]`. Matches [`crate::host_attestor::DIGEST_LEN`].
pub const NONCE_LEN: usize = DIGEST_LEN;

/// The only `schema_version` this build understands.
pub const CHALLENGE_SCHEMA_VERSION: u16 = 1;

/// Hard cap on one framed challenge request/response — both are tiny
/// (a 32-byte key / nonce + a couple of integers). Bounds a hostile
/// length before any allocation.
pub const MAX_CHALLENGE_FRAME_BYTES: usize = 4 * 1024;

/// Connect/exchange timeout for one challenge round-trip, seconds.
pub const CHALLENGE_TIMEOUT_SECS: u64 = 20;

/// Guest → host: "mint me a fresh enrollment nonce bound to this signer
/// public key." The `node_id` is NOT carried in the body — vali stamps it
/// from the Edge mTLS peer identity (never a body-declared node).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostChallengeRequest {
    /// Wire-format version. MUST be [`CHALLENGE_SCHEMA_VERSION`].
    pub schema_version: u16,
    /// The attestor's per-boot Ed25519 signer public key — the value the
    /// minted nonce is bound to (and later folded into the enrollment
    /// `REPORT_DATA` alongside the nonce).
    pub signer_pubkey: [u8; PUBKEY_LEN],
}

impl HostChallengeRequest {
    /// Build a request for `signer_pubkey`.
    pub fn new(signer_pubkey: [u8; PUBKEY_LEN]) -> Self {
        Self {
            schema_version: CHALLENGE_SCHEMA_VERSION,
            signer_pubkey,
        }
    }

    /// Semantic invariants. Run by BOTH [`canonical`](Self::canonical) and
    /// [`decode`](Self::decode).
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != CHALLENGE_SCHEMA_VERSION {
            return Err(schema(format!(
                "unknown schema_version {} (want {CHALLENGE_SCHEMA_VERSION})",
                self.schema_version
            )));
        }
        Ok(())
    }

    /// Deterministic-CBOR encoding. Fails closed on any
    /// [`validate`](Self::validate) violation. Field ordering: alphabetic
    /// on text keys (RFC 8949 §4.2.1) — canonical so the Edge wire gate's
    /// `assert_canonical` accepts it.
    pub fn canonical(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let v = Value::Map(vec![
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

    /// Decode + validate a canonical-CBOR request. Hostile-origin parser.
    /// Rejects: non-canonical encoding (incl. trailing bytes), non-text /
    /// duplicated map key, unknown field, unknown `schema_version`,
    /// wrong-length `signer_pubkey`.
    pub fn decode(body: &[u8]) -> Result<HostChallengeRequest> {
        assert_canonical(body).map_err(|e| schema(format!("body: {e}")))?;
        let value: Value =
            ciborium::de::from_reader(body).map_err(|e| schema(format!("decode: {e}")))?;
        let mut map = into_string_map(value)?;

        let schema_version = take_u16(&mut map, "schema_version")?;
        if schema_version != CHALLENGE_SCHEMA_VERSION {
            return Err(schema(format!(
                "unknown schema_version {schema_version} (want {CHALLENGE_SCHEMA_VERSION})"
            )));
        }
        let signer_pubkey = take_byte_array::<PUBKEY_LEN>(&mut map, "signer_pubkey")?;
        reject_leftover(&map, "host challenge request")?;

        let req = HostChallengeRequest {
            schema_version,
            signer_pubkey,
        };
        req.validate()?;
        Ok(req)
    }
}

/// Host → guest: the vali-minted fresh nonce for this enrollment, plus
/// the guest's `node_id`.
///
/// ## `node_id` over the response (PR-10b-S2a wire amendment)
///
/// The attestor **cannot** learn its `node_id` from the measured kernel
/// cmdline: a per-node `node_id` on the cmdline would produce a per-node
/// SNP launch measurement, breaking the fleet-wide operator-pinned
/// host-attestor measurement (must-have #1). vali therefore returns the
/// `node_id` it stamped from the miner's authenticated Edge mTLS peer
/// identity in THIS response — the same `node_id` it bound the minted
/// nonce to (`{node_id, signer_pubkey}`). The guest folds it into both
/// [`HostEnrollment::node_id`](crate::host_attestor::HostEnrollment) and
/// the enrollment `REPORT_DATA[32..64]` binding.
///
/// Security: a lying miner can only supply ITS OWN peer-stamped
/// `node_id` (it cannot forge another miner's mTLS identity), and the
/// minted nonce is bound to that `{node_id, pk}` — so a mismatched
/// `REPORT_DATA` at cert-ingest is rejected. A miner can only break its
/// own attestation, never claim another host's identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostChallengeResponse {
    /// Wire-format version. MUST be [`CHALLENGE_SCHEMA_VERSION`].
    pub schema_version: u16,
    /// The vali-chosen single-use nonce — becomes `REPORT_DATA[0..32]`.
    pub nonce: [u8; NONCE_LEN],
    /// The host `node_id` vali stamped from the miner's mTLS peer identity
    /// (never a body-declared value). The nonce is bound to it; the guest
    /// uses it as its enrollment `node_id`. MUST be non-empty.
    pub node_id: String,
    /// Hard expiry, Unix seconds. Informational to the guest (it should
    /// enrol promptly); vali is the authority that rejects a spent /
    /// expired nonce at cert-ingest. MUST be non-zero.
    pub expiry_unix: u64,
}

impl HostChallengeResponse {
    /// Build a response for `nonce` bound to `node_id`, expiring at
    /// `expiry_unix`.
    pub fn new(nonce: [u8; NONCE_LEN], node_id: String, expiry_unix: u64) -> Self {
        Self {
            schema_version: CHALLENGE_SCHEMA_VERSION,
            nonce,
            node_id,
            expiry_unix,
        }
    }

    /// Semantic invariants. Run by BOTH [`canonical`](Self::canonical) and
    /// [`decode`](Self::decode).
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != CHALLENGE_SCHEMA_VERSION {
            return Err(schema(format!(
                "unknown schema_version {} (want {CHALLENGE_SCHEMA_VERSION})",
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

    /// Deterministic-CBOR encoding. Fails closed on any
    /// [`validate`](Self::validate) violation. Field ordering: alphabetic
    /// on text keys (RFC 8949 §4.2.1).
    pub fn canonical(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let v = Value::Map(vec![
            (
                Value::Text("expiry_unix".into()),
                Value::Integer(self.expiry_unix.into()),
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
        ]);
        to_canonical_vec(&v).map_err(|e| schema(format!("encode: {e}")))
    }

    /// Decode + validate a canonical-CBOR response. Hostile-origin parser.
    pub fn decode(body: &[u8]) -> Result<HostChallengeResponse> {
        assert_canonical(body).map_err(|e| schema(format!("body: {e}")))?;
        let value: Value =
            ciborium::de::from_reader(body).map_err(|e| schema(format!("decode: {e}")))?;
        let mut map = into_string_map(value)?;

        let schema_version = take_u16(&mut map, "schema_version")?;
        if schema_version != CHALLENGE_SCHEMA_VERSION {
            return Err(schema(format!(
                "unknown schema_version {schema_version} (want {CHALLENGE_SCHEMA_VERSION})"
            )));
        }
        let nonce = take_byte_array::<NONCE_LEN>(&mut map, "nonce")?;
        let node_id = take_text(&mut map, "node_id")?;
        let expiry_unix = take_u64(&mut map, "expiry_unix")?;
        reject_leftover(&map, "host challenge response")?;

        let resp = HostChallengeResponse {
            schema_version,
            nonce,
            node_id,
            expiry_unix,
        };
        resp.validate()?;
        Ok(resp)
    }
}

// ── decode helpers (module-local, mirroring `host_attestor.rs`) ─────
//
// Reuse the `HostAttestorSchema` error variant (via `schema`) so no new
// `HippiusTypesError` variant is introduced — the `kbs-core`
// `From<HippiusTypesError>` exhaustive match stays untouched.

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

fn take_bytes(map: &mut BTreeMap<String, Value>, key: &str) -> Result<Vec<u8>> {
    match take(map, key)? {
        Value::Bytes(b) => Ok(b),
        _ => Err(schema(format!("field {key:?} is not a byte string"))),
    }
}

fn take_text(map: &mut BTreeMap<String, Value>, key: &str) -> Result<String> {
    match take(map, key)? {
        Value::Text(s) => Ok(s),
        _ => Err(schema(format!("field {key:?} is not a text string"))),
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

    fn sample_request() -> HostChallengeRequest {
        HostChallengeRequest::new([0x22; PUBKEY_LEN])
    }

    fn sample_response() -> HostChallengeResponse {
        HostChallengeResponse::new([0x11; NONCE_LEN], "node-host-1".into(), 1_800_000_900)
    }

    #[test]
    fn ports_are_distinct_from_sibling_vsock_channels() {
        assert_ne!(CHALLENGE_PORT, crate::ticket_vsock::PORT);
        assert_ne!(CHALLENGE_PORT, crate::kbs_vsock::PORT);
        // The guest→host relay / pusher port.
        assert_ne!(CHALLENGE_PORT, 5000);
        // The host-attestor enroll/beacon UP-relay port (PR-10b-S2a) is
        // distinct from the challenge port AND every sibling channel.
        assert_ne!(ENROLL_BEACON_PORT, CHALLENGE_PORT);
        assert_ne!(ENROLL_BEACON_PORT, crate::ticket_vsock::PORT);
        assert_ne!(ENROLL_BEACON_PORT, crate::kbs_vsock::PORT);
        assert_ne!(ENROLL_BEACON_PORT, 5000);
    }

    #[test]
    fn response_carries_the_node_id_round_trip() {
        // The PR-10b-S2a wire amendment: node_id rides the challenge
        // response so the guest never needs it on the measured cmdline.
        let a = HostChallengeResponse::new([0x11; NONCE_LEN], "node-host-42".into(), 1_800_000_900);
        let body = a.canonical().unwrap();
        let decoded = HostChallengeResponse::decode(&body).unwrap();
        assert_eq!(decoded.node_id, "node-host-42");
        assert_eq!(a, decoded);
        assert_eq!(body, decoded.canonical().unwrap());
    }

    #[test]
    fn response_rejects_empty_node_id() {
        let mut a = sample_response();
        a.node_id = String::new();
        assert!(a.canonical().is_err());
    }

    #[test]
    fn response_decode_rejects_missing_node_id() {
        // A pre-amendment response (no node_id) must fail closed under the
        // new schema — the guest can't enrol without its node_id.
        let entries = vec![
            (
                Value::Text("expiry_unix".into()),
                Value::Integer(1_800_000_900.into()),
            ),
            (
                Value::Text("nonce".into()),
                Value::Bytes(vec![0x11; NONCE_LEN]),
            ),
            (
                Value::Text("schema_version".into()),
                Value::Integer(1.into()),
            ),
        ];
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(HostChallengeResponse::decode(&body).is_err());
    }

    #[test]
    fn request_round_trips_through_decode() {
        let a = sample_request();
        let body = a.canonical().unwrap();
        let decoded = HostChallengeRequest::decode(&body).unwrap();
        assert_eq!(a, decoded);
        assert_eq!(body, decoded.canonical().unwrap());
    }

    #[test]
    fn request_canonical_is_canonical() {
        let body = sample_request().canonical().unwrap();
        assert_canonical(&body).unwrap();
    }

    #[test]
    fn request_decode_rejects_unknown_field() {
        let a = sample_request();
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(a.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        entries.push((Value::Text("rogue".into()), Value::Integer(1.into())));
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(HostChallengeRequest::decode(&body).is_err());
    }

    #[test]
    fn request_decode_rejects_short_pubkey() {
        let a = sample_request();
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
        assert!(HostChallengeRequest::decode(&body).is_err());
    }

    #[test]
    fn request_decode_rejects_trailing_bytes() {
        let mut body = sample_request().canonical().unwrap();
        body.push(0x00);
        assert!(HostChallengeRequest::decode(&body).is_err());
    }

    #[test]
    fn request_decode_rejects_unknown_schema_version() {
        let entries = vec![
            (
                Value::Text("schema_version".into()),
                Value::Integer(99.into()),
            ),
            (
                Value::Text("signer_pubkey".into()),
                Value::Bytes(vec![0u8; PUBKEY_LEN]),
            ),
        ];
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(HostChallengeRequest::decode(&body).is_err());
    }

    #[test]
    fn response_round_trips_through_decode() {
        let a = sample_response();
        let body = a.canonical().unwrap();
        let decoded = HostChallengeResponse::decode(&body).unwrap();
        assert_eq!(a, decoded);
        assert_eq!(body, decoded.canonical().unwrap());
    }

    #[test]
    fn response_canonical_is_canonical() {
        let body = sample_response().canonical().unwrap();
        assert_canonical(&body).unwrap();
    }

    #[test]
    fn response_rejects_zero_expiry_at_encode() {
        let mut a = sample_response();
        a.expiry_unix = 0;
        assert!(a.canonical().is_err());
    }

    #[test]
    fn response_decode_rejects_unknown_field() {
        let a = sample_response();
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(a.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        entries.push((Value::Text("rogue".into()), Value::Integer(1.into())));
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(HostChallengeResponse::decode(&body).is_err());
    }

    #[test]
    fn response_decode_rejects_short_nonce() {
        let a = sample_response();
        let mut entries =
            match ciborium::de::from_reader::<Value, _>(a.canonical().unwrap().as_slice()).unwrap()
            {
                Value::Map(e) => e,
                _ => unreachable!(),
            };
        for (k, v) in &mut entries {
            if matches!(k, Value::Text(t) if t == "nonce") {
                *v = Value::Bytes(vec![0u8; 31]);
            }
        }
        let body = to_canonical_vec(&Value::Map(entries)).unwrap();
        assert!(HostChallengeResponse::decode(&body).is_err());
    }
}

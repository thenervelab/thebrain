//! Wire protocol for the SNP-attestation-bound Vault broker (#102).
//!
//! The KBS (a kata-snp confidential guest) authenticates to the
//! in-cluster broker before reading a LUKS KEK from Vault. Two
//! round-trips, both canonical-CBOR over TLS:
//!
//! 1. **challenge** — the KBS POSTs the per-VM [`BrokerScope`] it wants
//!    a capability for; the broker returns a fresh single-use
//!    [`ChallengeResponse`] (32-byte CSPRNG nonce + short expiry).
//! 2. **redeem** — the KBS produces a FRESH SNP self-report whose
//!    `REPORT_DATA = challenge_nonce(32) ‖ auth_pubkey(32)`, and POSTs
//!    [`RedeemRequest`] (the raw report + the scope + the nonce +
//!    the auth pubkey). The broker verifies the report against the AMD
//!    chain, checks `report_data == nonce ‖ auth_pubkey`, checks the
//!    KBS measurement ∈ its allowlist + TCB/policy, then mints a
//!    per-VM-scoped short-TTL Vault token and returns it in
//!    [`RedeemResponse`].
//!
//! ## Why no signature on these messages
//!
//! Neither message is Ed25519-signed. Authenticity comes from the
//! layers underneath: the TLS channel protects the bytes in transit,
//! and the [`RedeemRequest`]'s `snp_report` is self-authenticating
//! (AMD-rooted) — the broker re-verifies it. The `challenge_nonce`
//! gives freshness/anti-replay. So these are plain canonical-CBOR
//! request/response shapes with a fail-closed decoder, NOT signed
//! envelopes like `evidence_bundle` / `release`.
//!
//! ## §20 secret discipline
//!
//! The minted Vault token in [`RedeemResponse`] IS a secret. The struct
//! does **not** derive `Debug`; a manual redacting `Debug` prints the
//! token length only. Callers wrap the token in `Zeroizing` the moment
//! they decode it — this module stays `no_std` + crypto-free and only
//! owns the wire shape.

#[allow(unused_imports)]
use alloc::{
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};
use core::fmt;

use crate::cbor::{assert_canonical, to_canonical_vec};
use crate::{HippiusTypesError, Result};
use ciborium::value::Value;

/// 32-byte challenge nonce / X25519-or-Ed25519 auth pubkey length.
pub const NONCE_LEN: usize = 32;
/// KBS auth pubkey length (the key the capability is bound to).
pub const AUTH_PUBKEY_LEN: usize = 32;
/// Minimum SEV-SNP report length (full report is 1184 B).
pub const MIN_SNP_REPORT_LEN: usize = 1184;
/// Hard cap so a hostile body can't claim a multi-MB report.
pub const MAX_SNP_REPORT_LEN: usize = 4096;
/// Cap on the report-carried VEK leaf certificate (DER). A VCEK/VLEK
/// X.509 cert is ~1.3 KB; 8 KiB is generous. May be empty (the broker
/// then falls back to its operator-mounted VEK).
pub const MAX_VEK_DER_LEN: usize = 8192;
/// Cap on a minted Vault token (Vault tokens are < 200 B; generous).
pub const MAX_VAULT_TOKEN_LEN: usize = 1024;
/// Cap on any single canonical body decoded from a hostile peer.
pub const MAX_BODY_LEN: usize = 16 * 1024;

fn schema(msg: String) -> HippiusTypesError {
    HippiusTypesError::VaultBrokerSchema(msg)
}

/// The per-VM Vault scope a capability authorizes — the exact
/// `path@version` pair (§19). Mirrors `kbs_core::vault::VaultScope` but
/// as a crypto-free wire type so both the broker and the KBS client
/// depend only on `hippius-types`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrokerScope {
    pub vm_id: String,
    pub luks_path: String,
    pub luks_version: u64,
    pub userdata_path: String,
    pub userdata_version: u64,
    /// §7 per-VM guest lifecycle SIGNING key (Ed25519 seed). OPTIONAL:
    /// `None` for pre-§7 VMs whose launch never staged a lifecycle key
    /// — the scope then carries exactly the two legacy paths and the
    /// broker mints a two-path token, wire-identical to the pre-§7
    /// shape (the `lifecycle_*` map keys are simply absent). `Some`
    /// adds a THIRD read-only path to the minted capability so the KBS
    /// can read the private key from Vault and HPKE-wrap it to the
    /// attested guest. The version is pinned (§19 exact `path@version`),
    /// just like luks/userdata. Both fields move together — `validate`
    /// rejects a half-set pair.
    pub lifecycle_path: Option<String>,
    pub lifecycle_version: Option<u64>,
}

/// The Vault KV segment name each per-VM secret path MUST end with,
/// mirroring `vali launch.py` (`{prefix}/{vm_id}/<segment>`) and
/// `kbs-core release.rs` (`LUKS_KEK_SEGMENT` / `LIFECYCLE_KEY_SEGMENT`).
const LUKS_SEGMENT: &str = "luks-kek";
const USERDATA_SEGMENT: &str = "userdata";
const LIFECYCLE_SEGMENT: &str = "lifecycle-key";

/// Validate a per-VM Vault secret path carried in a [`BrokerScope`].
///
/// The path + `vm_id` select WHICH Vault secret the minted capability token
/// may read (the KBS derives the release paths from the ticket, and the
/// `vm_id` becomes the token's `entity_alias` that the fixed templated cap
/// policy resolves — KEK-HSM Phase 3). So it MUST be (a) free of metachars
/// (`"` `{` `}`, whitespace, control bytes) — historically to stop HCL
/// injection when the broker still wrote per-VM ACLs; still enforced because
/// `vm_id` flows into a Vault ACL-template alias name; and (b) bound to THIS
/// `vm_id` + the exact secret segment — else a compromised KBS or a
/// maliciously-signed ticket could scope the broker at ANOTHER tenant's KEK.
/// (Audit H6; RA-KBS-M2.)
fn validate_scope_path(path: &str, field: &str, vm_id: &str, segment: &str) -> Result<()> {
    let p = path.trim_start_matches('/');
    if p.is_empty() {
        return Err(schema(format!("scope.{field} must be non-empty")));
    }
    // Charset: ASCII alphanumerics + `. _ - /` ONLY. Rejects `"` `{` `}`,
    // whitespace, and every control byte → no string-interpolation escape
    // (the vm_id also flows into a Vault ACL-template alias name).
    if !p
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
    {
        return Err(schema(format!("scope.{field} has an illegal character")));
    }
    // No empty / `.` / `..` segment → no `//`, no path traversal.
    if p.split('/')
        .any(|seg| seg.is_empty() || seg == "." || seg == "..")
    {
        return Err(schema(format!(
            "scope.{field} has an empty or traversal path segment"
        )));
    }
    // The last three segments MUST be `tenants/<vm_id>/<segment>` — binds the
    // minted token to THIS tenant namespace + vm + the exact key, so a ticket
    // can never scope the broker at another tenant's or a different secret.
    let mut it = p.rsplit('/');
    let last = it.next().unwrap_or("");
    let mid = it.next().unwrap_or("");
    let pre = it.next().unwrap_or("");
    if last != segment || mid != vm_id || pre != "tenants" {
        return Err(schema(format!(
            "scope.{field} must end in tenants/<vm_id>/{segment}"
        )));
    }
    Ok(())
}

impl BrokerScope {
    pub fn validate(&self) -> Result<()> {
        // `vm_id` is interpolated into the policy NAME + every path suffix
        // check — charset-lock it to the same set vali mints (`[a-z0-9-]`,
        // 1..=64) so it can never carry a path separator or HCL metachar.
        if self.vm_id.is_empty() || self.vm_id.len() > 64 {
            return Err(schema("scope.vm_id must be 1..=64 chars".into()));
        }
        if !self
            .vm_id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(schema("scope.vm_id must match [a-z0-9-]".into()));
        }
        validate_scope_path(&self.luks_path, "luks_path", &self.vm_id, LUKS_SEGMENT)?;
        validate_scope_path(
            &self.userdata_path,
            "userdata_path",
            &self.vm_id,
            USERDATA_SEGMENT,
        )?;
        // The §7 lifecycle path + version are all-or-nothing: a path with
        // no version (or vice-versa) is a malformed scope. An empty path
        // string with `Some(_)` is likewise rejected.
        match (&self.lifecycle_path, self.lifecycle_version) {
            (Some(p), Some(_)) if !p.is_empty() => {
                validate_scope_path(p, "lifecycle_path", &self.vm_id, LIFECYCLE_SEGMENT)?;
            }
            (None, None) => {}
            _ => {
                return Err(schema(
                    "scope.lifecycle_path and scope.lifecycle_version must both be set (non-empty path) or both absent".into(),
                ))
            }
        }
        Ok(())
    }

    fn to_value(&self) -> Value {
        // Two legacy keys are ALWAYS present; the two §7 keys are emitted
        // ONLY when set, so a pre-§7 scope canonicalises byte-identically
        // to before (no wire break for a deployed broker).
        let mut entries = vec![
            (
                Value::Text("luks_path".into()),
                Value::Text(self.luks_path.clone()),
            ),
            (
                Value::Text("luks_version".into()),
                Value::Integer(self.luks_version.into()),
            ),
            (
                Value::Text("userdata_path".into()),
                Value::Text(self.userdata_path.clone()),
            ),
            (
                Value::Text("userdata_version".into()),
                Value::Integer(self.userdata_version.into()),
            ),
            (Value::Text("vm_id".into()), Value::Text(self.vm_id.clone())),
        ];
        if let (Some(p), Some(v)) = (&self.lifecycle_path, self.lifecycle_version) {
            entries.push((Value::Text("lifecycle_path".into()), Value::Text(p.clone())));
            entries.push((
                Value::Text("lifecycle_version".into()),
                Value::Integer(v.into()),
            ));
        }
        // `to_canonical_vec` re-sorts the map keys, so push-order here is
        // irrelevant to the on-wire bytes.
        Value::Map(entries)
    }

    fn from_map(m: &Map) -> Result<Self> {
        // The §7 keys are optional — absent ⇒ `None` (pre-§7 scope).
        let lifecycle_path = match m.get("lifecycle_path") {
            Ok(_) => Some(m.text("lifecycle_path")?),
            Err(_) => None,
        };
        let lifecycle_version = match m.get("lifecycle_version") {
            Ok(_) => Some(m.uint("lifecycle_version")?),
            Err(_) => None,
        };
        let s = Self {
            vm_id: m.text("vm_id")?,
            luks_path: m.text("luks_path")?,
            luks_version: m.uint("luks_version")?,
            userdata_path: m.text("userdata_path")?,
            userdata_version: m.uint("userdata_version")?,
            lifecycle_path,
            lifecycle_version,
        };
        s.validate()?;
        Ok(s)
    }
}

/// KBS → broker: "issue me a challenge for this scope".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChallengeRequest {
    pub scope: BrokerScope,
}

impl ChallengeRequest {
    pub fn canonical(&self) -> Result<Vec<u8>> {
        self.scope.validate()?;
        let v = Value::Map(vec![(Value::Text("scope".into()), self.scope.to_value())]);
        encode_capped(&v)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let m = decode_top(bytes, &["scope"])?;
        Ok(Self {
            scope: BrokerScope::from_map(&m.map("scope")?)?,
        })
    }
}

/// broker → KBS: a fresh single-use challenge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChallengeResponse {
    pub nonce: [u8; NONCE_LEN],
    pub expiry_unix: u64,
}

impl ChallengeResponse {
    pub fn canonical(&self) -> Result<Vec<u8>> {
        if self.expiry_unix == 0 {
            return Err(schema("challenge expiry_unix must be non-zero".into()));
        }
        let v = Value::Map(vec![
            (
                Value::Text("expiry_unix".into()),
                Value::Integer(self.expiry_unix.into()),
            ),
            (
                Value::Text("nonce".into()),
                Value::Bytes(self.nonce.to_vec()),
            ),
        ]);
        encode_capped(&v)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let m = decode_top(bytes, &["expiry_unix", "nonce"])?;
        let expiry_unix = m.uint("expiry_unix")?;
        if expiry_unix == 0 {
            return Err(schema("challenge expiry_unix must be non-zero".into()));
        }
        Ok(Self {
            nonce: m.byte_array::<NONCE_LEN>("nonce")?,
            expiry_unix,
        })
    }
}

/// KBS → broker: the SNP self-report + scope + the challenge nonce +
/// the auth pubkey the capability is bound to. The broker verifies the
/// report's `REPORT_DATA == challenge_nonce ‖ auth_pubkey`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedeemRequest {
    pub scope: BrokerScope,
    pub challenge_nonce: [u8; NONCE_LEN],
    pub auth_pubkey: [u8; AUTH_PUBKEY_LEN],
    pub snp_report: Vec<u8>,
    /// The report-carried VEK leaf certificate (DER) — the VCEK/VLEK
    /// the host PSP returns in the SNP extended-report cert table
    /// (§17 / #394). The broker prefers this over its mounted VEK so
    /// the chain always matches the report's current TCB (no static
    /// per-host staging that goes stale on TCB rotation). MAY be empty
    /// when the producer cannot read an extended report — the broker
    /// then falls back to its operator-mounted VEK.
    pub vek_der: Vec<u8>,
}

impl RedeemRequest {
    pub fn validate(&self) -> Result<()> {
        self.scope.validate()?;
        if self.snp_report.len() < MIN_SNP_REPORT_LEN {
            return Err(schema(format!(
                "snp_report must be >= {MIN_SNP_REPORT_LEN} bytes, got {}",
                self.snp_report.len()
            )));
        }
        if self.snp_report.len() > MAX_SNP_REPORT_LEN {
            return Err(schema(format!(
                "snp_report must be <= {MAX_SNP_REPORT_LEN} bytes, got {}",
                self.snp_report.len()
            )));
        }
        if self.vek_der.len() > MAX_VEK_DER_LEN {
            return Err(schema(format!(
                "vek_der must be <= {MAX_VEK_DER_LEN} bytes, got {}",
                self.vek_der.len()
            )));
        }
        Ok(())
    }

    pub fn canonical(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let v = Value::Map(vec![
            (
                Value::Text("auth_pubkey".into()),
                Value::Bytes(self.auth_pubkey.to_vec()),
            ),
            (
                Value::Text("challenge_nonce".into()),
                Value::Bytes(self.challenge_nonce.to_vec()),
            ),
            (Value::Text("scope".into()), self.scope.to_value()),
            (
                Value::Text("snp_report".into()),
                Value::Bytes(self.snp_report.clone()),
            ),
            (
                Value::Text("vek_der".into()),
                Value::Bytes(self.vek_der.clone()),
            ),
        ]);
        encode_capped(&v)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let m = decode_top(
            bytes,
            &[
                "auth_pubkey",
                "challenge_nonce",
                "scope",
                "snp_report",
                "vek_der",
            ],
        )?;
        let r = Self {
            scope: BrokerScope::from_map(&m.map("scope")?)?,
            challenge_nonce: m.byte_array::<NONCE_LEN>("challenge_nonce")?,
            auth_pubkey: m.byte_array::<AUTH_PUBKEY_LEN>("auth_pubkey")?,
            snp_report: m.bytes("snp_report")?,
            vek_der: m.bytes("vek_der")?,
        };
        r.validate()?;
        Ok(r)
    }
}

/// broker → KBS: the minted per-VM-scoped short-TTL Vault token.
///
/// `vault_token` is a SECRET. No `Debug` derive — a manual redacting
/// `Debug` prints the length only (§20). Callers `Zeroizing`-wrap the
/// token immediately on decode.
#[derive(Clone, PartialEq, Eq)]
pub struct RedeemResponse {
    pub scope: BrokerScope,
    pub cap_expiry_unix: u64,
    pub vault_token: Vec<u8>,
}

impl fmt::Debug for RedeemResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RedeemResponse")
            .field("scope", &self.scope)
            .field("cap_expiry_unix", &self.cap_expiry_unix)
            .field("vault_token_len", &self.vault_token.len())
            .finish()
    }
}

impl RedeemResponse {
    pub fn validate(&self) -> Result<()> {
        self.scope.validate()?;
        if self.cap_expiry_unix == 0 {
            return Err(schema("cap_expiry_unix must be non-zero".into()));
        }
        if self.vault_token.is_empty() {
            return Err(schema("vault_token must be non-empty".into()));
        }
        if self.vault_token.len() > MAX_VAULT_TOKEN_LEN {
            return Err(schema(format!(
                "vault_token must be <= {MAX_VAULT_TOKEN_LEN} bytes, got {}",
                self.vault_token.len()
            )));
        }
        Ok(())
    }

    pub fn canonical(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let v = Value::Map(vec![
            (
                Value::Text("cap_expiry_unix".into()),
                Value::Integer(self.cap_expiry_unix.into()),
            ),
            (Value::Text("scope".into()), self.scope.to_value()),
            (
                Value::Text("vault_token".into()),
                Value::Bytes(self.vault_token.clone()),
            ),
        ]);
        encode_capped(&v)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        let m = decode_top(bytes, &["cap_expiry_unix", "scope", "vault_token"])?;
        let r = Self {
            scope: BrokerScope::from_map(&m.map("scope")?)?,
            cap_expiry_unix: m.uint("cap_expiry_unix")?,
            vault_token: m.bytes("vault_token")?,
        };
        r.validate()?;
        Ok(r)
    }
}

// ── canonical-CBOR map helpers (fail-closed) ────────────────────────

fn encode_capped(v: &Value) -> Result<Vec<u8>> {
    let bytes = to_canonical_vec(v).map_err(|e| schema(format!("encode: {e}")))?;
    if bytes.len() > MAX_BODY_LEN {
        return Err(schema(format!(
            "canonical body must be <= {MAX_BODY_LEN} bytes, got {}",
            bytes.len()
        )));
    }
    Ok(bytes)
}

/// A decoded top-level CBOR map with text keys, fail-closed: rejects
/// non-canonical encodings, non-text or duplicate keys, and any key
/// outside the expected allowlist (`#[serde(deny_unknown_fields)]`
/// equivalent).
struct Map {
    entries: Vec<(String, Value)>,
}

impl Map {
    fn get(&self, key: &str) -> Result<&Value> {
        self.entries
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v)
            .ok_or_else(|| schema(format!("missing field `{key}`")))
    }
    fn text(&self, key: &str) -> Result<String> {
        match self.get(key)? {
            Value::Text(t) => Ok(t.clone()),
            _ => Err(schema(format!("field `{key}` must be text"))),
        }
    }
    fn uint(&self, key: &str) -> Result<u64> {
        match self.get(key)? {
            Value::Integer(i) => {
                u64::try_from(*i).map_err(|_| schema(format!("field `{key}` must be a u64")))
            }
            _ => Err(schema(format!("field `{key}` must be an integer"))),
        }
    }
    fn bytes(&self, key: &str) -> Result<Vec<u8>> {
        match self.get(key)? {
            Value::Bytes(b) => Ok(b.clone()),
            _ => Err(schema(format!("field `{key}` must be bytes"))),
        }
    }
    fn byte_array<const N: usize>(&self, key: &str) -> Result<[u8; N]> {
        let b = self.bytes(key)?;
        if b.len() != N {
            return Err(schema(format!(
                "field `{key}` must be {N} bytes, got {}",
                b.len()
            )));
        }
        let mut out = [0u8; N];
        out.copy_from_slice(&b);
        Ok(out)
    }
    fn map(&self, key: &str) -> Result<Map> {
        match self.get(key)? {
            Value::Map(pairs) => map_from_pairs(pairs),
            _ => Err(schema(format!("field `{key}` must be a map"))),
        }
    }
}

fn map_from_pairs(pairs: &[(Value, Value)]) -> Result<Map> {
    let mut entries: Vec<(String, Value)> = Vec::with_capacity(pairs.len());
    for (k, v) in pairs {
        let key = match k {
            Value::Text(t) => t.clone(),
            _ => return Err(schema("map keys must be text".into())),
        };
        if entries.iter().any(|(e, _)| e == &key) {
            return Err(schema(format!("duplicate map key `{key}`")));
        }
        entries.push((key, v.clone()));
    }
    Ok(Map { entries })
}

/// Decode `bytes` as a canonical-CBOR top-level map and reject any key
/// outside `allowed` (deny-unknown-fields).
fn decode_top(bytes: &[u8], allowed: &[&str]) -> Result<Map> {
    if bytes.len() > MAX_BODY_LEN {
        return Err(schema(format!(
            "body must be <= {MAX_BODY_LEN} bytes, got {}",
            bytes.len()
        )));
    }
    assert_canonical(bytes).map_err(|e| schema(format!("non-canonical: {e}")))?;
    let v: Value = ciborium::de::from_reader(bytes).map_err(|e| schema(format!("cbor: {e}")))?;
    let pairs = match v {
        Value::Map(p) => p,
        _ => return Err(schema("top-level value must be a map".into())),
    };
    let m = map_from_pairs(&pairs)?;
    for (k, _) in &m.entries {
        if !allowed.contains(&k.as_str()) {
            return Err(schema(format!("unknown field `{k}`")));
        }
    }
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scope() -> BrokerScope {
        BrokerScope {
            vm_id: "vm-1".into(),
            luks_path: "hippius-compute/kbs/tenants/vm-1/luks-kek".into(),
            luks_version: 2,
            userdata_path: "hippius-compute/kbs/tenants/vm-1/userdata".into(),
            userdata_version: 1,
            lifecycle_path: None,
            lifecycle_version: None,
        }
    }

    fn scope_with_lifecycle() -> BrokerScope {
        BrokerScope {
            lifecycle_path: Some("hippius-compute/kbs/tenants/vm-1/lifecycle-key".into()),
            lifecycle_version: Some(1),
            ..scope()
        }
    }

    // ── Scope path authz (audit H6) ──────────────────────────────────

    #[test]
    fn validate_accepts_a_well_formed_scope() {
        assert!(scope().validate().is_ok());
        assert!(scope_with_lifecycle().validate().is_ok());
    }

    #[test]
    fn validate_rejects_a_cross_tenant_path() {
        // luks_path bound to vm-2 while vm_id is vm-1 → a ticket cannot
        // scope the broker at ANOTHER tenant's KEK.
        let mut s = scope();
        s.luks_path = "hippius-compute/kbs/tenants/vm-2/luks-kek".into();
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_a_wrong_secret_segment() {
        let mut s = scope();
        s.luks_path = "hippius-compute/kbs/tenants/vm-1/signing-key".into();
        assert!(s.validate().is_err());
        // userdata path pointed at the KEK is likewise rejected.
        let mut s2 = scope();
        s2.userdata_path = "hippius-compute/kbs/tenants/vm-1/luks-kek".into();
        assert!(s2.validate().is_err());
    }

    #[test]
    fn validate_rejects_hcl_injection_chars() {
        for evil in [
            "hippius-compute/kbs/tenants/vm-1/luks-kek\" { capabilities=[\"read\"] }\npath \"secret/*",
            "hippius-compute/kbs/tenants/vm-1/luks-kek}",
            "hippius-compute/kbs/tenants/vm-1/luks kek",
            "hippius-compute/kbs/tenants/vm-1/luks-kek\n",
        ] {
            let mut s = scope();
            s.luks_path = evil.into();
            assert!(s.validate().is_err(), "must reject: {evil:?}");
        }
    }

    #[test]
    fn validate_rejects_path_traversal() {
        let mut s = scope();
        s.luks_path = "hippius-compute/kbs/tenants/../tenants/vm-1/luks-kek".into();
        // `..` segment → rejected.
        assert!(s.validate().is_err());
    }

    #[test]
    fn validate_rejects_a_bad_vm_id() {
        for bad in ["VM-1", "vm_1", "vm/1", "vm 1", &"v".repeat(65), ""] {
            let mut s = scope();
            s.vm_id = bad.into();
            assert!(s.validate().is_err(), "must reject vm_id {bad:?}");
        }
    }

    #[test]
    fn challenge_request_round_trips() {
        let r = ChallengeRequest { scope: scope() };
        let bytes = r.canonical().unwrap();
        assert_eq!(ChallengeRequest::decode(&bytes).unwrap(), r);
    }

    #[test]
    fn scope_with_lifecycle_round_trips() {
        // §7: a scope carrying the optional lifecycle path+version
        // round-trips with both fields preserved.
        let r = ChallengeRequest {
            scope: scope_with_lifecycle(),
        };
        let bytes = r.canonical().unwrap();
        let back = ChallengeRequest::decode(&bytes).unwrap();
        assert_eq!(back, r);
        assert_eq!(
            back.scope.lifecycle_path.as_deref(),
            Some("hippius-compute/kbs/tenants/vm-1/lifecycle-key")
        );
        assert_eq!(back.scope.lifecycle_version, Some(1));
    }

    #[test]
    fn pre_s7_scope_canonical_omits_lifecycle_keys() {
        // Wire-compat: a `None` lifecycle scope must encode WITHOUT the
        // two §7 keys, byte-identical to a pre-§7 broker's encoding.
        let bytes = ChallengeRequest { scope: scope() }.canonical().unwrap();
        // The canonical bytes must not mention either §7 map key.
        let needle_path = b"lifecycle_path";
        let needle_ver = b"lifecycle_version";
        assert!(!bytes.windows(needle_path.len()).any(|w| w == needle_path));
        assert!(!bytes.windows(needle_ver.len()).any(|w| w == needle_ver));
    }

    #[test]
    fn half_set_lifecycle_scope_is_rejected() {
        // A path with no version (or vice-versa) is a malformed scope.
        let mut s = scope();
        s.lifecycle_path = Some("hippius-compute/kbs/tenants/vm-1/lifecycle-key".into());
        s.lifecycle_version = None;
        assert!(s.validate().is_err());
        let mut s2 = scope();
        s2.lifecycle_path = None;
        s2.lifecycle_version = Some(1);
        assert!(s2.validate().is_err());
    }

    #[test]
    fn challenge_response_round_trips() {
        let r = ChallengeResponse {
            nonce: [7u8; NONCE_LEN],
            expiry_unix: 1_000,
        };
        let bytes = r.canonical().unwrap();
        assert_eq!(ChallengeResponse::decode(&bytes).unwrap(), r);
    }

    #[test]
    fn redeem_request_round_trips() {
        let r = RedeemRequest {
            scope: scope(),
            challenge_nonce: [3u8; NONCE_LEN],
            auth_pubkey: [9u8; AUTH_PUBKEY_LEN],
            snp_report: vec![0u8; MIN_SNP_REPORT_LEN],
            vek_der: vec![1u8; 64],
        };
        let bytes = r.canonical().unwrap();
        assert_eq!(RedeemRequest::decode(&bytes).unwrap(), r);
    }

    #[test]
    fn redeem_response_round_trips_and_redacts_token() {
        let r = RedeemResponse {
            scope: scope(),
            cap_expiry_unix: 1_234,
            vault_token: b"hvs.CAESxxxxsecret".to_vec(),
        };
        let bytes = r.canonical().unwrap();
        let back = RedeemResponse::decode(&bytes).unwrap();
        assert_eq!(back, r);
        // §20: Debug must NOT print the token bytes.
        let dbg = format!("{back:?}");
        assert!(!dbg.contains("secret"), "token leaked in Debug: {dbg}");
        assert!(dbg.contains("vault_token_len"));
    }

    #[test]
    fn decode_rejects_unknown_field() {
        let v = Value::Map(vec![
            (Value::Text("scope".into()), scope().to_value()),
            (Value::Text("evil".into()), Value::Integer(1.into())),
        ]);
        let bytes = to_canonical_vec(&v).unwrap();
        assert!(ChallengeRequest::decode(&bytes).is_err());
    }

    #[test]
    fn decode_rejects_short_report() {
        let r = RedeemRequest {
            scope: scope(),
            challenge_nonce: [3u8; NONCE_LEN],
            auth_pubkey: [9u8; AUTH_PUBKEY_LEN],
            snp_report: vec![0u8; 100],
            vek_der: Vec::new(),
        };
        assert!(r.canonical().is_err());
    }

    #[test]
    fn decode_rejects_wrong_nonce_len() {
        let v = Value::Map(vec![
            (Value::Text("expiry_unix".into()), Value::Integer(1.into())),
            (Value::Text("nonce".into()), Value::Bytes(vec![0u8; 16])),
        ]);
        let bytes = to_canonical_vec(&v).unwrap();
        assert!(ChallengeResponse::decode(&bytes).is_err());
    }

    #[test]
    fn empty_vault_token_rejected() {
        let r = RedeemResponse {
            scope: scope(),
            cap_expiry_unix: 1,
            vault_token: Vec::new(),
        };
        assert!(r.canonical().is_err());
    }
}

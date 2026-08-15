//! Miner-agent **guest-boot progress report** — a display-only,
//! non-authoritative signal the miner-agent emits AFTER a launch order is
//! accepted, so vali (and, through it, the API + SDK) can show a tenant
//! VM's fine boot phases: the guest booting → the KEK released → the
//! guest running. ARCHITECTURE.md §K (miner→vali telemetry channel).
//!
//! This is a SEPARATE signed payload from the periodic
//! [`crate::heartbeat`] and the one-shot [`crate::graceful_exit`] —
//! deliberately so. The heartbeat is liveness-critical and the
//! graceful-exit drives a quarantine; a boot-progress report drives
//! NEITHER — it only advances a `boot_phase` display field on vali's
//! `lifecycle.Vm` row. It gets its own low-frequency envelope + its own
//! replay domain so a signature minted over it can never be lifted into
//! either scheme.
//!
//! The miner-agent signs a [`VmProgressReport`] body with the SAME
//! Ed25519 identity key it signs heartbeats with; vali verifies the
//! [`SignedVmProgress`] envelope against the out-of-band-registered miner
//! key (the same trust anchor as the heartbeat), enforces a `±300 s` skew
//! window, then advances the VM's `boot_phase` MONOTONICALLY (never
//! regresses — this replaces a per-miner `sequence` replay counter: a
//! replayed earlier milestone is a no-op because vali only advances).
//!
//! [`DOMAIN`] is **distinct** from every other signed-payload domain in
//! the stack (`HIPPIUS_MINER_HEARTBEAT_V1`,
//! `HIPPIUS_MINER_GRACEFUL_EXIT_V1`, `RELEASE_DOMAIN`, …); the domain tag
//! is itself a signed field.

use alloc::string::String;
use alloc::vec::Vec;

use ciborium::value::Value;
use serde::{Deserialize, Serialize};

use crate::cbor::to_canonical_vec;

/// Replay-domain separator — the first field (by sort order) of every
/// signed vm-progress body. Distinct from every other signed-payload
/// domain in the stack.
pub const DOMAIN: &str = "HIPPIUS_VM_PROGRESS_V1";

/// Wire-format version. An unknown value fails closed at
/// [`VmProgressReport::validate`].
pub const SCHEMA_VERSION: u8 = 1;

/// Length of an Ed25519 signature — the `sig` field of the envelope.
pub const SIGNATURE_LEN: usize = 64;

/// Max accepted `miner_id` length (matches the heartbeat bound).
pub const MAX_MINER_ID_LEN: usize = 64;

/// Max accepted `vm_id` length — mirrors vali's `lifecycle.Vm.vm_id`
/// column (`max_length=256`).
pub const MAX_VM_ID_LEN: usize = 256;

/// The three boot milestones the miner-agent can observe on the host,
/// in monotonic order. The wire value (kebab-case) is what the signed
/// body carries; vali maps it to its `boot_phase` display field.
///
/// - [`Booting`](VmProgressMilestone::Booting) — the domain has started
///   (libvirt `Started` lifecycle event).
/// - [`KekReleased`](VmProgressMilestone::KekReleased) — the KBS released
///   the LUKS KEK to the attested guest (the host-side kbs-proxy saw a
///   `forwarded status=200` on the release path).
/// - [`Running`](VmProgressMilestone::Running) — the guest is up and has
///   emitted its first served-receipt (the vsock relay saw
///   `forward-ServedReceipt=ok`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VmProgressMilestone {
    Booting,
    KekReleased,
    Running,
}

impl VmProgressMilestone {
    /// The kebab-case wire string carried in the signed body.
    pub fn as_wire(self) -> &'static str {
        match self {
            VmProgressMilestone::Booting => "booting",
            VmProgressMilestone::KekReleased => "kek-released",
            VmProgressMilestone::Running => "running",
        }
    }

    /// Parse a wire string back to a milestone. Unknown ⇒ `None`.
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "booting" => Some(VmProgressMilestone::Booting),
            "kek-released" => Some(VmProgressMilestone::KekReleased),
            "running" => Some(VmProgressMilestone::Running),
            _ => None,
        }
    }
}

/// Fixed-classifier errors encoding / validating a vm-progress report.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum VmProgressError {
    /// `schema_version` was not [`SCHEMA_VERSION`].
    #[error("vm-progress-schema-version")]
    SchemaVersion,
    /// `domain` was not [`DOMAIN`].
    #[error("vm-progress-domain")]
    Domain,
    /// `miner_id` empty or over [`MAX_MINER_ID_LEN`].
    #[error("vm-progress-miner-id")]
    MinerId,
    /// `vm_id` empty or over [`MAX_VM_ID_LEN`].
    #[error("vm-progress-vm-id")]
    VmId,
    /// `milestone` was not one of the [`VmProgressMilestone`] wire values.
    #[error("vm-progress-milestone")]
    Milestone,
    /// `sig` was not [`SIGNATURE_LEN`] bytes.
    #[error("vm-progress-signature-length")]
    SignatureLength,
    /// Deterministic-CBOR encode failed.
    #[error("vm-progress-encode")]
    Encode,
}

/// Result alias for this module.
pub type Result<T> = core::result::Result<T, VmProgressError>;

/// The body the miner-agent signs (and vali re-derives + verifies) to
/// report a guest-boot milestone. The fields pin the report to a single
/// VM on a single miner at a single instant:
///
/// - `miner_id` — which registered miner (the `vali` registry key);
/// - `vm_id` — which tenant VM the milestone is about;
/// - `milestone` — the boot phase reached (kebab-case wire value);
/// - `timestamp_unix` — the miner's wall-clock, anti-skew-checked;
/// - `domain` — the replay-context separator (also in the signed bytes).
///
/// No `sequence` counter: vali advances `boot_phase` monotonically, so a
/// replayed earlier milestone is inert (see the module docs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmProgressReport {
    /// Wire-format version. MUST be [`SCHEMA_VERSION`].
    pub schema_version: u8,
    /// Operator-assigned miner identifier — the `vali` registry key.
    pub miner_id: String,
    /// The tenant VM this milestone is about.
    pub vm_id: String,
    /// The boot milestone reached.
    pub milestone: VmProgressMilestone,
    /// The miner's wall-clock at build time, Unix seconds.
    pub timestamp_unix: i64,
    /// Replay-domain tag. MUST be [`DOMAIN`].
    pub domain: String,
}

impl VmProgressReport {
    /// Construct a report for `milestone` on `vm_id` from `miner_id` at
    /// `timestamp_unix`. `domain` is forced to [`DOMAIN`] and the
    /// version to [`SCHEMA_VERSION`] so the caller cannot mint an
    /// off-scheme body.
    pub fn new(
        miner_id: String,
        vm_id: String,
        milestone: VmProgressMilestone,
        timestamp_unix: i64,
    ) -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            miner_id,
            vm_id,
            milestone,
            timestamp_unix,
            domain: DOMAIN.into(),
        }
    }

    /// Every semantic invariant of a well-formed report. Run by
    /// [`canonical`](Self::canonical) so a signature over an impossible
    /// value can never be produced; a decoder MUST run it too.
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(VmProgressError::SchemaVersion);
        }
        if self.domain != DOMAIN {
            return Err(VmProgressError::Domain);
        }
        if self.miner_id.is_empty() || self.miner_id.len() > MAX_MINER_ID_LEN {
            return Err(VmProgressError::MinerId);
        }
        if self.vm_id.is_empty() || self.vm_id.len() > MAX_VM_ID_LEN {
            return Err(VmProgressError::VmId);
        }
        // `milestone` is a typed enum here, so it is always a valid wire
        // value on the encode path; the check exists for parity with the
        // decode path (a decoder builds this struct from wire text).
        if VmProgressMilestone::from_wire(self.milestone.as_wire()).is_none() {
            return Err(VmProgressError::Milestone);
        }
        Ok(())
    }

    /// Deterministic-CBOR encoding of the to-be-signed body — the
    /// signature preimage. Keys are emitted in sorted order (the
    /// `to_canonical_vec` invariant); `validate` runs first so an
    /// impossible body never reaches the signer.
    pub fn canonical(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let v = Value::Map(alloc::vec![
            (Value::Text("domain".into()), Value::Text(DOMAIN.into())),
            (
                Value::Text("milestone".into()),
                Value::Text(self.milestone.as_wire().into()),
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
                Value::Text("timestamp_unix".into()),
                Value::Integer(self.timestamp_unix.into()),
            ),
            (Value::Text("vm_id".into()), Value::Text(self.vm_id.clone())),
        ]);
        to_canonical_vec(&v).map_err(|_| VmProgressError::Encode)
    }
}

/// The `{body, sig}` envelope the miner-agent POSTs to vali (relayed by
/// the Edge). `body` is the canonical [`VmProgressReport`] bytes; `sig`
/// is the Ed25519 signature over them by the miner's identity key.
///
/// `#[serde(deny_unknown_fields)]` blocks an extra-key smuggle past the
/// envelope decode — defence in depth for the relay path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SignedVmProgress {
    #[serde(with = "serde_bytes")]
    pub body: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub sig: Vec<u8>,
}

impl SignedVmProgress {
    /// Deterministic-CBOR encoding of the `{body, sig}` envelope — the
    /// exact bytes the miner-agent POSTs and vali's verifier ingests.
    pub fn canonical(&self) -> Result<Vec<u8>> {
        if self.sig.len() != SIGNATURE_LEN {
            return Err(VmProgressError::SignatureLength);
        }
        let v = Value::Map(alloc::vec![
            (Value::Text("body".into()), Value::Bytes(self.body.clone())),
            (Value::Text("sig".into()), Value::Bytes(self.sig.clone())),
        ]);
        to_canonical_vec(&v).map_err(|_| VmProgressError::Encode)
    }

    /// Extract `timestamp_unix` from the canonical-CBOR `body` without a
    /// full struct decode — the miner-agent's sender uses it to drop a
    /// report that has aged past vali's skew window (a signed envelope
    /// cannot be re-signed without breaking the canonical invariant).
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

    fn sample() -> VmProgressReport {
        VmProgressReport::new(
            "miner-a".into(),
            "vm-abc-123".into(),
            VmProgressMilestone::KekReleased,
            1_700_000_000,
        )
    }

    #[test]
    fn canonical_round_trips_and_is_canonical() {
        let body = sample().canonical().unwrap();
        assert_canonical(&body).unwrap();
        let env = SignedVmProgress {
            body: body.clone(),
            sig: alloc::vec![9u8; SIGNATURE_LEN],
        };
        let enc = env.canonical().unwrap();
        assert_canonical(&enc).unwrap();
        assert_eq!(env.timestamp_unix(), Some(1_700_000_000));
    }

    #[test]
    fn canonical_is_stable() {
        let a = sample().canonical().unwrap();
        let b = sample().canonical().unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn milestone_wire_round_trips() {
        for m in [
            VmProgressMilestone::Booting,
            VmProgressMilestone::KekReleased,
            VmProgressMilestone::Running,
        ] {
            assert_eq!(VmProgressMilestone::from_wire(m.as_wire()), Some(m));
        }
        assert_eq!(VmProgressMilestone::from_wire("nope"), None);
    }

    #[test]
    fn changing_any_field_changes_signed_bytes() {
        let base = sample().canonical().unwrap();
        let mutate: &[fn(&mut VmProgressReport)] = &[
            |r| r.miner_id = "other-miner".into(),
            |r| r.vm_id = "other-vm".into(),
            |r| r.milestone = VmProgressMilestone::Running,
            |r| r.timestamp_unix += 1,
        ];
        for m in mutate {
            let mut r = sample();
            m(&mut r);
            assert_ne!(
                base,
                r.canonical().unwrap(),
                "a field escaped the signature"
            );
        }
    }

    #[test]
    fn bad_schema_version_fails_closed() {
        let mut r = sample();
        r.schema_version = 9;
        assert_eq!(r.validate(), Err(VmProgressError::SchemaVersion));
        assert!(r.canonical().is_err());
    }

    #[test]
    fn bad_domain_miner_id_and_vm_id_fail_closed() {
        let mut r = sample();
        r.domain = "WRONG".into();
        assert_eq!(r.validate(), Err(VmProgressError::Domain));
        let mut r = sample();
        r.miner_id = String::new();
        assert_eq!(r.validate(), Err(VmProgressError::MinerId));
        let mut r = sample();
        r.vm_id = String::new();
        assert_eq!(r.validate(), Err(VmProgressError::VmId));
        let mut r = sample();
        r.vm_id = "x".repeat(MAX_VM_ID_LEN + 1);
        assert_eq!(r.validate(), Err(VmProgressError::VmId));
    }

    #[test]
    fn envelope_rejects_wrong_sig_length() {
        let env = SignedVmProgress {
            body: sample().canonical().unwrap(),
            sig: alloc::vec![0u8; 63],
        };
        assert_eq!(env.canonical(), Err(VmProgressError::SignatureLength));
    }

    #[test]
    fn error_classifiers_are_stable() {
        assert_eq!(
            VmProgressError::SchemaVersion.to_string(),
            "vm-progress-schema-version"
        );
        assert_eq!(VmProgressError::Domain.to_string(), "vm-progress-domain");
        assert_eq!(VmProgressError::MinerId.to_string(), "vm-progress-miner-id");
        assert_eq!(VmProgressError::VmId.to_string(), "vm-progress-vm-id");
        assert_eq!(
            VmProgressError::Milestone.to_string(),
            "vm-progress-milestone"
        );
        assert_eq!(
            VmProgressError::SignatureLength.to_string(),
            "vm-progress-signature-length"
        );
        assert_eq!(VmProgressError::Encode.to_string(), "vm-progress-encode");
    }
}

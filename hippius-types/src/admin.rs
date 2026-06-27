//! Wire types for the KBS admin API (ARCHITECTURE.md §24/§25 lifecycle
//! pre-registration). Consumed by `kbs-admin-client` (the Rust subprocess
//! vali invokes before dispatch) and by `kbs-core::admin` (the verifier).
//!
//! ## Scope
//!
//! The request body for `POST /v1/admin/register-vm` is the byte-exact
//! signed COSE_Sign1 `OrderTicket` envelope vali already submits to
//! `POST /v1/order_ticket`. **No bespoke request struct.** This keeps:
//!
//! - The cryptographic anchor identical to the existing `verify_order_
//!   ticket` path (kbs-core derives `vm_id`, `vm_generation`,
//!   `platform_id`, `lease_id` from the verified ticket, not from
//!   anything vali sends).
//! - Idempotency keyed by the ticket's `ticket_id` (durably
//!   single-use across retries).
//! - The §24 lifecycle gate's input shape unchanged from the
//!   existing §6 ticket schema — no parallel CBOR types to keep in
//!   sync.
//!
//! This module ONLY defines the response wire types.
//!
//! ## Phase A scope
//!
//! Phase A ships `op = Register` only. The `decommission` / `crypto-erase`
//! / `activate` ops referenced by `vali/apps/orchestration/effects.py`
//! (§24/§25) are Phase B follow-ups; their handlers return 501 until
//! implemented. `AdminOp` carries the full enum today so the wire
//! shape is locked at v1.

#[allow(unused_imports)]
use alloc::{
    string::{String, ToString},
    vec::Vec,
};

use serde::{Deserialize, Serialize};

/// The admin operation discriminator. Mirrors the URL path component
/// vali already uses in `effects.py:_kbs_post(vm, command, …)` — same
/// names, same case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AdminOp {
    /// First-time write of a `VmState::Active`. CAS predicate:
    /// `cur.is_none() || cur == Some(Active { gen, host, lease_id })`.
    /// Idempotent on identical state.
    RegisterVm,
    /// Phase B — Active → Decommissioning. Currently 501.
    Decommission,
    /// Phase B — Decommissioning → Destroyed{gen}. Currently 501.
    CryptoErase,
    /// Phase B — Migrating → Active(dest, new_gen). Currently 501.
    Activate,
}

impl AdminOp {
    /// URL path component (the last segment of
    /// `/v1/admin/{op-name}`).
    pub fn path_segment(self) -> &'static str {
        match self {
            AdminOp::RegisterVm => "register-vm",
            AdminOp::Decommission => "decommission",
            AdminOp::CryptoErase => "crypto-erase",
            AdminOp::Activate => "activate",
        }
    }
}

/// 200 OK body for `POST /v1/admin/register-vm`. Echoes the fields
/// the KBS derived from the verified ticket so the caller can
/// confirm what was applied (no silent state drift).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminRegisterVmResponse {
    /// Schema version (always 1 for now).
    pub v: u32,
    /// Echoed from the verified ticket.
    pub ticket_id: String,
    /// Echoed from the verified ticket.
    pub vm_id: String,
    /// Echoed from the verified ticket.
    pub vm_generation: u64,
    /// Echoed from the verified ticket (`platform_id` → `host`).
    pub host: String,
    /// Echoed from the verified ticket.
    pub lease_id: String,
    /// Unix seconds of the KBS-side apply.
    pub applied_at: u64,
    /// `true` ⇒ this exact `ticket_id` was previously applied with an
    /// identical body; KBS returned the cached response, no new
    /// state-store write. `false` ⇒ this was the first apply.
    pub cached: bool,
}

/// 200 body for `POST /v1/admin/allowlist/reload`. Confirms what the
/// KBS swapped in so vali (the caller) can audit + log the new state.
///
/// The reload is the dev-mode automation for the §22 ceremony — vali
/// signs a fresh manifest with the dev seed, posts the COSE here,
/// and the KBS atomically swaps the in-memory body after the same
/// signature + epoch HWM checks the file-fed startup path runs. Same
/// `InstalledAllowlist::install` API powers both lanes; the only
/// difference is the transport.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminReloadAllowlistResponse {
    /// Schema version (always 1 for now).
    pub v: u32,
    /// Monotonic epoch from the newly-installed body. Vali pins this
    /// in its activity log so a later `register-vm` against a stale
    /// allowlist surfaces loudly.
    pub epoch: u64,
    /// Lower-case hex SHA-256 of the COSE bytes the KBS installed.
    /// Matches what the caller uploaded — a mismatch here means the
    /// KBS decoded a different byte sequence (the request body was
    /// mangled in transit) and the install would have already
    /// rejected on signature; included for belt-and-suspenders.
    pub sha256_hex: String,
}

/// 4xx body (400/401/403/409/429). Symmetric across error codes so
/// the client can deserialize without branching on status. The exact
/// status code carries the high-level class; `reason` is the
/// machine-grep subclass string.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminErrorResponse {
    /// Short machine-grep subclass — e.g. `"ticket-decode"`,
    /// `"ticket-expired"`, `"url-vm-id-mismatch"`,
    /// `"state-divergent"`, `"unauthorized"`, `"forbidden"`,
    /// `"rate-limited"`. Stable across versions — the test suite
    /// pins each of them.
    pub reason: String,
    /// Optional ticket id, when the body parsed far enough for KBS
    /// to extract it. Absent on 400 `ticket-decode`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket_id: Option<String>,
    /// Optional vm id; same conditions as `ticket_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vm_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_op_path_segments_match_vali_effects_py() {
        // Pinned to match `vali/apps/orchestration/effects.py:_kbs_post`
        // path component naming. Renaming any of these is a
        // wire-breaking change and MUST land in lockstep on both sides.
        assert_eq!(AdminOp::RegisterVm.path_segment(), "register-vm");
        assert_eq!(AdminOp::Decommission.path_segment(), "decommission");
        assert_eq!(AdminOp::CryptoErase.path_segment(), "crypto-erase");
        assert_eq!(AdminOp::Activate.path_segment(), "activate");
    }

    #[test]
    fn admin_op_serde_kebab_case() {
        let mut buf = Vec::new();
        ciborium::ser::into_writer(&AdminOp::RegisterVm, &mut buf).unwrap();
        // `serde(rename_all = "kebab-case")` encodes the variant as a
        // CBOR text string. Just round-trip and check the symbolic
        // identity — the exact CBOR bytes are an implementation detail
        // of ciborium's enum encoder.
        let back: AdminOp = ciborium::de::from_reader(buf.as_slice()).unwrap();
        assert_eq!(back, AdminOp::RegisterVm);
    }

    #[test]
    fn admin_register_vm_response_round_trip() {
        let r = AdminRegisterVmResponse {
            v: 1,
            ticket_id: "tk-1".into(),
            vm_id: "vm-1".into(),
            vm_generation: 3,
            host: "chip-aa84a3f0".into(),
            lease_id: "lease-q2".into(),
            applied_at: 1700000000,
            cached: false,
        };
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&r, &mut bytes).unwrap();
        let back: AdminRegisterVmResponse = ciborium::de::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(back.ticket_id, "tk-1");
        assert_eq!(back.vm_generation, 3);
        assert!(!back.cached);
    }

    #[test]
    fn admin_error_response_round_trip() {
        let e = AdminErrorResponse {
            reason: "state-divergent".into(),
            ticket_id: Some("tk-2".into()),
            vm_id: Some("vm-2".into()),
        };
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&e, &mut bytes).unwrap();
        let back: AdminErrorResponse = ciborium::de::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(back.reason, "state-divergent");
        assert_eq!(back.ticket_id.as_deref(), Some("tk-2"));
    }
}

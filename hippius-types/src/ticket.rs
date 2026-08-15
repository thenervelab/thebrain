//! OrderTicket schema (ARCHITECTURE.md §6). This crate carries the
//! **types only** — COSE_Sign1 verification, expiry/now checks, and
//! the `accepts_l1_kid(measurement, kid)` allowlist gate all live in
//! kbs-core (KBS responsibility) so L1 (the minter) can depend on the
//! types without pulling in the verification stack.

#[allow(unused_imports)]
// per-module slice of the alloc prelude — not every module needs every item
use alloc::{
    boxed::Box,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use serde::Deserialize;
use serde_bytes::ByteBuf;

use crate::flavor::Flavor;

/// The only OrderTicket schema version this control plane accepts (§6).
///
/// # History
/// - **v1** (initial) — carried `resource_class: String`, a free-form
///   audit label.
/// - **v2** (#312) — `resource_class` retired; replaced with
///   `flavor: Flavor`, the strongly-typed catalogue identifier. Wire
///   bytes change because `Value::Text("flavor")` ≠
///   `Value::Text("resource_class")`. Single-use, short-lived tickets
///   mean there are no in-flight v1 envelopes to migrate.
pub const SCHEMA_V: u32 = 2;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct VaultRef {
    pub path: String,
    pub version: u64,
}

/// All fields are signed (§6). `deny_unknown_fields` — an unknown field
/// is a hard reject (schema is fixed by `v`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrderTicket {
    pub v: u32,
    pub ticket_id: String,
    pub issue_time: u64,
    pub expiry: u64,
    /// Ticket single-use nonce. NOTE: the §7 release-once primitive is
    /// keyed by `(ticket_id, KBS_nonce)` — this field is a separate
    /// L1-supplied per-ticket nonce.
    pub nonce: ByteBuf,
    pub tenant_id: String,
    pub user_id: String,
    pub vm_id: String,
    /// §24/§25 lifecycle binding.
    pub lease_id: String,
    pub vm_generation: u64,
    /// Intended placement constraint (§6/§7 lifecycle/generation binding).
    pub node_id: String,
    pub platform_id: String,
    /// Each entry is a 48-byte SNP launch measurement.
    pub allowed_measurements: Vec<ByteBuf>,
    pub userdata_vault_ref: VaultRef,
    pub luks_vault_ref: VaultRef,
    /// 32-byte SHA-256 (§20) of the sealed user-data + binding fields.
    pub allowed_userdata_digest: ByteBuf,
    /// Tenant VM size (§F / #312). Replaces v1's free-form
    /// `resource_class: String`. The mint side picks one variant;
    /// the KBS verifier + the future miner-agent enforcer
    /// authoritatively derive `vcpus` / `memory_mb` / `disk_gb` from
    /// this single signed identifier.
    pub flavor: Flavor,
    pub lifecycle_perms: Vec<String>,
}

impl OrderTicket {
    pub fn nonce(&self) -> &[u8] {
        self.nonce.as_ref()
    }
    pub fn allowed_userdata_digest(&self) -> &[u8] {
        self.allowed_userdata_digest.as_ref()
    }
}

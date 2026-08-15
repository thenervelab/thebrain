//! # hippius-types
//!
//! Canonical, cross-implementation shared types for the Hippius
//! Confidential Compute control plane (ARCHITECTURE.md §6/§20/§22):
//!
//! - `cbor` — deterministic CBOR (RFC 8949 §4.2.1) encoder + the
//!   reject-non-canonical helper. Every wire format in the stack
//!   round-trips through this — L1, KBS, and the guest must agree on
//!   the exact bytes.
//! - `ticket` — the signed-immutable OrderTicket (§6). Schema only,
//!   no verification (signature verify lives in kbs-core).
//! - `release` — the release-context, wrapped-secret, signed response,
//!   and signed denial wire structs (§20).
//! - `report_data` — exact SEV-SNP `REPORT_DATA` layouts (§20). The
//!   tenant `nonce ‖ x25519_pubkey` layout AND the distinct Audit-VM
//!   `nonce ‖ SHA256(audit_vm_pubkey ‖ ctx ‖ node_id ‖ platform_id)`
//!   binding.
//! - `digest` — the pinned `allowed_userdata_digest` preimage (§20):
//!   length-prefixed streaming SHA-256 the L1 minter and the KBS must
//!   compute identically. NOT a `ciborium::Value` so the plaintext is
//!   never copied into non-zeroizing heap buffers.
//!
//! All types use `#[serde(deny_unknown_fields)]` where applicable so a
//! schema drift between impls fails closed at decode.
//!
//! ## `no_std` build (PR-I3)
//!
//! The crate is `#![cfg_attr(not(feature = "std"), no_std)]`. The
//! default-on `std` feature enables `std::error::Error` impls + the
//! ciborium / serde / sha2 `std` features (for consumers that don't
//! care about WASM size). Substrate runtimes (the `pallet-compute-
//! scoring` consumer) build with `default-features = false`, picking
//! up only the `alloc`-shaped surface (`Vec`, `String`, `format!`)
//! — no `std::io`, no `std::error::Error`.

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

extern crate alloc;

pub mod admin;
pub mod audit_vm;
pub mod audit_vm_cert;
pub mod cbor;
pub mod digest;
pub mod evidence_bundle;
pub mod flavor;
pub mod graceful_exit;
pub mod heartbeat;
pub mod host_attestor;
pub mod host_attestor_challenge;
pub mod kbs_vsock;
pub mod live_attestation;
pub mod provenance;
pub mod release;
pub mod report_data;
pub mod served_receipt;
pub mod stopped;
pub mod telemetry_cert;
pub mod ticket;
pub mod ticket_vsock;
pub mod vault_broker;
pub mod vm_progress;

use alloc::string::String;

/// Crate-wide error. Mirrors what the consumers (kbs-core) need, but
/// kept narrow on purpose — this crate has no I/O, no crypto verify.
#[derive(Debug, thiserror::Error)]
pub enum HippiusTypesError {
    #[error("cbor: {0}")]
    Cbor(String),
    #[error("ticket schema: {0}")]
    TicketSchema(String),
    #[error("provenance schema: {0}")]
    ProvenanceSchema(String),
    #[error("audit-vm cert schema: {0}")]
    AuditVmCertSchema(String),
    #[error("evidence bundle schema: {0}")]
    EvidenceBundleSchema(String),
    #[error("live attestation schema: {0}")]
    LiveAttestationSchema(String),
    #[error("vault broker schema: {0}")]
    VaultBrokerSchema(String),
    #[error("host attestor schema: {0}")]
    HostAttestorSchema(String),
}

pub type Result<T> = core::result::Result<T, HippiusTypesError>;

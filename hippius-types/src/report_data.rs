//! Exact SEV-SNP `REPORT_DATA` layouts (ARCHITECTURE.md §20).

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
use crate::{HippiusTypesError, Result};
use ciborium::value::Value;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

pub const REPORT_DATA_LEN: usize = 64;
pub const AUDIT_VM_DOMAIN: &[u8] = b"HIPPIUS_AUDIT_VM_V1";

/// Domain tag for the §23 tenant telemetry-signer `REPORT_DATA`
/// binding. Distinct from every other signed/hashed domain in the
/// stack so a hash bound under one scheme cannot be reused as another.
pub const TENANT_TELEMETRY_DOMAIN: &str = "HIPPIUS_TENANT_TELEMETRY_V1";

/// Domain tag for the §322 live-attestation `REPORT_DATA` binding.
/// Distinct from `LIVE_ATTESTATION_DOMAIN` (used by the on-chain
/// signed body) and every other domain in the stack — a report
/// minted under a release / telemetry / audit-vm binding cannot
/// replay as a live-attestation report.
pub const LIVE_ATTESTATION_REPORT_DOMAIN: &str = "HIPPIUS_LIVE_ATTESTATION_REPORT_V1";

/// Expected `REPORT_DATA` for a tenant guest.
pub fn tenant(nonce: &[u8; 32], guest_x25519_pub: &[u8; 32]) -> [u8; REPORT_DATA_LEN] {
    let mut out = [0u8; REPORT_DATA_LEN];
    out[0..32].copy_from_slice(nonce);
    out[32..64].copy_from_slice(guest_x25519_pub);
    out
}

/// Expected `REPORT_DATA` for the per-node Audit VM (§23). The
/// Ed25519 pubkey doesn't fit the X25519 layout, so we bind a SHA-256
/// over the pubkey, the domain, the `node_id` and the `platform_id` —
/// with each variable-length field length-prefixed (exact framing
/// below; do NOT hand-roll a plain `‖` concat — it will not match).
///
/// Each **variable-length** field (`audit_vm_ed25519_pub`, `node_id`,
/// `platform_id`) is `u32`-big-endian **length-prefixed** before it is
/// hashed, so the binding is **injective**: a plain concatenation of
/// three variable-length fields is ambiguous — `node_id=b"ab",
/// platform_id=b"c"` and `node_id=b"a", platform_id=b"bc"` would hash
/// identically — which would let an attested guest later be certified
/// for a *different* identity split. `AUDIT_VM_DOMAIN` is a fixed
/// constant so it carries no prefix. This refines the informal §20 `‖`
/// notation; a digest mismatch with a plain-concat verifier is a bug,
/// not a compatibility concern (this is the only producer).
pub fn audit_vm(
    nonce: &[u8; 32],
    audit_vm_ed25519_pub: &[u8],
    node_id: &[u8],
    platform_id: &[u8],
) -> [u8; REPORT_DATA_LEN] {
    let mut h = Sha256::new();
    h.update((audit_vm_ed25519_pub.len() as u32).to_be_bytes());
    h.update(audit_vm_ed25519_pub);
    h.update(AUDIT_VM_DOMAIN);
    h.update((node_id.len() as u32).to_be_bytes());
    h.update(node_id);
    h.update((platform_id.len() as u32).to_be_bytes());
    h.update(platform_id);
    let digest = h.finalize();
    let mut out = [0u8; REPORT_DATA_LEN];
    out[0..32].copy_from_slice(nonce);
    out[32..64].copy_from_slice(&digest);
    out
}

/// Expected `REPORT_DATA` for the §23 tenant telemetry-signer key.
///
/// The telemetry signer is **Ed25519** — like the Audit VM (§20) it
/// does not fit the tenant `[32:64] = X25519` layout, so `[32:64]` is
/// a SHA-256 binding instead. The preimage is a **canonical-CBOR map**
/// (RFC 8949 §4.2.1, sorted keys) carrying the domain tag, the signer
/// public key, and the `(node_id, vm_id)` identity. A CBOR map is
/// self-delimiting, so the variable-length `node_id` / `vm_id` cannot
/// be confused for one another — no concatenation-collision surface
/// (the plain-concat `audit_vm` layout above predates this helper).
///
/// The guest generates the Ed25519 signer keypair, folds this
/// `REPORT_DATA` into its SNP report, and the KBS recomputes the
/// identical 64 bytes before issuing the telemetry certificate — both
/// sides MUST call this one function.
pub fn tenant_telemetry(
    nonce: &[u8; 32],
    telemetry_signer_pubkey: &[u8; 32],
    node_id: &[u8],
    vm_id: &str,
) -> Result<[u8; REPORT_DATA_LEN]> {
    // Source key order is cosmetic — `to_canonical_vec` re-sorts to the
    // RFC 8949 §4.2.1 canonical order.
    let preimage = to_canonical_vec(&Value::Map(vec![
        (
            Value::Text("domain".into()),
            Value::Text(TENANT_TELEMETRY_DOMAIN.into()),
        ),
        (
            Value::Text("node_id".into()),
            Value::Bytes(node_id.to_vec()),
        ),
        (
            Value::Text("signer_pubkey".into()),
            Value::Bytes(telemetry_signer_pubkey.to_vec()),
        ),
        (Value::Text("vm_id".into()), Value::Text(vm_id.into())),
    ]))
    .map_err(|e| HippiusTypesError::Cbor(format!("tenant_telemetry preimage: {e}")))?;
    let digest = Sha256::digest(&preimage);
    let mut out = [0u8; REPORT_DATA_LEN];
    out[0..32].copy_from_slice(nonce);
    out[32..64].copy_from_slice(&digest);
    Ok(out)
}

/// Expected `REPORT_DATA` for a §322 live-attestation keepalive
/// report. The KBS issues a single-use nonce (same
/// [`crate::live_attestation`] flow as release), the in-VM guest
/// agent issues `SNP_GET_REPORT` with this `REPORT_DATA`, and the
/// KBS recomputes the same 64 bytes to verify before signing the
/// on-chain attestation.
///
/// `REPORT_DATA[0..32]` is the KBS-minted nonce (single-use, TTL-
/// bounded); `REPORT_DATA[32..64]` is a SHA-256 over a canonical-
/// CBOR map binding the `LIVE_ATTESTATION_REPORT_DOMAIN` tag + the
/// `vm_id`. Self-delimiting preimage (no concat-collision surface
/// like the legacy `audit_vm` layout). Same shape as
/// [`tenant_telemetry`].
pub fn live_attestation(nonce: &[u8; 32], vm_id: &str) -> Result<[u8; REPORT_DATA_LEN]> {
    let preimage = to_canonical_vec(&Value::Map(vec![
        (
            Value::Text("domain".into()),
            Value::Text(LIVE_ATTESTATION_REPORT_DOMAIN.into()),
        ),
        (Value::Text("vm_id".into()), Value::Text(vm_id.into())),
    ]))
    .map_err(|e| HippiusTypesError::Cbor(format!("live_attestation preimage: {e}")))?;
    let digest = Sha256::digest(&preimage);
    let mut out = [0u8; REPORT_DATA_LEN];
    out[0..32].copy_from_slice(nonce);
    out[32..64].copy_from_slice(&digest);
    Ok(out)
}

/// Constant-time equality.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.ct_eq(b).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_layout_is_exact() {
        let rd = tenant(&[1u8; 32], &[2u8; 32]);
        assert_eq!(&rd[0..32], &[1u8; 32]);
        assert_eq!(&rd[32..64], &[2u8; 32]);
    }

    #[test]
    fn audit_vm_layout_binds_node_and_platform() {
        let a = audit_vm(&[1u8; 32], &[9u8; 32], b"node-1", b"chip-1");
        let b = audit_vm(&[1u8; 32], &[9u8; 32], b"node-2", b"chip-1");
        assert!(!ct_eq(&a, &b));
        assert!(ct_eq(
            &a,
            &audit_vm(&[1u8; 32], &[9u8; 32], b"node-1", b"chip-1")
        ));
    }

    #[test]
    fn audit_vm_binding_is_injective_across_field_splits() {
        // Without length-prefixing, `node_id="ab" + platform_id="c"`
        // and `node_id="a" + platform_id="bc"` would concatenate to the
        // same bytes and hash identically. The length prefixes must
        // keep the two distinct.
        let a = audit_vm(&[1u8; 32], &[9u8; 32], b"ab", b"c");
        let b = audit_vm(&[1u8; 32], &[9u8; 32], b"a", b"bc");
        assert!(!ct_eq(&a, &b));
        // Same for an ambiguous pubkey / node_id split.
        let c = audit_vm(&[1u8; 32], b"xy", b"z", b"p");
        let d = audit_vm(&[1u8; 32], b"x", b"yz", b"p");
        assert!(!ct_eq(&c, &d));
    }

    #[test]
    fn ct_eq_rejects_length_mismatch() {
        assert!(!ct_eq(&[0u8; 3], &[0u8; 4]));
    }

    #[test]
    fn tenant_telemetry_places_nonce_then_digest() {
        let rd = tenant_telemetry(&[0x11; 32], &[0x22; 32], b"node-1", "vm-1").unwrap();
        assert_eq!(rd.len(), 64);
        assert_eq!(&rd[0..32], &[0x11; 32], "REPORT_DATA[0..32] = nonce");
        // [32..64] is the SHA-256 binding — not the raw pubkey.
        assert_ne!(&rd[32..64], &[0x22; 32]);
    }

    #[test]
    fn tenant_telemetry_binds_every_field() {
        let base = tenant_telemetry(&[1; 32], &[2; 32], b"node-1", "vm-1").unwrap();
        // Each input change ⇒ a different binding.
        assert_ne!(
            base,
            tenant_telemetry(&[9; 32], &[2; 32], b"node-1", "vm-1").unwrap()
        );
        assert_ne!(
            base,
            tenant_telemetry(&[1; 32], &[9; 32], b"node-1", "vm-1").unwrap()
        );
        assert_ne!(
            base,
            tenant_telemetry(&[1; 32], &[2; 32], b"node-2", "vm-1").unwrap()
        );
        assert_ne!(
            base,
            tenant_telemetry(&[1; 32], &[2; 32], b"node-1", "vm-2").unwrap()
        );
    }

    #[test]
    fn tenant_telemetry_node_vm_boundary_is_unambiguous() {
        // The canonical-CBOR-map preimage is self-delimiting, so a
        // plain-concatenation collision — (node="ab", vm="c") vs
        // (node="a", vm="bc") — cannot happen.
        let a = tenant_telemetry(&[0; 32], &[0; 32], b"ab", "c").unwrap();
        let b = tenant_telemetry(&[0; 32], &[0; 32], b"a", "bc").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn tenant_telemetry_is_deterministic() {
        let a = tenant_telemetry(&[7; 32], &[8; 32], b"node-x", "vm-x").unwrap();
        let b = tenant_telemetry(&[7; 32], &[8; 32], b"node-x", "vm-x").unwrap();
        assert_eq!(a, b);
    }

    /// Known-answer vector — frozen so a canonical-CBOR encoding change
    /// or a domain-tag edit (which would silently shift every tenant
    /// telemetry attestation) fails CI loudly. Inputs: `nonce = [0x11;
    /// 32]`, `signer_pubkey = [0x22; 32]`, `node_id = b"node-kat-1"`,
    /// `vm_id = "vm-kat-1"`.
    #[test]
    fn tenant_telemetry_known_answer() {
        let rd = tenant_telemetry(&[0x11; 32], &[0x22; 32], b"node-kat-1", "vm-kat-1").unwrap();
        let mut hex = String::new();
        for b in rd {
            hex.push_str(&format!("{b:02x}"));
        }
        assert_eq!(
            hex,
            "1111111111111111111111111111111111111111111111111111111111111111\
             08230cc2052e7b4824aaaa966671bb125f0376322c94a71b7cbd3eddb66ba746"
        );
    }

    // --- §322 live attestation ---

    #[test]
    fn live_attestation_places_nonce_then_digest() {
        let rd = live_attestation(&[0xAA; 32], "vm-keepalive-1").unwrap();
        assert_eq!(rd.len(), 64);
        assert_eq!(&rd[0..32], &[0xAA; 32], "REPORT_DATA[0..32] = nonce");
        assert_ne!(&rd[32..64], &[0u8; 32]);
    }

    #[test]
    fn live_attestation_binds_nonce_and_vm_id() {
        let base = live_attestation(&[1; 32], "vm-A").unwrap();
        assert_ne!(base, live_attestation(&[9; 32], "vm-A").unwrap());
        assert_ne!(base, live_attestation(&[1; 32], "vm-B").unwrap());
    }

    #[test]
    fn live_attestation_is_deterministic() {
        let a = live_attestation(&[7; 32], "vm-x").unwrap();
        let b = live_attestation(&[7; 32], "vm-x").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn live_attestation_distinct_from_tenant_telemetry() {
        // Different domain tags ⇒ different `[32..64]` even when
        // every other input is identical. Locks the cross-domain
        // separation: a tenant-telemetry SNP report cannot replay
        // as a live-attestation report.
        let lk = live_attestation(&[0; 32], "vm-1").unwrap();
        let tt = tenant_telemetry(&[0; 32], &[0; 32], b"", "vm-1").unwrap();
        assert_ne!(&lk[32..64], &tt[32..64]);
    }
}

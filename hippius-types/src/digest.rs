//! Pinned `allowed_userdata_digest` preimage (ARCHITECTURE.md §20).
//!
//! Authoritative definition: L1 (the minter) and the KBS MUST compute
//! this byte-identically. The implementation streams length-prefixed
//! fields into SHA-256 directly so the user-data plaintext is never
//! copied into a non-zeroizing heap buffer.

#[allow(unused_imports)]
// per-module slice of the alloc prelude — not every module needs every item
use alloc::{
    boxed::Box,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use sha2::{Digest, Sha256};

pub const USERDATA_DIGEST_DOMAIN: &str = "HIPPIUS_USERDATA_DIGEST_V1";

#[inline]
fn put_framed(h: &mut Sha256, s: &[u8]) {
    h.update((s.len() as u64).to_le_bytes());
    h.update(s);
}

/// Compute the `allowed_userdata_digest` over the canonical preimage.
/// Caller passes a borrowed `plaintext` slice (typically backed by a
/// `Zeroizing<Vec<u8>>` on the KBS side; L1 may zeroize similarly).
#[allow(clippy::too_many_arguments)]
pub fn userdata_digest(
    tenant_id: &str,
    vm_id: &str,
    ticket_id: &str,
    secret_type: &str,
    path: &str,
    version: u64,
    plaintext: &[u8],
) -> [u8; 32] {
    let mut h = Sha256::new();
    put_framed(&mut h, USERDATA_DIGEST_DOMAIN.as_bytes());
    put_framed(&mut h, tenant_id.as_bytes());
    put_framed(&mut h, vm_id.as_bytes());
    put_framed(&mut h, ticket_id.as_bytes());
    put_framed(&mut h, secret_type.as_bytes());
    put_framed(&mut h, path.as_bytes());
    h.update(version.to_le_bytes());
    put_framed(&mut h, plaintext);
    h.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn digest_is_deterministic_and_field_sensitive() {
        let d1 = userdata_digest("t", "v", "tk", "userdata", "/p", 1, b"plain");
        let d2 = userdata_digest("t", "v", "tk", "userdata", "/p", 1, b"plain");
        assert_eq!(d1, d2);
        // Any field change ⇒ different digest.
        assert_ne!(
            d1,
            userdata_digest("OTHER", "v", "tk", "userdata", "/p", 1, b"plain")
        );
        assert_ne!(
            d1,
            userdata_digest("t", "v", "tk", "luks", "/p", 1, b"plain")
        );
        assert_ne!(
            d1,
            userdata_digest("t", "v", "tk", "userdata", "/p", 2, b"plain")
        );
        assert_ne!(
            d1,
            userdata_digest("t", "v", "tk", "userdata", "/p", 1, b"PLAIN")
        );
    }

    #[test]
    fn length_prefix_disambiguates_concatenation_attacks() {
        // Without length-prefix, ("ab","c") and ("a","bc") would collide.
        let a = userdata_digest("ab", "c", "tk", "ud", "/p", 1, b"");
        let b = userdata_digest("a", "bc", "tk", "ud", "/p", 1, b"");
        assert_ne!(a, b);
    }
}

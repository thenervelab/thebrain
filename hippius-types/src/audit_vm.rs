//! Per-node Audit-VM signed `ServedDeliveryAggregate` (ARCHITECTURE.md §23).
//!
//! Wire schema + deterministic-CBOR encoding shared by:
//! - the Audit VM guest agent (signs aggregates inside the attested CVM),
//! - the validator (verifies before submitting on-chain),
//! - the on-chain pallet (verifies again).
//!
//! Full replay/fork domain on every Audit-VM signature: `{domain,
//! chain_genesis, pallet_instance, validator_id, family_id, node_id,
//! audit_vm_key_id, epoch, challenge_nonce, expiry}` — same rigor as
//! the §7 release contract. A signature lifted from one (chain, pallet,
//! validator, family, node, epoch, nonce, window) cannot replay into
//! any other.

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
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Domain tag for the aggregate — distinct from every other signed
/// payload (release, denial, stopped-ack).
pub const AGGREGATE_DOMAIN: &str = "HIPPIUS_AUDIT_VM_AGGREGATE_V1";

/// One per-receipt entry. The `digest` is a SHA-256 over the canonical
/// CBOR of the tenant-signed `ServedDeliveryReceipt` body (separate
/// scheme, owned by the tenant guest agent); the audit-VM never sees
/// the receipt body — only its digest, the `(vm_id, lease_id,
/// monotonic_seq)` tuple, and the validator's promise that the digest
/// corresponds to a billable lease (§23 anti-self-dealing).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServedReceiptEntry {
    pub vm_id: String,
    pub lease_id: String,
    pub monotonic_seq: u64,
    #[serde(with = "serde_bytes")]
    pub digest: Vec<u8>,
}

/// Per-resource-class served-units total over the aggregate interval.
/// `served_units` is the §23 u128 fixed-point quantity that feeds the
/// reward function `f`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceClassTotal {
    pub class: String,
    pub served_units: u128,
}

/// `map_root(per-receipt {vm_id, lease_id, monotonic_seq, digest})` —
/// the SHA-256 over deterministic-CBOR of the SORTED list of entries.
/// Sorted by `(vm_id, lease_id, monotonic_seq)` so two honest
/// constructors over the same set always emit byte-identical roots.
///
/// Empty list ⇒ SHA-256 of canonical CBOR for the empty array — a
/// well-defined value, not all-zeros.
pub fn map_root(entries: &[ServedReceiptEntry]) -> Result<[u8; 32]> {
    let mut sorted: Vec<&ServedReceiptEntry> = entries.iter().collect();
    sorted.sort_by(|a, b| {
        a.vm_id
            .cmp(&b.vm_id)
            .then(a.lease_id.cmp(&b.lease_id))
            .then(a.monotonic_seq.cmp(&b.monotonic_seq))
    });
    // Fail fast on invalid digests; all digests must be 32 bytes (SHA-256).
    for entry in sorted.iter() {
        if entry.digest.len() != 32 {
            return Err(HippiusTypesError::Cbor(format!(
                "invalid digest length for entry ({}, {}, {}): expected 32, got {}",
                entry.vm_id,
                entry.lease_id,
                entry.monotonic_seq,
                entry.digest.len()
            )));
        }
    }

    // Detect duplicate (vm_id, lease_id, monotonic_seq) — double-counting
    // is exactly the gaming pattern §23 forbids; fail closed.
    for w in sorted.windows(2) {
        if w[0].vm_id == w[1].vm_id
            && w[0].lease_id == w[1].lease_id
            && w[0].monotonic_seq == w[1].monotonic_seq
        {
            return Err(HippiusTypesError::Cbor(format!(
                "duplicate receipt entry: ({}, {}, {})",
                w[0].vm_id, w[0].lease_id, w[0].monotonic_seq
            )));
        }
    }
    let arr = Value::Array(
        sorted
            .into_iter()
            .map(|e| {
                Value::Map(vec![
                    (Value::Text("digest".into()), Value::Bytes(e.digest.clone())),
                    (
                        Value::Text("lease_id".into()),
                        Value::Text(e.lease_id.clone()),
                    ),
                    (
                        Value::Text("monotonic_seq".into()),
                        Value::Integer(e.monotonic_seq.into()),
                    ),
                    (Value::Text("vm_id".into()), Value::Text(e.vm_id.clone())),
                ])
            })
            .collect(),
    );
    let bytes = to_canonical_vec(&arr)
        .map_err(|e| HippiusTypesError::Cbor(format!("aggregate map_root: {e}")))?;
    let mut out = [0u8; 32];
    out.copy_from_slice(Sha256::digest(&bytes).as_slice());
    Ok(out)
}

/// RFC 8949 §3.4.3 / §4.2 "preferred serialization" for `u128`:
/// - values that fit in `u64` use major type 0 (unsigned integer);
/// - larger values use tag 2 (unsigned bignum) with the SHORTEST
///   big-endian byte string — leading zero bytes stripped (a
///   non-minimal bignum is non-canonical).
///
/// A value equal to 0 in the bignum branch never happens here
/// (0 always fits the u64 path) but if it did the canonical form is
/// the empty byte string per RFC 8949.
fn canonical_u128(n: u128) -> Value {
    if let Ok(small) = u64::try_from(n) {
        return Value::Integer(small.into());
    }
    let raw = n.to_be_bytes();
    let mut start = 0usize;
    while start < raw.len() && raw[start] == 0 {
        start += 1;
    }
    Value::Tag(2, Box::new(Value::Bytes(raw[start..].to_vec())))
}

/// SHA-256 over canonical CBOR of the sorted resource-class totals —
/// same idea as `map_root` but for the totals tuple.
pub fn totals_root(totals: &[ResourceClassTotal]) -> Result<[u8; 32]> {
    let mut sorted: Vec<&ResourceClassTotal> = totals.iter().collect();
    sorted.sort_by(|a, b| a.class.cmp(&b.class));
    for w in sorted.windows(2) {
        if w[0].class == w[1].class {
            return Err(HippiusTypesError::Cbor(format!(
                "duplicate resource class: {}",
                w[0].class
            )));
        }
    }
    let arr = Value::Array(
        sorted
            .into_iter()
            .map(|t| {
                Value::Map(vec![
                    (Value::Text("class".into()), Value::Text(t.class.clone())),
                    (
                        Value::Text("served_units".into()),
                        canonical_u128(t.served_units),
                    ),
                ])
            })
            .collect(),
    );
    let bytes =
        to_canonical_vec(&arr).map_err(|e| HippiusTypesError::Cbor(format!("totals root: {e}")))?;
    let mut out = [0u8; 32];
    out.copy_from_slice(Sha256::digest(&bytes).as_slice());
    Ok(out)
}

/// The body the Audit VM signs (and a verifier re-derives).
///
/// Replay-domain fields (`chain_genesis`, `pallet_instance`,
/// `validator_id`, `family_id`, `node_id`, `audit_vm_key_id`, `epoch`,
/// `challenge_nonce`, `expiry`) MUST be set by the validator and
/// echoed in the on-chain submission. The Audit VM accepts them as
/// inputs but never invents them — the validator-issued challenge
/// nonce pins the window.
///
/// Borrowed fields keep the heaviest data (receipt entries, totals)
/// out of the body bytes — only their roots travel inside the signed
/// aggregate, which the pallet+validator can recompute given the
/// underlying lists.
#[derive(Debug, Clone)]
pub struct ServedDeliveryAggregate<'a> {
    pub chain_genesis: &'a [u8; 32],
    pub pallet_instance: &'a [u8; 32],
    pub validator_id: &'a [u8],
    pub family_id: &'a [u8],
    pub node_id: &'a [u8],
    pub audit_vm_key_id: &'a [u8],
    pub epoch: u64,
    pub challenge_nonce: &'a [u8; 32],
    pub interval_start: u64,
    pub interval_end: u64,
    pub map_root: &'a [u8; 32],
    pub totals_root: &'a [u8; 32],
    pub prev_aggregate_hash: &'a [u8; 32],
    pub expiry: u64,
}

impl ServedDeliveryAggregate<'_> {
    /// Deterministic-CBOR encoding (stable across the Audit VM, the
    /// validator, and the on-chain pallet).
    pub fn canonical(&self) -> Result<Vec<u8>> {
        if self.interval_end < self.interval_start {
            return Err(HippiusTypesError::Cbor(
                "aggregate: interval_end < interval_start".into(),
            ));
        }
        if self.expiry < self.interval_end {
            return Err(HippiusTypesError::Cbor(
                "aggregate: expiry < interval_end".into(),
            ));
        }
        let v = Value::Map(vec![
            (
                Value::Text("audit_vm_key_id".into()),
                Value::Bytes(self.audit_vm_key_id.to_vec()),
            ),
            (
                Value::Text("chain_genesis".into()),
                Value::Bytes(self.chain_genesis.to_vec()),
            ),
            (
                Value::Text("challenge_nonce".into()),
                Value::Bytes(self.challenge_nonce.to_vec()),
            ),
            (
                Value::Text("domain".into()),
                Value::Text(AGGREGATE_DOMAIN.into()),
            ),
            (
                Value::Text("epoch".into()),
                Value::Integer(self.epoch.into()),
            ),
            (
                Value::Text("expiry".into()),
                Value::Integer(self.expiry.into()),
            ),
            (
                Value::Text("family_id".into()),
                Value::Bytes(self.family_id.to_vec()),
            ),
            (
                Value::Text("interval_end".into()),
                Value::Integer(self.interval_end.into()),
            ),
            (
                Value::Text("interval_start".into()),
                Value::Integer(self.interval_start.into()),
            ),
            (
                Value::Text("map_root".into()),
                Value::Bytes(self.map_root.to_vec()),
            ),
            (
                Value::Text("node_id".into()),
                Value::Bytes(self.node_id.to_vec()),
            ),
            (
                Value::Text("pallet_instance".into()),
                Value::Bytes(self.pallet_instance.to_vec()),
            ),
            (
                Value::Text("prev_aggregate_hash".into()),
                Value::Bytes(self.prev_aggregate_hash.to_vec()),
            ),
            (
                Value::Text("totals_root".into()),
                Value::Bytes(self.totals_root.to_vec()),
            ),
            (
                Value::Text("validator_id".into()),
                Value::Bytes(self.validator_id.to_vec()),
            ),
        ]);
        to_canonical_vec(&v).map_err(|e| HippiusTypesError::Cbor(format!("aggregate: {e}")))
    }
}

/// Signed envelope. `body` is the canonical CBOR of
/// [`ServedDeliveryAggregate`]; `sig` is Ed25519 over `body`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedServedDeliveryAggregate {
    #[serde(with = "serde_bytes")]
    pub body: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub sig: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cbor::assert_canonical;

    fn entry(vm: &str, lease: &str, seq: u64) -> ServedReceiptEntry {
        ServedReceiptEntry {
            vm_id: vm.into(),
            lease_id: lease.into(),
            monotonic_seq: seq,
            digest: vec![0xAA; 32],
        }
    }
    fn total(class: &str, units: u128) -> ResourceClassTotal {
        ResourceClassTotal {
            class: class.into(),
            served_units: units,
        }
    }

    #[test]
    fn map_root_is_order_independent() {
        let a = [entry("v1", "l1", 1), entry("v2", "l1", 1)];
        let b = [entry("v2", "l1", 1), entry("v1", "l1", 1)];
        assert_eq!(map_root(&a).unwrap(), map_root(&b).unwrap());
    }

    #[test]
    fn map_root_rejects_duplicate_tuple() {
        let dup = [entry("v1", "l1", 1), entry("v1", "l1", 1)];
        assert!(map_root(&dup).is_err());
    }

    #[test]
    fn totals_root_is_order_independent_and_rejects_dup_class() {
        let a = [total("std", 1_000), total("high-mem", 500)];
        let b = [total("high-mem", 500), total("std", 1_000)];
        assert_eq!(totals_root(&a).unwrap(), totals_root(&b).unwrap());
        let dup = [total("std", 1), total("std", 2)];
        assert!(totals_root(&dup).is_err());
    }

    #[test]
    fn aggregate_canonical_is_stable_and_canonical() {
        let cg = [1u8; 32];
        let pi = [2u8; 32];
        let cn = [3u8; 32];
        let mr = [4u8; 32];
        let tr = [5u8; 32];
        let ph = [6u8; 32];
        let agg = ServedDeliveryAggregate {
            chain_genesis: &cg,
            pallet_instance: &pi,
            validator_id: b"validator-1",
            family_id: b"family-1",
            node_id: b"node-1",
            audit_vm_key_id: b"audit-key-1",
            epoch: 42,
            challenge_nonce: &cn,
            interval_start: 1_000,
            interval_end: 1_060,
            map_root: &mr,
            totals_root: &tr,
            prev_aggregate_hash: &ph,
            expiry: 2_000,
        };
        let a = agg.canonical().unwrap();
        let b = agg.canonical().unwrap();
        assert_eq!(a, b);
        assert_canonical(&a).unwrap();
    }

    #[test]
    fn aggregate_rejects_interval_inversion() {
        let z = [0u8; 32];
        let agg = ServedDeliveryAggregate {
            chain_genesis: &z,
            pallet_instance: &z,
            validator_id: b"v",
            family_id: b"f",
            node_id: b"n",
            audit_vm_key_id: b"k",
            epoch: 1,
            challenge_nonce: &z,
            interval_start: 200,
            interval_end: 100,
            map_root: &z,
            totals_root: &z,
            prev_aggregate_hash: &z,
            expiry: 300,
        };
        assert!(agg.canonical().is_err());
    }

    #[test]
    fn aggregate_rejects_expiry_before_interval_end() {
        let z = [0u8; 32];
        let agg = ServedDeliveryAggregate {
            chain_genesis: &z,
            pallet_instance: &z,
            validator_id: b"v",
            family_id: b"f",
            node_id: b"n",
            audit_vm_key_id: b"k",
            epoch: 1,
            challenge_nonce: &z,
            interval_start: 100,
            interval_end: 200,
            map_root: &z,
            totals_root: &z,
            prev_aggregate_hash: &z,
            expiry: 150,
        };
        assert!(agg.canonical().is_err());
    }

    #[test]
    fn changing_any_replay_field_changes_signed_bytes() {
        // Spot-check several fields. The full proof of "every field
        // matters" comes from canonical CBOR being byte-stable + the
        // `domain` tag breaking cross-scheme reuse.
        let z = [0u8; 32];
        let mut base = ServedDeliveryAggregate {
            chain_genesis: &z,
            pallet_instance: &z,
            validator_id: b"v",
            family_id: b"f",
            node_id: b"n",
            audit_vm_key_id: b"k",
            epoch: 1,
            challenge_nonce: &z,
            interval_start: 100,
            interval_end: 200,
            map_root: &z,
            totals_root: &z,
            prev_aggregate_hash: &z,
            expiry: 300,
        };
        let bytes_a = base.canonical().unwrap();
        base.epoch = 2;
        assert_ne!(bytes_a, base.canonical().unwrap());
        base.epoch = 1;
        let other = [9u8; 32];
        base.challenge_nonce = &other;
        assert_ne!(bytes_a, base.canonical().unwrap());
    }

    #[test]
    fn map_root_rejects_invalid_digest_len() {
        let mut e = entry("v1", "l1", 1);
        e.digest = vec![0xBB; 16]; // Not 32 bytes
        assert!(map_root(&[e.clone()]).is_err());
        e.digest = vec![0xCC; 32]; // Correct length
        assert!(map_root(&[e]).is_ok());
    }

    #[test]
    fn map_root_sorts_by_seq_correctly() {
        let a = [entry("v1", "l1", 2), entry("v1", "l1", 1)];
        let b = [entry("v1", "l1", 1), entry("v1", "l1", 2)];
        assert_eq!(map_root(&a).unwrap(), map_root(&b).unwrap());
        assert_ne!(
            map_root(&a).unwrap(),
            map_root(&[entry("v1", "l1", 3)]).unwrap()
        );
    }

    #[test]
    fn map_root_empty_is_ok() {
        // Must produce a well-defined, non-zero hash of an empty CBOR array (`0x80`).
        let root = map_root(&[]).unwrap();
        let empty_cbor_array_hash = Sha256::digest([0x80]);
        assert_eq!(root, empty_cbor_array_hash.as_slice());
    }

    #[test]
    fn totals_root_empty_is_ok() {
        let root = totals_root(&[]).unwrap();
        let empty_cbor_array_hash = Sha256::digest([0x80]);
        assert_eq!(root, empty_cbor_array_hash.as_slice());
    }

    #[test]
    fn totals_root_u128_canonical_encoding() {
        // u64-sized value must be encoded as a standard int.
        let small = [total("std", 100)];
        // > u64::MAX value must be encoded as a bignum.
        let large = [total("std", u64::MAX as u128 + 1)];
        let root_small = totals_root(&small).unwrap();
        let root_large = totals_root(&large).unwrap();
        assert_ne!(root_small, root_large);

        // Manually verify the CBOR for the small value to be sure.
        let total_val = total("std", 100);
        let val = Value::Map(vec![
            (Value::Text("class".into()), Value::Text(total_val.class)),
            (
                Value::Text("served_units".into()),
                Value::Integer(100.into()),
            ),
        ]);
        let arr = Value::Array(vec![val]);
        let bytes = to_canonical_vec(&arr).unwrap();
        assert_eq!(Sha256::digest(&bytes).as_slice(), root_small.as_slice());

        // Manually verify the CBOR for the large value, using the RFC 8949
        // MINIMAL bignum encoding (leading zero bytes stripped). u64::MAX+1
        // is 0x0000_0000_0000_0001_0000_0000_0000_0000 BE; minimal form
        // drops the seven leading zero bytes ⇒ 9 bytes.
        let total_val_large = total("std", u128::from(u64::MAX) + 1);
        let mut be = total_val_large.served_units.to_be_bytes().to_vec();
        while be.first() == Some(&0) {
            be.remove(0);
        }
        let val_large = Value::Map(vec![
            (
                Value::Text("class".into()),
                Value::Text(total_val_large.class),
            ),
            (
                Value::Text("served_units".into()),
                Value::Tag(2, Box::new(Value::Bytes(be))),
            ),
        ]);
        let arr_large = Value::Array(vec![val_large]);
        let bytes_large = to_canonical_vec(&arr_large).unwrap();
        assert_eq!(
            Sha256::digest(&bytes_large).as_slice(),
            root_large.as_slice()
        );
    }

    #[test]
    fn canonical_u128_boundary_values() {
        // Each call independent — we just exercise the helper.
        assert!(matches!(canonical_u128(0), Value::Integer(_)));
        assert!(matches!(canonical_u128(23), Value::Integer(_)));
        assert!(matches!(canonical_u128(24), Value::Integer(_)));
        assert!(matches!(
            canonical_u128(u64::MAX as u128),
            Value::Integer(_)
        ));
        // First non-u64 value ⇒ bignum, minimal (leading zeros stripped).
        match canonical_u128(u128::from(u64::MAX) + 1) {
            Value::Tag(2, inner) => match *inner {
                Value::Bytes(b) => {
                    assert!(!b.is_empty());
                    assert_ne!(b[0], 0, "bignum must NOT have leading zero (non-minimal)");
                }
                _ => panic!("bignum payload must be bytes"),
            },
            _ => panic!("expected bignum tag 2"),
        }
        match canonical_u128(u128::MAX) {
            Value::Tag(2, inner) => match *inner {
                Value::Bytes(b) => {
                    // u128::MAX has no leading zeros ⇒ 16 bytes.
                    assert_eq!(b.len(), 16);
                    assert_eq!(b[0], 0xff);
                }
                _ => panic!(),
            },
            _ => panic!(),
        }
    }
}

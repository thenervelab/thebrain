//! Deterministic CBOR (RFC 8949 §4.2.1) shared by L1, KBS, and the guest.

#[allow(unused_imports)]
// per-module slice of the alloc prelude — not every module needs every item
use alloc::{
    boxed::Box,
    format,
    string::{String, ToString},
    vec,
    vec::Vec,
};

use crate::{HippiusTypesError, Result};
use ciborium::value::Value;

fn enc(value: &Value) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    ciborium::ser::into_writer(value, &mut buf)
        .map_err(|e| HippiusTypesError::Cbor(format!("encode: {e}")))?;
    Ok(buf)
}

fn canonicalize(value: &Value) -> Result<Value> {
    Ok(match value {
        Value::Map(entries) => {
            let mut canon: Vec<(Vec<u8>, Value, Value)> = Vec::with_capacity(entries.len());
            for (k, v) in entries {
                let ck = canonicalize(k)?;
                let cv = canonicalize(v)?;
                let kbytes = enc(&ck)?;
                canon.push((kbytes, ck, cv));
            }
            canon.sort_by(|a, b| a.0.cmp(&b.0));
            for w in canon.windows(2) {
                if w[0].0 == w[1].0 {
                    return Err(HippiusTypesError::Cbor("duplicate map key".into()));
                }
            }
            Value::Map(canon.into_iter().map(|(_, k, v)| (k, v)).collect())
        }
        Value::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                out.push(canonicalize(it)?);
            }
            Value::Array(out)
        }
        other => other.clone(),
    })
}

/// Canonical encoding of an arbitrary value.
pub fn to_canonical_vec(value: &Value) -> Result<Vec<u8>> {
    enc(&canonicalize(value)?)
}

/// Reject `bytes` unless they are already deterministically encoded.
pub fn assert_canonical(bytes: &[u8]) -> Result<()> {
    let value: Value = ciborium::de::from_reader(bytes)
        .map_err(|e| HippiusTypesError::Cbor(format!("decode: {e}")))?;
    let canon = to_canonical_vec(&value)?;
    if canon.as_slice() == bytes {
        Ok(())
    } else {
        Err(HippiusTypesError::Cbor("non-deterministic encoding".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_roundtrip_is_stable() {
        let v = Value::Map(vec![
            (Value::Text("b".into()), Value::Integer(2.into())),
            (Value::Text("a".into()), Value::Integer(1.into())),
        ]);
        let c = to_canonical_vec(&v).unwrap();
        assert_canonical(&c).unwrap();
    }

    #[test]
    fn unsorted_map_is_rejected() {
        let v = Value::Map(vec![
            (Value::Text("b".into()), Value::Integer(2.into())),
            (Value::Text("a".into()), Value::Integer(1.into())),
        ]);
        let mut noncanon = Vec::new();
        ciborium::ser::into_writer(&v, &mut noncanon).unwrap();
        assert!(assert_canonical(&noncanon).is_err());
    }

    #[test]
    fn duplicate_key_is_rejected() {
        let v = Value::Map(vec![
            (Value::Text("a".into()), Value::Integer(1.into())),
            (Value::Text("a".into()), Value::Integer(2.into())),
        ]);
        assert!(to_canonical_vec(&v).is_err());
    }
}

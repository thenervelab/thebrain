use sp_std::vec::Vec;
/// Signs a payload using the session key
pub fn sign_payload(
    keys: &[u8],
    payload: &[u8]
) -> Result<Vec<u8>, ()> {
    // Create signature by combining key and payload
    let mut signature = Vec::with_capacity(keys.len() + payload.len());
    signature.extend_from_slice(keys);
    signature.extend_from_slice(payload);
    
    Ok(signature)
}


use sp_consensus_babe::AuthorityId as BabeId;
use codec::Encode;
// converting babe id
pub fn babe_public_to_array(public: &BabeId) -> [u8; 32] {
    let raw_bytes = public.encode();
    let mut array = [0u8; 32];
    array.copy_from_slice(&raw_bytes[..32]);
    array
}
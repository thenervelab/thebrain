use crate::{ UID,  pallet::Config, types::Role};
use sp_core::{sr25519, crypto::Ss58Codec, Get};
use sp_std::{vec, vec::Vec};
use sp_runtime::{AccountId32, offchain::{http, Duration}};
use scale_info::prelude::format;

// pub const STORAGE_PREFIX: &str = "658faa385070e074c85bf6b568cf0555aab1b4e78e1ea8305462ee53b3686dc81300";

// new parse fn
pub fn decode_storage_key_to_address(encoded_key: Vec<u8>) -> Option<UID> {

    // Take the last 32 bytes which should be the AccountId
    if encoded_key.len() >= 32 {
        let account_bytes = &encoded_key[encoded_key.len() - 32..];
        
        // Convert to AccountId32
        if let Ok(account) = AccountId32::try_from(account_bytes) {
            // we can use this substrate address later 
            let _substrate_address = account.to_ss58check();
            // Convert the account to sr25519::Public
            let public = sr25519::Public::from_raw(account.clone().into());
            
            Some(UID { 
                address: public,  // Now using the correct type
                id: 0,
                role: Role::None,
                substrate_address: account,
            })
        } else {
            None
        }
    } else {
        None
    }
}


/// Parse a UID from a hex-encoded storage key
pub fn parse_uid_from_hex<T: Config>(hex: &str) -> Result<u16, http::Error> {

    let url = T::FinneyUrl::get();

    let json_payload = format!(r#"{{
        "id": 1,
        "jsonrpc": "2.0",
        "method": "state_getStorage",
        "params": ["{}"]
    }}"#, hex);

    let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(2_000));
        
    let body = vec![json_payload];
    let request = sp_runtime::offchain::http::Request::post(url, body);

    let pending = request
        .add_header("Content-Type", "application/json")
        .deadline(deadline)
        .send()
        .map_err(|err| {
            log::error!("❌ Error making Request: {:?}", err);
            sp_runtime::offchain::http::Error::IoError
        })?;

    let response = pending
        .try_wait(deadline)
        .map_err(|err| {
            log::error!("❌ Error getting Response: {:?}", err);
            sp_runtime::offchain::http::Error::DeadlineReached
        })??;

    if response.code != 200 {
        log::error!("❌ Unexpected status code: {}", response.code);
        return Err(http::Error::Unknown);
    }

    let body = response.body().collect::<Vec<u8>>();    
    let body_str = sp_std::str::from_utf8(&body).map_err(|_| {
        log::error!("❌ Response body is not valid UTF-8");
        http::Error::Unknown
    })?;

    // Find the start and end of the result field
    let result_start = body_str.find("\"result\":\"").ok_or_else(|| {
        log::error!("❌ 'result' field not found in JSON parsing uid hex");
        http::Error::Unknown
    })? + 10;

    let result_end = body_str[result_start..].find("\"").ok_or_else(|| {
        log::error!("❌ End of 'result' field not found");
        http::Error::Unknown
    })? + result_start;

    let result_str = &body_str[result_start..result_end]; 

    // Now, let's parse the result string (hex) to bytes
    let hex_str = result_str.trim(); // Remove extra spaces

    // Remove the "0x" prefix if it exists
    let hex_str = hex_str.strip_prefix("0x").unwrap_or(hex_str);

    // Decode the hex string into bytes
    let bytes = hex::decode(hex_str).map_err(|_| {
        log::error!("❌ Failed to decode hex string");
        http::Error::Unknown
    })?;

    Ok(bytes[0].into())
}

pub fn update_uids_with_roles(mut uids: Vec<UID>, dividends: &[u16]) -> Vec<UID> {
    for uid in uids.iter_mut() {
        // Check if uid.id is within the bounds of the dividends array
        if uid.id as usize >= dividends.len() {
            // If out of bounds, assign role as StorageMiner
            uid.role = Role::StorageMiner;
        } else if uid.id == 179 {
            // Assign role as Validator if uid.id is 179
            uid.role = Role::Validator;
        } else if dividends[uid.id as usize] > 0 {
            uid.role = Role::Validator;
        } else {
            uid.role = Role::StorageMiner;
        }
    }
    uids
}
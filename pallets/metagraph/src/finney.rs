use crate::{ UID, hotkeys, pallet::Config};
use sp_runtime::offchain::{http, Duration};
use sp_std::{prelude::*, collections::btree_map::BTreeMap, vec::Vec};
use sp_core::Get;
use sp_runtime::format;


/// Fetch UIDs from the finney API
pub fn fetch_uids_keys<T: Config>() -> Result<Vec<UID>, http::Error> {
    let url = T::FinneyUrl::get();

    let json_payload = format!(r#"{{
        "id": 1,
        "jsonrpc": "2.0",
        "method": "state_getKeys",
        "params": ["{}"]
    }}"#, T::UidsStorageKey::get());

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

    // Call the function
    let result = parse_uids_response::<T>(body_str);

    result
}

/// Parse the JSON response from finney API into a vector of UIDs
fn parse_uids_response<T: Config>(response: &str) -> Result<Vec<UID>, http::Error> {
    // Parse JSON response to extract the "result" array
    let result_start = response.find("\"result\":[").ok_or_else(|| {
        log::error!("❌ 'result' array not found in JSON");
        http::Error::Unknown
    })? + 9;
    
    let result_end = response[result_start..].find("]").ok_or_else(|| {
        log::error!("❌ End of 'result' array not found");
        http::Error::Unknown
    })? + result_start;
    
    let result_str = &response[result_start..=result_end];

    // Use BTreeMap to ensure unique IDs
    let mut uid_map = BTreeMap::new();
    
    // Parse each hex string into a UID
    for hex in result_str.split(',') {
        let hex = hex.trim_matches(|c| c == '[' || c == ']' || c == '"' || c == ' ');
        if !hex.is_empty() {            
            let bytes = hex::decode(&hex[2..])
            .map_err(|_| {
                log::error!("❌ Failed to decode hex string");
                http::Error::Unknown
            })?;
            let address = hotkeys::decode_storage_key_to_address(bytes.clone());

            match address {
                Some(mut uid) => {
                    // Try to parse the UID from hex and log it if successful
                    match hotkeys::parse_uid_from_hex::<T>(hex) {
                        Ok(id) => {
                            uid.id = id;
                        },
                        Err(e) => log::error!("❌ Failed to parse UID from hex: {:?}", e),
                    }
                    // Introduce a delay of 500 milliseconds (adjust as needed)
                    sp_io::offchain::sleep_until(
                        sp_io::offchain::timestamp().add(Duration::from_millis(500)),
                    );

                    uid_map.insert(uid.address, uid);
                },
                None => log::info!("No address found (None)"),
            }
        }
    }

    // Convert map values to vec
    let uids: Vec<_> = uid_map.into_values().collect();
    Ok(uids)
}

/// Fetch dividends from the finney API
pub fn fetch_dividends<T: Config>() -> Result<Vec<u16>, http::Error> {
    let url = T::FinneyUrl::get();
    log::info!("🔴 Fetching dividends from {}", url);

    let json_payload = format!(r#"{{
        "id": 1,
        "jsonrpc": "2.0",
        "method": "state_getStorage",
        "params": ["{}"]
    }}"#, T::DividendsStorageKey::get());

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

    parse_dividends_response::<T>(body_str)
}

/// Parse the JSON response from finney API into a vector of dividends
fn parse_dividends_response<T: Config>(response: &str) -> Result<Vec<u16>, http::Error> {
    // Find the result field
    let result_start = response.find("\"result\":\"").ok_or_else(|| {
        log::error!("❌ 'result' field not found in response");
        http::Error::Unknown
    })? + 10; // Skip past "result":\"

    // Find the end of the result value (next quote)
    let result_end = response[result_start..].find('\"').ok_or_else(|| {
        log::error!("❌ No closing quote found for result value");
        http::Error::Unknown
    })? + result_start;

    // Extract the hex string
    let hex_str = &response[result_start..result_end];

    // Parse the hex string into dividends
    match crate::dividends::parse_dividends_from_hex::<T>(hex_str) {
        Ok(dividends) => {
            // Log the parsed dividends
            Ok(dividends)
        }
        Err(e) => {
            log::error!("❌ Error parsing dividends: {:?}", e);
            Err(http::Error::Unknown)
        }
    }
}

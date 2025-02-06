use crate::{Error, utils, pallet::Config};
use sp_std::vec::Vec;
use sp_runtime::format;

/// Parse dividends from a hex-encoded storage value
pub fn parse_dividends_from_hex<T: Config>(hex: &str) -> Result<Vec<u16>, Error<T>> {    
    // Remove null value if present
    if hex == "null" {
        log::error!("❌ Received null value");
        return Ok(Vec::new());
    }

    // Ensure we have a hex string
    if !hex.starts_with("0x") {
        log::error!("❌ Invalid hex string - missing 0x prefix");
        return Err(Error::<T>::InvalidUIDFormat);
    }

    let bytes = utils::decode_hex::<T>(hex)?;

    // Check if we have enough bytes for dividends
    if bytes.len() < 4 {
        log::error!("❌ Not enough bytes for dividends: {}", bytes.len());
        return Err(Error::<T>::InvalidUIDFormat);
    }

    // Skip the first byte (header) and process the rest
    let data = &bytes[1..]; // Skip the first byte

    // Each dividend is a u16 (2 bytes) in little-endian format
    let mut dividends = Vec::with_capacity(data.len() / 2);
    for chunk in data.chunks_exact(2) {
        // Convert from little-endian bytes to u16
        let dividend = u16::from_le_bytes([chunk[0], chunk[1]]);
        dividends.push(dividend);
    }
    
    // Log non-zero dividends with their indices
    let non_zero = dividends.iter().enumerate()
        .filter(|(_, &v)| v > 0)
        .map(|(i, &v)| format!("index {}: {}", i + 1, v))
        .collect::<Vec<_>>();

    log::info!("Non-zero dividends: {:?}", non_zero);
    
    Ok(dividends) // Return the parsed dividends
}

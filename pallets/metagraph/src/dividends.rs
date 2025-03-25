use crate::{Error, utils, pallet::Config};
use sp_std::vec::Vec;
use sp_runtime::format;
use sp_std::vec;

/// Parse dividends from a hex-encoded storage value
pub fn parse_dividends_from_hex<T: Config>(hex: &str) -> Result<Vec<u16>, Error<T>> {
    
    // Handle null case
    if hex == "null" {
        return Ok(vec![0; 256]); // Return a vector of 256 zeros
    }

    // Validate hex format
    if !hex.starts_with("0x") {
        return Err(Error::<T>::InvalidUIDFormat);
    }

    let bytes = utils::decode_hex::<T>(hex)?;
   
    // Initialize a vector with 256 zeros (expected output size)
    let mut dividends = vec![0u16; 256];

    // Calculate how many u16 values we can extract from the byte array
    let num_values = bytes.len() / 2; // Each u16 is 2 bytes

    // Extract u16 values from the byte array
    for i in 0..num_values {
        if i < dividends.len() {
            dividends[i] = u16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]]);
        }
    }

    // Log non-zero dividends with their indices
    let non_zero = dividends.iter().enumerate()
        .filter(|(_, &v)| v > 0)
        .map(|(i, &v)| format!("index {}: {}", i, v))
        .collect::<Vec<_>>();

    // Remove the first item
    if !dividends.is_empty() {
        dividends.remove(0);
    }

    // Remove the last 36 items
    for _ in 0..36 {
        if !dividends.is_empty() {
            dividends.pop();
        }
    }

    Ok(dividends)
}
use crate::{pallet::Config, Error};
use sp_std::vec::Vec;

/// Decodes a hex string into a vector of bytes
pub fn decode_hex<T: Config>(hex: &str) -> Result<Vec<u8>, Error<T>> {
	if hex.len() % 2 != 0 {
		log::error!("❌ Hex string length not even: {}", hex.len());
		return Err(Error::<T>::DecodingError);
	}

	let hex = if hex.starts_with("0x") { &hex[2..] } else { hex };

	let mut bytes = Vec::with_capacity(hex.len() / 2);
	for i in (0..hex.len()).step_by(2) {
		let byte_str = &hex[i..i + 2];
		let byte = u8::from_str_radix(byte_str, 16).map_err(|_| {
			log::error!("❌ Failed to parse byte: {}", byte_str);
			Error::<T>::DecodingError
		})?;
		bytes.push(byte);
	}

	Ok(bytes)
}

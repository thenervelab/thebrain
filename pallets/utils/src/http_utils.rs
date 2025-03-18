use sp_runtime::offchain::{http, Duration};
use sp_std::{vec::Vec,vec};
use sp_runtime::format;

pub fn fetch_node_id(url: &str, method: &str) -> Result<Vec<u8>, http::Error> {
	let json_payload = format!(
		r#"{{
			"id": 1,
			"jsonrpc": "2.0",
			"method": "{}",
			"params": []
		}}"#,
		method
	);

	let deadline = sp_io::offchain::timestamp().add(Duration::from_millis(2_000));
	let body = vec![json_payload];
	let request = http::Request::post(url, body);

	let pending = request
		.add_header("Content-Type", "application/json")
		.deadline(deadline)
		.send()
		.map_err(|err| {
			log::warn!("Error making Request: {:?}", err);
			http::Error::IoError
		})?;

	let response = pending
		.try_wait(deadline)
		.map_err(|err| {
			log::warn!("Error getting Response: {:?}", err);
			http::Error::DeadlineReached
		})??;

	if response.code != 200 {
		log::warn!("Unexpected status code: {}", response.code);
		return Err(http::Error::Unknown);
	}

	let body = response.body().collect::<Vec<u8>>();
	let body_str = sp_std::str::from_utf8(&body).map_err(|_| {
		log::warn!("Response body is not valid UTF-8");
		http::Error::Unknown
	})?;

	if let Some(start_idx) = body_str.find("\"result\":\"PeerId(\\\"") {
		let peer_id_start = start_idx + "\"result\":\"PeerId(\\\"".len();
		if let Some(end_idx) = body_str[peer_id_start..].find("\\") {
			let peer_id = &body_str[peer_id_start..peer_id_start + end_idx];
			log::info!("The Brain Node Id: {}", peer_id);
			return Ok(peer_id.as_bytes().to_vec());
		} else {
			log::warn!("End pattern '\\)' not found in response");
			return Err(http::Error::Unknown);
		}
	} else {
		log::warn!("Start pattern '\"result\":\"PeerId(\\\"' not found in response");
		return Err(http::Error::Unknown);
	}
}
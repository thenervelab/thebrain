use codec::Compact;
use codec::{Decode, Encode};
use fp_rpc::EthereumRuntimeRPCApi;
use lite_json::json::JsonValue;
use reqwest::blocking::Client;
pub use rpc_core_weight::WeightsInfoApiServer;
use serde_json::json;
use sp_api::ProvideRuntimeApi;
use sp_blockchain::HeaderBackend;
use sp_core::crypto::CryptoTypeId;
use sp_core::crypto::Ss58Codec;
use sp_core::offchain::KeyTypeId;
use sp_core::sr25519;
use sp_keystore::{Keystore, KeystorePtr};
use sp_runtime::generic::Era;
use sp_runtime::traits::Block as BlockT;
use sp_runtime::AccountId32;
use sp_runtime::RuntimeDebug;
use sp_runtime::{MultiAddress, MultiSignature};
use std::sync::Arc;
use rpc_core_weight::types::SystemInfo;
use rpc_core_weight::types::NodeType;

/// An identifier used to match public keys against sr25519 keys
pub const CRYPTO_ID: CryptoTypeId = CryptoTypeId(*b"sr25");

/// Net API implementation.
pub struct WeightsInfoImpl<B: BlockT, C> {
	_client: Arc<C>,
	keystore: KeystorePtr,
	_phantom_data: std::marker::PhantomData<B>,
}

impl<B: BlockT, C> WeightsInfoImpl<B, C> {
	pub fn new(_client: Arc<C>, keystore: KeystorePtr) -> Self {
		Self { _client, keystore, _phantom_data: Default::default() }
	}
}

impl<B, C> WeightsInfoApiServer for WeightsInfoImpl<B, C>
where
	B: BlockT,
	C: ProvideRuntimeApi<B> + 'static,
	C::Api: EthereumRuntimeRPCApi<B>,
	C: HeaderBackend<B> + Send + Sync,
{
	fn submit_weights(&self, params: rpc_core_weight::SubmitWeightsParams) -> String {
		let netuid = params.netuid;
		let dests = params.dests;
		let weights = params.weights;
		let version_key = params.version_key;
		let default_spec_version = params.default_spec_version;
		let default_genesis_hash = params.default_genesis_hash;
		let finney_api_url = params.finney_api_url;

		let extrinsic = create_signed_extrinsic(
			netuid,
			dests,
			weights,
			version_key,
			self.keystore.clone(),
			default_spec_version,
			&default_genesis_hash,
			&finney_api_url,
		);
		let hex = hex::encode(&extrinsic);

		hex
	}

	fn submit_hardware(&self, params: rpc_core_weight::SubmitHardwareParams) -> String {
		let node_id = params.node_id;
		let system_info = params.system_info;
		let default_spec_version = params.default_spec_version;
		let default_genesis_hash = params.default_genesis_hash;
		let local_rpc_url = params.local_rpc_url;

		let extrinsic = create_signed_extrinsic_hardware(
			node_id,
			system_info,
			self.keystore.clone(),
			default_spec_version,
			&default_genesis_hash,
			&local_rpc_url,
		);
		let hex = hex::encode(&extrinsic);
		hex
	}

	fn submit_metrics(&self, params: rpc_core_weight::SubmitMetricsParams) -> String {
		let node_id = params.node_id;
		let storage_proof_time_ms = params.storage_proof_time_ms;
		let latency_ms = params.latency_ms;
		let peer_count = params.peer_count;
		let failed_challenges_count = params.failed_challenges_count;
		let successful_challenges = params.successful_challenges;
		let total_challenges = params.total_challenges;
		let uptime_minutes = params.uptime_minutes;
		let total_minutes = params.total_minutes;
		let consecutive_reliable_days = params.consecutive_reliable_days;
		let recent_downtime_hours = params.recent_downtime_hours;
		let block_number = params.block_number;
		let default_spec_version = params.default_spec_version;
		let default_genesis_hash = params.default_genesis_hash;
		let local_rpc_url = params.local_rpc_url;

		let extrinsic = create_signed_extrinsic_metrics(
			node_id,
			storage_proof_time_ms,
			latency_ms,
			peer_count,
			failed_challenges_count,
			successful_challenges,
			total_challenges,
			uptime_minutes,
			total_minutes,
			consecutive_reliable_days,
			recent_downtime_hours,
			block_number,
			self.keystore.clone(),
			default_spec_version,
			&default_genesis_hash,
			&local_rpc_url,
		);
		let hex = hex::encode(&extrinsic);
		hex
	}

	fn submit_rankings(&self, params: rpc_core_weight::SubmitRankingsParams) -> String {
		let weights = params.weights;
		let all_nodes_ss58 = params.all_nodes_ss58;
		let node_ids = params.node_ids;
		let node_types = params.node_types;
		let default_spec_version = params.default_spec_version;
		let default_genesis_hash = params.default_genesis_hash;
		let local_rpc_url = params.local_rpc_url;
		let ranking_instance_id = params.ranking_instance_id;

		let extrinsic = create_signed_extrinsic_rankings(
			weights,
			all_nodes_ss58,
			node_ids,
			node_types,
			self.keystore.clone(),
			default_spec_version,
			&default_genesis_hash,
			&local_rpc_url,
			ranking_instance_id,
		);
		let hex = hex::encode(&extrinsic);
		hex
	}
}

/// Function to convert a hex string to &[u8; 32]
fn hex_to_array(hex: &str) -> Result<[u8; 32], &'static str> {
	// Remove the "0x" prefix if present
	let trimmed = hex.trim_start_matches("0x");

	// Decode hex string into a vector of bytes
	let decoded = hex::decode(trimmed).map_err(|_| "Failed to decode hex string")?;

	// Check the length to ensure it is 32 bytes
	if decoded.len() != 32 {
		return Err("Invalid length, must be 32 bytes");
	}
	// Convert to array
	let mut array = [0u8; 32];
	array.copy_from_slice(&decoded);
	Ok(array)
}

/// Creates a signed extrinsic
pub fn create_signed_extrinsic(
	netuid: u16,
	dests: Vec<u16>,
	weights: Vec<u16>,
	version_key: u32,
	keystore: KeystorePtr,
	default_spec_version: u32,
	default_genesis_hash: &str,
	finney_api_url: &str,
) -> Vec<u8> {
	let data = (netuid, dests, weights, version_key as u64);

	let rpc_url = finney_api_url;
	let bittensor_subtensor_module_pallet_index = 0x07u8;
	let bittensor_set_weigths_extrinsci_index = 0x00u8;

	let call_data = UnsignedExtrinsic {
		pallet_id: bittensor_subtensor_module_pallet_index,
		call_id: bittensor_set_weigths_extrinsci_index,
		call: data,
	};

	let transaction_version: u32 = 1; // Replace with actual transaction version
	let tip: u128 = 0; // Adjust as needed
	let mode: u8 = 0; // Adjust as needed
	let metadata_hash: Option<[u8; 32]> = None; // Adjust as needed

	// Try to fetch the spec version, fallback to default if it fails
	let spec_version = match fetch_spec_version(rpc_url) {
		Ok(version) => version,
		Err(e) => {
			eprintln!("Error fetching spec version: {}", e);
			default_spec_version
		},
	};

	// Try to fetch the genesis hash, fallback to default if it fails
	let fetched_hash = match fetch_genesis_hash(rpc_url) {
		Ok(hash) => hash,
		Err(e) => {
			eprintln!("Error fetching genesis hash: {}", e);
			default_genesis_hash.to_string()
		},
	};

	// Convert the hash to an array
	let genesis_hash_result = hex_to_array(&fetched_hash);

	let gensis_hash = genesis_hash_result.unwrap();
	let era_checkpoint = gensis_hash;

	// Create the additional parameters
	let additional_params =
		(spec_version, transaction_version, gensis_hash, era_checkpoint, metadata_hash);

	let key_type_id = KeyTypeId(*b"hips");

	let public_keys = keystore.sr25519_public_keys(key_type_id);

	if public_keys.is_empty() {
		log::error!("No public keys found for KeyTypeId: {:?}", key_type_id);
		return Vec::new();
	}

	let key = public_keys[0]; // Use the first key or implement custom selection logic

	let derived_public_key = key;
	let public_key_bytes: [u8; 32] = derived_public_key.into();

	let nonce: u64 = match fetch_nonce(&public_key_bytes, rpc_url) {
		Ok(value) => value,
		Err(_e) => 27,
	};

	// Create the extra parameters (without era and nonce)
	let extra = (Era::Immortal, Compact(nonce), Compact(tip), mode);

	let mut bytes = Vec::new();
	call_data.encode_to(&mut bytes);

	extra.encode_to(&mut bytes);
	additional_params.encode_to(&mut bytes);

	// Derive the AccountId (32-byte representation of the public key)
	let account_id: AccountId32 = AccountId32::from(derived_public_key);

	// Sign the payload
	let signature = if bytes.len() > 256 {
		let data_to_sign = sp_core_hashing::blake2_256(&bytes);

		let signature =
			match keystore.sign_with(key_type_id, CRYPTO_ID, &public_keys[0], &data_to_sign) {
				Ok(result) => result,
				Err(e) => {
					log::error!("Failed to sign the message: {:?}", e);
					return Vec::new();
				},
			};
		signature
	} else {
		let data_to_sign = bytes;
		let signature =
			match keystore.sign_with(key_type_id, CRYPTO_ID, &public_keys[0], &data_to_sign) {
				Ok(result) => result,
				Err(e) => {
					log::error!("Failed to sign the message: {:?}", e);
					return Vec::new();
				},
			};
		signature
	};

	// Convert Vec<u8> to schnorrkel::sign::Signature
	let sr25519_signature = match sr25519::Signature::try_from(signature.unwrap().as_slice()) {
		Ok(sig) => sig,
		Err(e) => {
			log::error!("Failed to parse signature into sr25519::Signature: {:?}", e);
			return Vec::new();
		},
	};

	// Use the converted signature in MultiSignature
	let multi_signature = MultiSignature::Sr25519(sr25519_signature);

	// Encode Extrinsic
	let extrinsic = {
		// Construct the full signed extrinsic
		let mut full_extrinsic = Vec::new();
		// Version byte (4 = signed, 1 = version)
		(0b10000000 + 4u8).encode_to(&mut full_extrinsic);

		let multi_address: MultiAddress<[u8; 32], u32> = MultiAddress::Id(account_id.into());
		multi_address.encode_to(&mut full_extrinsic);

		// let multi_signature = MultiSignature::Sr25519(signature);
		multi_signature.encode_to(&mut full_extrinsic);

		extra.encode_to(&mut full_extrinsic);
		call_data.encode_to(&mut full_extrinsic);

		let len = Compact(
			u32::try_from(full_extrinsic.len()).expect("extrinsic size expected to be <4GB"),
		);
		let mut encoded = Vec::new();
		len.encode_to(&mut encoded);
		encoded.extend(full_extrinsic);
		encoded
	};

	extrinsic
}

/// Creates a signed extrinsic
pub fn create_signed_extrinsic_rankings(
	weights: Vec<u16>,
	all_nodes_ss58: Vec<Vec<u8>>,
	node_ids: Vec<Vec<u8>>,
	node_types: Vec<NodeType>,
	keystore: KeystorePtr,
	default_spec_version: u32,
	default_genesis_hash: &str,
	local_rpc_url: &str,
	ranking_instance_id: u32,
) -> Vec<u8> {
	let data = (weights, all_nodes_ss58, node_ids, node_types);
	let rpc_url = local_rpc_url;

	let bittensor_subtensor_module_pallet_index = match ranking_instance_id {
		1 => 0x3Fu8,   // RankingStorage
		2 => 0x44u8,   // RankingCompute (commented out in original)
		3 => 0x46u8,   // RankingValidators
		4 => 0x47u8,   // RankingGpu (commented out in original)
		5 => 0x4Du8,   // RankingS3 (commented out in original)
		_ => 0x46u8,   // Default to RankingValidators
	};
	let bittensor_set_rankings_extrinsci_index = 0x00u8;

	let call_data = UnsignedRankingsExtrinsic {
		pallet_id: bittensor_subtensor_module_pallet_index,
		call_id: bittensor_set_rankings_extrinsci_index,
		call: data,
	};

	let transaction_version: u32 = 1; // Replace with actual transaction version
	let tip: u128 = 0; // Adjust as needed
	let mode: u8 = 0; // Adjust as needed
	let metadata_hash: Option<[u8; 32]> = None; // Adjust as needed

	// Try to fetch the spec version, fallback to default if it fails
	let spec_version = match fetch_spec_version(rpc_url) {
		Ok(version) => version,
		Err(e) => {
			eprintln!("Error fetching spec version: {}", e);
			default_spec_version
		},
	};

	// Try to fetch the genesis hash, fallback to default if it fails
	let fetched_hash = match fetch_genesis_hash(rpc_url) {
		Ok(hash) => hash,
		Err(e) => {
			eprintln!("Error fetching genesis hash: {}", e);
			default_genesis_hash.to_string()
		},
	};

	// Convert the hash to an array
	let genesis_hash_result = hex_to_array(&fetched_hash);

	let gensis_hash = genesis_hash_result.unwrap();
	let era_checkpoint = gensis_hash;

	// Create the additional parameters
	let additional_params =
		(spec_version, transaction_version, gensis_hash, era_checkpoint, metadata_hash);

	let key_type_id = KeyTypeId(*b"hips");

	let public_keys = keystore.sr25519_public_keys(key_type_id);

	if public_keys.is_empty() {
		log::error!("No public keys found for KeyTypeId: {:?}", key_type_id);
		return Vec::new();
	}

	let key = public_keys[0]; // Use the first key or implement custom selection logic

	let derived_public_key = key;
	let public_key_bytes: [u8; 32] = derived_public_key.into();

	let nonce: u64 = match fetch_nonce(&public_key_bytes, rpc_url) {
		Ok(value) => value,
		Err(_e) => 27,
	};

	// Create the extra parameters (without era and nonce)
	let extra = (Era::Immortal, Compact(nonce), Compact(tip), mode);

	let mut bytes = Vec::new();
	call_data.encode_to(&mut bytes);

	extra.encode_to(&mut bytes);
	additional_params.encode_to(&mut bytes);

	// Derive the AccountId (32-byte representation of the public key)
	let account_id: AccountId32 = AccountId32::from(derived_public_key);

	// Sign the payload
	let signature = if bytes.len() > 256 {
		let data_to_sign = sp_core_hashing::blake2_256(&bytes);

		let signature =
			match keystore.sign_with(key_type_id, CRYPTO_ID, &public_keys[0], &data_to_sign) {
				Ok(result) => result,
				Err(e) => {
					log::error!("Failed to sign the message: {:?}", e);
					return Vec::new();
				},
			};
		signature
	} else {
		let data_to_sign = bytes;
		let signature =
			match keystore.sign_with(key_type_id, CRYPTO_ID, &public_keys[0], &data_to_sign) {
				Ok(result) => result,
				Err(e) => {
					log::error!("Failed to sign the message: {:?}", e);
					return Vec::new();
				},
			};
		signature
	};

	// Convert Vec<u8> to schnorrkel::sign::Signature
	let sr25519_signature = match sr25519::Signature::try_from(signature.unwrap().as_slice()) {
		Ok(sig) => sig,
		Err(e) => {
			log::error!("Failed to parse signature into sr25519::Signature: {:?}", e);
			return Vec::new();
		},
	};

	// Use the converted signature in MultiSignature
	let multi_signature = MultiSignature::Sr25519(sr25519_signature);

	// Encode Extrinsic
	let extrinsic = {
		// Construct the full signed extrinsic
		let mut full_extrinsic = Vec::new();
		// Version byte (4 = signed, 1 = version)
		(0b10000000 + 4u8).encode_to(&mut full_extrinsic);

		let multi_address: MultiAddress<[u8; 32], u32> = MultiAddress::Id(account_id.into());
		multi_address.encode_to(&mut full_extrinsic);

		// let multi_signature = MultiSignature::Sr25519(signature);
		multi_signature.encode_to(&mut full_extrinsic);

		extra.encode_to(&mut full_extrinsic);
		call_data.encode_to(&mut full_extrinsic);

		let len = Compact(
			u32::try_from(full_extrinsic.len()).expect("extrinsic size expected to be <4GB"),
		);
		let mut encoded = Vec::new();
		len.encode_to(&mut encoded);
		encoded.extend(full_extrinsic);
		encoded
	};

	extrinsic
}

/// Creates a signed extrinsic
pub fn create_signed_extrinsic_hardware(
	node_id: Vec<u8>,
	system_info: SystemInfo,
	keystore: KeystorePtr,
	default_spec_version: u32,
	default_genesis_hash: &str,
	local_rpc_url: &str,
) -> Vec<u8> {

	let data = (node_id, system_info);
	let rpc_url = local_rpc_url;
	let bittensor_subtensor_module_pallet_index = 0x36u8;
	let bittensor_set_weigths_extrinsci_index = 0x00u8;

	let call_data = UnsignedHardwareExtrinsic {
		pallet_id: bittensor_subtensor_module_pallet_index,
		call_id: bittensor_set_weigths_extrinsci_index,
		call: data,
	};

	let transaction_version: u32 = 1; // Replace with actual transaction version
	let tip: u128 = 0; // Adjust as needed
	let mode: u8 = 0; // Adjust as needed
	let metadata_hash: Option<[u8; 32]> = None; // Adjust as needed

	// Try to fetch the spec version, fallback to default if it fails
	let spec_version = match fetch_spec_version(rpc_url) {
		Ok(version) => version,
		Err(e) => {
			eprintln!("Error fetching spec version: {}", e);
			default_spec_version
		},
	};

	// Try to fetch the genesis hash, fallback to default if it fails
	let fetched_hash = match fetch_genesis_hash(rpc_url) {
		Ok(hash) => hash,
		Err(e) => {
			eprintln!("Error fetching genesis hash: {}", e);
			default_genesis_hash.to_string()
		},
	};

	// Convert the hash to an array
	let genesis_hash_result = hex_to_array(&fetched_hash);

	let gensis_hash = genesis_hash_result.unwrap();
	let era_checkpoint = gensis_hash;

	// Create the additional parameters
	let additional_params =
		(spec_version, transaction_version, gensis_hash, era_checkpoint, metadata_hash);

	let key_type_id = KeyTypeId(*b"hips");

	let public_keys = keystore.sr25519_public_keys(key_type_id);

	if public_keys.is_empty() {
		log::error!("No public keys found for KeyTypeId: {:?}", key_type_id);
		return Vec::new();
	}

	let key = public_keys[0]; // Use the first key or implement custom selection logic

	let derived_public_key = key;
	let public_key_bytes: [u8; 32] = derived_public_key.into();

	let nonce: u64 = match fetch_nonce(&public_key_bytes, rpc_url) {
		Ok(value) => value,
		Err(_e) => 27,
	};

	// Create the extra parameters (without era and nonce)
	let extra = (Era::Immortal, Compact(nonce), Compact(tip), mode);

	let mut bytes = Vec::new();
	call_data.encode_to(&mut bytes);

	extra.encode_to(&mut bytes);
	additional_params.encode_to(&mut bytes);

	// Derive the AccountId (32-byte representation of the public key)
	let account_id: AccountId32 = AccountId32::from(derived_public_key);

	// Sign the payload
	let signature = if bytes.len() > 256 {
		let data_to_sign = sp_core_hashing::blake2_256(&bytes);

		let signature =
			match keystore.sign_with(key_type_id, CRYPTO_ID, &public_keys[0], &data_to_sign) {
				Ok(result) => result,
				Err(e) => {
					log::error!("Failed to sign the message: {:?}", e);
					return Vec::new();
				},
			};
		signature
	} else {
		let data_to_sign = bytes;
		let signature =
			match keystore.sign_with(key_type_id, CRYPTO_ID, &public_keys[0], &data_to_sign) {
				Ok(result) => result,
				Err(e) => {
					log::error!("Failed to sign the message: {:?}", e);
					return Vec::new();
				},
			};
		signature
	};

	// Convert Vec<u8> to schnorrkel::sign::Signature
	let sr25519_signature = match sr25519::Signature::try_from(signature.unwrap().as_slice()) {
		Ok(sig) => sig,
		Err(e) => {
			log::error!("Failed to parse signature into sr25519::Signature: {:?}", e);
			return Vec::new();
		},
	};

	// Use the converted signature in MultiSignature
	let multi_signature = MultiSignature::Sr25519(sr25519_signature);

	// Encode Extrinsic
	let extrinsic = {
		// Construct the full signed extrinsic
		let mut full_extrinsic = Vec::new();
		// Version byte (4 = signed, 1 = version)
		(0b10000000 + 4u8).encode_to(&mut full_extrinsic);

		let multi_address: MultiAddress<[u8; 32], u32> = MultiAddress::Id(account_id.into());
		multi_address.encode_to(&mut full_extrinsic);

		// let multi_signature = MultiSignature::Sr25519(signature);
		multi_signature.encode_to(&mut full_extrinsic);

		extra.encode_to(&mut full_extrinsic);
		call_data.encode_to(&mut full_extrinsic);

		let len = Compact(
			u32::try_from(full_extrinsic.len()).expect("extrinsic size expected to be <4GB"),
		);
		let mut encoded = Vec::new();
		len.encode_to(&mut encoded);
		encoded.extend(full_extrinsic);
		encoded
	};

	extrinsic
}


/// Creates a signed extrinsic for metrics
pub fn create_signed_extrinsic_metrics(
	node_id: Vec<u8>,
    storage_proof_time_ms: u32,
    latency_ms: u32,
    peer_count: u32,
    failed_challenges_count: u32,
    successful_challenges: u32,
    total_challenges: u32,
    uptime_minutes: u32,
    total_minutes: u32,
    consecutive_reliable_days: u32,
    recent_downtime_hours: u32,
    block_number: u32,
	keystore: KeystorePtr,
	default_spec_version: u32,
	default_genesis_hash: &str,
	local_rpc_url: &str,
) -> Vec<u8> {
	let data = (node_id, storage_proof_time_ms, latency_ms, peer_count, failed_challenges_count, successful_challenges, total_challenges, uptime_minutes, total_minutes, consecutive_reliable_days, recent_downtime_hours, block_number);
	let rpc_url = local_rpc_url;
	let bittensor_subtensor_module_pallet_index = 0x36u8;
	let bittensor_set_metrics_extrinsci_index = 0x01u8;

	let call_data = UnsignedMetricsExtrinsic {
		pallet_id: bittensor_subtensor_module_pallet_index,
		call_id: bittensor_set_metrics_extrinsci_index,
		call: data,
	};

	let transaction_version: u32 = 1; // Replace with actual transaction version
	let tip: u128 = 0; // Adjust as needed
	let mode: u8 = 0; // Adjust as needed
	let metadata_hash: Option<[u8; 32]> = None; // Adjust as needed

	// Try to fetch the spec version, fallback to default if it fails
	let spec_version = match fetch_spec_version(rpc_url) {
		Ok(version) => version,
		Err(e) => {
			eprintln!("Error fetching spec version: {}", e);
			default_spec_version
		},
	};

	// Try to fetch the genesis hash, fallback to default if it fails
	let fetched_hash = match fetch_genesis_hash(rpc_url) {
		Ok(hash) => hash,
		Err(e) => {
			eprintln!("Error fetching genesis hash: {}", e);
			default_genesis_hash.to_string()
		},
	};

	// Convert the hash to an array
	let genesis_hash_result = hex_to_array(&fetched_hash);

	let gensis_hash = genesis_hash_result.unwrap();
	let era_checkpoint = gensis_hash;

	// Create the additional parameters
	let additional_params =
		(spec_version, transaction_version, gensis_hash, era_checkpoint, metadata_hash);

	let key_type_id = KeyTypeId(*b"hips");

	let public_keys = keystore.sr25519_public_keys(key_type_id);

	if public_keys.is_empty() {
		log::error!("No public keys found for KeyTypeId: {:?}", key_type_id);
		return Vec::new();
	}

	let key = public_keys[0]; // Use the first key or implement custom selection logic

	let derived_public_key = key;
	let public_key_bytes: [u8; 32] = derived_public_key.into();

	let nonce: u64 = match fetch_nonce(&public_key_bytes, rpc_url) {
		Ok(value) => value,
		Err(_e) => 27,
	};

	// Create the extra parameters (without era and nonce)
	let extra = (Era::Immortal, Compact(nonce), Compact(tip), mode);

	let mut bytes = Vec::new();
	call_data.encode_to(&mut bytes);

	extra.encode_to(&mut bytes);
	additional_params.encode_to(&mut bytes);

	// Derive the AccountId (32-byte representation of the public key)
	let account_id: AccountId32 = AccountId32::from(derived_public_key);

	// Sign the payload
	let signature = if bytes.len() > 256 {
		let data_to_sign = sp_core_hashing::blake2_256(&bytes);

		let signature =
			match keystore.sign_with(key_type_id, CRYPTO_ID, &public_keys[0], &data_to_sign) {
				Ok(result) => result,
				Err(e) => {
					log::error!("Failed to sign the message: {:?}", e);
					return Vec::new();
				},
			};
		signature
	} else {
		let data_to_sign = bytes;
		let signature =
			match keystore.sign_with(key_type_id, CRYPTO_ID, &public_keys[0], &data_to_sign) {
				Ok(result) => result,
				Err(e) => {
					log::error!("Failed to sign the message: {:?}", e);
					return Vec::new();
				},
			};
		signature
	};

	// Convert Vec<u8> to schnorrkel::sign::Signature
	let sr25519_signature = match sr25519::Signature::try_from(signature.unwrap().as_slice()) {
		Ok(sig) => sig,
		Err(e) => {
			log::error!("Failed to parse signature into sr25519::Signature: {:?}", e);
			return Vec::new();
		},
	};

	// Use the converted signature in MultiSignature
	let multi_signature = MultiSignature::Sr25519(sr25519_signature);

	// Encode Extrinsic
	let extrinsic = {
		// Construct the full signed extrinsic
		let mut full_extrinsic = Vec::new();
		// Version byte (4 = signed, 1 = version)
		(0b10000000 + 4u8).encode_to(&mut full_extrinsic);

		let multi_address: MultiAddress<[u8; 32], u32> = MultiAddress::Id(account_id.into());
		multi_address.encode_to(&mut full_extrinsic);

		// let multi_signature = MultiSignature::Sr25519(signature);
		multi_signature.encode_to(&mut full_extrinsic);

		extra.encode_to(&mut full_extrinsic);
		call_data.encode_to(&mut full_extrinsic);

		let len = Compact(
			u32::try_from(full_extrinsic.len()).expect("extrinsic size expected to be <4GB"),
		);
		let mut encoded = Vec::new();
		len.encode_to(&mut encoded);
		encoded.extend(full_extrinsic);
		encoded
	};

	extrinsic
}

// Define the TransactionData type as a tuple
pub type TransactionData = (u16, Vec<u16>, Vec<u16>, u64);

#[derive(Encode, Decode, RuntimeDebug)]
pub struct UnsignedExtrinsic {
	pub pallet_id: u8,
	pub call_id: u8,
	pub call: TransactionData,
}

// Define the HardwareTransactionData type as a tuple
pub type HardwareTransactionData = (Vec<u8>, SystemInfo);

#[derive(Encode, Decode, RuntimeDebug)]
pub struct UnsignedHardwareExtrinsic {
	pub pallet_id: u8,
	pub call_id: u8,
	pub call: HardwareTransactionData,
}

// Define the MetricsTransactionData type as a tuple
pub type MetricsTransactionData = (Vec<u8>, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32, u32);

#[derive(Encode, Decode, RuntimeDebug)]
pub struct UnsignedMetricsExtrinsic {
	pub pallet_id: u8,
	pub call_id: u8,
	pub call: MetricsTransactionData,
}

// Define the RankingsTransactionData type as a tuple
pub type RankingsTransactionData = (
	Vec<u16>,
	Vec<Vec<u8>>,
	Vec<Vec<u8>>,
	Vec<NodeType>,
);

#[derive(Encode, Decode, RuntimeDebug)]
pub struct UnsignedRankingsExtrinsic {
	pub pallet_id: u8,
	pub call_id: u8,
	pub call: RankingsTransactionData,
}

// Function to fetch the genesis hash from the RPC
fn fetch_genesis_hash(rpc_url: &str) -> Result<String, Box<dyn std::error::Error>> {
	let client = reqwest::blocking::Client::new();

	// Prepare the request body
	let request_body = json!({
		"jsonrpc": "2.0",
		"method": "chain_getBlockHash",
		"params": [0],
		"id": 1
	});

	// Send the request
	let response: serde_json::Value = client.post(rpc_url).json(&request_body).send()?.json()?;

	// Extract the genesis hash from the response
	let genesis_hash = response["result"].as_str().unwrap_or_default().to_string();

	Ok(genesis_hash)
}

/// Function to fetch nonce from remote node
fn fetch_nonce(account: &[u8; 32], remote_url: &str) -> Result<u64, &'static str> {
	// Convert account to SS58 address format
	let address = sp_core::crypto::AccountId32::new(*account);
	let ss58_address = address.to_ss58check();

	// Construct RPC request to get nonce using SS58 address
	let rpc_payload = format!(
		r#"{{"jsonrpc":"2.0","id":1,"method":"system_accountNextIndex","params":["{}"]}}"#,
		ss58_address
	);

	// Create a blocking HTTP client
	let client = Client::new();

	// Send the request
	let response = client
		.post(remote_url)
		.header("Content-Type", "application/json")
		.body(rpc_payload)
		.send()
		.map_err(|_| {
			log::error!("Error sending nonce request");
			"Error sending nonce request"
		})?;

	if response.status() != 200 {
		log::error!("HTTP request failed with status: {}", response.status());
		return Err("HTTP request failed");
	}

	let body = response.text().map_err(|_| "Failed to read response body")?;

	// Parse JSON response to get nonce
	let val = lite_json::parse_json(&body);

	let nonce = match val {
		Ok(JsonValue::Object(obj)) => {
			// Find the "result" field in the response
			match obj.into_iter().find(|(k, _)| k.iter().copied().eq("result".chars())) {
				Some((_, value)) => {
					// Extract the number value
					match value {
						JsonValue::Number(number) => {
							let nonce = number.integer as u64;
							nonce
						},
						_ => {
							log::error!(
								"Result field is not a number, got unexpected JSON value type"
							);
							return Err("Result field is not a number");
						},
					}
				},
				None => {
					log::error!("No 'result' field found in response");
					return Err("No 'result' field found in response");
				},
			}
		},
		Ok(_) => {
			log::error!("Expected JSON object in response");
			return Err("Expected JSON object in response");
		},
		Err(_) => {
			log::error!("Failed to parse JSON");
			return Err("Failed to parse JSON");
		},
	};

	Ok(nonce)
}

// Function to fetch the spec version from the RPC
fn fetch_spec_version(rpc_url: &str) -> Result<u32, Box<dyn std::error::Error>> {
	let client = reqwest::blocking::Client::new();

	// Prepare the request body
	let request_body = json!({
		"jsonrpc": "2.0",
		"method": "state_getRuntimeVersion",
		"params": [],
		"id": 1
	});

	// Send the request
	let response: serde_json::Value = client.post(rpc_url).json(&request_body).send()?.json()?;

	// Extract the spec version from the response
	let spec_version = response["result"]["specVersion"].as_u64().unwrap_or(247) as u32;

	Ok(spec_version)
}


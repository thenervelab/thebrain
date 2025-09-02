// This file is part of The Brain.
// Copyright (C) 2022-2024 The Nerve Lab
//
// Licensed under the MIT License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

#![allow(clippy::type_complexity)]

// use crate::mainnet_fixtures::get_bootnodes;
use hippius_mainnet_runtime::{AccountId, Balance, Perbill, StakerStatus, UNIT, WASM_BINARY};
use hippius_primitives::MAINNET_LOCAL_SS58_PREFIX;
use pallet_im_online::sr25519::AuthorityId as ImOnlineId;
use sc_consensus_grandpa::AuthorityId as GrandpaId;
use sc_service::ChainType;
use sp_consensus_babe::AuthorityId as BabeId;
use sp_core::{crypto::Ss58Codec, ed25519, sr25519, H160, U256};
use std::{collections::BTreeMap, str::FromStr};
// use sc_network::config::MultiaddrWithPeerId;
// use hex::FromHex;

/// Specialized `ChainSpec`. This is a specialization of the general Substrate ChainSpec type.
pub type ChainSpec = sc_service::GenericChainSpec;

pub const ENDOWMENT: Balance = 100 * UNIT;

// Our validator's sr25519 key for BABE
const VALIDATOR_SR25519: &str = "5G1Qj93Fy22grpiGKq6BEvqqmS2HVRs3jaEdMhq9absQzs6g";
// Our validator's ed25519 key for GRANDPA
const VALIDATOR_ED25519: &str = "5CnJrbg2PTeL3jkKc8ozaGpjKMerXx1M1Y4uc8ByNxBceauD";


// Our validator's sr25519 key for BABE
const TESTNET_VALIDATOR_SR25519: &str = "5CP9wzk9G3kMdJmNyAsWGWVWDQE7Goe1dUvKtMv51EoXs563";
// Our validator's ed25519 key for GRANDPA
const TESTNET_VALIDATOR_ED25519: &str = "5D7iUQJnrtiuuwj3bHebP8oNPAk4jR7V76EV9JKmsscwrD4N";

/// Helper function to get account from SS58 string
fn get_account_from_ss58(ss58: &str) -> AccountId {
	AccountId::from_ss58check_with_version(ss58).expect("Invalid SS58 address").0
}

/// Helper function to get sr25519 public key from SS58 string
fn get_sr25519_from_ss58(ss58: &str) -> sr25519::Public {
	sr25519::Public::from_ss58check_with_version(ss58)
		.expect("Invalid SS58 address")
		.0
}

/// Helper function to get ed25519 public key from SS58 string
fn get_ed25519_from_ss58(ss58: &str) -> ed25519::Public {
	ed25519::Public::from_ss58check_with_version(ss58)
		.expect("Invalid SS58 address")
		.0
}

/// Generate authority keys for our validator
pub fn get_authority_keys() -> (AccountId, BabeId, GrandpaId, ImOnlineId) {
	let account = get_account_from_ss58(VALIDATOR_SR25519);
	let sr25519_key = get_sr25519_from_ss58(VALIDATOR_SR25519);
	let ed25519_key = get_ed25519_from_ss58(VALIDATOR_ED25519);

	(account, sr25519_key.into(), ed25519_key.into(), sr25519_key.into())
}

// Sudo account
const SUDO_ACCOUNT: &str = "5Cf8Sx31MqeyJMwvF7VAE89woWavsDXNxZY1ci1wmCiyHjX3";

/// Get the sudo account
fn get_sudo_account() -> AccountId {
	get_account_from_ss58(SUDO_ACCOUNT)
}

/// Generate authority keys for our validator
pub fn get_testnet_authority_keys() -> (AccountId, BabeId, GrandpaId, ImOnlineId) {
	let account = get_account_from_ss58(TESTNET_VALIDATOR_SR25519);
	let sr25519_key = get_sr25519_from_ss58(TESTNET_VALIDATOR_SR25519);
	let ed25519_key = get_ed25519_from_ss58(TESTNET_VALIDATOR_ED25519);

	(account, sr25519_key.into(), ed25519_key.into(), sr25519_key.into())
}

// Sudo account
const TESTNET_SUDO_ACCOUNT: &str = "5CP9wzk9G3kMdJmNyAsWGWVWDQE7Goe1dUvKtMv51EoXs563";

/// Get the sudo account
fn get_testnet_sudo_account() -> AccountId {
	get_account_from_ss58(TESTNET_SUDO_ACCOUNT)
}


/// Generate the session keys from individual elements.
fn generate_session_keys(
	babe: BabeId,
	grandpa: GrandpaId,
	im_online: ImOnlineId,
) -> hippius_mainnet_runtime::opaque::SessionKeys {
	hippius_mainnet_runtime::opaque::SessionKeys { babe, grandpa, im_online }
}

pub fn local_benchmarking_config(chain_id: u64) -> Result<ChainSpec, String> {
	let mut properties = sc_chain_spec::Properties::new();
	properties.insert("tokenSymbol".into(), "hALPHA".into());
	properties.insert("tokenDecimals".into(), 18u32.into());
	properties.insert("ss58Format".into(), 42.into());

	let authority = get_authority_keys();
	let account_id = authority.0.clone();

	Ok(ChainSpec::builder(WASM_BINARY.expect("WASM not available"), Default::default())
		.with_name("Local Mainnet")
		.with_id("local_mainnet")
		.with_chain_type(ChainType::Local)
		.with_properties(properties)
		.with_genesis_config_patch(mainnet_genesis(
			// Initial PoA authorities
			vec![authority.clone()],
			// Pre-funded accounts
			vec![
				(account_id.clone(), ENDOWMENT),
				(
					// Convert sudo account to AccountId32
					sp_core::sr25519::Public::from_ss58check(SUDO_ACCOUNT)
						.expect("Invalid SS58 address")
						.into(),
					// Add a substantial endowment, e.g., 1 million tokens
					ENDOWMENT * 9,
				),
			],
			// Sudo account
			get_sudo_account(),
			chain_id,
			vec![],
			vec![],
		))
		.build())
}

pub fn hippius_testnet_config(chain_id: u64) -> Result<ChainSpec, String> {
	let mut properties = sc_chain_spec::Properties::new();
	properties.insert("tokenSymbol".into(), "thALPHA".into());
	properties.insert("tokenDecimals".into(), 18u32.into());
	properties.insert("ss58Format".into(), 42.into());

	let authority = get_testnet_authority_keys();
	let _account_id = authority.0.clone();

	Ok(ChainSpec::builder(WASM_BINARY.expect("WASM not available"), Default::default())
		.with_name("Hippius Testnet")
		.with_id("local_testnet")
		.with_chain_type(ChainType::Live)
		.with_properties(properties)
		.with_genesis_config_patch(testnet_genesis(
			// Initial validators
			vec![authority.clone()],
			// Endowed accounts
			vec![
				// (account_id.clone(), ENDOWMENT),
				(
					// Convert sudo account to AccountId32
					sp_core::sr25519::Public::from_ss58check(TESTNET_SUDO_ACCOUNT)
						.expect("Invalid SS58 address")
						.into(),
					// Add a substantial endowment, e.g., 1 million tokens
					ENDOWMENT * 9,
				),
			],
			// Sudo account
			get_testnet_sudo_account(),
			// EVM chain ID
			chain_id,
			vec![], // Temporarily disabled vesting for initial setup
			vec![], // endowed evm accounts
		))
		.build())
}

pub fn local_mainnet_config(chain_id: u64) -> Result<ChainSpec, String> {
	let mut properties = sc_chain_spec::Properties::new();
	properties.insert("tokenSymbol".into(), "hALPHA".into());
	properties.insert("tokenDecimals".into(), 18u32.into());
	properties.insert("ss58Format".into(), MAINNET_LOCAL_SS58_PREFIX.into());

	let authority = get_authority_keys();
	let account_id = authority.0.clone();

	Ok(ChainSpec::builder(WASM_BINARY.expect("WASM not available"), Default::default())
		.with_name("Local Mainnet")
		.with_id("local_mainnet")
		.with_chain_type(ChainType::Local)
		.with_properties(properties)
		.with_genesis_config_patch(mainnet_genesis(
			// Initial PoA authorities
			vec![authority.clone()],
			// Pre-funded accounts
			vec![
				(account_id.clone(), ENDOWMENT),
				(
					// Convert sudo account to AccountId32
					sp_core::sr25519::Public::from_ss58check(SUDO_ACCOUNT)
						.expect("Invalid SS58 address")
						.into(),
					// Add a substantial endowment, e.g., 1 million tokens
					ENDOWMENT * 9,
				),
			],
			// Sudo account
			get_sudo_account(),
			chain_id,
			vec![],
			vec![],
		))
		.build())
}

pub fn hippius_mainnet_config(_chain_id: u64) -> Result<ChainSpec, String> {
    // Load the custom chainspec from JSON file
    let custom_chainspec_bytes = include_bytes!("../../../chainspecs/mainnet/hippius_raw_bak.json");
    let mut chain_spec_json: serde_json::Value = serde_json::from_slice(custom_chainspec_bytes)
        .map_err(|e| e.to_string())?;

		println!(
			"Switching runtime: Code, Block number = 68900"
		);

		let code_substitute_68900 = include_bytes!("hippius_mainnet_runtime.compact.compressed.wasm");
		sc_chain_spec::set_code_substitute_in_json_chain_spec(
			&mut chain_spec_json,
			code_substitute_68900,
			68899,
		);

    // // Load and set the code substitute
    // let code_substitute_68900_hex = include_bytes!("code_substitute_68900.txt");


    // sc_chain_spec::set_code_substitute_in_json_chain_spec(
    //     &mut chain_spec_json,
    //     Vec::from_hex(code_substitute_68900_hex)
    //         .map_err(|e| e.to_string())?
    //         .as_slice(),
    //     68899,
    // );

    // Convert the modified JSON back to a ChainSpec
    let chain_spec_bytes = chain_spec_json.to_string().into_bytes();
    ChainSpec::from_json_bytes(chain_spec_bytes).map_err(|e| e.to_string())
}

/// Configure initial storage state for FRAME modules.
#[allow(clippy::too_many_arguments)]
fn mainnet_genesis(
	initial_authorities: Vec<(AccountId, BabeId, GrandpaId, ImOnlineId)>,
	endowed_accounts: Vec<(AccountId, Balance)>,
	root_key: AccountId,
	chain_id: u64,
	_genesis_non_airdrop: Vec<(u128, u64, u64, u128)>,
	genesis_evm_distribution: Vec<(H160, fp_evm::GenesisAccount)>,
) -> serde_json::Value {
	// stakers: all validators and nominators.
	let stakers: Vec<(AccountId, AccountId, Balance, StakerStatus<AccountId>)> =
		initial_authorities
			.iter()
			.map(|x| (x.0.clone(), x.0.clone(), 3 * UNIT, StakerStatus::<AccountId>::Validator))
			.collect::<Vec<_>>();

	let evm_accounts = {
		let mut map = BTreeMap::new();

		for (address, account) in genesis_evm_distribution {
			map.insert(address, account);
		}

		let fully_loaded_accounts = get_fully_funded_accounts_for([]);

		map.extend(fully_loaded_accounts);

		map
	};

	let council_members: Vec<AccountId> = vec![root_key.clone()];
	serde_json::json!({
		"balances": {
			"balances": endowed_accounts.to_vec(),
		},
		"sudo": {
			"key": Some(root_key)
		},
		"session": {
			"keys": initial_authorities
				.iter()
				.map(|x| {
					(
						x.0.clone(),
						x.0.clone(),
						generate_session_keys(x.1.clone(), x.2.clone(), x.3.clone()),
					)
				})
				.collect::<Vec<_>>()
		},
		"staking": {
			"validatorCount": initial_authorities.len() as u32,
			"minimumValidatorCount": 1,
			"invulnerables": initial_authorities.iter().map(|x| x.0.clone()).collect::<Vec<_>>(),
			"slashRewardFraction": Perbill::from_percent(10),
			"stakers": stakers,
		},
		"council": {
			"members": council_members,
		},
		"babe": {
			"epochConfig": hippius_mainnet_runtime::BABE_GENESIS_EPOCH_CONFIG,
		},
		"evm": {
			"accounts": evm_accounts
		},
		"evmChainId": { "chainId": chain_id },
		// Vesting configuration is temporarily disabled
		"vesting": {
			"vesting": Vec::<(AccountId, u64, u64, u128)>::new(),
		}
	})
}

fn generate_fully_loaded_evm_account_for(acc: &str) -> (H160, fp_evm::GenesisAccount) {
	(
		H160::from_str(acc).expect("internal H160 is valid; qed"),
		fp_evm::GenesisAccount {
			balance: U256::zero(),
			code: Default::default(),
			nonce: Default::default(),
			storage: Default::default(),
		},
	)
}

fn get_fully_funded_accounts_for<'a, T: AsRef<[&'a str]>>(
	addrs: T,
) -> Vec<(H160, fp_evm::GenesisAccount)> {
	addrs
		.as_ref()
		.iter()
		.map(|acc| generate_fully_loaded_evm_account_for(acc))
		.collect()
}


/// Configure initial storage state for FRAME modules.
#[allow(clippy::too_many_arguments)]
fn testnet_genesis(
    initial_authorities: Vec<(AccountId, BabeId, GrandpaId, ImOnlineId)>,
    endowed_accounts: Vec<(AccountId, Balance)>,
    root_key: AccountId,
    chain_id: u64,
    _genesis_non_airdrop: Vec<(u128, u64, u64, u128)>,
    genesis_evm_distribution: Vec<(H160, fp_evm::GenesisAccount)>,
) -> serde_json::Value {
    let stakers: Vec<(AccountId, AccountId, Balance, StakerStatus<AccountId>)> =
        initial_authorities
            .iter()
            .map(|x| (x.0.clone(), x.0.clone(), 3 * UNIT, StakerStatus::<AccountId>::Validator))
            .collect::<Vec<_>>();

    let evm_accounts = {
        let mut map = BTreeMap::new();
        for (address, account) in genesis_evm_distribution {
            map.insert(address, account);
        }
        let fully_loaded_accounts = get_fully_funded_accounts_for([]);
        map.extend(fully_loaded_accounts);
        map
    };

    let council_members: Vec<AccountId> = vec![root_key.clone()];
    
    // Generate session keys for both session.keys and session.nextKeys
    let session_keys: Vec<(AccountId, AccountId, hippius_mainnet_runtime::opaque::SessionKeys)> = initial_authorities
        .iter()
        .map(|x| {
            (
                x.0.clone(),
                x.0.clone(),
                generate_session_keys(x.1.clone(), x.2.clone(), x.3.clone()),
            )
        })
        .collect::<Vec<_>>();

    serde_json::json!({
        "balances": {
            "balances": endowed_accounts.to_vec(),
        },
        "sudo": {
            "key": Some(root_key)
        },
        "session": {
            "keys": session_keys.clone(),
        },
        "staking": {
            "validatorCount": initial_authorities.len() as u32,
            "minimumValidatorCount": 1,
            "invulnerables": initial_authorities.iter().map(|x| x.0.clone()).collect::<Vec<_>>(),
            "slashRewardFraction": Perbill::from_percent(10),
            "stakers": stakers,
        },
        "council": {
            "members": council_members,
        },
        "babe": {
            "epochConfig": hippius_mainnet_runtime::BABE_GENESIS_EPOCH_CONFIG,
        },
        "evm": {
            "accounts": evm_accounts
        },
        "evmChainId": { "chainId": chain_id },
        "vesting": {
            "vesting": Vec::<(AccountId, u64, u64, u128)>::new(),
        }
    })
}
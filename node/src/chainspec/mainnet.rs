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

// use crate::mainnet_fixtures::{get_bootnodes, get_initial_authorities, get_root_key};
use hex_literal::hex;
use pallet_im_online::sr25519::AuthorityId as ImOnlineId;
use parity_scale_codec::alloc::collections::BTreeMap;
use sc_consensus_grandpa::AuthorityId as GrandpaId;
use sc_service::ChainType;
use sp_consensus_babe::AuthorityId as BabeId;
// use sp_core::{ Pair, Public};0
use sp_core::H160;
// use sp_runtime::traits::{  Verify};
// use hippius_primitives::types::Signature;
use hippius_runtime::{
	AccountId, Balance, Perbill, StakerStatus,  UNIT,
	WASM_BINARY,
};
use crate::chainspec::testnet::get_authority_keys;
use crate::chainspec::testnet::ENDOWMENT;

/// Specialized `ChainSpec`. This is a specialization of the general Substrate ChainSpec type.
pub type ChainSpec = sc_service::GenericChainSpec;

// /// Generate a crypto pair from seed.
// pub fn get_from_seed<TPublic: Public>(seed: &str) -> <TPublic::Pair as Pair>::Public {
// 	TPublic::Pair::from_string(&format!("//{seed}"), None)
// 		.expect("static values are valid; qed")
// 		.public()
// }

// type AccountPublic = <Signature as Verify>::Signer;

/// Generate an account ID from seed.
// pub fn get_account_id_from_seed<TPublic: Public>(seed: &str) -> AccountId
// where
// 	AccountPublic: From<<TPublic::Pair as Pair>::Public>,
// {
// 	AccountPublic::from(get_from_seed::<TPublic>(seed)).into_account()
// }

// /// Generate an babe authority key.
// pub fn authority_keys_from_seed(stash: &str) -> (AccountId, BabeId, GrandpaId, ImOnlineId) {
// 	(
// 		get_account_id_from_seed::<sr25519::Public>(stash),
// 		get_from_seed::<BabeId>(stash),
// 		get_from_seed::<GrandpaId>(stash),
// 		get_from_seed::<ImOnlineId>(stash),
// 	)
// }

/// Generate the session keys from individual elements.
///
/// The input must be a tuple of individual keys (a single arg for now since we
/// have just one key).
fn generate_session_keys(
	babe: BabeId,
	grandpa: GrandpaId,
	im_online: ImOnlineId,
) -> hippius_runtime::opaque::SessionKeys {
	hippius_runtime::opaque::SessionKeys { babe, grandpa, im_online }
}

pub fn local_mainnet_config(chain_id: u64) -> Result<ChainSpec, String> {
	let mut properties = sc_chain_spec::Properties::new();
	properties.insert("tokenSymbol".into(), "HIP".into());
	properties.insert("tokenDecimals".into(), 18u32.into());
	properties.insert("ss58Format".into(), hippius_primitives::MAINNET_SS58_PREFIX.into());

	let authority = get_authority_keys();
    let account_id = authority.0.clone();

	// let endowment: Balance = 10_000_000 * UNIT;
	Ok(ChainSpec::builder(WASM_BINARY.expect("WASM not available"), Default::default())
		.with_name("Local Hippius Mainnet")
		.with_id("local-hippius-mainnet")
		.with_chain_type(ChainType::Local)
		.with_properties(properties)
		.with_genesis_config_patch(mainnet_genesis(
			// Initial PoA authorities
			vec![authority.clone()],
			// Endowed accounts
			vec![(account_id.clone(), ENDOWMENT)],
			// Sudo account
			account_id.clone(),
			// EVM chain ID
			chain_id,
			vec![

			],
			// endowed evm accounts
			vec![],
		))
		.build())
}

pub fn hippius_mainnet_config(chain_id: u64) -> Result<ChainSpec, String> {
	// let _boot_nodes = get_bootnodes();
	let mut properties = sc_chain_spec::Properties::new();
	properties.insert("tokenSymbol".into(), "HIP".into());
	properties.insert("tokenDecimals".into(), 18u32.into());
	properties.insert("ss58Format".into(), 42.into());

	let authority = get_authority_keys();
    let account_id = authority.0.clone();

	Ok(ChainSpec::builder(WASM_BINARY.expect("WASM not available"), Default::default())
		.with_name("Hippius Mainnet")
		.with_id("hippius-mainnet")
		.with_chain_type(ChainType::Live)
		.with_properties(properties)
		.with_genesis_config_patch(mainnet_genesis(
			// Initial PoA authorities
			vec![authority.clone()],
			// Endowed accounts
			vec![(account_id.clone(), ENDOWMENT)],
			// Sudo account
			account_id.clone(),
			// EVM chain ID
			chain_id,

			vec![

			],
			// endowed evm accounts
			vec![],
		))
		.build())
}

#[allow(clippy::too_many_arguments)]
fn mainnet_genesis(
	initial_authorities: Vec<(AccountId, BabeId, GrandpaId, ImOnlineId)>,
	endowed_accounts: Vec<(AccountId, Balance)>,
	root_key: AccountId,
	chain_id: u64,
	_genesis_non_airdrop: Vec<( u128, u64, u64, u128)>,
	genesis_evm_distribution: Vec<(H160, fp_evm::GenesisAccount)>,
) -> serde_json::Value {
	// stakers: all validators and nominators.
	let stakers: Vec<(AccountId, AccountId, Balance, StakerStatus<AccountId>)> =
		initial_authorities
			.iter()
			.map(|x| (x.0.clone(), x.0.clone(), 100 * UNIT, StakerStatus::<AccountId>::Validator))
			.collect();


	let evm_accounts = {
		let mut map = BTreeMap::new();

		for (address, account) in genesis_evm_distribution {
			map.insert(address, account);
		}
		map
	};

	let council_members: Vec<AccountId> = vec![
		hex!["483b466832e094f01b1779a7ed07025df319c492dac5160aca89a3be117a7b6d"].into(),
		hex!["86d08e7bbe77bc74e3d88ee22edc53368bc13d619e05b66fe6c4b8e2d5c7015a"].into(),
		hex!["e421301e5aa5dddee51f0d8c73e794df16673e53157c5ea657be742e35b1793f"].into(),
		hex!["4ce3a4da3a7c1ce65f7edeff864dc3dd42e8f47eecc2726d99a0a80124698217"].into(),
		hex!["dcd9b70a0409b7626cba1a4016d8da19f4df5ce9fc5e8d16b789e71bb1161d73"].into(),
	]
	.into_iter()
	.collect();

	serde_json::json!({
		"sudo": { "key": Some(root_key) },
		"balances": {
			"balances": endowed_accounts.to_vec(),
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
		"vesting": {
			"vesting": Vec::<(AccountId, u64, u64, u128)>::new(),
		},
		"staking": {
			"validatorCount": initial_authorities.len() as u32,
			"minimumValidatorCount": initial_authorities.len() as u32 - 1,
			"invulnerables": initial_authorities.iter().map(|x| x.0.clone()).collect::<Vec<_>>(),
			"slashRewardFraction": Perbill::from_percent(10),
			"stakers" : stakers,
		},
		"council": {
			"members": council_members,
		},
		"babe": {
			"epochConfig": hippius_runtime::BABE_GENESIS_EPOCH_CONFIG,
		},
		"evm" : {
			"accounts": evm_accounts
		},

		"evmChainId": { "chainId": chain_id },
	})
}

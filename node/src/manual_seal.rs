// This file is part of The Brain.
// Copyright (C) 2022-2024 The Nerve Lab
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
// http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Service and ServiceFactory implementation. Specialized wrapper over substrate service.
use crate::cli::Sealing;
pub use crate::eth::{db_config_dir, EthConfiguration};
use crate::eth::{
	new_frontier_partial, spawn_frontier_tasks, BackendType, EthApi, FrontierBackend,
	FrontierBlockImport, FrontierPartialComponents, RpcConfig, StorageOverride,
	StorageOverrideHandler,
};
use futures::future;
use futures::FutureExt;
use futures::{channel::mpsc, prelude::*};
use futures_timer::Delay;
use hippius_primitives::Block;
use sc_client_api::{Backend, BlockBackend};
use sc_consensus::BasicQueue;
use sc_consensus_babe::BabeLink;
use sc_consensus_babe::{BabeWorkerHandle, SlotProportion};
use sc_consensus_grandpa::SharedVoterState;
#[allow(deprecated)]
pub use sc_executor::NativeElseWasmExecutor;
use sc_service::{error::Error as ServiceError, ChainType, Configuration, TaskManager};
use sc_telemetry::TelemetryHandle;
use sc_telemetry::{Telemetry, TelemetryWorker};
use sc_transaction_pool::FullPool;
use sc_transaction_pool_api::OffchainTransactionPoolFactory;
use sp_core::U256;
use sp_runtime::traits::Block as BlockT;
use std::cell::RefCell;
use std::{path::Path, sync::Arc, time::Duration};
use substrate_prometheus_endpoint::Registry;

#[cfg(not(feature = "mainnet"))]
use hippius_runtime::{self, RuntimeApi, TransactionConverter};

#[cfg(feature = "mainnet")]
use hippius_mainnet_runtime::{self, RuntimeApi, TransactionConverter};

/// The minimum period of blocks on which justifications will be
/// imported and generated.
const GRANDPA_JUSTIFICATION_PERIOD: u32 = 512;

#[cfg(not(feature = "mainnet"))]
pub mod hippius {
	// Our native executor instance.
	pub struct ExecutorDispatch;

	impl sc_executor::NativeExecutionDispatch for ExecutorDispatch {
		/// Only enable the benchmarking host functions when we actually want to benchmark.
		#[cfg(feature = "runtime-benchmarks")]
		type ExtendHostFunctions =
			(frame_benchmarking::benchmarking::HostFunctions, primitives_ext::ext::HostFunctions);
		/// Otherwise we only use the default Substrate host functions.
		#[cfg(not(feature = "runtime-benchmarks"))]
		type ExtendHostFunctions = primitives_ext::ext::HostFunctions;

		fn dispatch(method: &str, data: &[u8]) -> Option<Vec<u8>> {
			hippius_runtime::api::dispatch(method, data)
		}

		fn native_version() -> sc_executor::NativeVersion {
			hippius_runtime::native_version()
		}
	}
}

#[cfg(feature = "mainnet")]
pub mod mainnet {
	// Our native executor instance.
	pub struct ExecutorDispatch;

	impl sc_executor::NativeExecutionDispatch for ExecutorDispatch {
		/// Only enable the benchmarking host functions when we actually want to benchmark.
		#[cfg(feature = "runtime-benchmarks")]
		type ExtendHostFunctions =
			(frame_benchmarking::benchmarking::HostFunctions, primitives_ext::ext::HostFunctions);
		/// Otherwise we only use the default Substrate host functions.
		#[cfg(not(feature = "runtime-benchmarks"))]
		type ExtendHostFunctions = primitives_ext::ext::HostFunctions;

		fn dispatch(method: &str, data: &[u8]) -> Option<Vec<u8>> {
			hippius_mainnet_runtime::api::dispatch(method, data)
		}

		fn native_version() -> sc_executor::NativeVersion {
			hippius_mainnet_runtime::native_version()
		}
	}
}

#[cfg(not(feature = "mainnet"))]
#[allow(deprecated)]
pub(crate) type FullClient =
	sc_service::TFullClient<Block, RuntimeApi, NativeElseWasmExecutor<hippius::ExecutorDispatch>>;

#[cfg(feature = "mainnet")]
#[allow(deprecated)]
pub(crate) type FullClient =
	sc_service::TFullClient<Block, RuntimeApi, NativeElseWasmExecutor<mainnet::ExecutorDispatch>>;

pub(crate) type FullBackend = sc_service::TFullBackend<Block>;
type FullSelectChain = sc_consensus::LongestChain<FullBackend, Block>;

type GrandpaLinkHalf<Client> = sc_consensus_grandpa::LinkHalf<Block, Client, FullSelectChain>;
type BoxBlockImport = sc_consensus::BoxBlockImport<Block>;
type GrandpaBlockImport =
	sc_consensus_grandpa::GrandpaBlockImport<FullBackend, Block, FullClient, FullSelectChain>;

#[allow(clippy::type_complexity)]
pub fn new_partial<BIQ>(
	config: &Configuration,
	eth_config: &EthConfiguration,
	build_import_queue: BIQ,
) -> Result<
	sc_service::PartialComponents<
		FullClient,
		FullBackend,
		FullSelectChain,
		sc_consensus::DefaultImportQueue<Block>,
		sc_transaction_pool::FullPool<Block, FullClient>,
		(
			Option<Telemetry>,
			BoxBlockImport,
			GrandpaLinkHalf<FullClient>,
			sc_consensus_babe::BabeLink<Block>,
			FrontierBackend,
			Arc<dyn StorageOverride<Block>>,
			Option<BabeWorkerHandle<Block>>,
		),
	>,
	ServiceError,
>
where
	BIQ: FnOnce(
		Arc<FullClient>,
		&Configuration,
		&EthConfiguration,
		&TaskManager,
		Option<TelemetryHandle>,
		GrandpaBlockImport,
	) -> Result<(BasicQueue<Block>, BoxBlockImport), ServiceError>,
{
	println!("   
'##::::'##:'####:'########::'########::'####:'##::::'##::'######:::::::::::::::::'########:'##::::'##:'########::::'########::'########:::::'###::::'####:'##::: ##:
 ##:::: ##:. ##:: ##.... ##: ##.... ##:. ##:: ##:::: ##:'##... ##::::::::::::::::... ##..:: ##:::: ##: ##.....::::: ##.... ##: ##.... ##:::'## ##:::. ##:: ###:: ##:
 ##:::: ##:: ##:: ##:::: ##: ##:::: ##:: ##:: ##:::: ##: ##:::..:::::::::::::::::::: ##:::: ##:::: ##: ##:::::::::: ##:::: ##: ##:::: ##::'##:. ##::: ##:: ####: ##:
 #########:: ##:: ########:: ########::: ##:: ##:::: ##:. ######:::::'#######::::::: ##:::: #########: ######:::::: ########:: ########::'##:::. ##:: ##:: ## ## ##:
 ##.... ##:: ##:: ##.....::: ##.....:::: ##:: ##:::: ##::..... ##::::........::::::: ##:::: ##.... ##: ##...::::::: ##.... ##: ##.. ##::: #########:: ##:: ##. ####:
 ##:::: ##:: ##:: ##:::::::: ##::::::::: ##:: ##:::: ##:'##::: ##::::::::::::::::::: ##:::: ##:::: ##: ##:::::::::: ##:::: ##: ##::. ##:: ##.... ##:: ##:: ##:. ###:
 ##:::: ##:'####: ##:::::::: ##::::::::'####:. #######::. ######:::::::::::::::::::: ##:::: ##:::: ##: ########:::: ########:: ##:::. ##: ##:::: ##:'####: ##::. ##:
..:::::..::....::..:::::::::..:::::::::....:::.......::::......:::::::::::::::::::::..:::::..:::::..::........:::::........:::..:::::..::..:::::..::....::..::::..::
 \n");

	let telemetry = config
		.telemetry_endpoints
		.clone()
		.filter(|x| !x.is_empty())
		.map(|endpoints| -> Result<_, sc_telemetry::Error> {
			let worker = TelemetryWorker::new(16)?;
			let telemetry = worker.handle().new_telemetry(endpoints);
			Ok((worker, telemetry))
		})
		.transpose()?;

	#[allow(deprecated)]
	let executor = sc_service::new_native_or_wasm_executor(config);

	let (client, backend, keystore_container, task_manager) =
		sc_service::new_full_parts::<Block, RuntimeApi, _>(
			config,
			telemetry.as_ref().map(|(_, telemetry)| telemetry.handle()),
			executor,
		)?;
	let client = Arc::new(client);

	let telemetry = telemetry.map(|(worker, telemetry)| {
		task_manager.spawn_handle().spawn("telemetry", None, worker.run());
		telemetry
	});

	let select_chain = sc_consensus::LongestChain::new(backend.clone());

	let transaction_pool = sc_transaction_pool::BasicPool::new_full(
		config.transaction_pool.clone(),
		config.role.is_authority().into(),
		config.prometheus_registry(),
		task_manager.spawn_essential_handle(),
		client.clone(),
	);

	let (grandpa_block_import, grandpa_link) = sc_consensus_grandpa::block_import(
		client.clone(),
		GRANDPA_JUSTIFICATION_PERIOD,
		&client,
		select_chain.clone(),
		telemetry.as_ref().map(|x| x.handle()),
	)?;

	let storage_override = Arc::new(StorageOverrideHandler::<Block, _, _>::new(client.clone()));
	let frontier_backend = match eth_config.frontier_backend_type {
		BackendType::KeyValue => FrontierBackend::KeyValue(Arc::new(fc_db::kv::Backend::open(
			Arc::clone(&client),
			&config.database,
			&db_config_dir(config),
		)?)),
		BackendType::Sql => {
			let db_path = db_config_dir(config).join("sql");
			std::fs::create_dir_all(&db_path).expect("failed creating sql db directory");
			let backend = futures::executor::block_on(fc_db::sql::Backend::new(
				fc_db::sql::BackendConfig::Sqlite(fc_db::sql::SqliteBackendConfig {
					path: Path::new("sqlite:///")
						.join(db_path)
						.join("frontier.db3")
						.to_str()
						.unwrap(),
					create_if_missing: true,
					thread_count: eth_config.frontier_sql_backend_thread_count,
					cache_size: eth_config.frontier_sql_backend_cache_size,
				}),
				eth_config.frontier_sql_backend_pool_size,
				std::num::NonZeroU32::new(eth_config.frontier_sql_backend_num_ops_timeout),
				storage_override.clone(),
			))
			.unwrap_or_else(|err| panic!("failed creating sql backend: {:?}", err));
			FrontierBackend::Sql(Arc::new(backend))
		},
	};

	let (block_import, babe_link) = sc_consensus_babe::block_import(
		sc_consensus_babe::configuration(&*client)?,
		grandpa_block_import.clone(),
		client.clone(),
	)?;

	//let slot_duration = babe_link.config().slot_duration();
	let slot_duration = 0; // This is important to allow continous block

	let target_gas_price = eth_config.target_gas_price;

	let import_queue = sc_consensus_manual_seal::import_queue(
		Box::new(block_import.clone()),
		&task_manager.spawn_essential_handle(),
		config.prometheus_registry(),
	);

	Ok(sc_service::PartialComponents {
		client,
		backend,
		keystore_container,
		task_manager,
		select_chain,
		import_queue,
		transaction_pool,
		other: (
			telemetry,
			Box::new(block_import),
			grandpa_link,
			babe_link,
			frontier_backend,
			storage_override,
			None,
		),
	})
}

#[allow(dead_code)]
pub struct RunFullParams {
	pub config: Configuration,
	pub eth_config: EthConfiguration,
	pub rpc_config: RpcConfig,
	pub debug_output: Option<std::path::PathBuf>,
	pub auto_insert_keys: bool,
	pub sealing: Sealing,
}

pub fn build_manual_seal_import_queue(
	client: Arc<FullClient>,
	config: &Configuration,
	_eth_config: &EthConfiguration,
	task_manager: &TaskManager,
	_telemetry: Option<TelemetryHandle>,
	_grandpa_block_import: GrandpaBlockImport,
) -> Result<(BasicQueue<Block>, BoxBlockImport), ServiceError> {
	let frontier_block_import = FrontierBlockImport::new(client.clone(), client);
	Ok((
		sc_consensus_manual_seal::import_queue(
			Box::new(frontier_block_import.clone()),
			&task_manager.spawn_essential_handle(),
			config.prometheus_registry(),
		),
		Box::new(frontier_block_import),
	))
}

/// Builds a new service for a full client.
pub async fn new_full<Network: sc_network::NetworkBackend<Block, <Block as BlockT>::Hash>>(
	RunFullParams {
		mut config,
		eth_config,
		rpc_config,
		debug_output: _,
		auto_insert_keys,
		sealing,
	}: RunFullParams,
) -> Result<TaskManager, ServiceError> {
	let sc_service::PartialComponents {
		client,
		backend,
		mut task_manager,
		import_queue,
		keystore_container,
		select_chain,
		transaction_pool,
		other:
			(
				mut telemetry,
				block_import,
				grandpa_link,
				babe_link,
				frontier_backend,
				storage_override,
				babe_worker_handle,
			),
	} = new_partial(&config, &eth_config, build_manual_seal_import_queue)?;

	let FrontierPartialComponents { filter_pool, fee_history_cache, fee_history_cache_limit } =
		new_frontier_partial(&eth_config)?;

	let mut net_config = sc_network::config::FullNetworkConfiguration::<
		Block,
		<Block as BlockT>::Hash,
		Network,
	>::new(&config.network);

	let peer_store_handle = net_config.peer_store_handle();
	let metrics = Network::register_notification_metrics(
		config.prometheus_config.as_ref().map(|cfg| &cfg.registry),
	);

	let grandpa_protocol_name = sc_consensus_grandpa::protocol_standard_name(
		&client.block_hash(0).ok().flatten().expect("Genesis block exists; qed"),
		&config.chain_spec,
	);

	let (grandpa_protocol_config, grandpa_notification_service) =
		sc_consensus_grandpa::grandpa_peers_set_config::<_, Network>(
			grandpa_protocol_name.clone(),
			metrics.clone(),
			peer_store_handle,
		);

	net_config.add_notification_protocol(grandpa_protocol_config);

	let warp_sync = Arc::new(sc_consensus_grandpa::warp_proof::NetworkProvider::new(
		backend.clone(),
		grandpa_link.shared_authority_set().clone(),
		Vec::default(),
	));

	let (network, system_rpc_tx, tx_handler_controller, network_starter, sync_service) =
		sc_service::build_network(sc_service::BuildNetworkParams {
			config: &config,
			net_config,
			client: client.clone(),
			transaction_pool: transaction_pool.clone(),
			spawn_handle: task_manager.spawn_handle(),
			import_queue,
			block_announce_validator_builder: None,
			warp_sync_params: Some(sc_service::WarpSyncParams::WithProvider(warp_sync)),
			block_relay: None,
			metrics,
		})?;

	let role = config.role.clone();
	let force_authoring = config.force_authoring;
	let name = config.network.node_name.clone();
	let enable_grandpa = !config.disable_grandpa;
	let prometheus_registry = config.prometheus_registry().cloned();

	if config.offchain_worker.enabled {
		task_manager.spawn_handle().spawn(
			"offchain-workers-runner",
			"offchain-work",
			sc_offchain::OffchainWorkers::new(sc_offchain::OffchainWorkerOptions {
				runtime_api_provider: client.clone(),
				keystore: Some(keystore_container.keystore()),
				offchain_db: backend.offchain_storage(),
				transaction_pool: Some(OffchainTransactionPoolFactory::new(
					transaction_pool.clone(),
				)),
				network_provider: Arc::new(network.clone()),
				is_validator: role.is_authority(),
				enable_http_requests: true,
				custom_extensions: move |_| vec![],
			})
			.run(client.clone(), task_manager.spawn_handle())
			.boxed(),
		);
	}

	// Sinks for pubsub notifications.
	// Everytime a new subscription is created, a new mpsc channel is added to the sink pool.
	// The MappingSyncWorker sends through the channel on block import and the subscription emits a
	// notification to the subscriber on receiving a message through this channel. This way we avoid
	// race conditions when using native substrate block import notification stream.
	let pubsub_notification_sinks: fc_mapping_sync::EthereumBlockNotificationSinks<
		fc_mapping_sync::EthereumBlockNotification<Block>,
	> = Default::default();
	let pubsub_notification_sinks = Arc::new(pubsub_notification_sinks);

	// for ethereum-compatibility rpc.
	config.rpc_id_provider = Some(Box::new(fc_rpc::EthereumSubIdProvider));

	let slot_duration = babe_link.config().slot_duration();
	let target_gas_price = eth_config.target_gas_price;
	let frontier_backend = Arc::new(frontier_backend);

	let ethapi_cmd = rpc_config.ethapi.clone();
	let tracing_requesters =
		if ethapi_cmd.contains(&EthApi::Debug) || ethapi_cmd.contains(&EthApi::Trace) {
			crate::rpc::tracing::spawn_tracing_tasks(
				&task_manager,
				client.clone(),
				backend.clone(),
				frontier_backend.clone(),
				storage_override.clone(),
				&rpc_config,
				prometheus_registry.clone(),
			)
		} else {
			crate::rpc::tracing::RpcRequesters { debug: None, trace: None }
		};

	let pending_create_inherent_data_providers = move |_, ()| async move {
		let current = sp_timestamp::InherentDataProvider::from_system_time();
		let next_slot = current.timestamp().as_millis() + slot_duration.as_millis();
		let timestamp = sp_timestamp::InherentDataProvider::new(next_slot.into());
		let slot =
			sp_consensus_babe::inherents::InherentDataProvider::from_timestamp_and_slot_duration(
				*timestamp,
				slot_duration,
			);
		let dynamic_fee = fp_dynamic_fee::InherentDataProvider(U256::from(target_gas_price));
		Ok((slot, timestamp, dynamic_fee))
	};

	let network_clone = network.clone();

	let eth_rpc_params = crate::rpc::EthDeps {
		client: client.clone(),
		pool: transaction_pool.clone(),
		graph: transaction_pool.pool().clone(),
		converter: Some(TransactionConverter),
		is_authority: config.role.is_authority(),
		enable_dev_signer: eth_config.enable_dev_signer,
		network: network_clone,
		sync: sync_service.clone(),
		frontier_backend: match &*frontier_backend {
			fc_db::Backend::KeyValue(b) => b.clone(),
			fc_db::Backend::Sql(b) => b.clone(),
		},
		storage_override: storage_override.clone(),
		block_data_cache: Arc::new(fc_rpc::EthBlockDataCacheTask::new(
			task_manager.spawn_handle(),
			storage_override.clone(),
			eth_config.eth_log_block_cache,
			eth_config.eth_statuses_cache,
			prometheus_registry.clone(),
		)),
		filter_pool: filter_pool.clone(),
		max_past_logs: eth_config.max_past_logs,
		fee_history_cache: fee_history_cache.clone(),
		fee_history_cache_limit,
		execute_gas_limit_multiplier: eth_config.execute_gas_limit_multiplier,
		forced_parent_hashes: None,
		tracing_config: Some(crate::rpc::eth::TracingConfig {
			tracing_requesters: tracing_requesters.clone(),
			trace_filter_max_count: rpc_config.ethapi_trace_max_count,
		}),
		pending_create_inherent_data_providers,
	};

	let keystore = keystore_container.keystore();
	let select_chain_clone = select_chain.clone();
	let rpc_builder = {
		let client = client.clone();
		let pool = transaction_pool.clone();
		let pubsub_notification_sinks = pubsub_notification_sinks.clone();
		let justification_stream = grandpa_link.justification_stream();
		let shared_authority_set = grandpa_link.shared_authority_set().clone();
		let shared_voter_state = sc_consensus_grandpa::SharedVoterState::empty();
		let _shared_voter_state2 = shared_voter_state.clone();

		let finality_proof_provider = sc_consensus_grandpa::FinalityProofProvider::new_for_service(
			backend.clone(),
			Some(shared_authority_set.clone()),
		);

		Box::new(
			move |deny_unsafe, subscription_task_executor: sc_rpc::SubscriptionTaskExecutor| {
				let deps = crate::rpc::FullDeps {
					client: client.clone(),
					pool: pool.clone(),
					deny_unsafe,
					eth: eth_rpc_params.clone(),
					babe: None,
					select_chain: select_chain_clone.clone(),
					grandpa: crate::rpc::GrandpaDeps {
						shared_voter_state: shared_voter_state.clone(),
						shared_authority_set: shared_authority_set.clone(),
						justification_stream: justification_stream.clone(),
						subscription_executor: subscription_task_executor.clone(),
						finality_provider: finality_proof_provider.clone(),
					},
				};

				crate::rpc::create_full(
					deps,
					subscription_task_executor,
					pubsub_notification_sinks.clone(),
				)
				.map_err(Into::into)
			},
		)
	};

	spawn_frontier_tasks(
		&task_manager,
		client.clone(),
		backend.clone(),
		frontier_backend,
		filter_pool,
		storage_override.clone(),
		fee_history_cache,
		fee_history_cache_limit,
		sync_service.clone(),
		pubsub_notification_sinks,
	)
	.await;

	let params = sc_service::SpawnTasksParams {
		network: network.clone(),
		client: client.clone(),
		keystore: keystore_container.keystore(),
		task_manager: &mut task_manager,
		transaction_pool: transaction_pool.clone(),
		rpc_builder,
		backend: backend.clone(),
		system_rpc_tx,
		tx_handler_controller,
		sync_service: sync_service.clone(),
		config,
		telemetry: telemetry.as_mut(),
	};
	let _rpc_handlers = sc_service::spawn_tasks(params)?;

	// if the node isn't actively participating in consensus then it doesn't
	// need a keystore, regardless of which protocol we use below.
	let keystore = if role.is_authority() { Some(keystore_container.keystore()) } else { None };

	if role.is_authority() {
		let proposer_factory = sc_basic_authorship::ProposerFactory::new(
			task_manager.spawn_handle(),
			client.clone(),
			transaction_pool.clone(),
			prometheus_registry.clone().as_ref(),
			telemetry.as_ref().map(|x| x.handle()),
		);

		let babe_consensus_data_provider =
			sc_consensus_manual_seal::consensus::babe::BabeConsensusDataProvider::new(
				client.clone(),
				keystore_container.keystore(),
				babe_link.epoch_changes().clone(),
				vec![(
					sp_consensus_babe::AuthorityId::from(
						sp_keyring::sr25519::Keyring::Alice.public(),
					),
					1000,
				)],
			)
			.expect("failed to create BabeConsensusDataProvider");

		let target_gas_price = eth_config.target_gas_price;

		let manual_seal = match sealing {
			Sealing::Manual => {
				let (mut sink, commands_stream) = futures::channel::mpsc::channel(1024);
				task_manager.spawn_handle().spawn("block_authoring", None, async move {
					loop {
						futures_timer::Delay::new(std::time::Duration::from_millis(1000)).await;
						sink.try_send(sc_consensus_manual_seal::EngineCommand::SealNewBlock {
							create_empty: true,
							finalize: true,
							parent_hash: None,
							sender: None,
						})
						.unwrap();
					}
				});

				future::Either::Left(sc_consensus_manual_seal::run_manual_seal(
					sc_consensus_manual_seal::ManualSealParams {
						block_import,
						env: proposer_factory,
						client,
						pool: transaction_pool,
						commands_stream,
						select_chain,
						consensus_data_provider: Some(Box::new(babe_consensus_data_provider)),
						create_inherent_data_providers: pending_create_inherent_data_providers,
					},
				))
			},
			Sealing::Instant => future::Either::Right(sc_consensus_manual_seal::run_instant_seal(
				sc_consensus_manual_seal::InstantSealParams {
					block_import,
					env: proposer_factory,
					client,
					pool: transaction_pool,
					select_chain,
					consensus_data_provider: Some(Box::new(babe_consensus_data_provider)),
					create_inherent_data_providers: pending_create_inherent_data_providers,
				},
			)),
		};

		// we spawn the future on a background thread managed by service.
		task_manager
			.spawn_essential_handle()
			.spawn_blocking("manual-seal", None, manual_seal);

		network_starter.start_network();
		log::info!("Manual Seal Ready");
		return Ok(task_manager);
	}

	let grandpa_config = sc_consensus_grandpa::Config {
		// FIXME #1578 make this available through chainspec
		gossip_duration: Duration::from_millis(333),
		justification_generation_period: GRANDPA_JUSTIFICATION_PERIOD,
		name: Some(name),
		observer_enabled: false,
		keystore,
		local_role: role,
		telemetry: telemetry.as_ref().map(|x| x.handle()),
		protocol_name: grandpa_protocol_name,
	};

	if enable_grandpa {
		// start the full GRANDPA voter
		// NOTE: non-authorities could run the GRANDPA observer protocol, but at
		// this point the full voter should provide better guarantees of block
		// and vote data availability than the observer. The observer has not
		// been tested extensively yet and having most nodes in a network run it
		// could lead to finality stalls.
		let grandpa_config = sc_consensus_grandpa::GrandpaParams {
			config: grandpa_config,
			link: grandpa_link,
			network,
			sync: Arc::new(sync_service),
			notification_service: grandpa_notification_service,
			voting_rule: sc_consensus_grandpa::VotingRulesBuilder::default().build(),
			prometheus_registry,
			shared_voter_state: SharedVoterState::empty(),
			telemetry: telemetry.as_ref().map(|x| x.handle()),
			offchain_tx_pool_factory: OffchainTransactionPoolFactory::new(transaction_pool),
		};

		// the GRANDPA voter task is considered infallible, i.e.
		// if it fails we take down the service with it.
		task_manager.spawn_essential_handle().spawn_blocking(
			"grandpa-voter",
			None,
			sc_consensus_grandpa::run_grandpa_voter(grandpa_config)?,
		);
	}

	network_starter.start_network();
	Ok(task_manager)
}

#[allow(clippy::type_complexity)]
pub fn new_chain_ops(
	config: &mut Configuration,
	eth_config: &EthConfiguration,
) -> Result<
	(Arc<FullClient>, Arc<FullBackend>, BasicQueue<Block>, TaskManager, FrontierBackend),
	ServiceError,
> {
	config.keystore = sc_service::config::KeystoreConfig::InMemory;
	let sc_service::PartialComponents {
		client, backend, import_queue, task_manager, other, ..
	} = new_partial(config, eth_config, build_manual_seal_import_queue)?;
	Ok((client, backend, import_queue, task_manager, other.4))
}

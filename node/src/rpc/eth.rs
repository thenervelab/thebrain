// This file is part of The Brain.
// Copyright (C) 2022-2024 The Nerve Lab
//
// Hippius is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// Hippius is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with Hippius.  If not, see <http://www.gnu.org/licenses/>.
use std::{collections::BTreeMap, sync::Arc};

use jsonrpsee::RpcModule;
// Substrate
use sc_client_api::{
	backend::{Backend, StorageProvider},
	client::BlockchainEvents,
};
use sc_network::service::traits::NetworkService;
use sc_network_sync::SyncingService;
use sc_rpc::SubscriptionTaskExecutor;
use sc_transaction_pool::{ChainApi, Pool};
use sc_transaction_pool_api::TransactionPool;
use sp_api::{CallApiAt, ProvideRuntimeApi};
use sp_block_builder::BlockBuilder as BlockBuilderApi;
use sp_blockchain::{Error as BlockChainError, HeaderBackend, HeaderMetadata};
use sp_core::H256;
use sp_inherents::CreateInherentDataProviders;
use sp_runtime::traits::{Block as BlockT, Header as HeaderT};
// Frontier
pub use fc_rpc::{EthBlockDataCacheTask, EthConfig};
pub use fc_rpc_core::types::{FeeHistoryCache, FeeHistoryCacheLimit, FilterPool};
use fc_storage::StorageOverride;
use fp_rpc::{ConvertTransaction, ConvertTransactionRuntimeApi, EthereumRuntimeRPCApi};
#[cfg(feature = "txpool")]
use rpc_txpool::TxPoolServer;

use sp_consensus_babe::BabeApi;
use sp_keystore::KeystorePtr;

#[derive(Clone)]
pub struct TracingConfig {
	pub tracing_requesters: crate::rpc::tracing::RpcRequesters,
	pub trace_filter_max_count: u32,
}

/// Extra dependencies for Ethereum compatibility.
pub struct EthDeps<C, P, A: ChainApi, CT, B: BlockT, CIDP> {
	/// The client instance to use.
	pub client: Arc<C>,
	/// Transaction pool instance.
	pub pool: Arc<P>,
	/// Graph pool instance.
	pub graph: Arc<Pool<A>>,
	/// Ethereum transaction converter.
	pub converter: Option<CT>,
	/// The Node authority flag
	pub is_authority: bool,
	/// Whether to enable dev signer
	pub enable_dev_signer: bool,
	/// Network service
	pub network: Arc<dyn NetworkService>,
	/// Chain syncing service
	pub sync: Arc<SyncingService<B>>,
	/// Frontier Backend.
	pub frontier_backend: Arc<dyn fc_api::Backend<B>>,
	/// Ethereum data access overrides.
	pub storage_override: Arc<dyn StorageOverride<B>>,
	/// Cache for Ethereum block data.
	pub block_data_cache: Arc<EthBlockDataCacheTask<B>>,
	/// EthFilterApi pool.
	pub filter_pool: Option<FilterPool>,
	/// Maximum number of logs in a query.
	pub max_past_logs: u32,
	/// Fee history cache.
	pub fee_history_cache: FeeHistoryCache,
	/// Maximum fee history cache size.
	pub fee_history_cache_limit: FeeHistoryCacheLimit,
	/// Maximum allowed gas limit will be ` block.gas_limit * execute_gas_limit_multiplier` when
	/// using eth_call/eth_estimateGas.
	pub execute_gas_limit_multiplier: u64,
	/// Mandated parent hashes for a given block hash.
	pub forced_parent_hashes: Option<BTreeMap<H256, H256>>,
	/// Optional tracing config.
	pub tracing_config: Option<TracingConfig>,
	/// Something that can create the inherent data providers for pending state
	pub pending_create_inherent_data_providers: CIDP,
	/// The keystore
	pub keystore: KeystorePtr,
}

impl<C, P, A: ChainApi, CT: Clone, B: BlockT, CIDP: Clone> Clone for EthDeps<C, P, A, CT, B, CIDP> {
	fn clone(&self) -> Self {
		Self {
			client: self.client.clone(),
			pool: self.pool.clone(),
			graph: self.graph.clone(),
			converter: self.converter.clone(),
			is_authority: self.is_authority,
			enable_dev_signer: self.enable_dev_signer,
			network: self.network.clone(),
			sync: self.sync.clone(),
			frontier_backend: self.frontier_backend.clone(),
			storage_override: self.storage_override.clone(),
			block_data_cache: self.block_data_cache.clone(),
			filter_pool: self.filter_pool.clone(),
			max_past_logs: self.max_past_logs,
			fee_history_cache: self.fee_history_cache.clone(),
			fee_history_cache_limit: self.fee_history_cache_limit,
			execute_gas_limit_multiplier: self.execute_gas_limit_multiplier,
			forced_parent_hashes: self.forced_parent_hashes.clone(),
			tracing_config: self.tracing_config.clone(),
			pending_create_inherent_data_providers: self
				.pending_create_inherent_data_providers
				.clone(),
			keystore: self.keystore.clone(),
		}
	}
}

/// Instantiate Ethereum-compatible RPC extensions.
pub fn create_eth<B, C, BE, P, A, CT, CIDP, EC>(
	mut io: RpcModule<()>,
	deps: EthDeps<C, P, A, CT, B, CIDP>,
	subscription_task_executor: SubscriptionTaskExecutor,
	pubsub_notification_sinks: Arc<
		fc_mapping_sync::EthereumBlockNotificationSinks<
			fc_mapping_sync::EthereumBlockNotification<B>,
		>,
	>,
) -> Result<RpcModule<()>, Box<dyn std::error::Error + Send + Sync>>
where
	B: BlockT<Hash = sp_core::H256>,
	C: CallApiAt<B> + ProvideRuntimeApi<B>,
	C::Api: BabeApi<B>
		+ BlockBuilderApi<B>
		+ ConvertTransactionRuntimeApi<B>
		+ EthereumRuntimeRPCApi<B>
		+ rpc_primitives_debug::DebugRuntimeApi<B>
		+ rpc_primitives_txpool::TxPoolRuntimeApi<B>
		+ rpc_primitives_node_metrics::NodeMetricsRuntimeApi<B>,
	C: BlockchainEvents<B> + StorageProvider<B, BE> + 'static,
	C: HeaderBackend<B> + HeaderMetadata<B, Error = BlockChainError> + StorageProvider<B, BE>,
	BE: Backend<B> + 'static,
	P: TransactionPool<Block = B> + 'static,
	A: ChainApi<Block = B> + 'static,
	CT: ConvertTransaction<<B as BlockT>::Extrinsic> + Send + Sync + 'static,
	CIDP: CreateInherentDataProviders<B, ()> + Send + Sync + 'static,
	EC: EthConfig<B, C>,
	B::Header: HeaderT<Number = u64>,
{
	use fc_rpc::{
		Eth, EthApiServer, EthDevSigner, EthFilter, EthFilterApiServer, EthPubSub,
		EthPubSubApiServer, EthSigner, Net, NetApiServer, Web3, Web3ApiServer,
	};
	use rpc_debug::{Debug, DebugServer};
	use rpc_docker_registry::{DockerRegistryApiServer, DockerRegistryImpl};
	use rpc_net::{Extra, ExtraApiServer};
	use rpc_node_metrics::{NodeMetricsApiServer, NodeMetricsImpl};
	use rpc_system::{SystemInfoApiServer, SystemInfoImpl};
	use rpc_trace::{Trace, TraceServer};
	use rpc_weight::{WeightsInfoApiServer, WeightsInfoImpl};

	#[cfg(feature = "txpool")]
	use fc_rpc::{TxPool, TxPoolApiServer};
	let EthDeps {
		client,
		pool,
		graph,
		converter,
		is_authority,
		enable_dev_signer,
		network,
		sync,
		frontier_backend,
		storage_override,
		block_data_cache,
		filter_pool,
		max_past_logs,
		fee_history_cache,
		fee_history_cache_limit,
		execute_gas_limit_multiplier,
		forced_parent_hashes,
		tracing_config,
		pending_create_inherent_data_providers,
		keystore: _,
	} = deps;

	let mut signers = Vec::new();
	if enable_dev_signer {
		signers.push(Box::new(EthDevSigner::new()) as Box<dyn EthSigner>);
	}

	io.merge(
		Eth::<B, C, P, CT, BE, A, CIDP, EC>::new(
			client.clone(),
			pool.clone(),
			graph.clone(),
			converter,
			sync.clone(),
			signers,
			storage_override.clone(),
			frontier_backend.clone(),
			is_authority,
			block_data_cache.clone(),
			fee_history_cache,
			fee_history_cache_limit,
			execute_gas_limit_multiplier,
			forced_parent_hashes,
			pending_create_inherent_data_providers,
			None,
		)
		.replace_config::<EC>()
		.into_rpc(),
	)?;

	if let Some(filter_pool) = filter_pool {
		io.merge(
			EthFilter::new(
				client.clone(),
				frontier_backend,
				graph.clone(),
				filter_pool,
				500_usize, // max stored filters
				max_past_logs,
				block_data_cache,
			)
			.into_rpc(),
		)?;
	}

	io.merge(
		EthPubSub::new(
			pool,
			client.clone(),
			sync,
			subscription_task_executor,
			storage_override.clone(),
			pubsub_notification_sinks,
		)
		.into_rpc(),
	)?;

	io.merge(
		Net::new(
			client.clone(),
			network.clone(),
			// Whether to format the `peer_count` response as Hex (default) or not.
			true,
		)
		.into_rpc(),
	)?;

	io.merge(
		Extra::new(
			client.clone(),
			network,
			// Whether to format the `peer_count` response as Hex (default) or not.
			true,
		)
		.into_rpc(),
	)?;

	io.merge(SystemInfoImpl::new(client.clone()).into_rpc())?;
	io.merge(DockerRegistryImpl::new(client.clone()).into_rpc())?;
	io.merge(NodeMetricsImpl::new(client.clone()).into_rpc())?;

	// Get the keystore
	io.merge(
		WeightsInfoImpl::new(
			client.clone(),
			deps.keystore.clone(), // Assuming keystore is part of deps
		)
		.into_rpc(),
	)?;

	io.merge(Web3::new(client.clone()).into_rpc())?;

	#[cfg(feature = "txpool")]
	io.merge(rpc_txpool::TxPool::new(Arc::clone(&client), graph).into_rpc())?;

	if let Some(tracing_config) = tracing_config {
		if let Some(trace_filter_requester) = tracing_config.tracing_requesters.trace {
			io.merge(
				Trace::new(client, trace_filter_requester, tracing_config.trace_filter_max_count)
					.into_rpc(),
			)?;
		}

		if let Some(debug_requester) = tracing_config.tracing_requesters.debug {
			io.merge(Debug::new(debug_requester).into_rpc())?;
		}
	}
	Ok(io)
}

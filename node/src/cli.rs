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

#![allow(clippy::all)]
use sc_cli::RunCmd;

use crate::service::EthConfiguration;

/// Available Sealing methods.
#[derive(Copy, Clone, Debug, Default, clap::ValueEnum)]
pub enum Sealing {
	/// Seal using rpc method.
	#[default]
	Manual,
	/// Seal when transaction is executed.
	Instant,
}

#[derive(Debug, clap::Parser)]
pub struct Cli {
	#[command(subcommand)]
	pub subcommand: Option<Subcommand>,

	#[allow(missing_docs)]
	#[command(flatten)]
	pub run: RunCmd,

	#[arg(long, short = 'o')]
	pub output_path: Option<std::path::PathBuf>,

	#[command(flatten)]
	pub eth: EthConfiguration,

	#[arg(short, long)]
	pub auto_insert_keys: bool,

	/// Choose sealing method.
	#[cfg(feature = "manual-seal")]
	#[arg(long, value_enum, ignore_case = true)]
	pub sealing: Sealing,

	/// Custom metrics argument (optional).
	#[arg(
		long,
		help = "Specify the importance in weight of bandwidth_mbps to use in calculation (e.g., 2, 3, 4.)"
	)]
	pub bandwidth_mbps: Option<f32>,

	/// Custom metrics argument (optional).
	#[arg(
		long,
		help = "Specify the importance in weight of storage_bytes to use in calculation (e.g., 2, 3.)"
	)]
	pub storage_bytes: Option<f32>,

	/// Custom metrics argument (optional).
	#[arg(
		long,
		help = "Specify the importance in weight of uptime to use in calculation (e.g., 2, 3, 4.)"
	)]
	pub uptime: Option<f32>,

	/// Custom metrics argument (optional).
	#[arg(
		long,
		help = "Specify the importance in weight of peers to use in calculation (e.g., 2, 3.)"
	)]
	pub peers: Option<f32>,

	/// Custom metrics argument (optional).
	#[arg(
		long,
		help = "Specify the importance in weight of latency to use in calculation (e.g., 2, 3.)"
	)]
	pub latency: Option<f32>,
}

#[derive(Debug, clap::Subcommand)]
pub enum Subcommand {
	/// Key management cli utilities
	#[command(subcommand)]
	Key(sc_cli::KeySubcommand),

	/// Build a chain specification.
	BuildSpec(sc_cli::BuildSpecCmd),

	/// Validate blocks.
	CheckBlock(sc_cli::CheckBlockCmd),

	/// Export blocks.
	ExportBlocks(sc_cli::ExportBlocksCmd),

	/// Export the state of a given block into a chain spec.
	ExportState(sc_cli::ExportStateCmd),

	/// Import blocks.
	ImportBlocks(sc_cli::ImportBlocksCmd),

	/// Remove the whole chain.
	PurgeChain(sc_cli::PurgeChainCmd),

	/// Revert the chain to a previous state.
	Revert(sc_cli::RevertCmd),

	/// Sub-commands concerned with benchmarking.
	#[command(subcommand)]
	Benchmark(frame_benchmarking_cli::BenchmarkCmd),

	/// Try some command against runtime state.
	#[cfg(feature = "try-runtime")]
	TryRuntime(try_runtime_cli::TryRuntimeCmd),

	/// Try some command against runtime state. Note: `try-runtime` feature must be enabled.
	#[cfg(not(feature = "try-runtime"))]
	TryRuntime,

	/// Db meta columns information.
	ChainInfo(sc_cli::ChainInfoCmd),

	/// Db meta columns information.
	FrontierDb(fc_cli::FrontierDbCmd),
}

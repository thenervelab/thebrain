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
//
#![cfg_attr(not(feature = "std"), no_std)]
// `construct_runtime!` does a lot of recursion and requires us to increase the limit to 256.
#![recursion_limit = "512"]

// Make the WASM binary available.
#[cfg(feature = "std")]
include!(concat!(env!("OUT_DIR"), "/wasm_binary.rs"));

mod filters;
pub mod frontier_evm;
pub mod impls;
pub mod migrations;
pub mod precompiles;
// pub mod hippius_services;
pub mod voter_bags;
use frame_election_provider_support::{
	bounds::{ElectionBounds, ElectionBoundsBuilder},
	onchain, BalancingConfig, ElectionDataProvider, SequentialPhragmen, VoteWeight,
};
use frame_support::derive_impl;
use frame_support::genesis_builder_helper::build_state;
use frame_support::genesis_builder_helper::get_preset;
use frame_support::{
	traits::{
		tokens::{PayFromAccount, UnityAssetBalanceConversion},
		AsEnsureOriginWithArg, Contains, OnFinalize, WithdrawReasons,
	},
	// weights::ConstantMultiplier,
};
use scale_info::prelude::string::String;
use frame_system::EnsureSigned;
use pallet_election_provider_multi_phase::{GeometricDepositBase, SolutionAccuracyOf};
use pallet_evm::GasWeightMapping;
use pallet_grandpa::{
	fg_primitives, AuthorityId as GrandpaId, AuthorityList as GrandpaAuthorityList,
};
use pallet_im_online::sr25519::AuthorityId as ImOnlineId;
use pallet_session::historical as pallet_session_historical;
pub use pallet_staking::StakerStatus;
#[allow(deprecated)]
use pallet_transaction_payment::{
	CurrencyAdapter, FeeDetails, Multiplier, RuntimeDispatchInfo,
};
use pallet_tx_pause::RuntimeCallNameOf;
use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use serde::{Deserialize, Serialize};
use sp_api::impl_runtime_apis;
use sp_core::{crypto::KeyTypeId, OpaqueMetadata, H160, H256, U256};
use sp_genesis_builder::PresetId;
use sp_runtime::{
	create_runtime_str,
	generic, impl_opaque_keys,
	traits::{
		self, BlakeTwo256, Block as BlockT, Bounded, Convert, ConvertInto, DispatchInfoOf,
		Dispatchable, IdentityLookup, NumberFor, OpaqueKeys, PostDispatchInfoOf, StaticLookup,
		UniqueSaturatedInto,
	},
	transaction_validity::{
		TransactionPriority, TransactionSource, TransactionValidity, TransactionValidityError,
	},
	ApplyExtrinsicResult, FixedPointNumber, FixedU128, Perquintill, RuntimeDebug,
	SaturatedConversion,
};
use sp_staking::currency_to_vote::U128CurrencyToVote;
use frame_support::traits::ExistenceRequirement;
// use hippius_primitives::services::RpcServicesWithBlueprint;
// pub use hippius_services::PalletServicesConstraints;

#[cfg(any(feature = "std", test))]
pub use frame_system::Call as SystemCall;
use sp_std::prelude::*;
#[cfg(feature = "std")]
use sp_version::NativeVersion;
use sp_version::RuntimeVersion;
use static_assertions::const_assert;

pub use frame_support::{
	construct_runtime,
	dispatch::DispatchClass,
	pallet_prelude::Get,
	parameter_types,
	traits::{
		ConstU128, ConstU16, ConstU32, Currency, EitherOfDiverse, EqualPrivilegeOnly, Everything,
		Imbalance, InstanceFilter, KeyOwnerProofSystem, LockIdentifier, OnUnbalanced,
	},
	weights::{
		constants::{
			BlockExecutionWeight, ExtrinsicBaseWeight, RocksDbWeight, WEIGHT_REF_TIME_PER_MILLIS,
		},
		IdentityFee, Weight,
	},
	PalletId, StorageValue,
};
use frame_system::{EnsureRoot, EnsureWithSuccess};
pub use pallet_balances::Call as BalancesCall;
pub use pallet_timestamp::Call as TimestampCall;
use sp_runtime::generic::Era;
#[cfg(any(feature = "std", test))]
pub use sp_runtime::BuildStorage;
pub use sp_runtime::{MultiAddress, Perbill, Percent, Permill};
use sp_runtime::AccountId32;
use sp_core::crypto::Ss58Codec;
pub use hippius_primitives::{
	currency::*,
	fee::*,
	time::*,
	types::{
		AccountId, AccountIndex, Address, Balance, BlockNumber, Hash, Header, Index, Moment,
		Signature,
	},
	AVERAGE_ON_INITIALIZE_RATIO, MAXIMUM_BLOCK_WEIGHT, NORMAL_DISPATCH_RATIO,
};
use hippius_primitives::{
	democracy::{
		COOLOFF_PERIOD, ENACTMENT_PERIOD, FASTTRACK_VOTING_PERIOD, LAUNCH_PERIOD, MAX_PROPOSALS,
		MINIMUM_DEPOSIT, VOTING_PERIOD,
	},
	elections::{
		CANDIDACY_BOND, DESIRED_MEMBERS, DESIRED_RUNNERS_UP, ELECTIONS_PHRAGMEN_PALLET_ID,
		MAX_CANDIDATES, MAX_VOTERS, MAX_VOTES_PER_VOTER, TERM_DURATION,
	},
	staking::{
		BONDING_DURATION, HISTORY_DEPTH, MAX_NOMINATOR_REWARDED_PER_VALIDATOR, OFFCHAIN_REPEAT,
		SESSIONS_PER_ERA, SLASH_DEFER_DURATION,
	},
	treasury::{
		BURN, DATA_DEPOSIT_PER_BYTE, MAXIMUM_REASON_LENGTH, MAX_APPROVALS, PROPOSAL_BOND,
		PROPOSAL_BOND_MINIMUM, SPEND_PERIOD, TIP_COUNTDOWN, TIP_FINDERS_FEE,
		TIP_REPORT_DEPOSIT_BASE, TREASURY_PALLET_ID,
	},
};

// Frontier
use fp_rpc::TransactionStatus;
use pallet_ethereum::{Call::transact, Transaction as EthereumTransaction};
use pallet_evm::{Account as EVMAccount, FeeCalculator,  Runner};
pub type Nonce = u32;

/// The BABE epoch configuration at genesis.
pub const BABE_GENESIS_EPOCH_CONFIG: sp_consensus_babe::BabeEpochConfiguration =
	sp_consensus_babe::BabeEpochConfiguration {
		c: PRIMARY_PROBABILITY,
		allowed_slots: sp_consensus_babe::AllowedSlots::PrimaryAndSecondaryPlainSlots,
	};

/// This runtime version.
#[sp_version::runtime_version]
pub const VERSION: RuntimeVersion = RuntimeVersion {
	spec_name: create_runtime_str!("hippius"),
	impl_name: create_runtime_str!("hippius"),
	authoring_version: 1,
	spec_version: 1205,
	impl_version: 1,
	apis: RUNTIME_API_VERSIONS,
	transaction_version: 1,
	state_version: 0,
};

/// The version information used to identify this runtime when compiled natively.
#[cfg(feature = "std")]
pub fn native_version() -> NativeVersion {
	NativeVersion { runtime_version: VERSION, can_author_with: Default::default() }
}

pub const MAXIMUM_BLOCK_LENGTH: u32 = 5 * 1024 * 1024;

parameter_types! {
	pub const Version: RuntimeVersion = VERSION;
	pub const BlockHashCount: BlockNumber = 256;
	pub BlockWeights: frame_system::limits::BlockWeights = frame_system::limits::BlockWeights
		::with_sensible_defaults(MAXIMUM_BLOCK_WEIGHT, NORMAL_DISPATCH_RATIO);
	pub BlockLength: frame_system::limits::BlockLength = frame_system::limits::BlockLength
		::max_with_normal_ratio(MAXIMUM_BLOCK_LENGTH, NORMAL_DISPATCH_RATIO);
	pub const SS58Prefix: u16 = hippius_primitives::MAINNET_SS58_PREFIX;
}

/// Opaque types. These are used by the CLI to instantiate machinery that don't need to know
/// the specifics of the runtime. They can then be made to be agnostic over specific formats
/// of data like extrinsics, allowing for them to continue syncing the network through upgrades
/// to even the core data structures.
pub mod opaque {
	use super::*;

	pub use sp_runtime::OpaqueExtrinsic as UncheckedExtrinsic;

	/// Opaque block identifier type.
	pub type BlockId = generic::BlockId<Block>;

	impl_opaque_keys! {
		pub struct SessionKeys {
			pub babe: Babe,
			pub grandpa: Grandpa,
			pub im_online: ImOnline,
		}
	}
}

#[derive_impl(frame_system::config_preludes::SolochainDefaultConfig)]
impl frame_system::Config for Runtime {
	type AccountData = pallet_balances::AccountData<Balance>;
	type AccountId = AccountId;
	type BaseCallFilter = filters::MainnetCallFilter;
	type BlockHashCount = BlockHashCount;
	type BlockLength = BlockLength;
	type Block = Block;
	type BlockWeights = BlockWeights;
	type RuntimeCall = RuntimeCall;
	type DbWeight = RocksDbWeight;
	type RuntimeEvent = RuntimeEvent;
	type Hash = Hash;
	type Nonce = Nonce;
	type Hashing = BlakeTwo256;
	type Lookup = Indices;
	type MaxConsumers = frame_support::traits::ConstU32<16>;
	type OnKilledAccount = ();
	type OnNewAccount = ();
	type OnSetCode = ();
	type RuntimeTask = RuntimeTask;
	type RuntimeOrigin = RuntimeOrigin;
	type PalletInfo = PalletInfo;
	type SS58Prefix = SS58Prefix;
	type SystemWeightInfo = frame_system::weights::SubstrateWeight<Runtime>;
	type Version = Version;
}

parameter_types! {
	pub const IndexDeposit: Balance = UNIT;
}

impl pallet_indices::Config for Runtime {
	type AccountIndex = AccountIndex;
	type Currency = Balances;
	type Deposit = IndexDeposit;
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = pallet_indices::weights::SubstrateWeight<Runtime>;
}

parameter_types! {
	pub const MinimumPeriod: u64 = SLOT_DURATION / 2;
}

impl pallet_timestamp::Config for Runtime {
	/// A timestamp: milliseconds since the unix epoch.
	type Moment = u64;
	type OnTimestampSet = Babe;
	type MinimumPeriod = MinimumPeriod;
	type WeightInfo = pallet_timestamp::weights::SubstrateWeight<Runtime>;
}

parameter_types! {
	// 1e18 / 1e8 = 1e10 existential balance
	// pub const ExistentialDeposit: u128 = EXISTENTIAL_DEPOSIT / 100_000_000;
	pub const ExistentialDeposit: u64 = 500;
	pub const TransferFee: u128 = MILLIUNIT;
	pub const CreationFee: u128 = MILLIUNIT;
	pub const MaxLocks: u32 = 50;
	pub const MaxReserves: u32 = 50;
	pub const MaxFreezes: u32 = 50;
}



pub type NegativeImbalance<T> = <pallet_balances::Pallet<T> as Currency<
	<T as frame_system::Config>::AccountId,
>>::NegativeImbalance;

impl pallet_balances::Config for Runtime {
	/// The type for recording an account's balance.
	type Balance = Balance;
	/// The ubiquitous event type
	type RuntimeEvent = RuntimeEvent;
	type DustRemoval = TransferDustToTreasury;
	type ExistentialDeposit = ExistentialDeposit;
	type AccountStore = System;
	type WeightInfo = pallet_balances::weights::SubstrateWeight<Runtime>;
	type MaxLocks = MaxLocks;
	type MaxReserves = MaxReserves;
	type ReserveIdentifier = [u8; 8];
	type RuntimeHoldReason = RuntimeHoldReason;
	type RuntimeFreezeReason = RuntimeFreezeReason;
	type FreezeIdentifier = RuntimeFreezeReason;
	type MaxFreezes = MaxFreezes;
}

parameter_types! {
	// pub const TransactionByteFee: Balance = MILLIUNIT;
	pub const TransactionByteFee: Balance = MILLIUNIT / 2;
	// pub const OperationalFeeMultiplier: u8 = 5;
	pub const OperationalFeeMultiplier: u8 = 2;
	pub const TargetBlockFullness: Perquintill = Perquintill::from_percent(25);
	pub AdjustmentVariable: Multiplier = Multiplier::saturating_from_rational(1, 100_000);
	// pub MinimumMultiplier: Multiplier = Multiplier::saturating_from_rational(1, 1_000_000_000u128);
	pub MinimumMultiplier: Multiplier = Multiplier::saturating_from_rational(1, 10_000_000u128);
	pub MaximumMultiplier: Multiplier = Bounded::max_value();
	pub FeeMultiplier: Multiplier = Multiplier::saturating_from_rational(1,1);
}

impl pallet_transaction_payment::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	#[allow(deprecated)]
	type OnChargeTransaction = CurrencyAdapter<Balances, impls::DealWithFees<Runtime>>;
	type OperationalFeeMultiplier = OperationalFeeMultiplier;
	type WeightToFee = IdentityFee<Balance>;
	type LengthToFee = IdentityFee<Balance>;
	type FeeMultiplierUpdate = pallet_transaction_payment::ConstFeeMultiplier<FeeMultiplier>;
	// type FeeMultiplierUpdate = TargetedFeeAdjustment<
	// 	Self,
	// 	TargetBlockFullness,
	// 	AdjustmentVariable,
	// 	MinimumMultiplier,
	// 	MaximumMultiplier,
	// >;
}

parameter_types! {
	pub MaximumSchedulerWeight: Weight = Perbill::from_percent(80) *
		BlockWeights::get().max_block;
}

impl pallet_scheduler::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type RuntimeOrigin = RuntimeOrigin;
	type PalletsOrigin = OriginCaller;
	type RuntimeCall = RuntimeCall;
	type MaximumWeight = MaximumSchedulerWeight;
	type ScheduleOrigin = EnsureRoot<AccountId>;
	type MaxScheduledPerBlock = ConstU32<512>;
	type WeightInfo = pallet_scheduler::weights::SubstrateWeight<Runtime>;
	type OriginPrivilegeCmp = EqualPrivilegeOnly;
	type Preimages = Preimage;
}

parameter_types! {
	pub const PreimageBaseDeposit: Balance = 100 * UNIT;
	// One cent: $10,000 / MB
	pub const PreimageByteDeposit: Balance = 10 * MILLIUNIT;
}

impl pallet_preimage::Config for Runtime {
	type Consideration = ();
	type Currency = Balances;
	type RuntimeEvent = RuntimeEvent;
	type ManagerOrigin = EnsureRoot<AccountId>;
	type WeightInfo = pallet_preimage::weights::SubstrateWeight<Runtime>;
}

impl pallet_randomness_collective_flip::Config for Runtime {}

impl pallet_sudo::Config for Runtime {
	type RuntimeCall = RuntimeCall;
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = pallet_sudo::weights::SubstrateWeight<Runtime>;
}

parameter_types! {
	// NOTE: Currently it is not possible to change the epoch duration after the chain has started.
	//       Attempting to do so will brick block production.
	pub const EpochDuration: u64 = EPOCH_DURATION_IN_SLOTS;
	pub const ExpectedBlockTime: Moment = MILLISECS_PER_BLOCK;
	pub const ReportLongevity: u64 =
		BondingDuration::get() as u64 * SessionsPerEra::get() as u64 * EpochDuration::get();
	pub const MaxAuthorities: u32 = 1000;
	pub const MaxNominators: u32 = 1000;
}

impl pallet_babe::Config for Runtime {
	type EpochDuration = EpochDuration;
	type ExpectedBlockTime = ExpectedBlockTime;
	type EpochChangeTrigger = pallet_babe::ExternalTrigger;
	type DisabledValidators = Session;
	type WeightInfo = ();
	type MaxAuthorities = MaxAuthorities;
	type MaxNominators = MaxNominatorRewardedPerValidator;
	type KeyOwnerProof =
		<Historical as KeyOwnerProofSystem<(KeyTypeId, pallet_babe::AuthorityId)>>::Proof;
	type EquivocationReportSystem =
		pallet_babe::EquivocationReportSystem<Self, Offences, Historical, ReportLongevity>;
}

impl pallet_grandpa::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type MaxSetIdSessionEntries = frame_support::traits::ConstU64<0>;
	type MaxAuthorities = MaxAuthorities;
	type EquivocationReportSystem =
		pallet_grandpa::EquivocationReportSystem<Self, Offences, Historical, ReportLongevity>;
	type KeyOwnerProof = <Historical as KeyOwnerProofSystem<(KeyTypeId, GrandpaId)>>::Proof;
	type MaxNominators = MaxNominatorRewardedPerValidator;
	type WeightInfo = ();
}

impl pallet_authorship::Config for Runtime {
	type EventHandler = Staking;
	type FindAuthor = pallet_session::FindAccountFromAuthorIndex<Self, Babe>;
}

use crate::opaque::SessionKeys;

impl pallet_session::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type ValidatorId = <Self as frame_system::Config>::AccountId;
	type ValidatorIdOf = pallet_staking::StashOf<Self>;
	type ShouldEndSession = Babe;
	type NextSessionRotation = Babe;
	type SessionManager = pallet_session::historical::NoteHistoricalRoot<Self, Staking>;
	type SessionHandler = <SessionKeys as OpaqueKeys>::KeyTypeIdProviders;
	type Keys = SessionKeys;
	type WeightInfo = pallet_session::weights::SubstrateWeight<Runtime>;
}

impl pallet_session::historical::Config for Runtime {
	type FullIdentification = pallet_staking::Exposure<AccountId, Balance>;
	type FullIdentificationOf = pallet_staking::ExposureOf<Runtime>;
}


parameter_types! {
	// Six sessions in an era (24 hours).
	pub const SessionsPerEra: sp_staking::SessionIndex = SESSIONS_PER_ERA;
	// 14 days for unbonding (14 days).
	pub const BondingDuration: sp_staking::EraIndex = BONDING_DURATION;
	// 10 days for slash defer.
	pub const SlashDeferDuration: sp_staking::EraIndex = SLASH_DEFER_DURATION;
	pub const MaxNominatorRewardedPerValidator: u32 = MAX_NOMINATOR_REWARDED_PER_VALIDATOR;
	pub OffchainRepeat: BlockNumber = OFFCHAIN_REPEAT;
	pub const HistoryDepth: u32 = HISTORY_DEPTH;
	pub const MaxControllersInDeprecationBatch: u32 = 500;
	pub const MaxExposurePageSize: u32 = 64;
}

pub struct StakingBenchmarkingConfig;
impl pallet_staking::BenchmarkingConfig for StakingBenchmarkingConfig {
	type MaxNominators = ConstU32<1000>;
	type MaxValidators = ConstU32<1000>;
}

pub struct FixedPayout;

impl pallet_staking::EraPayout<Balance> for FixedPayout {
    fn era_payout(
        _total_staked: Balance,
        _total_issuance: Balance,
        _era_duration_millis: u64,
    ) -> (Balance, Balance) {
        // Fixed payout: 10 coins per era.
        (10u32.into(), 0u32.into()) // Reward, remainder (usually 0).
    }
}

/// Upper limit on the number of NPOS nominations.
const MAX_QUOTA_NOMINATIONS: u32 = 16;

impl pallet_staking::Config for Runtime {
	type Currency = Balances;
	type CurrencyBalance = Balance;
	type AdminOrigin = EnsureRoot<AccountId>;
	type UnixTime = Timestamp;
	type CurrencyToVote = U128CurrencyToVote;
	type RewardRemainder = Treasury;
	type RuntimeEvent = RuntimeEvent;
	type Slash = Treasury; // send the slashed funds to the treasury.
	type Reward = (); // rewards are minted from the void
	type SessionsPerEra = SessionsPerEra;
	type BondingDuration = BondingDuration;
	type SlashDeferDuration = SlashDeferDuration;
	type SessionInterface = Self;
	type TargetList = pallet_staking::UseValidatorsMap<Runtime>;
	type EraPayout = FixedPayout;
	type NextNewSession = Session;
	type MaxExposurePageSize = MaxExposurePageSize;
	type MaxControllersInDeprecationBatch = MaxControllersInDeprecationBatch;
	type ElectionProvider = ElectionProviderMultiPhase;
	type GenesisElectionProvider = onchain::OnChainExecution<OnChainSeqPhragmen>;
	type VoterList = BagsList;
	type MaxUnlockingChunks = ConstU32<32>;
	type HistoryDepth = HistoryDepth;
	type EventListeners = NominationPools;
	type WeightInfo = pallet_staking::weights::SubstrateWeight<Runtime>;
	type NominationsQuota = pallet_staking::FixedNominationsQuota<MAX_QUOTA_NOMINATIONS>;
	type BenchmarkingConfig = StakingBenchmarkingConfig;
	type DisablingStrategy = pallet_staking::UpToLimitDisablingStrategy;
}

parameter_types! {
	pub const LaunchPeriod: BlockNumber = LAUNCH_PERIOD;
	pub const VotingPeriod: BlockNumber = VOTING_PERIOD;
	pub const FastTrackVotingPeriod: BlockNumber = FASTTRACK_VOTING_PERIOD;
	pub const MinimumDeposit: Balance = MINIMUM_DEPOSIT;
	pub const EnactmentPeriod: BlockNumber = ENACTMENT_PERIOD;
	pub const CooloffPeriod: BlockNumber = COOLOFF_PERIOD;
	pub const MaxProposals: u32 = MAX_PROPOSALS;
}

type EnsureRootOrHalfCouncil = EitherOfDiverse<
	EnsureRoot<AccountId>,
	pallet_collective::EnsureProportionMoreThan<AccountId, CouncilCollective, 1, 2>,
>;

impl pallet_democracy::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type Currency = Balances;
	type SubmitOrigin = frame_system::EnsureSigned<AccountId>;
	type EnactmentPeriod = EnactmentPeriod;
	type LaunchPeriod = LaunchPeriod;
	type VotingPeriod = VotingPeriod;
	type VoteLockingPeriod = EnactmentPeriod; // Same as EnactmentPeriod
	type MinimumDeposit = MinimumDeposit;
	/// A straight majority of the council can decide what their next motion is.
	type ExternalOrigin =
		pallet_collective::EnsureProportionAtLeast<AccountId, CouncilCollective, 1, 2>;
	/// A super-majority can have the next scheduled referendum be a straight majority-carries vote.
	type ExternalMajorityOrigin =
		pallet_collective::EnsureProportionAtLeast<AccountId, CouncilCollective, 3, 4>;
	/// A unanimous council can have the next scheduled referendum be a straight default-carries
	/// (NTB) vote.
	type ExternalDefaultOrigin =
		pallet_collective::EnsureProportionAtLeast<AccountId, CouncilCollective, 1, 1>;
	/// Two thirds of the council can have an ExternalMajority/ExternalDefault vote
	/// be tabled immediately and with a shorter voting/enactment period.
	type FastTrackOrigin =
		pallet_collective::EnsureProportionAtLeast<AccountId, CouncilCollective, 2, 3>;
	type InstantOrigin =
		pallet_collective::EnsureProportionAtLeast<AccountId, CouncilCollective, 1, 1>;
	type InstantAllowed = frame_support::traits::ConstBool<true>;
	type FastTrackVotingPeriod = FastTrackVotingPeriod;
	// To cancel a proposal which has been passed, 2/3 of the council must agree to it.
	type CancellationOrigin =
		pallet_collective::EnsureProportionAtLeast<AccountId, CouncilCollective, 2, 3>;
	// To cancel a proposal before it has been passed, the council must be unanimous or
	// Root must agree.
	type CancelProposalOrigin = EitherOfDiverse<
		EnsureRoot<AccountId>,
		pallet_collective::EnsureProportionAtLeast<AccountId, CouncilCollective, 1, 1>,
	>;
	type BlacklistOrigin = EnsureRoot<AccountId>;
	// Any single council member may veto a coming council proposal, however they can
	// only do it once and it lasts only for the cool-off period.
	type VetoOrigin = pallet_collective::EnsureMember<AccountId, CouncilCollective>;
	type CooloffPeriod = CooloffPeriod;
	type Slash = Treasury;
	type Scheduler = Scheduler;
	type PalletsOrigin = OriginCaller;
	type MaxVotes = ConstU32<100>;
	type WeightInfo = pallet_democracy::weights::SubstrateWeight<Runtime>;
	type MaxProposals = MaxProposals;
	type Preimages = Preimage;
	type MaxDeposits = ConstU32<100>;
	type MaxBlacklisted = ConstU32<100>;
}

parameter_types! {
	pub const CouncilMotionDuration: BlockNumber = 5 * DAYS;
	pub const CouncilMaxProposals: u32 = 100;
	pub const CouncilMaxMembers: u32 = 100;
	pub MaxProposalWeight: Weight = Perbill::from_percent(50) * BlockWeights::get().max_block;
}

type CouncilCollective = pallet_collective::Instance1;
impl pallet_collective::Config<CouncilCollective> for Runtime {
	type RuntimeOrigin = RuntimeOrigin;
	type Proposal = RuntimeCall;
	type RuntimeEvent = RuntimeEvent;
	type MotionDuration = CouncilMotionDuration;
	type MaxProposals = CouncilMaxProposals;
	type SetMembersOrigin = EnsureRoot<AccountId>;
	type MaxMembers = CouncilMaxMembers;
	type DefaultVote = pallet_collective::PrimeDefaultVote;
	type WeightInfo = pallet_collective::weights::SubstrateWeight<Runtime>;
	type MaxProposalWeight = MaxProposalWeight;
}

parameter_types! {
	// phase durations. 1/4 of the last session for each.
	pub const SignedPhase: u64 = EPOCH_DURATION_IN_BLOCKS / 4;
	pub const UnsignedPhase: u64 = EPOCH_DURATION_IN_BLOCKS / 4;

	// signed config
	pub const SignedRewardBase: Balance = UNIT;
	pub const SignedDepositBase: Balance = UNIT;
	pub const SignedDepositByte: Balance = CENT;

	pub BetterUnsignedThreshold: Perbill = Perbill::from_rational(1u32, 10_000);

	// miner configs
	pub const MultiPhaseUnsignedPriority: TransactionPriority = StakingUnsignedPriority::get() - 1u64;
	pub MinerMaxWeight: Weight = BlockWeights::get()
		.get(DispatchClass::Normal)
		.max_extrinsic.expect("Normal extrinsics have a weight limit configured; qed")
		.saturating_sub(BlockExecutionWeight::get());
	// Solution can occupy 90% of normal block size
	pub MinerMaxLength: u32 = Perbill::from_rational(9u32, 10) *
		*BlockLength::get()
		.max
		.get(DispatchClass::Normal);
}

frame_election_provider_support::generate_solution_type!(
	#[compact]
	pub struct NposSolution16::<
		VoterIndex = u32,
		TargetIndex = u16,
		Accuracy = sp_runtime::PerU16,
		MaxVoters = MaxElectingVoters,
	>(16)
);

parameter_types! {
	pub MaxNominations: u32 = <NposSolution16 as frame_election_provider_support::NposSolution>::LIMIT as u32;
	pub MaxElectingVoters: u32 = 40_000;
	pub MaxElectableTargets: u16 = 10_000;
	// OnChain values are lower.
	pub MaxOnChainElectingVoters: u32 = 5000;
	pub MaxOnChainElectableTargets: u16 = 1250;
	// The maximum winners that can be elected by the Election pallet which is equivalent to the
	// maximum active validators the staking pallet can have.
	pub MaxActiveValidators: u32 = 1000;
	pub ElectionBoundsOnChain: ElectionBounds = ElectionBoundsBuilder::default()
		.voters_count(5_000.into()).targets_count(1_250.into()).build();
	pub ElectionBoundsMultiPhase: ElectionBounds = ElectionBoundsBuilder::default()
		.voters_count(10_000.into()).targets_count(1_500.into()).build();
}

/// The numbers configured here could always be more than the the maximum limits of staking pallet
/// to ensure election snapshot will not run out of memory. For now, we set them to smaller values
/// since the staking is bounded and the weight pipeline takes hours for this single pallet.
pub struct ElectionProviderBenchmarkConfig;
impl pallet_election_provider_multi_phase::BenchmarkingConfig for ElectionProviderBenchmarkConfig {
	const VOTERS: [u32; 2] = [1000, 2000];
	const TARGETS: [u32; 2] = [500, 1000];
	const ACTIVE_VOTERS: [u32; 2] = [500, 800];
	const DESIRED_TARGETS: [u32; 2] = [200, 400];
	const SNAPSHOT_MAXIMUM_VOTERS: u32 = 1000;
	const MINER_MAXIMUM_VOTERS: u32 = 1000;
	const MAXIMUM_TARGETS: u32 = 300;
}

/// Maximum number of iterations for balancing that will be executed in the embedded OCW
/// miner of election provider multi phase.
pub const MINER_MAX_ITERATIONS: u32 = 10;

/// A source of random balance for NposSolver, which is meant to be run by the OCW election miner.
pub struct OffchainRandomBalancing;
impl Get<Option<BalancingConfig>> for OffchainRandomBalancing {
	fn get() -> Option<BalancingConfig> {
		use sp_runtime::traits::TrailingZeroInput;
		let iterations = match MINER_MAX_ITERATIONS {
			0 => 0,
			max => {
				let seed = sp_io::offchain::random_seed();
				let random = <u32>::decode(&mut TrailingZeroInput::new(&seed))
					.expect("input is padded with zeroes; qed")
					% max.saturating_add(1);
				random as usize
			},
		};

		let config = BalancingConfig { iterations, tolerance: 0 };
		Some(config)
	}
}

pub struct OnChainSeqPhragmen;
impl onchain::Config for OnChainSeqPhragmen {
	type System = Runtime;
	type Solver = SequentialPhragmen<
		AccountId,
		pallet_election_provider_multi_phase::SolutionAccuracyOf<Runtime>,
	>;
	type DataProvider = <Runtime as pallet_election_provider_multi_phase::Config>::DataProvider;
	type WeightInfo = frame_election_provider_support::weights::SubstrateWeight<Runtime>;
	type MaxWinners = <Runtime as pallet_election_provider_multi_phase::Config>::MaxWinners;
	type Bounds = ElectionBoundsOnChain;
}

impl pallet_election_provider_multi_phase::MinerConfig for Runtime {
	type AccountId = AccountId;
	type MaxLength = MinerMaxLength;
	type MaxWeight = MinerMaxWeight;
	type Solution = NposSolution16;
	type MaxWinners = MaxActiveValidators;
	type MaxVotesPerVoter =
	<<Self as pallet_election_provider_multi_phase::Config>::DataProvider as ElectionDataProvider>::MaxVotesPerVoter;

	// The unsigned submissions have to respect the weight of the submit_unsigned call, thus their
	// weight estimate function is wired to this call's weight.
	fn solution_weight(v: u32, t: u32, a: u32, d: u32) -> Weight {
		<
			<Self as pallet_election_provider_multi_phase::Config>::WeightInfo
			as
			pallet_election_provider_multi_phase::WeightInfo
		>::submit_unsigned(v, t, a, d)
	}
}

parameter_types! {
	pub const SignedFixedDeposit: Balance = 1; // Need to be updated with correct value.
	pub const SignedDepositIncreaseFactor: Percent = Percent::from_percent(10);
}

impl pallet_election_provider_multi_phase::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type Currency = Balances;
	type EstimateCallFee = TransactionPayment;
	type SignedPhase = SignedPhase;
	type UnsignedPhase = UnsignedPhase;
	type BetterSignedThreshold = ();
	type OffchainRepeat = OffchainRepeat;
	type MinerTxPriority = MultiPhaseUnsignedPriority;
	type MinerConfig = Self;
	type SignedMaxSubmissions = ConstU32<10>;
	type SignedRewardBase = SignedRewardBase;
	type SignedDepositBase =
		GeometricDepositBase<Balance, SignedFixedDeposit, SignedDepositIncreaseFactor>;
	type SignedDepositByte = SignedDepositByte;
	type SignedMaxRefunds = ConstU32<3>;
	type SignedDepositWeight = ();
	type SignedMaxWeight = MinerMaxWeight;
	type SlashHandler = (); // burn slashes
	type RewardHandler = (); // nothing to do upon rewards
	type DataProvider = Staking;
	type Fallback = onchain::OnChainExecution<OnChainSeqPhragmen>;
	type GovernanceFallback = onchain::OnChainExecution<OnChainSeqPhragmen>;
	type Solver = SequentialPhragmen<AccountId, SolutionAccuracyOf<Self>, OffchainRandomBalancing>;
	type ForceOrigin = EnsureRootOrHalfCouncil;
	type MaxWinners = MaxActiveValidators;
	type BenchmarkingConfig = ElectionProviderBenchmarkConfig;
	type ElectionBounds = ElectionBoundsMultiPhase;
	type WeightInfo = pallet_election_provider_multi_phase::weights::SubstrateWeight<Self>;
}

parameter_types! {
	pub const BagThresholds: &'static [u64] = &voter_bags::THRESHOLDS;
}

impl pallet_bags_list::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type ScoreProvider = Staking;
	type WeightInfo = pallet_bags_list::weights::SubstrateWeight<Runtime>;
	type BagThresholds = BagThresholds;
	type Score = VoteWeight;
}

parameter_types! {
  pub const PostUnbondPoolsWindow: u32 = 4;
  pub const NominationPoolsPalletId: PalletId = PalletId(*b"py/nopls");
  pub const MaxPointsToBalance: u8 = 10;
}

pub struct BalanceToU256;
impl Convert<Balance, sp_core::U256> for BalanceToU256 {
	fn convert(balance: Balance) -> sp_core::U256 {
		sp_core::U256::from(balance)
	}
}
pub struct U256ToBalance;
impl Convert<sp_core::U256, Balance> for U256ToBalance {
	fn convert(n: sp_core::U256) -> Balance {
		n.try_into().unwrap_or(Balance::MAX)
	}
}

impl pallet_nomination_pools::Config for Runtime {
	type WeightInfo = ();
	type RuntimeEvent = RuntimeEvent;
	type Currency = Balances;
	type RewardCounter = FixedU128;
	type BalanceToU256 = BalanceToU256;
	type U256ToBalance = U256ToBalance;
	type PostUnbondingPoolsWindow = PostUnbondPoolsWindow;
	type MaxMetadataLen = ConstU32<256>;
	type MaxUnbonding = ConstU32<8>;
	type PalletId = NominationPoolsPalletId;
	type MaxPointsToBalance = MaxPointsToBalance;
	type RuntimeFreezeReason = RuntimeFreezeReason;
	type AdminOrigin = EnsureRoot<AccountId>;
	type StakeAdapter = pallet_nomination_pools::adapter::TransferStake<Self, Staking>;
}

parameter_types! {
	pub const ImOnlineUnsignedPriority: TransactionPriority = TransactionPriority::MAX;
	/// We prioritize im-online heartbeats over election solution submission.
	pub const StakingUnsignedPriority: TransactionPriority = TransactionPriority::MAX / 2;
}

impl<LocalCall> frame_system::offchain::CreateSignedTransaction<LocalCall> for Runtime
where
	RuntimeCall: From<LocalCall>,
{
	fn create_transaction<C: frame_system::offchain::AppCrypto<Self::Public, Self::Signature>>(
		call: RuntimeCall,
		public: <Signature as traits::Verify>::Signer,
		account: AccountId,
		nonce: Nonce,
	) -> Option<(RuntimeCall, <UncheckedExtrinsic as traits::Extrinsic>::SignaturePayload)> {
		let tip = 0;
		// take the biggest period possible.
		let period = BlockHashCount::get().checked_next_power_of_two().map(|c| c / 2).unwrap_or(2);
		let current_block = System::block_number()
			.saturated_into::<u64>()
			// The `System::block_number` is initialized with `n+1`,
			// so the actual block number is `n`.
			.saturating_sub(1);
		let era = Era::mortal(period, current_block);
		let extra = (
			frame_system::CheckNonZeroSender::<Runtime>::new(),
			frame_system::CheckSpecVersion::<Runtime>::new(),
			frame_system::CheckTxVersion::<Runtime>::new(),
			frame_system::CheckGenesis::<Runtime>::new(),
			frame_system::CheckEra::<Runtime>::from(era),
			frame_system::CheckNonce::<Runtime>::from(nonce),
			frame_system::CheckWeight::<Runtime>::new(),
			pallet_transaction_payment::ChargeTransactionPayment::<Runtime>::from(tip),
			frame_metadata_hash_extension::CheckMetadataHash::<Runtime>::new(true),
		);
		let raw_payload = SignedPayload::new(call, extra)
			.map_err(|e| {
				log::warn!("Unable to create signed payload: {:?}", e);
			})
			.ok()?;
		let signature = raw_payload.using_encoded(|payload| C::sign(payload, public))?;
		let address = Indices::unlookup(account);
		let (call, extra, _) = raw_payload.deconstruct();
		Some((call, (address, signature, extra)))
	}
}

impl frame_system::offchain::SigningTypes for Runtime {
	type Public = <Signature as traits::Verify>::Signer;
	type Signature = Signature;
}

impl<C> frame_system::offchain::SendTransactionTypes<C> for Runtime
where
	RuntimeCall: From<C>,
{
	type Extrinsic = UncheckedExtrinsic;
	type OverarchingCall = RuntimeCall;
}

parameter_types! {
	pub const MinVestedTransfer: Balance = 10 * UNIT;
	pub UnvestedFundsAllowedWithdrawReasons: WithdrawReasons =
		WithdrawReasons::except(WithdrawReasons::TRANSFER | WithdrawReasons::RESERVE);
}

impl pallet_vesting::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type Currency = Balances;
	type BlockNumberToBalance = ConvertInto;
	type BlockNumberProvider = System;
	type MinVestedTransfer = MinVestedTransfer;
	type WeightInfo = pallet_vesting::weights::SubstrateWeight<Runtime>;
	type UnvestedFundsAllowedWithdrawReasons = UnvestedFundsAllowedWithdrawReasons;
	// `VestingInfo` encode length is 36bytes. 28 schedules gets encoded as 1009 bytes, which is the
	// highest number of schedules that encodes less than 2^10.
	const MAX_VESTING_SCHEDULES: u32 = 28;
}

parameter_types! {
    pub const FinneyApiUrl: &'static str = "https://tao-testnet.hippius.network";
    pub const FinneyUidsStorageKey: &'static str = "0x658faa385070e074c85bf6b568cf0555aab1b4e78e1ea8305462ee53b3686dc80100";
    pub const FinneyDividendsStorageKey: &'static str = "0x658faa385070e074c85bf6b568cf055586752d66f11480ecef37769cdd736b9b0100";
	pub const UidsSubmissionInterval: u32 = 33; // Set your desired value here
}

impl pallet_metagraph::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type FinneyUrl = FinneyApiUrl;
    type UidsStorageKey = FinneyUidsStorageKey;
    type DividendsStorageKey = FinneyDividendsStorageKey;
	type UidsSubmissionInterval = UidsSubmissionInterval;
	type AuthorityId = pallet_metagraph::crypto::TestAuthId;
}

parameter_types! {
    pub const IpfsBaseUrl: &'static str = "http://127.0.0.1:5001";
	pub const GarbageCollectorInterval : u32 = 14;
	pub const MinerIPFSCHeckInterval : u32 = 10;
}

impl pallet_ipfs_pin::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = pallet_ipfs_pin::weights::SubstrateWeight<Runtime>;
    type IpfsBaseUrl = IpfsBaseUrl;
	type GarbageCollectorInterval = GarbageCollectorInterval;
	type MinerIPFSCHeckInterval = MinerIPFSCHeckInterval;
	type AuthorityId = pallet_ipfs_pin::crypto::TestAuthId;
}

parameter_types! {
    pub const AlphaPalletId: PalletId = PalletId(*b"Alpha123");
}

impl pallet_alpha_bridge::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type MinConfirmations = frame_support::traits::ConstU32<1>; // 2-of-3
	type Balance = Balance;
	type PalletId = AlphaPalletId;
}

parameter_types! {
    pub const VersionKeyStorageKey: &'static str = "0x658faa385070e074c85bf6b568cf0555d8cb0c0627a5cd77797c62415dbef9621300";
	pub const BittensorCallSubmission : u32 = 20;
}

impl pallet_bittensor::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type FinneyRpcUrl = FinneyApiUrl;
	type VersionKeyStorageKey = VersionKeyStorageKey;
	type BittensorCallSubmission = BittensorCallSubmission;
	type NetUid = ConstU16<30>;
}

parameter_types! {
    pub const ResgisterPalletId: PalletId = PalletId(*b"register");
    pub const StorageMinerInitialFee: Balance = 100_000_000_000; // 100 tokens
	pub const StorageMiners3InitialFee: Balance = 100_000_000_000; // 100 tokens
    pub const ValidatorInitialFee: Balance = 200_000_000_000; // 200 tokens
    pub const ComputeMinerInitialFee: Balance = 150_000_000_000; // 150 tokens
	pub const GpuMinerInitialFee: Balance = 150_000_000_000; // 150 tokens
}

impl pallet_registration::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    // Use Pallet instead of the crate name
    type MetagraphInfo = pallet_metagraph::Pallet<Runtime>;
	type MinerStakeThreshold = ConstU32<0>;
	type ChainDecimals = ConstU32<18>;
	type PalletId = ResgisterPalletId;
	type StorageMinerInitialFee = StorageMinerInitialFee;
    type ValidatorInitialFee = ValidatorInitialFee;
    type ComputeMinerInitialFee = ComputeMinerInitialFee;
	type StorageMiners3InitialFee = StorageMiners3InitialFee;
    type GpuMinerInitialFee = GpuMinerInitialFee;
	type BlocksPerDay = BlocksPerDay;
}

parameter_types! {
	pub const BlocksPerDay: u32 = HOURS as u32 * 24;
	pub const BlocksPerBackupCheck: u32 =  30;
}

impl pallet_backup::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
	type AuthorityId = pallet_backup::crypto::TestAuthId;
	type BlocksPerDay = BlocksPerBackupCheck ;
}

use pallet_credits::crypto::TestAuthId; // Adjust the path as necessary

impl pallet_credits::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type AuthorityId = TestAuthId;
	type RefferallCoolDOwnPeriod = RefferallCoolDOwnPeriod;
	// type OnRuntimeUpgrade = pallet_credits::migrations::Migrate<Runtime>;
}

parameter_types! {
	pub const IpReleasePeriod: u64 = 15 * DAYS;
}

impl pallet_compute::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type AuthorityId = pallet_compute::crypto::TestAuthId;
	type OffchainWorkerInterval = ConstU32<19>;
	type IpReleasePeriod = IpReleasePeriod;
}

parameter_types! {
	pub const MaxCidLenght: u32 = 2;
}

impl pallet_container_registry::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type MaxLength = MaxCidLenght;
}


parameter_types! {
    pub const RankingPalletId: PalletId = PalletId(*b"ranking1");
    pub const SecondRankingPalletId: PalletId = PalletId(*b"ranking2");
	pub const ThirdRankingPalletId: PalletId = PalletId(*b"ranking3");
	pub const FourthRankingPalletId: PalletId = PalletId(*b"ranking4");
    pub const ComputeNodesRewardPercentage: u32 = 40;
    pub const MinerNodesRewardPercentage: u32 = 60;
	pub const RankingsInstanceId1: u16 = 1;
	pub const RankingsInstanceId2: u16 = 2;
	pub const RankingsInstanceId3: u16 = 3;
	pub const RankingsInstanceId4: u16 = 4;
	pub const RankingsInstanceId5: u16 = 5;
}

// First ranking pallet implementation remains the same
impl pallet_rankings::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type PalletId = RankingPalletId;
    type ComputeNodesRewardPercentage = ComputeNodesRewardPercentage;
    type MinerNodesRewardPercentage = MinerNodesRewardPercentage;
	type InstanceID = RankingsInstanceId1;
	type AuthorityId = pallet_rankings::crypto::TestAuthId;
	type BlocksPerEra = BlocksPerEra;
}

// Add a second ranking pallet implementation
impl pallet_rankings::Config<pallet_rankings::Instance2> for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type PalletId = SecondRankingPalletId;
    type ComputeNodesRewardPercentage = ComputeNodesRewardPercentage;
    type MinerNodesRewardPercentage = MinerNodesRewardPercentage;
	type InstanceID = RankingsInstanceId2;
	type AuthorityId = pallet_rankings::crypto::TestAuthId;
	type BlocksPerEra = BlocksPerEra;
	// type Public = sp_core::sr25519::Public; // Using sr25519
    // type Signature = sp_core::sr25519::Signature; // Using sr25519
}

// Add a second ranking pallet implementation
impl pallet_rankings::Config<pallet_rankings::Instance3> for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type PalletId = ThirdRankingPalletId;
    type ComputeNodesRewardPercentage = ComputeNodesRewardPercentage;
    type MinerNodesRewardPercentage = MinerNodesRewardPercentage;
	type InstanceID = RankingsInstanceId3;
	type AuthorityId = pallet_rankings::crypto::TestAuthId;
	type BlocksPerEra = BlocksPerEra;
	// type Public = sp_core::sr25519::Public; // Using sr25519
    // type Signature = sp_core::sr25519::Signature; // Using sr25519
}


// Add a second ranking pallet implementation
impl pallet_rankings::Config<pallet_rankings::Instance4> for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type PalletId = FourthRankingPalletId;
    type ComputeNodesRewardPercentage = ComputeNodesRewardPercentage;
    type MinerNodesRewardPercentage = MinerNodesRewardPercentage;
	type InstanceID = RankingsInstanceId4;
	type AuthorityId = pallet_rankings::crypto::TestAuthId;
	type BlocksPerEra = BlocksPerEra;
}

// Add a second ranking pallet implementation
impl pallet_rankings::Config<pallet_rankings::Instance5> for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type PalletId = FourthRankingPalletId;
    type ComputeNodesRewardPercentage = ComputeNodesRewardPercentage;
    type MinerNodesRewardPercentage = MinerNodesRewardPercentage;
	type InstanceID = RankingsInstanceId5;
	type AuthorityId = pallet_rankings::crypto::TestAuthId;
	type BlocksPerEra = BlocksPerEra;
}

parameter_types! {
    pub const LocalRpcUrl: &'static str = "http://localhost:9944";
    pub const IpfsPinRpcMethod: &'static str = "extra_peerId";
}

impl pallet_utils::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
	type LocalRpcUrl = LocalRpcUrl;
    type RpcMethod = IpfsPinRpcMethod;
}


impl pallet_account_profile::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
}

parameter_types! {
    pub const CooldownPeriodInBlocks: u32 = 20;
}

impl pallet_notifications::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type CooldownPeriod = CooldownPeriodInBlocks;
}

parameter_types! {
    pub const ExecutionUnitRpcUrl: &'static str = "http://localhost:9944";
    pub const ExecutionUnitSystemInfoRpcMethod: &'static str = "sys_getSystemInfo";
	pub const BlockTimeSecs :u64 =  SECONDS_PER_BLOCK;
	/// number of blocks at which uptime will be checked
	pub const BlockCheckInterval : u32 = 12;
	pub const GetReadProofRpcMethod: &'static str = "state_getReadProof";
	pub const SystemHealthRpcMethod: &'static str = "system_health";
	pub const IPFSBaseUrl: &'static str = "http://localhost:5001";
}

impl pallet_execution_unit::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = pallet_execution_unit::weights::SubstrateWeight<Runtime>;
    type LocalRpcUrl = ExecutionUnitRpcUrl;
    type SystemInfoRpcMethod = ExecutionUnitSystemInfoRpcMethod;
	type BlockTime = BlockTimeSecs;
	type BlockCheckInterval = BlockCheckInterval;
	type GetReadProofRpcMethod = GetReadProofRpcMethod;
	type SystemHealthRpcMethod = SystemHealthRpcMethod;
	type IPFSBaseUrl = IPFSBaseUrl;
	type AuthorityId = pallet_execution_unit::crypto::TestAuthId;	
}

parameter_types! {
    pub const MarketplaceMinSubscriptionBlocks: BlockNumber = MONTH;
    pub const MaxActiveSubscriptionsPerUser: u32 = 5;
}

pub struct EnsureSubscriptionOwner;
impl<OuterOrigin> frame_support::traits::EnsureOrigin<OuterOrigin> for EnsureSubscriptionOwner 
where
    OuterOrigin: Into<Result<frame_system::RawOrigin<AccountId>, OuterOrigin>>
        + From<frame_system::RawOrigin<AccountId>>,
{
    type Success = AccountId;
    fn try_origin(o: OuterOrigin) -> Result<Self::Success, OuterOrigin> {
        o.into().and_then(|o| match o {
            frame_system::RawOrigin::Signed(who) => Ok(who),
            r => Err(OuterOrigin::from(r)),
        })
    }

    #[cfg(feature = "runtime-benchmarks")]
    fn try_successful_origin() -> Result<OuterOrigin, ()> {
        let zero_account = AccountId::decode(&mut sp_runtime::traits::TrailingZeroInput::zeroes())
            .expect("infinite length input; no invalid inputs for type; qed");
        Ok(OuterOrigin::from(frame_system::RawOrigin::Signed(zero_account)))
    }
}

pub struct MarketplaceRewardPayout;

impl pallet_staking::EraPayout<Balance> for MarketplaceRewardPayout {
    fn era_payout(
        _total_staked: Balance,
        _total_issuance: Balance,
        _era_duration_millis: u64,
    ) -> (Balance, Balance) {
		
        // Fetch the balance available in the marketplace
        let marketplace_balance = pallet_marketplace::Pallet::<Runtime>::balance();
		let registration_balance = pallet_registration::Pallet::<Runtime>::balance();
		let marketplace_account = pallet_marketplace::Pallet::<Runtime>::account_id();
		let registration_account = pallet_registration::Pallet::<Runtime>::account_id();
		// Transfer to treasury
		let recipient_account = AccountId32::from_ss58check("5GEudEYMVWJr64Y3599urXfG1tg4u7iNFWmBYZUET2YTdPkn")
			.expect("Invalid SS58 address");

        if marketplace_balance > 0 {
            // Calculate amounts for each destination
            let staking_amount = marketplace_balance
                .checked_mul(65u32.into())
                .and_then(|x| x.checked_div(100u32.into()))
                .unwrap_or_default();

            let treasury_amount = marketplace_balance
                .checked_mul(35u32.into())
                .and_then(|x| x.checked_div(100u32.into()))
                .unwrap_or_default();

			// Transfer to the specific account
			let _ = pallet_balances::Pallet::<Runtime>::transfer(
				&marketplace_account.clone(),
				&recipient_account,
				treasury_amount,
				ExistenceRequirement::KeepAlive
			);

            // // Burn the staking amount
            // let _ = pallet_balances::Pallet::<Runtime>::burn(
            //     frame_system::RawOrigin::Signed(marketplace_account.clone()).into(),
            //     staking_amount,
            //     false, // keep_alive set to false to allow burning entire balance
            // );

            // Get the list of validators from the session
            let validators = <pallet_session::Pallet<Runtime>>::validators(); // Ensure you have the correct type here
            let num_validators = validators.len() as u32;
            if num_validators > 0 {
                let amount_per_validator = staking_amount.checked_div(num_validators.into()).unwrap_or_default();

                for validator in validators {
					let _ = pallet_balances::Pallet::<Runtime>::transfer(
						&marketplace_account.clone(),
						&validator,
						amount_per_validator,
						ExistenceRequirement::KeepAlive,
					);

					let _ = pallet_staking::Pallet::<Runtime>::bond(
						frame_system::RawOrigin::Signed(validator.clone()).into(),
						amount_per_validator,
						pallet_staking::RewardDestination::Staked,
					);
                }
            }
        }

		if registration_balance > 0 {
			
            // Calculate amounts for each destination
            let staking_amount = registration_balance
                .checked_mul(50u32.into())
                .and_then(|x| x.checked_div(100u32.into()))
                .unwrap_or_default();

            let treasury_amount = registration_balance
                .checked_mul(50u32.into())
                .and_then(|x| x.checked_div(100u32.into()))
                .unwrap_or_default();

			// Transfer to the specific account
			let _ = pallet_balances::Pallet::<Runtime>::transfer(
				&registration_account.clone(),
				&recipient_account,
				treasury_amount,
				ExistenceRequirement::KeepAlive
			);

            // Get the list of validators from the session
            let validators = <pallet_session::Pallet<Runtime>>::validators(); // Ensure you have the correct type here
            let num_validators = validators.len() as u32;
            if num_validators > 0 {
                let amount_per_validator = staking_amount.checked_div(num_validators.into()).unwrap_or_default();

                for validator in validators {
					// Transfer the amount to the validator's account first
					let _ = pallet_balances::Pallet::<Runtime>::transfer(
						&registration_account.clone(),
						&validator,
						amount_per_validator,
						ExistenceRequirement::KeepAlive,
					);

					// Now bond the amount as staked
					let _ = pallet_staking::Pallet::<Runtime>::bond(
						frame_system::RawOrigin::Signed(validator.clone()).into(),
						amount_per_validator,
						pallet_staking::RewardDestination::Staked,
					);
                }
            }
        }

        // No payout if no funds are available
        (0u32.into(), 0u32.into())
    }
}


pub struct TransferDustToTreasury;

type FungibleImbalance = frame_support::traits::fungible::Imbalance<
    Balance,
    frame_support::traits::fungible::DecreaseIssuance<AccountId, Balances>,
    frame_support::traits::fungible::IncreaseIssuance<AccountId, Balances>,
>;

impl OnUnbalanced<FungibleImbalance> for TransferDustToTreasury {
    fn on_unbalanced(amount: FungibleImbalance) {
        let treasury_account = Treasury::account_id();
        
        // Convert the imbalance to the correct type
        let negative_imbalance = pallet_balances::NegativeImbalance::<Runtime>::new(amount.peek());
        
        // Resolve the converted imbalance to the treasury account
        Balances::resolve_creating(&treasury_account, negative_imbalance);
    }
}

parameter_types! {
    pub const MarketplacePalletId: PalletId = PalletId(*b"mrktplce");
	pub const BlockDurationMillis: u64 = MILLISECS_PER_BLOCK;
	pub const BlocksPerHour: u32 = HOURS as u32;
	// as era is of 6 hours 
	pub const BlocksPerEra: u32 = (HOURS * 6) as u32;
	pub const RefferallCoolDOwnPeriod : u32 = 200;
	pub const BlockChargeCheckInterval: u32 = 8;
	// storage and compute grace periods
	pub const StorageGracePeriod: u32 = 0;
	pub const ComputeGracePeriod: u32 = 0;
}

impl pallet_marketplace::Config for Runtime {
    type RuntimeEvent = RuntimeEvent;
    type Currency = Balances;
	type Balance = Balance;
    type MinSubscriptionBlocks = MarketplaceMinSubscriptionBlocks;
    type MaxActiveSubscriptions = MaxActiveSubscriptionsPerUser;
    type UpdateOrigin = EnsureSubscriptionOwner;
	type PalletId = MarketplacePalletId;
	type BlockDurationMillis = BlockDurationMillis;
	type BlocksPerEra = BlocksPerEra;
	type StorageGracePeriod = StorageGracePeriod;
	type ComputeGracePeriod = ComputeGracePeriod;
	type CustomHash = sp_core::H256;
	type BlocksPerHour = BlocksPerHour;
	type BlockChargeCheckInterval = BlockChargeCheckInterval;
	type AuthorityId = pallet_marketplace::crypto::TestAuthId;	
}

impl pallet_offences::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type IdentificationTuple = pallet_session::historical::IdentificationTuple<Self>;
	type OnOffenceHandler = Staking;
}

parameter_types! {
	pub const CandidacyBond: Balance = CANDIDACY_BOND;
	// 1 storage item created, key size is 32 bytes, value size is 16+16.
	pub const VotingBondBase: Balance = deposit(1, 64);
	// additional data per vote is 32 bytes (account id).
	pub const VotingBondFactor: Balance = deposit(0, 32);
	pub const TermDuration: BlockNumber = TERM_DURATION;
	pub const DesiredMembers: u32 = DESIRED_MEMBERS;
	pub const DesiredRunnersUp: u32 = DESIRED_RUNNERS_UP;
	pub const MaxCandidates: u32 = MAX_CANDIDATES;
	pub const MaxVoters: u32 = MAX_VOTERS;
	pub const MaxVotesPerVoter: u32 = MAX_VOTES_PER_VOTER;
	pub const ElectionsPhragmenPalletId: LockIdentifier = ELECTIONS_PHRAGMEN_PALLET_ID;
}

// Make sure that there are no more than `MaxMembers` members elected via
// elections-phragmen.
const_assert!(DesiredMembers::get() <= CouncilMaxMembers::get());

impl pallet_elections_phragmen::Config for Runtime {
	type CandidacyBond = CandidacyBond;
	type ChangeMembers = Council;
	type Currency = Balances;
	type CurrencyToVote = U128CurrencyToVote;
	type DesiredMembers = DesiredMembers;
	type DesiredRunnersUp = DesiredRunnersUp;
	type RuntimeEvent = RuntimeEvent;
	type MaxVotesPerVoter = MaxVotesPerVoter;
	// NOTE: this implies that council's genesis members cannot be set directly and
	// must come from this module.
	type InitializeMembers = Council;
	type KickedMember = ();
	type LoserCandidate = ();
	type PalletId = ElectionsPhragmenPalletId;
	type TermDuration = TermDuration;
	type MaxCandidates = MaxCandidates;
	type MaxVoters = MaxVoters;
	type VotingBondBase = VotingBondBase;
	type VotingBondFactor = VotingBondFactor;
	type WeightInfo = pallet_elections_phragmen::weights::SubstrateWeight<Runtime>;
}

parameter_types! {
	pub const ProposalBond: Permill = PROPOSAL_BOND;
	pub const ProposalBondMinimum: Balance = PROPOSAL_BOND_MINIMUM;
	pub const SpendPeriod: BlockNumber = SPEND_PERIOD;
	pub const Burn: Permill = BURN;
	pub const TipCountdown: BlockNumber = TIP_COUNTDOWN;
	pub const TipFindersFee: Percent = TIP_FINDERS_FEE;
	pub const TipReportDepositBase: Balance = TIP_REPORT_DEPOSIT_BASE;
	pub const DataDepositPerByte: Balance = DATA_DEPOSIT_PER_BYTE;
	pub const TreasuryPalletId: PalletId = TREASURY_PALLET_ID;
	pub const MaximumReasonLength: u32 = MAXIMUM_REASON_LENGTH;
	pub const MaxApprovals: u32 = MAX_APPROVALS;
	pub const PayoutPeriod: BlockNumber = 30 * DAYS;
	pub TreasuryAccount: AccountId = Treasury::account_id();
	pub const MaxBalance: Balance = Balance::MAX;
}

impl pallet_treasury::Config for Runtime {
	type PalletId = TreasuryPalletId;
	type Currency = Balances;
	type RejectOrigin = EitherOfDiverse<
		EnsureRoot<AccountId>,
		pallet_collective::EnsureProportionMoreThan<AccountId, CouncilCollective, 1, 2>,
	>;
	type RuntimeEvent = RuntimeEvent;
	type AssetKind = ();
	type Beneficiary = AccountId;
	type BeneficiaryLookup = IdentityLookup<AccountId>;
	type Paymaster = PayFromAccount<Balances, TreasuryAccount>;
	type BalanceConverter = UnityAssetBalanceConversion;
	type PayoutPeriod = PayoutPeriod;
	type SpendPeriod = SpendPeriod;
	type Burn = Burn;
	type BurnDestination = ();
	type SpendOrigin = EnsureWithSuccess<EnsureRoot<AccountId>, AccountId, MaxBalance>;
	type SpendFunds = Bounties;
	type WeightInfo = pallet_treasury::weights::SubstrateWeight<Runtime>;
	type MaxApprovals = MaxApprovals;
}

parameter_types! {
	pub const BountyValueMinimum: Balance = 100 * UNIT;
	pub const BountyDepositBase: Balance = UNIT;
	pub const CuratorDepositMultiplier: Permill = Permill::from_percent(50);
	pub const CuratorDepositMin: Balance = UNIT;
	pub const CuratorDepositMax: Balance = 10_000 * UNIT;
	pub const BountyDepositPayoutDelay: BlockNumber = DAYS;
	pub const BountyUpdatePeriod: BlockNumber = 14 * DAYS;
}

impl pallet_bounties::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type BountyDepositBase = BountyDepositBase;
	type BountyDepositPayoutDelay = BountyDepositPayoutDelay;
	type BountyUpdatePeriod = BountyUpdatePeriod;
	type CuratorDepositMultiplier = CuratorDepositMultiplier;
	type CuratorDepositMin = CuratorDepositMin;
	type CuratorDepositMax = CuratorDepositMax;
	type BountyValueMinimum = BountyValueMinimum;
	type DataDepositPerByte = DataDepositPerByte;
	type MaximumReasonLength = MaximumReasonLength;
	type WeightInfo = pallet_bounties::weights::SubstrateWeight<Runtime>;
	type OnSlash = Treasury;
	type ChildBountyManager = ChildBounties;
}

parameter_types! {
	pub const ChildBountyValueMinimum: Balance = 10 * UNIT;
}

impl pallet_child_bounties::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type MaxActiveChildBountyCount = ConstU32<5>;
	type ChildBountyValueMinimum = ChildBountyValueMinimum;
	type WeightInfo = pallet_child_bounties::weights::SubstrateWeight<Runtime>;
}

parameter_types! {
	pub const SubAccountStringLimit: u32 = 300;
}

impl pallet_subaccount::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type WeightInfo = pallet_subaccount::weights::SubstrateWeight<Runtime>;
	type StringLimit = SubAccountStringLimit;
	// type OnRuntimeUpgrade = pallet_subaccount::migrations::MigrateToNewStorageFormat<Runtime>;
}

impl pallet_storage::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
}

parameter_types! {
	pub const MaxKeys: u32 = 10_000;
	pub const MaxPeerInHeartbeats: u32 = 10_000;
}

impl pallet_im_online::Config for Runtime {
	type AuthorityId = ImOnlineId;
	type RuntimeEvent = RuntimeEvent;
	type NextSessionRotation = Babe;
	type ValidatorSet = Historical;
	type ReportUnresponsiveness = ();
	type UnsignedPriority = ImOnlineUnsignedPriority;
	type WeightInfo = pallet_im_online::weights::SubstrateWeight<Runtime>;
	type MaxKeys = MaxKeys;
	type MaxPeerInHeartbeats = MaxPeerInHeartbeats;
}

/// Calls that cannot be paused by the tx-pause pallet.
pub struct TxPauseWhitelistedCalls;
/// Whitelist `Balances::transfer_keep_alive`, all others are pauseable.
impl Contains<RuntimeCallNameOf<Runtime>> for TxPauseWhitelistedCalls {
	fn contains(full_name: &RuntimeCallNameOf<Runtime>) -> bool {
		matches!(
			(full_name.0.as_slice(), full_name.1.as_slice()),
			(b"Balances", b"transfer_keep_alive")
		)
	}
}

impl pallet_tx_pause::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type RuntimeCall = RuntimeCall;
	type PauseOrigin = EnsureRoot<AccountId>;
	type UnpauseOrigin = EnsureRoot<AccountId>;
	type WhitelistedCalls = TxPauseWhitelistedCalls;
	type MaxNameLen = ConstU32<256>;
	type WeightInfo = pallet_tx_pause::weights::SubstrateWeight<Runtime>;
}

parameter_types! {
	pub const BasicDeposit: Balance = deposit(0, 100);
	pub const ByteDeposit: Balance = deposit(0, 100);
	pub const SubAccountDeposit: Balance = deposit(1, 1);
	pub const MaxSubAccounts: u32 = 100;
	#[derive(Serialize, Deserialize)]
	pub const MaxAdditionalFields: u32 = 100;
	pub const MaxRegistrars: u32 = 20;
	pub const PendingUsernameExpiration: u64 = 7 * DAYS;
}

impl pallet_identity::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type Currency = Balances;
	/// The amount held on deposit for a registered subaccount.
	type SubAccountDeposit = SubAccountDeposit;
	/// The maximum number of sub-accounts allowed per identified account.
	type MaxSubAccounts = MaxSubAccounts;
	/// Maximum number of registrars allowed in the system.
	type MaxRegistrars = MaxRegistrars;
	type Slashed = Treasury;
	/// The origin which may forcibly set or remove a name. Root can always do this.
	type ForceOrigin = EnsureRoot<Self::AccountId>;
	/// The origin which may add or remove registrars. Root can always do this.
	type RegistrarOrigin = EnsureRoot<Self::AccountId>;
	/// The amount held on deposit for a registered identity.
	type BasicDeposit = BasicDeposit;
	/// The amount held on deposit per additional bytes in additional fields for a registered
	/// identity
	type ByteDeposit = ByteDeposit;
	type IdentityInformation = pallet_identity::legacy::IdentityInfo<MaxAdditionalFields>;
	type OffchainSignature = Signature;
	type SigningPublicKey = <Signature as traits::Verify>::Signer;
	type UsernameAuthorityOrigin = EnsureRoot<AccountId>;
	type PendingUsernameExpiration = PendingUsernameExpiration;
	type MaxSuffixLength = ConstU32<7>;
	type MaxUsernameLength = ConstU32<32>;
	type WeightInfo = ();
}

impl pallet_utility::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type RuntimeCall = RuntimeCall;
	type PalletsOrigin = OriginCaller;
	type WeightInfo = ();
}

parameter_types! {
	// One storage item; key size is 32; value is size 4+4+16+32 bytes = 56 bytes.
	pub const DepositBase: Balance = deposit(1, 88); // roughly 1 TNT
	// Additional storage item size of 32 bytes.
	pub const DepositFactor: Balance = deposit(0, 32);
}

impl pallet_multisig::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type RuntimeCall = RuntimeCall;
	type Currency = Balances;
	type DepositBase = DepositBase;
	type DepositFactor = DepositFactor;
	type MaxSignatories = ConstU32<100>;
	type WeightInfo = pallet_multisig::weights::SubstrateWeight<Runtime>;
}


parameter_types! {
	// One storage item; key size 32, value size 8; .
	pub const ProxyDepositBase: Balance = deposit(1, 8);
	// Additional storage item size of 33 bytes.
	pub const ProxyDepositFactor: Balance = deposit(0, 33);
	pub const AnnouncementDepositBase: Balance = deposit(1, 8);
	pub const AnnouncementDepositFactor: Balance = deposit(0, 66);
}

/// The type used to represent the kinds of proxying allowed.
#[derive(
	Copy,
	Clone,
	Eq,
	PartialEq,
	Ord,
	PartialOrd,
	Encode,
	Decode,
	RuntimeDebug,
	MaxEncodedLen,
	scale_info::TypeInfo,
)]
pub enum ProxyType {
	Any,
	NonTransfer,
	Governance,
	Staking,
}
impl Default for ProxyType {
	fn default() -> Self {
		Self::Any
	}
}
impl InstanceFilter<RuntimeCall> for ProxyType {
	fn filter(&self, c: &RuntimeCall) -> bool {
		match self {
			ProxyType::Any => true,
			ProxyType::NonTransfer => !matches!(
				c,
				RuntimeCall::Balances(..)
					| RuntimeCall::Vesting(pallet_vesting::Call::vested_transfer { .. })
			),
			ProxyType::Governance => matches!(
				c,
				RuntimeCall::Democracy(..)
					| RuntimeCall::Council(..)
					| RuntimeCall::Elections(..)
					| RuntimeCall::Treasury(..)
			),
			ProxyType::Staking => {
				matches!(c, RuntimeCall::Staking(..))
			},
		}
	}
	fn is_superset(&self, o: &Self) -> bool {
		match (self, o) {
			(x, y) if x == y => true,
			(ProxyType::Any, _) => true,
			(_, ProxyType::Any) => false,
			(ProxyType::NonTransfer, _) => true,
			_ => false,
		}
	}
}

impl pallet_proxy::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type RuntimeCall = RuntimeCall;
	type Currency = Balances;
	type ProxyType = ProxyType;
	type ProxyDepositBase = ProxyDepositBase;
	type ProxyDepositFactor = ProxyDepositFactor;
	type MaxProxies = ConstU32<32>;
	type WeightInfo = pallet_proxy::weights::SubstrateWeight<Runtime>;
	type MaxPending = ConstU32<32>;
	type CallHasher = BlakeTwo256;
	type AnnouncementDepositBase = AnnouncementDepositBase;
	type AnnouncementDepositFactor = AnnouncementDepositFactor;
}

parameter_types! {
	pub const AssetDeposit: Balance = 10 * UNIT;
	pub const AssetAccountDeposit: Balance = DOLLAR;
	pub const ApprovalDeposit: Balance = ExistentialDeposit::get() as u128;
	pub const AssetsStringLimit: u32 = 50;
	pub const MetadataDepositBase: Balance = deposit(1, 68);
	pub const MetadataDepositPerByte: Balance = deposit(0, 1);
}

#[cfg(not(feature = "runtime-benchmarks"))]
pub type AssetId = u128;

#[cfg(feature = "runtime-benchmarks")]
pub type AssetId = u32;

impl pallet_assets::Config for Runtime {
	type RuntimeEvent = RuntimeEvent;
	type Balance = Balance;
	type AssetId = AssetId;
	type AssetIdParameter = parity_scale_codec::Compact<AssetId>;
	type Currency = Balances;
	type CreateOrigin = AsEnsureOriginWithArg<EnsureSigned<AccountId>>;
	type ForceOrigin = frame_system::EnsureRoot<Self::AccountId>;
	type AssetDeposit = AssetDeposit;
	type AssetAccountDeposit = AssetAccountDeposit;
	type MetadataDepositBase = MetadataDepositBase;
	type MetadataDepositPerByte = MetadataDepositPerByte;
	type ApprovalDeposit = ApprovalDeposit;
	type StringLimit = AssetsStringLimit;
	type RemoveItemsLimit = ConstU32<1000>;
	type Freezer = ();
	type Extra = ();
	type CallbackHandle = ();
	type WeightInfo = pallet_assets::weights::SubstrateWeight<Runtime>;
	#[cfg(feature = "runtime-benchmarks")]
	type BenchmarkHelper = ();
}



// Create the runtime by composing the FRAME pallets that were previously configured.
construct_runtime!(
	pub enum Runtime {
		System: frame_system = 1,
		Timestamp: pallet_timestamp = 2,

		Sudo: pallet_sudo = 3,
		RandomnessCollectiveFlip: pallet_randomness_collective_flip = 4,

		Balances: pallet_balances = 5,
		TransactionPayment: pallet_transaction_payment = 6,

		Authorship: pallet_authorship = 7,
		Babe: pallet_babe = 8,
		Grandpa: pallet_grandpa = 9,

		Indices: pallet_indices = 10,
		Democracy: pallet_democracy = 11,
		Council: pallet_collective::<Instance1> = 12,
		Vesting: pallet_vesting = 13,

		Elections: pallet_elections_phragmen = 14,
		ElectionProviderMultiPhase: pallet_election_provider_multi_phase = 15,
		Staking: pallet_staking = 16,
		Session: pallet_session = 17,
		Historical: pallet_session_historical = 18,
		Treasury: pallet_treasury = 19,
		Bounties: pallet_bounties = 20,
		ChildBounties: pallet_child_bounties = 21,
		BagsList: pallet_bags_list = 22,
		NominationPools: pallet_nomination_pools = 23,

		Scheduler: pallet_scheduler = 24,
		Preimage: pallet_preimage = 25,
		Offences: pallet_offences = 26,
		Proxy: pallet_proxy = 27,

		TxPause: pallet_tx_pause = 28,
		ImOnline: pallet_im_online = 29,
		Identity: pallet_identity = 30,
		Utility: pallet_utility = 31,
		Multisig: pallet_multisig = 32,

		Ethereum: pallet_ethereum = 33,
		EVM: pallet_evm = 34,
		EVMChainId: pallet_evm_chain_id = 35,
		DynamicFee: pallet_dynamic_fee = 36,
		BaseFee: pallet_base_fee = 37,
		HotfixSufficients: pallet_hotfix_sufficients = 38,

		Assets: pallet_assets = 44,
		IpfsPin : pallet_ipfs_pin=52,
		Registration : pallet_registration=53,
		ExecutionUnit : pallet_execution_unit=54,
		Metagraph : pallet_metagraph=55,
		Marketplace: pallet_marketplace = 56,
		Bittensor: pallet_bittensor=57,
		SubAccount: pallet_subaccount= 58,
		Notifications: pallet_notifications = 59,
		AccountProfile: pallet_account_profile = 60,
        RankingStorage: pallet_rankings =63,
        RankingCompute: pallet_rankings::<Instance2> = 91,
		RankingValidators: pallet_rankings::<Instance3> = 92,
		RankingGpu: pallet_rankings::<Instance4> = 93,
		RankingS3: pallet_rankings::<Instance5> = 97,
		Utils: pallet_utils = 64,
		Backup: pallet_backup = 65,
		Credits: pallet_credits = 66,
		Compute: pallet_compute = 89,
		ContainerRegistry: pallet_container_registry = 90,
		Storage: pallet_storage = 94,
		AlphaBridge: pallet_alpha_bridge = 76
	}
);

#[derive(Clone, Serialize, Deserialize)]
pub struct TransactionConverter;

impl fp_rpc::ConvertTransaction<UncheckedExtrinsic> for TransactionConverter {
	fn convert_transaction(&self, transaction: pallet_ethereum::Transaction) -> UncheckedExtrinsic {
		UncheckedExtrinsic::new_unsigned(
			pallet_ethereum::Call::<Runtime>::transact { transaction }.into(),
		)
	}
}

impl fp_rpc::ConvertTransaction<opaque::UncheckedExtrinsic> for TransactionConverter {
	fn convert_transaction(
		&self,
		transaction: pallet_ethereum::Transaction,
	) -> opaque::UncheckedExtrinsic {
		let extrinsic = UncheckedExtrinsic::new_unsigned(
			pallet_ethereum::Call::<Runtime>::transact { transaction }.into(),
		);
		let encoded = extrinsic.encode();
		opaque::UncheckedExtrinsic::decode(&mut &encoded[..])
			.expect("Encoded extrinsic is always valid")
	}
}

/// Block type as expected by this runtime.
pub type Block = generic::Block<Header, UncheckedExtrinsic>;
/// A Block signed with a Justification
pub type SignedBlock = generic::SignedBlock<Block>;
/// BlockId type as expected by this runtime.
pub type BlockId = generic::BlockId<Block>;
/// The SignedExtension to the basic transaction logic.
pub type SignedExtra = (
	frame_system::CheckNonZeroSender<Runtime>,
	frame_system::CheckSpecVersion<Runtime>,
	frame_system::CheckTxVersion<Runtime>,
	frame_system::CheckGenesis<Runtime>,
	frame_system::CheckEra<Runtime>,
	frame_system::CheckNonce<Runtime>,
	frame_system::CheckWeight<Runtime>,
	pallet_transaction_payment::ChargeTransactionPayment<Runtime>,
	frame_metadata_hash_extension::CheckMetadataHash<Runtime>,
);
/// Unchecked extrinsic type as expected by this runtime.
pub type UncheckedExtrinsic =
	fp_self_contained::UncheckedExtrinsic<Address, RuntimeCall, Signature, SignedExtra>;
/// Extrinsic type that has already been checked.
pub type CheckedExtrinsic =
	fp_self_contained::CheckedExtrinsic<AccountId, RuntimeCall, SignedExtra, H160>;
/// The payload being signed in transactions.
pub type SignedPayload = generic::SignedPayload<RuntimeCall, SignedExtra>;
/// Executive: handles dispatch to the various modules.
pub type Executive = frame_executive::Executive<
	Runtime,
	Block,
	frame_system::ChainContext<Runtime>,
	Runtime,
	AllPalletsWithSystem,
	migrations::MigrateSessionKeys<Runtime>,
>;

impl fp_self_contained::SelfContainedCall for RuntimeCall {
	type SignedInfo = H160;

	fn is_self_contained(&self) -> bool {
		match self {
			RuntimeCall::Ethereum(call) => call.is_self_contained(),
			_ => false,
		}
	}

	fn check_self_contained(&self) -> Option<Result<Self::SignedInfo, TransactionValidityError>> {
		match self {
			RuntimeCall::Ethereum(call) => call.check_self_contained(),
			_ => None,
		}
	}

	fn validate_self_contained(
		&self,
		info: &Self::SignedInfo,
		dispatch_info: &DispatchInfoOf<RuntimeCall>,
		len: usize,
	) -> Option<TransactionValidity> {
		match self {
			RuntimeCall::Ethereum(call) => call.validate_self_contained(info, dispatch_info, len),
			_ => None,
		}
	}

	fn pre_dispatch_self_contained(
		&self,
		info: &Self::SignedInfo,
		dispatch_info: &DispatchInfoOf<RuntimeCall>,
		len: usize,
	) -> Option<Result<(), TransactionValidityError>> {
		match self {
			RuntimeCall::Ethereum(call) => {
				call.pre_dispatch_self_contained(info, dispatch_info, len)
			},
			_ => None,
		}
	}

	fn apply_self_contained(
		self,
		info: Self::SignedInfo,
	) -> Option<sp_runtime::DispatchResultWithInfo<PostDispatchInfoOf<Self>>> {
		match self {
			call @ RuntimeCall::Ethereum(pallet_ethereum::Call::transact { .. }) => {
				Some(call.dispatch(RuntimeOrigin::from(
					pallet_ethereum::RawOrigin::EthereumTransaction(info),
				)))
			},
			_ => None,
		}
	}
}

#[cfg(feature = "runtime-benchmarks")]
#[macro_use]
extern crate frame_benchmarking;

#[cfg(feature = "runtime-benchmarks")]
mod benches {
	define_benchmarks!(
		[frame_benchmarking, BaselineBench::<Runtime>]
		[frame_system, SystemBench::<Runtime>]
		[pallet_balances, Balances]
		[pallet_timestamp, Timestamp]
	);
}

impl_runtime_apis! {
	impl sp_api::Core<Block> for Runtime {
		fn version() -> RuntimeVersion {
			VERSION
		}

		fn execute_block(block: Block) {
			Executive::execute_block(block)
		}

		fn initialize_block(header: &<Block as BlockT>::Header) -> sp_runtime::ExtrinsicInclusionMode {
			Executive::initialize_block(header)
		}
	}

	impl sp_api::Metadata<Block> for Runtime {
		fn metadata() -> OpaqueMetadata {
			OpaqueMetadata::new(Runtime::metadata().into())
		}
		fn metadata_at_version(version: u32) -> Option<OpaqueMetadata> {
			Runtime::metadata_at_version(version)
		}
		fn metadata_versions() -> sp_std::vec::Vec<u32> {
			Runtime::metadata_versions()
		}
	}

	impl sp_block_builder::BlockBuilder<Block> for Runtime {
		fn apply_extrinsic(
			extrinsic: <Block as BlockT>::Extrinsic,
		) -> ApplyExtrinsicResult {
			Executive::apply_extrinsic(extrinsic)
		}

		fn finalize_block() -> <Block as BlockT>::Header {
			Executive::finalize_block()
		}

		fn inherent_extrinsics(data: sp_inherents::InherentData) -> Vec<<Block as BlockT>::Extrinsic> {
			data.create_extrinsics()
		}

		fn check_inherents(
			block: Block,
			data: sp_inherents::InherentData,
		) -> sp_inherents::CheckInherentsResult {
			data.check_extrinsics(&block)
		}
	}

	impl fp_rpc::EthereumRuntimeRPCApi<Block> for Runtime {
		fn chain_id() -> u64 {
			<Runtime as pallet_evm::Config>::ChainId::get()
		}

		fn account_basic(address: H160) -> EVMAccount {
			let (account, _) = pallet_evm::Pallet::<Runtime>::account_basic(&address);
			account
		}

		fn gas_price() -> U256 {
			let (gas_price, _) = <Runtime as pallet_evm::Config>::FeeCalculator::min_gas_price();
			gas_price
		}

		fn account_code_at(address: H160) -> Vec<u8> {
			pallet_evm::AccountCodes::<Runtime>::get(address)
		}

		fn author() -> H160 {
			<pallet_evm::Pallet<Runtime>>::find_author()
		}

		fn storage_at(address: H160, index: U256) -> H256 {
			let mut tmp = [0u8; 32];
			index.to_big_endian(&mut tmp);
			pallet_evm::AccountStorages::<Runtime>::get(address, H256::from_slice(&tmp[..]))
		}

		fn call(
			from: H160,
			to: H160,
			data: Vec<u8>,
			value: U256,
			gas_limit: U256,
			max_fee_per_gas: Option<U256>,
			max_priority_fee_per_gas: Option<U256>,
			nonce: Option<U256>,
			estimate: bool,
			access_list: Option<Vec<(H160, Vec<H256>)>>,
		) -> Result<pallet_evm::CallInfo, sp_runtime::DispatchError> {
			use pallet_evm::GasWeightMapping;
			let config = if estimate {
				let mut config = <Runtime as pallet_evm::Config>::config().clone();
				config.estimate = true;
				Some(config)
			} else {
				None
			};

			let is_transactional = false;
			let validate = true;
			let mut estimated_transaction_len = data.len() +
				// to: 20
				// from: 20
				// value: 32
				// gas_limit: 32
				// nonce: 32
				// 1 byte transaction action variant
				// chain id 8 bytes
				// 65 bytes signature
				210;
			if max_fee_per_gas.is_some() {
				estimated_transaction_len += 32;
			}
			if max_priority_fee_per_gas.is_some() {
				estimated_transaction_len += 32;
			}
			if access_list.is_some() {
				estimated_transaction_len += access_list.encoded_size();
			}

			let gas_limit = gas_limit.min(u64::MAX.into()).low_u64();
			let without_base_extrinsic_weight = true;
			let (weight_limit, proof_size_base_cost) =
				match <Runtime as pallet_evm::Config>::GasWeightMapping::gas_to_weight(
					gas_limit,
					without_base_extrinsic_weight
				) {
					weight_limit if weight_limit.proof_size() > 0 => {
						(Some(weight_limit), Some(estimated_transaction_len as u64))
					}
					_ => (None, None),
				};
			let evm_config = config.as_ref().unwrap_or(<Runtime as pallet_evm::Config>::config());
			<Runtime as pallet_evm::Config>::Runner::call(
				from,
				to,
				data,
				value,
				gas_limit.unique_saturated_into(),
				max_fee_per_gas,
				max_priority_fee_per_gas,
				nonce,
				access_list.unwrap_or_default(),
				is_transactional,
				validate,
				weight_limit,
				proof_size_base_cost,
				evm_config,
			).map_err(|err| err.error.into())
		}

		fn create(
			from: H160,
			data: Vec<u8>,
			value: U256,
			gas_limit: U256,
			max_fee_per_gas: Option<U256>,
			max_priority_fee_per_gas: Option<U256>,
			nonce: Option<U256>,
			estimate: bool,
			access_list: Option<Vec<(H160, Vec<H256>)>>,
		) -> Result<pallet_evm::CreateInfo, sp_runtime::DispatchError> {
			use pallet_evm::GasWeightMapping;
			let config = if estimate {
				let mut config = <Runtime as pallet_evm::Config>::config().clone();
				config.estimate = true;
				Some(config)
			} else {
				None
			};

			let is_transactional = false;
			let validate = true;
			let mut estimated_transaction_len = data.len() +
				// from: 20
				// value: 32
				// gas_limit: 32
				// nonce: 32
				// 1 byte transaction action variant
				// chain id 8 bytes
				// 65 bytes signature
				190;
			if max_fee_per_gas.is_some() {
				estimated_transaction_len += 32;
			}
			if max_priority_fee_per_gas.is_some() {
				estimated_transaction_len += 32;
			}
			if access_list.is_some() {
				estimated_transaction_len += access_list.encoded_size();
			}

			let gas_limit = gas_limit.min(u64::MAX.into()).low_u64();
			let without_base_extrinsic_weight = true;
			let (weight_limit, proof_size_base_cost) =
				match <Runtime as pallet_evm::Config>::GasWeightMapping::gas_to_weight(
					gas_limit,
					without_base_extrinsic_weight
				) {
					weight_limit if weight_limit.proof_size() > 0 => {
						(Some(weight_limit), Some(estimated_transaction_len as u64))
					}
					_ => (None, None),
				};
			let evm_config = config.as_ref().unwrap_or(<Runtime as pallet_evm::Config>::config());
		
			let whitelist =  pallet_evm::WhitelistedCreators::<Runtime>::get();
		
			<Runtime as pallet_evm::Config>::Runner::create(
				from,
				data,
				value,
				gas_limit.unique_saturated_into(),
				max_fee_per_gas,
				max_priority_fee_per_gas,
				nonce,
				access_list.unwrap_or_default(),
				whitelist,
				is_transactional,
				validate,
				weight_limit,
				proof_size_base_cost,
				evm_config,
			).map_err(|err| err.error.into())
		}

		fn current_transaction_statuses() -> Option<Vec<TransactionStatus>> {
			pallet_ethereum::CurrentTransactionStatuses::<Runtime>::get()
		}

		fn current_block() -> Option<pallet_ethereum::Block> {
			pallet_ethereum::CurrentBlock::<Runtime>::get()
		}

		fn current_receipts() -> Option<Vec<pallet_ethereum::Receipt>> {
			pallet_ethereum::CurrentReceipts::<Runtime>::get()
		}

		fn current_all() -> (
			Option<pallet_ethereum::Block>,
			Option<Vec<pallet_ethereum::Receipt>>,
			Option<Vec<TransactionStatus>>
		) {
			(
				pallet_ethereum::CurrentBlock::<Runtime>::get(),
				pallet_ethereum::CurrentReceipts::<Runtime>::get(),
				pallet_ethereum::CurrentTransactionStatuses::<Runtime>::get()
			)
		}

		fn extrinsic_filter(
			xts: Vec<<Block as BlockT>::Extrinsic>,
		) -> Vec<EthereumTransaction> {
			xts.into_iter().filter_map(|xt| match xt.0.function {
				RuntimeCall::Ethereum(transact { transaction }) => Some(transaction),
				_ => None
			}).collect::<Vec<EthereumTransaction>>()
		}

		fn elasticity() -> Option<Permill> {
			Some(pallet_base_fee::Elasticity::<Runtime>::get())
		}

		fn gas_limit_multiplier_support() {}

		fn pending_block(
			xts: Vec<<Block as BlockT>::Extrinsic>,
		) -> (Option<pallet_ethereum::Block>, Option<Vec<TransactionStatus>>) {
			for ext in xts.into_iter() {
				let _ = Executive::apply_extrinsic(ext);
			}

			Ethereum::on_finalize(System::block_number() + 1);

			(
				pallet_ethereum::CurrentBlock::<Runtime>::get(),
				pallet_ethereum::CurrentTransactionStatuses::<Runtime>::get()
			)
		}

		fn initialize_pending_block(header: &<Block as BlockT>::Header) {
			Executive::initialize_block(header);
		}
	}

	impl fp_rpc::ConvertTransactionRuntimeApi<Block> for Runtime {
		fn convert_transaction(transaction: EthereumTransaction) -> <Block as BlockT>::Extrinsic {
			UncheckedExtrinsic::new_unsigned(
				pallet_ethereum::Call::<Runtime>::transact { transaction }.into(),
			)
		}
	}

	impl sp_transaction_pool::runtime_api::TaggedTransactionQueue<Block> for Runtime {
		fn validate_transaction(
			source: TransactionSource,
			tx: <Block as BlockT>::Extrinsic,
			block_hash: <Block as BlockT>::Hash,
		) -> TransactionValidity {
			Executive::validate_transaction(source, tx, block_hash)
		}
	}

	impl sp_offchain::OffchainWorkerApi<Block> for Runtime {
		fn offchain_worker(header: &<Block as BlockT>::Header) {
			Executive::offchain_worker(header)
		}
	}

	impl sp_session::SessionKeys<Block> for Runtime {
		fn decode_session_keys(
			encoded: Vec<u8>,
		) -> Option<Vec<(Vec<u8>, KeyTypeId)>> {
			opaque::SessionKeys::decode_into_raw_public_keys(&encoded)
		}

		fn generate_session_keys(seed: Option<Vec<u8>>) -> Vec<u8> {
			opaque::SessionKeys::generate(seed)
		}
	}

	impl sp_consensus_babe::BabeApi<Block> for Runtime {
		fn configuration() -> sp_consensus_babe::BabeConfiguration {
			let epoch_config = Babe::epoch_config().unwrap_or(BABE_GENESIS_EPOCH_CONFIG);
			sp_consensus_babe::BabeConfiguration {
				slot_duration: Babe::slot_duration(),
				epoch_length: EpochDuration::get(),
				c: epoch_config.c,
				authorities: Babe::authorities().to_vec(),
				randomness: Babe::randomness(),
				allowed_slots: epoch_config.allowed_slots,
			}
		}

		fn current_epoch_start() -> sp_consensus_babe::Slot {
			Babe::current_epoch_start()
		}

		fn current_epoch() -> sp_consensus_babe::Epoch {
			Babe::current_epoch()
		}

		fn next_epoch() -> sp_consensus_babe::Epoch {
			Babe::next_epoch()
		}

		fn generate_key_ownership_proof(
			_slot: sp_consensus_babe::Slot,
			authority_id: sp_consensus_babe::AuthorityId,
		) -> Option<sp_consensus_babe::OpaqueKeyOwnershipProof> {
			use parity_scale_codec::Encode;

			Historical::prove((sp_consensus_babe::KEY_TYPE, authority_id))
				.map(|p| p.encode())
				.map(sp_consensus_babe::OpaqueKeyOwnershipProof::new)
		}

		fn submit_report_equivocation_unsigned_extrinsic(
			equivocation_proof: sp_consensus_babe::EquivocationProof<<Block as BlockT>::Header>,
			key_owner_proof: sp_consensus_babe::OpaqueKeyOwnershipProof,
		) -> Option<()> {
			let key_owner_proof = key_owner_proof.decode()?;

			Babe::submit_unsigned_equivocation_report(
				equivocation_proof,
				key_owner_proof,
			)
		}
	}

	impl frame_system_rpc_runtime_api::AccountNonceApi<Block, AccountId, Nonce> for Runtime {
		fn account_nonce(account: AccountId) -> Nonce {
			System::account_nonce(account)
		}
	}

	impl pallet_transaction_payment_rpc_runtime_api::TransactionPaymentApi<
		Block,
		Balance,
	> for Runtime {
		fn query_info(uxt: <Block as BlockT>::Extrinsic, len: u32) -> RuntimeDispatchInfo<Balance> {
			TransactionPayment::query_info(uxt, len)
		}
		fn query_fee_details(uxt: <Block as BlockT>::Extrinsic, len: u32) -> FeeDetails<Balance> {
			TransactionPayment::query_fee_details(uxt, len)
		}
		fn query_weight_to_fee(weight: Weight) -> Balance {
			TransactionPayment::weight_to_fee(weight)
		}
		fn query_length_to_fee(length: u32) -> Balance {
			TransactionPayment::length_to_fee(length)
		}
	}

	impl fg_primitives::GrandpaApi<Block> for Runtime {
		fn grandpa_authorities() -> GrandpaAuthorityList {
			Grandpa::grandpa_authorities()
		}

		fn current_set_id() -> fg_primitives::SetId {
			Grandpa::current_set_id()
		}

		fn submit_report_equivocation_unsigned_extrinsic(
			equivocation_proof: fg_primitives::EquivocationProof<
				<Block as BlockT>::Hash,
				NumberFor<Block>,
			>,
			key_owner_proof: fg_primitives::OpaqueKeyOwnershipProof,
		) -> Option<()> {
			let key_owner_proof = key_owner_proof.decode()?;

			Grandpa::submit_unsigned_equivocation_report(
				equivocation_proof,
				key_owner_proof,
			)
		}

		fn generate_key_ownership_proof(
			_set_id: fg_primitives::SetId,
			authority_id: GrandpaId,
		) -> Option<fg_primitives::OpaqueKeyOwnershipProof> {
			use parity_scale_codec::Encode;

			Historical::prove((fg_primitives::KEY_TYPE, authority_id))
				.map(|p| p.encode())
				.map(fg_primitives::OpaqueKeyOwnershipProof::new)
		}
	}

	impl rpc_primitives_node_metrics::NodeMetricsRuntimeApi<Block> for Runtime {
		fn get_active_nodes_metrics_by_type(node_type: rpc_primitives_node_metrics::NodeType) -> Vec<Option<rpc_primitives_node_metrics::NodeMetricsData>> {
			// Convert RPC NodeType to Pallet NodeType
			let pallet_node_type = match node_type {
				rpc_primitives_node_metrics::NodeType::Validator => pallet_registration::NodeType::Validator,
				rpc_primitives_node_metrics::NodeType::StorageMiner => pallet_registration::NodeType::StorageMiner,
				rpc_primitives_node_metrics::NodeType::StorageS3 => pallet_registration::NodeType::StorageS3,   
				rpc_primitives_node_metrics::NodeType::ComputeMiner => pallet_registration::NodeType::ComputeMiner,
				rpc_primitives_node_metrics::NodeType::GpuMiner => pallet_registration::NodeType::GpuMiner,   
			};

			let node_metrics = <pallet_execution_unit::Pallet<Runtime>>::get_active_nodes_metrics_by_type(pallet_node_type);

			// Convert from execution unit NodeMetricsData to RPC primitives NodeMetricsData
			node_metrics.into_iter().map(|metrics_opt| {
				metrics_opt.map(|metrics| {
					rpc_primitives_node_metrics::NodeMetricsData {
						miner_id: String::from_utf8_lossy(&metrics.miner_id).into_owned(),
						bandwidth_bytes: metrics.bandwidth_mbps,
						current_storage_bytes: metrics.current_storage_bytes,
						total_storage_bytes: metrics.total_storage_bytes,
						geolocation: String::from_utf8_lossy(&metrics.geolocation).into_owned(),
						successful_pin_checks: metrics.successful_pin_checks,
						total_pin_checks: metrics.total_pin_checks,
						storage_proof_time_ms: metrics.storage_proof_time_ms,
						storage_growth_rate: metrics.storage_growth_rate,
						latency_ms: metrics.latency_ms,
						total_latency_ms: metrics.total_latency_ms,
						total_times_latency_checked: metrics.total_times_latency_checked,
						avg_response_time_ms: metrics.avg_response_time_ms,
						peer_count: metrics.peer_count,
						failed_challenges_count: metrics.failed_challenges_count,
						successful_challenges: metrics.successful_challenges,
						total_challenges: metrics.total_challenges,
						uptime_minutes: metrics.uptime_minutes,
						total_minutes: metrics.total_minutes,
						consecutive_reliable_days: metrics.consecutive_reliable_days,
						recent_downtime_hours: metrics.recent_downtime_hours,
						is_sev_enabled: metrics.is_sev_enabled,
						zfs_info: metrics.zfs_info.into_iter()
							.map(|info| String::from_utf8_lossy(&info).into_owned())
							.collect(),
						raid_info: metrics.raid_info.into_iter()
							.map(|info| String::from_utf8_lossy(&info).into_owned())
							.collect(),
						vm_count: metrics.vm_count,
						primary_network_interface: metrics.primary_network_interface.map(|nif| {
							rpc_primitives_node_metrics::NetworkInterfaceInfo {
								name: String::from_utf8_lossy(&nif.name).into_owned(),
								mac_address: nif.mac_address.map(|mac| String::from_utf8_lossy(&mac).into_owned()),
								uplink_mb: nif.uplink_mb,
								downlink_mb: nif.downlink_mb,
								network_details: nif.network_details.map(|nd| {
									rpc_primitives_node_metrics::NetworkDetails {
										network_type: match nd.network_type {
											pallet_execution_unit::types::NetworkType::Private => rpc_primitives_node_metrics::NetworkType::Private,
											pallet_execution_unit::types::NetworkType::Public => rpc_primitives_node_metrics::NetworkType::Public,
										},
										city: nd.city.map(|c| String::from_utf8_lossy(&c).into_owned()),
										region: nd.region.map(|r| String::from_utf8_lossy(&r).into_owned()),
										country: nd.country.map(|c| String::from_utf8_lossy(&c).into_owned()),
										loc: nd.loc.map(|l| String::from_utf8_lossy(&l).into_owned()),
									}
								}),
							}
						}),
						disks: metrics.disks.into_iter().map(|disk| {
							rpc_primitives_node_metrics::DiskInfo {
								name: String::from_utf8_lossy(&disk.name).into_owned(),
								disk_type: String::from_utf8_lossy(&disk.disk_type).into_owned(),
								total_space_mb: disk.total_space_mb,
								free_space_mb: disk.free_space_mb,
							}
						}).collect(),
						ipfs_repo_size: metrics.ipfs_repo_size,
						ipfs_storage_max: metrics.ipfs_storage_max,
						cpu_model: String::from_utf8_lossy(&metrics.cpu_model).into_owned(),
						cpu_cores: metrics.cpu_cores,
						memory_bytes: metrics.memory_mb,
						free_memory_bytes: metrics.free_memory_mb,
						gpu_name: metrics.gpu_name.map(|gpu| String::from_utf8_lossy(&gpu).into_owned()),
						gpu_memory_bytes: metrics.gpu_memory_mb,
						hypervisor_disk_type: metrics.hypervisor_disk_type.map(|hdt| String::from_utf8_lossy(&hdt).into_owned()),
						vm_pool_disk_type: metrics.vm_pool_disk_type.map(|vpdt| String::from_utf8_lossy(&vpdt).into_owned()),
					}
				})
			}).collect()
		}

		fn get_total_node_rewards(account: AccountId32) -> u128 {
			<pallet_rankings::Pallet<Runtime>>::get_total_node_rewards(account)
		}

		fn get_total_distributed_rewards_by_node_type(node_type: rpc_primitives_node_metrics::NodeType) -> u128 {
			// Convert RPC NodeType to Pallet NodeType
			let pallet_node_type = match node_type {
				rpc_primitives_node_metrics::NodeType::Validator => pallet_registration::NodeType::Validator,
				rpc_primitives_node_metrics::NodeType::StorageMiner => pallet_registration::NodeType::StorageMiner,
				rpc_primitives_node_metrics::NodeType::StorageS3 => pallet_registration::NodeType::StorageS3,   
				rpc_primitives_node_metrics::NodeType::ComputeMiner => pallet_registration::NodeType::ComputeMiner,
				rpc_primitives_node_metrics::NodeType::GpuMiner => pallet_registration::NodeType::GpuMiner,   
			};
			<pallet_rankings::Pallet<Runtime>>::get_total_distributed_rewards_by_node_type(pallet_node_type)
		}

		fn get_miners_total_rewards(node_type: rpc_primitives_node_metrics::NodeType) -> Vec<rpc_primitives_node_metrics::MinerRewardSummary>{

			// Convert RPC NodeType to Pallet NodeType
			let pallet_node_type = match node_type {
				rpc_primitives_node_metrics::NodeType::Validator => pallet_registration::NodeType::Validator,
				rpc_primitives_node_metrics::NodeType::StorageMiner => pallet_registration::NodeType::StorageMiner,
				rpc_primitives_node_metrics::NodeType::StorageS3 => pallet_registration::NodeType::StorageS3,   
				rpc_primitives_node_metrics::NodeType::ComputeMiner => pallet_registration::NodeType::ComputeMiner,
				rpc_primitives_node_metrics::NodeType::GpuMiner => pallet_registration::NodeType::GpuMiner,   
			};

			// <pallet_rankings::Pallet<Runtime>>::get_miners_total_rewards(pallet_node_type)

			// Convert pallet MinerRewardSummary to RPC MinerRewardSummary
			<pallet_rankings::Pallet<Runtime>>::get_miners_total_rewards(pallet_node_type)
			.into_iter()
			.map(|summary| rpc_primitives_node_metrics::MinerRewardSummary {
				account: summary.account.clone(),
				reward: summary.reward,
			})
			.collect()
		}

		fn get_account_pending_rewards( account: AccountId32) -> Vec<rpc_primitives_node_metrics::MinerRewardSummary>{
			<pallet_rankings::Pallet<Runtime>>::get_account_pending_rewards(account)
			.into_iter()
			.map(|summary| rpc_primitives_node_metrics::MinerRewardSummary {
				account: summary.account.clone(),
				reward: summary.reward,
			})
			.collect()
		}

		fn get_miners_pending_rewards(node_type: rpc_primitives_node_metrics::NodeType) -> Vec<rpc_primitives_node_metrics::MinerRewardSummary>{
			// Convert RPC NodeType to Pallet NodeType
			let pallet_node_type = match node_type {
				rpc_primitives_node_metrics::NodeType::Validator => pallet_registration::NodeType::Validator,
				rpc_primitives_node_metrics::NodeType::StorageMiner => pallet_registration::NodeType::StorageMiner,
				rpc_primitives_node_metrics::NodeType::StorageS3 => pallet_registration::NodeType::StorageS3,   
				rpc_primitives_node_metrics::NodeType::ComputeMiner => pallet_registration::NodeType::ComputeMiner,
				rpc_primitives_node_metrics::NodeType::GpuMiner => pallet_registration::NodeType::GpuMiner,   
			};
			
			<pallet_rankings::Pallet<Runtime>>::get_miners_pending_rewards(pallet_node_type)
			.into_iter()
			.map(|summary| rpc_primitives_node_metrics::MinerRewardSummary {
				account: summary.account.clone(),
				reward: summary.reward,
			})
			.collect()
		}

		fn calculate_total_file_size(account: AccountId32) -> u128 {
			<pallet_ipfs_pin::Pallet<Runtime>>::calculate_total_file_size(&account)
		}

		fn get_user_files(account: AccountId32) -> Vec<rpc_primitives_node_metrics::UserFile> {
			<pallet_ipfs_pin::Pallet<Runtime>>::get_user_files(&account)
			.into_iter()
			.map(|file| rpc_primitives_node_metrics::UserFile {
				hash: file.hash.clone(),
				size: file.size,
			})
			.collect()
		}
	}

	impl rpc_primitives_debug::DebugRuntimeApi<Block> for Runtime {
		fn trace_transaction(
			extrinsics: Vec<<Block as BlockT>::Extrinsic>,
			traced_transaction: &EthereumTransaction,
			header: &<Block as BlockT>::Header,
		) -> Result<
			(),
			sp_runtime::DispatchError,
		> {
			#[cfg(feature = "evm-tracing")]
			{
				use evm_tracer::tracer::EvmTracer;

				// Initialize block: calls the "on_initialize" hook on every pallet
				// in AllPalletsWithSystem.
				Executive::initialize_block(header);

				// Apply the a subset of extrinsics: all the substrate-specific or ethereum
				// transactions that preceded the requested transaction.
				for ext in extrinsics.into_iter() {
					let _ = match &ext.0.function {
						RuntimeCall::Ethereum(transact { transaction }) => {
							if transaction == traced_transaction {
								EvmTracer::new().trace(|| Executive::apply_extrinsic(ext));
								return Ok(());
							} else {
								Executive::apply_extrinsic(ext)
							}
						}
						_ => Executive::apply_extrinsic(ext),
					};
				}
				Ok(())
			}
			#[cfg(not(feature = "evm-tracing"))]
			Err(sp_runtime::DispatchError::Other(
				"Missing `evm-tracing` compile time feature flag.",
			))
		}

		fn trace_block(
			extrinsics: Vec<<Block as BlockT>::Extrinsic>,
			known_transactions: Vec<H256>,
			header: &<Block as BlockT>::Header,
		) -> Result<
			(),
			sp_runtime::DispatchError,
		> {
			#[cfg(feature = "evm-tracing")]
			{
				use evm_tracer::tracer::EvmTracer;

				let mut config = <Runtime as pallet_evm::Config>::config().clone();
				config.estimate = true;

				// Initialize block: calls the "on_initialize" hook on every pallet
				// in AllPalletsWithSystem.
				Executive::initialize_block(header);

				// Apply all extrinsics. Ethereum extrinsics are traced.
				for ext in extrinsics.into_iter() {
					match &ext.0.function {
						RuntimeCall::Ethereum(transact { transaction }) => {
							if known_transactions.contains(&transaction.hash()) {
								// Each known extrinsic is a new call stack.
								EvmTracer::emit_new();
								EvmTracer::new().trace(|| Executive::apply_extrinsic(ext));
							} else {
								let _ = Executive::apply_extrinsic(ext);
							}
						}
						_ => {
							let _ = Executive::apply_extrinsic(ext);
						}
					};
				}

				Ok(())
			}
			#[cfg(not(feature = "evm-tracing"))]
			Err(sp_runtime::DispatchError::Other(
				"Missing `evm-tracing` compile time feature flag.",
			))
		}

		fn trace_call(
			header: &<Block as BlockT>::Header,
			from: H160,
			to: H160,
			data: Vec<u8>,
			value: U256,
			gas_limit: U256,
			max_fee_per_gas: Option<U256>,
			max_priority_fee_per_gas: Option<U256>,
			nonce: Option<U256>,
			access_list: Option<Vec<(H160, Vec<H256>)>>,
		) -> Result<(), sp_runtime::DispatchError> {
			#[cfg(feature = "evm-tracing")]
			{
				use evm_tracer::tracer::EvmTracer;

				// Initialize block: calls the "on_initialize" hook on every pallet
				// in AllPalletsWithSystem.
				Executive::initialize_block(header);

				EvmTracer::new().trace(|| {
					let is_transactional = false;
					let validate = true;
					let without_base_extrinsic_weight = true;


					// Estimated encoded transaction size must be based on the heaviest transaction
					// type (EIP1559Transaction) to be compatible with all transaction types.
					let mut estimated_transaction_len = data.len() +
					// pallet ethereum index: 1
					// transact call index: 1
					// Transaction enum variant: 1
					// chain_id 8 bytes
					// nonce: 32
					// max_priority_fee_per_gas: 32
					// max_fee_per_gas: 32
					// gas_limit: 32
					// action: 21 (enum varianrt + call address)
					// value: 32
					// access_list: 1 (empty vec size)
					// 65 bytes signature
					258;

					if access_list.is_some() {
						estimated_transaction_len += access_list.encoded_size();
					}

					let gas_limit = gas_limit.min(u64::MAX.into()).low_u64();

					let (weight_limit, proof_size_base_cost) =
						match <Runtime as pallet_evm::Config>::GasWeightMapping::gas_to_weight(
							gas_limit,
							without_base_extrinsic_weight
						) {
							weight_limit if weight_limit.proof_size() > 0 => {
								(Some(weight_limit), Some(estimated_transaction_len as u64))
							}
							_ => (None, None),
						};

					let _ = <Runtime as pallet_evm::Config>::Runner::call(
						from,
						to,
						data,
						value,
						gas_limit,
						max_fee_per_gas,
						max_priority_fee_per_gas,
						nonce,
						access_list.unwrap_or_default(),
						is_transactional,
						validate,
						weight_limit,
						proof_size_base_cost,
						<Runtime as pallet_evm::Config>::config(),
					);
				});
				Ok(())
			}
			#[cfg(not(feature = "evm-tracing"))]
			Err(sp_runtime::DispatchError::Other(
				"Missing `evm-tracing` compile time feature flag.",
			))
		}
	}

	impl rpc_primitives_txpool::TxPoolRuntimeApi<Block> for Runtime {
		fn extrinsic_filter(
			xts_ready: Vec<<Block as BlockT>::Extrinsic>,
			xts_future: Vec<<Block as BlockT>::Extrinsic>,
		) -> rpc_primitives_txpool::TxPoolResponse {
			rpc_primitives_txpool::TxPoolResponse {
				ready: xts_ready
					.into_iter()
					.filter_map(|xt| match xt.0.function {
						RuntimeCall::Ethereum(transact { transaction }) => Some(transaction),
						_ => None,
					})
					.collect(),
				future: xts_future
					.into_iter()
					.filter_map(|xt| match xt.0.function {
						RuntimeCall::Ethereum(transact { transaction }) => Some(transaction),
						_ => None,
					})
					.collect(),
			}
		}
	}

	impl sp_genesis_builder::GenesisBuilder<Block> for Runtime {
		fn build_state(config: Vec<u8>) -> sp_genesis_builder::Result {
			build_state::<RuntimeGenesisConfig>(config)
		}

		fn get_preset(id: &Option<PresetId>) -> Option<Vec<u8>> {
			get_preset::<RuntimeGenesisConfig>(id, |_| None)
		}

		fn preset_names() -> Vec<sp_genesis_builder::PresetId> {
			vec![]
		}
	}

	#[cfg(feature = "runtime-benchmarks")]
	impl frame_benchmarking::Benchmark<Block> for Runtime {
		fn benchmark_metadata(extra: bool) -> (
			Vec<frame_benchmarking::BenchmarkList>,
			Vec<frame_support::traits::StorageInfo>,
		) {
			use frame_benchmarking::{baseline, Benchmarking, BenchmarkList};
			use frame_support::traits::StorageInfoTrait;
			use frame_system_benchmarking::Pallet as SystemBench;
			use baseline::Pallet as BaselineBench;

			let mut list = Vec::<BenchmarkList>::new();
			list_benchmarks!(list, extra);

			let storage_info = AllPalletsWithSystem::storage_info();

			(list, storage_info)
		}

		fn dispatch_benchmark(
			config: frame_benchmarking::BenchmarkConfig
		) -> Result<Vec<frame_benchmarking::BenchmarkBatch>, sp_runtime::RuntimeString> {
			use frame_benchmarking::{baseline, Benchmarking, BenchmarkBatch};
			use sp_storage::TrackedStorageKey;
			use frame_system_benchmarking::Pallet as SystemBench;
			use baseline::Pallet as BaselineBench;
			impl frame_system_benchmarking::Config for Runtime {}
			impl baseline::Config for Runtime {}

			use frame_support::traits::WhitelistedStorageKeys;
			let whitelist: Vec<TrackedStorageKey> = AllPalletsWithSystem::whitelisted_storage_keys();

			let mut batches = Vec::<BenchmarkBatch>::new();
			let params = (&config, &whitelist);
			add_benchmarks!(params, batches);

			Ok(batches)
		}
	}
}
use crate as pallet_account_profile;
use frame_support::{
	derive_impl, parameter_types,
	traits::{ConstU16, ConstU32, ConstU64, Everything},
};
use frame_system::offchain::SendTransactionTypes;
use sp_core::H256;
use sp_runtime::{
	traits::{BlakeTwo256, IdentifyAccount, IdentityLookup},
	BuildStorage, MultiSigner,
};

type Block = frame_system::mocking::MockBlock<Test>;
type UncheckedExtrinsic = frame_system::mocking::MockUncheckedExtrinsic<Test>;
type AccountId = <MultiSigner as IdentifyAccount>::AccountId;

// Configure a mock runtime to test the pallet.
frame_support::construct_runtime!(
	pub enum Test {
		System: frame_system,
		AccountProfile: pallet_account_profile,
		Registration: pallet_registration,
		Metagraph: pallet_metagraph,
		ExecutionUnit: pallet_execution_unit,

	}
);

parameter_types! {
	pub const RegisterPalletId: PalletId = PalletId(*b"register");
	pub const StorageMinerInitialFee: Balance = 100_000_000_000; // 100 tokens
	pub const StorageMiners3InitialFee: Balance = 100_000_000_000; // 100 tokens
	pub const ValidatorInitialFee: Balance = 200_000_000_000; // 200 tokens
	pub const ComputeMinerInitialFee: Balance = 150_000_000_000; // 150 tokens
	pub const GpuMinerInitialFee: Balance = 150_000_000_000; // 150 tokens
	pub const ReportRequestsClearInterval : u32 = 1000;
}

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type BaseCallFilter = Everything;
	type BlockWeights = ();
	type BlockLength = ();
	type DbWeight = ();
	type RuntimeOrigin = RuntimeOrigin;
	type RuntimeCall = RuntimeCall;
	type Block = Block;
	type Hash = H256;
	type Hashing = BlakeTwo256;
	type AccountId = AccountId;
	type Lookup = IdentityLookup<Self::AccountId>;
	type RuntimeEvent = RuntimeEvent;
	type BlockHashCount = ConstU64<250>;
	type Version = ();
	type PalletInfo = PalletInfo;
	type AccountData = ();
	type OnNewAccount = ();
	type OnKilledAccount = ();
	type SystemWeightInfo = ();
	type SS58Prefix = ConstU16<42>;
	type OnSetCode = ();
	type MaxConsumers = ConstU32<16>;
	type Nonce = u64;
}

impl pallet_account_profile::Config for Test {
	type RuntimeEvent = RuntimeEvent;
}

impl pallet_registration::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	// Use Pallet instead of the crate name
	type MetagraphInfo = pallet_metagraph::Pallet<Test>;
	type MetricsInfo = pallet_execution_unit::Pallet<Test>;
	type MinerStakeThreshold = ConstU32<0>;
	type ChainDecimals = ConstU32<18>;
	type PalletId = RegisterPalletId;
	type StorageMinerInitialFee = StorageMinerInitialFee;
	type ValidatorInitialFee = ValidatorInitialFee;
	type ComputeMinerInitialFee = ComputeMinerInitialFee;
	type StorageMiners3InitialFee = StorageMiners3InitialFee;
	type GpuMinerInitialFee = GpuMinerInitialFee;
	type BlocksPerDay = BlocksPerDay;
	type ProxyTypeCompatType = ProxyType;
	type NodeCooldownPeriod = ConstU64<500>;
	type MaxDeregRequestsPerPeriod = ConstU32<20>;
	type ConsensusThreshold = ConsensusThreshold;
	type ConsensusPeriod = ConsensusPeriod;
	type EpochDuration = ConstU32<100>; // epoch pin checks clear duration
	type ReportRequestsClearInterval = ReportRequestsClearInterval;
}

// Implement SendTransactionTypes for Test
impl SendTransactionTypes<crate::Call<Test>> for Test {
	type OverarchingCall = RuntimeCall;
	type Extrinsic = UncheckedExtrinsic;
}

// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Test>::default().build_storage().unwrap();

	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| {
		System::set_block_number(1);
	});
	ext
}

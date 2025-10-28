use crate as pallet_alpha_bridge;
use frame_support::{derive_impl, parameter_types, traits::ConstU64, PalletId};
use sp_keyring::AccountKeyring;
use sp_runtime::{
	traits::{IdentifyAccount, IdentityLookup, Verify},
	BuildStorage,
};

pub type Signature = sp_runtime::MultiSignature;
pub type AccountId = <<Signature as Verify>::Signer as IdentifyAccount>::AccountId;
pub type Balance = u128;

const DAYS: u64 = 24 * 60 * 60 * 1000 / 2000; // blocks per day (assuming 2s block time)

// Test configuration constants
pub const INITIAL_BALANCE: Balance = 1_000_000;
pub const DEFAULT_APPROVE_THRESHOLD: u16 = 2;
pub const DEFAULT_DENY_THRESHOLD: u16 = 2;
pub const DEFAULT_TTL_BLOCKS: u64 = 100;
pub const DEFAULT_MINT_CAP: u128 = 10_000_000;

// Configure a mock runtime to test the pallet
frame_support::construct_runtime!(
	pub enum Test
	{
		System: frame_system,
		Balances: pallet_balances,
		Timestamp: pallet_timestamp,
		PalletIp: pallet_ip,
		AlphaBridge: pallet_alpha_bridge,
	}
);

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Block = frame_system::mocking::MockBlock<Test>;
	type AccountId = AccountId;
	type AccountData = pallet_balances::AccountData<Balance>;
	type Lookup = IdentityLookup<Self::AccountId>;
}

impl frame_system::offchain::SigningTypes for Test {
	type Public = <Signature as Verify>::Signer;
	type Signature = Signature;
}

impl<C> frame_system::offchain::SendTransactionTypes<C> for Test
where
	RuntimeCall: From<C>,
{
	type Extrinsic = frame_system::mocking::MockUncheckedExtrinsic<Test>;
	type OverarchingCall = RuntimeCall;
}

parameter_types! {
	pub const ExistentialDeposit: Balance = 1;
}

#[derive_impl(pallet_balances::config_preludes::TestDefaultConfig)]
impl pallet_balances::Config for Test {
	type Balance = Balance;
	type AccountStore = System;
	type ExistentialDeposit = ExistentialDeposit;
}

impl pallet_timestamp::Config for Test {
	type Moment = u64;
	type OnTimestampSet = ();
	type MinimumPeriod = ConstU64<10000>;
	type WeightInfo = ();
}

parameter_types! {
	pub const IpReleasePeriod: u64 = 15 * DAYS;
}

impl pallet_ip::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type IpReleasePeriod = IpReleasePeriod;
}

parameter_types! {
	pub const AlphaBridgePalletId: PalletId = PalletId(*b"alphbrdg");
}

impl pallet_alpha_bridge::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type Balance = Balance;
	type PalletId = AlphaBridgePalletId;
	type WeightInfo = ();
}

// Test accounts
pub fn alice() -> AccountId {
	AccountKeyring::Alice.to_account_id()
}
pub fn bob() -> AccountId {
	AccountKeyring::Bob.to_account_id()
}
pub fn charlie() -> AccountId {
	AccountKeyring::Charlie.to_account_id()
}
pub fn user1() -> AccountId {
	AccountKeyring::Dave.to_account_id()
}
pub fn user2() -> AccountId {
	AccountKeyring::Eve.to_account_id()
}
pub fn dave() -> AccountId {
	AccountKeyring::Ferdie.to_account_id()
}

// Account not included in genesis - for testing account creation via minting
pub fn new_account() -> AccountId {
	AccountKeyring::One.to_account_id()
}

pub fn new_test_ext() -> sp_io::TestExternalities {
	let mut t = frame_system::GenesisConfig::<Test>::default().build_storage().unwrap();

	pallet_balances::GenesisConfig::<Test> {
		balances: vec![
			(alice(), INITIAL_BALANCE),
			(bob(), INITIAL_BALANCE),
			(charlie(), INITIAL_BALANCE),
			(user1(), INITIAL_BALANCE),
			(user2(), INITIAL_BALANCE),
			(dave(), INITIAL_BALANCE),
		],
	}
	.assimilate_storage(&mut t)
	.unwrap();

	let mut ext: sp_io::TestExternalities = t.into();

	ext.execute_with(|| {
		System::set_block_number(1);
		// Set initial guardian configuration
		pallet_alpha_bridge::Guardians::<Test>::put(vec![alice(), bob(), charlie()]);
		pallet_alpha_bridge::ApproveThreshold::<Test>::put(DEFAULT_APPROVE_THRESHOLD);
		pallet_alpha_bridge::DenyThreshold::<Test>::put(DEFAULT_DENY_THRESHOLD);
		pallet_alpha_bridge::SignatureTTLBlocks::<Test>::put(DEFAULT_TTL_BLOCKS);
		pallet_alpha_bridge::GlobalMintCap::<Test>::put(DEFAULT_MINT_CAP);
		pallet_alpha_bridge::Paused::<Test>::put(false);
	});
	ext
}

// Helper function to run to a specific block number
pub fn run_to_block(n: u64) {
	while System::block_number() < n {
		System::set_block_number(System::block_number() + 1);
	}
}

// Helper function to create a test deposit proposal with unique ID
pub fn create_deposit_proposal(
	recipient: AccountId,
	amount: Balance,
) -> pallet_alpha_bridge::DepositProposal<AccountId> {
	use sp_core::Hasher;
	use sp_runtime::traits::BlakeTwo256;

	// Generate a unique ID by hashing recipient + amount
	let mut data = Vec::new();
	data.extend_from_slice(recipient.as_ref());
	data.extend_from_slice(&amount.to_le_bytes());
	let id = BlakeTwo256::hash(&data);

	pallet_alpha_bridge::DepositProposal { id, recipient, amount }
}

use crate as pallet_hippocampus;
use core::cell::RefCell;
use frame_support::{derive_impl, parameter_types, traits::ConstU32, PalletId};
use sp_keyring::AccountKeyring;
use sp_runtime::{
	traits::{IdentifyAccount, IdentityLookup, Verify},
	BuildStorage,
};

pub type Signature = sp_runtime::MultiSignature;
pub type AccountId = <<Signature as Verify>::Signer as IdentifyAccount>::AccountId;
pub type Balance = u128;

pub const INITIAL_BALANCE: Balance = 10_000_000_000_000;
// Large enough that a payout share can land strictly between zero and the
// ED, so the failed-transfer skip path in `pay_storage_miners` is testable.
pub const EXISTENTIAL_DEPOSIT: Balance = 10;

frame_support::construct_runtime!(
	pub enum Test
	{
		System: frame_system,
		Balances: pallet_balances,
		Hippocampus: pallet_hippocampus,
	}
);

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Block = frame_system::mocking::MockBlock<Test>;
	type AccountId = AccountId;
	type AccountData = pallet_balances::AccountData<Balance>;
	type Lookup = IdentityLookup<Self::AccountId>;
}

parameter_types! {
	pub const ExistentialDeposit: Balance = EXISTENTIAL_DEPOSIT;
}

#[derive_impl(pallet_balances::config_preludes::TestDefaultConfig)]
impl pallet_balances::Config for Test {
	type Balance = Balance;
	type AccountStore = System;
	type ExistentialDeposit = ExistentialDeposit;
}

parameter_types! {
	pub const HippocampusPalletId: PalletId = PalletId(*b"hipocamp");
}

thread_local! {
	static RANKED_MINERS: RefCell<Vec<(AccountId, u16)>> = const { RefCell::new(Vec::new()) };
	static COMPUTE_MINERS: RefCell<Vec<(AccountId, u128)>> = const { RefCell::new(Vec::new()) };
	/// Epoch the mock weight set belongs to. Defaults to `None` so the tests
	/// that predate the replay guard keep exercising a source with no
	/// settlement period, and only the tests that opt in drive the cursor.
	static COMPUTE_EPOCH: RefCell<Option<u64>> = const { RefCell::new(None) };
}

/// Test control for the ranked-miner set `pay_storage_miners` reads.
pub fn set_ranked_miners(miners: Vec<(AccountId, u16)>) {
	RANKED_MINERS.with(|m| *m.borrow_mut() = miners);
}

/// Whitelist an account to call `pay_storage_miners`.
pub fn whitelist_miner_payment_caller(who: AccountId) {
	pallet_hippocampus::MinerPaymentWhitelist::<Test>::insert(who, ());
}

/// Test control for the weighted compute-miner set `pay_compute_miners` reads.
pub fn set_compute_miners(miners: Vec<(AccountId, u128)>) {
	COMPUTE_MINERS.with(|m| *m.borrow_mut() = miners);
}

pub struct MockRanking;
impl pallet_hippocampus::StorageMinerRanking<AccountId> for MockRanking {
	fn active_storage_miners() -> Vec<(AccountId, u16)> {
		RANKED_MINERS.with(|m| m.borrow().clone())
	}
}

/// Test control for the epoch the compute weight set belongs to. `None`
/// (the default) opts out of the bank's replay guard.
pub fn set_compute_epoch(epoch: Option<u64>) {
	COMPUTE_EPOCH.with(|e| *e.borrow_mut() = epoch);
}

pub struct MockComputeWeights;
impl pallet_hippocampus::ComputeMinerWeights<AccountId> for MockComputeWeights {
	fn active_compute_miners() -> Vec<(AccountId, u128)> {
		COMPUTE_MINERS.with(|m| m.borrow().clone())
	}

	fn current_weight_epoch() -> Option<u64> {
		COMPUTE_EPOCH.with(|e| *e.borrow())
	}
}

parameter_types! {
	pub const BlocksPer24Hours: u64 = 14_400; // ~24 hours at 6-second blocks
	// Deliberately tiny compared to the runtime's 3500-alpha constant so the
	// rate-limit tests can actually cross the cap with mock-scale balances.
	pub const Max24HourMinerPayout: Balance = 10_000;
}

impl pallet_hippocampus::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type Currency = Balances;
	type PalletId = HippocampusPalletId;
	type AdminOrigin = frame_system::EnsureRoot<AccountId>;
	type MinerRanking = MockRanking;
	type MaxMinersPerPayout = ConstU32<16>;
	type BlocksPer24Hours = BlocksPer24Hours;
	type Max24HourMinerPayout = Max24HourMinerPayout;
	type ComputeMinerWeights = MockComputeWeights;
	type MaxComputeMinersPerPayout = ConstU32<16>;
	type WeightInfo = ();
}

pub fn alice() -> AccountId {
	AccountKeyring::Alice.to_account_id()
}

pub fn bob() -> AccountId {
	AccountKeyring::Bob.to_account_id()
}

pub fn charlie() -> AccountId {
	AccountKeyring::Charlie.to_account_id()
}

pub fn dave() -> AccountId {
	AccountKeyring::Dave.to_account_id()
}

pub fn hippocampus_account() -> AccountId {
	pallet_hippocampus::Pallet::<Test>::account_id()
}

pub fn new_test_ext() -> sp_io::TestExternalities {
	let mut t = frame_system::GenesisConfig::<Test>::default().build_storage().unwrap();
	pallet_balances::GenesisConfig::<Test> {
		balances: vec![(alice(), INITIAL_BALANCE), (bob(), INITIAL_BALANCE)],
	}
	.assimilate_storage(&mut t)
	.unwrap();

	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| {
		System::set_block_number(1);
		set_ranked_miners(Vec::new());
		set_compute_miners(Vec::new());
	});
	ext
}

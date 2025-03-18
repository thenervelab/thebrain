use crate as pallet_subaccount;
use frame_support::{
    derive_impl, 
    traits::{ConstU16, ConstU32, ConstU64, Everything, Get},
    
};
use sp_core::H256;
use sp_runtime::{
    traits::{BlakeTwo256, IdentityLookup},
    BuildStorage,
};
use frame_system::offchain::SendTransactionTypes;

type Block = frame_system::mocking::MockBlock<Test>;
type UncheckedExtrinsic = frame_system::mocking::MockUncheckedExtrinsic<Test>;

// Balance of an account.
// pub type Balance = u128;
// pub type AccountId = u64;

/// Index of a transaction in the chain.
pub type Nonce = u32;

// Configure a mock runtime to test the pallet.
frame_support::construct_runtime!(
    pub enum Test {
        System: frame_system,
        SubAccounts: pallet_subaccount,
    }
);

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
    type AccountId = u64;
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
    type Nonce = Nonce;
}

// Mock StringLimit for tests
pub struct StringLimit;
impl Get<u32> for StringLimit {
    fn get() -> u32 { 50 }
}


impl pallet_subaccount::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type WeightInfo = ();
    type StringLimit = StringLimit;
}

// Implement SendTransactionTypes for Test
impl SendTransactionTypes<crate::Call<Test>> for Test {
    type OverarchingCall = RuntimeCall;
    type Extrinsic = UncheckedExtrinsic;
}


// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
    let t = frame_system::GenesisConfig::<Test>::default()
        .build_storage()
        .unwrap();

    let mut ext = sp_io::TestExternalities::new(t);
    ext.execute_with(|| {
        System::set_block_number(1);
    });
    ext
}
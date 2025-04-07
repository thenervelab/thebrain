use crate as pallet_compute;
use frame_support::{
	derive_impl,
	traits::{ConstU16, ConstU32, ConstU64, Everything},
};
use frame_system::offchain::SendTransactionTypes;
use sp_core::H256;
use sp_runtime::BuildStorage;
use sp_runtime::{
	traits::{BlakeTwo256, IdentifyAccount, IdentityLookup, Verify},
	MultiSignature, MultiSigner,
};

type Block = frame_system::mocking::MockBlock<Test>;
type UncheckedExtrinsic = frame_system::mocking::MockUncheckedExtrinsic<Test>;
type AccountId = <MultiSigner as IdentifyAccount>::AccountId;
type Signature = MultiSignature;

// Configure a mock runtime to test the pallet.
frame_support::construct_runtime!(
	pub enum Test {
		System: frame_system,
		Compute: pallet_compute,
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
	// type Header = Header;
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

impl frame_system::offchain::SigningTypes for Test {
	type Public = <Signature as Verify>::Signer;
	type Signature = Signature;
}

// Implement SendTransactionTypes for Test
impl SendTransactionTypes<crate::Call<Test>> for Test {
	type OverarchingCall = RuntimeCall;
	type Extrinsic = UncheckedExtrinsic;
}

impl pallet_compute::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type AuthorityId = crypto::TestAuthId;
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

// Import the crypto module for TestAuthId
use crate::crypto;

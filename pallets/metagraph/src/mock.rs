use crate as pallet_metagraph;
use frame_support::traits::{ConstU16, ConstU64};
use frame_system as system;
use sp_core::{sr25519, H256};
use sp_runtime::{
    traits::{BlakeTwo256, IdentityLookup, Verify},
    testing::{Header, TestXt},
    BuildStorage, RuntimeAppPublic,
};
use frame_system::offchain::{SigningTypes, SendTransactionTypes};
use sp_core::Pair;
use sp_keystore::testing::KeyStore;
use std::sync::Arc;
use sp_consensus_babe::{AuthorityId as BabeId, BabeEpochConfiguration, AllowedSlots};

type UncheckedExtrinsic = frame_system::mocking::MockUncheckedExtrinsic<Test>;
type Block = frame_system::mocking::MockBlock<Test>;

// Configure a mock runtime to test the pallet.
frame_support::construct_runtime!(
    pub enum Test where
        Block = Block,
        NodeBlock = Block,
        UncheckedExtrinsic = UncheckedExtrinsic,
    {
        System: frame_system,
        MetagraphModule: pallet_metagraph,
        Babe: pallet_babe,
    }
);

impl system::Config for Test {
    type BaseCallFilter = frame_support::traits::Everything;
    type BlockWeights = ();
    type BlockLength = ();
    type DbWeight = ();
    type RuntimeOrigin = RuntimeOrigin;
    type RuntimeCall = RuntimeCall;
    type Nonce = u64;
    type Hash = H256;
    type Hashing = BlakeTwo256;
    type AccountId = sr25519::Public;
    type Lookup = IdentityLookup<Self::AccountId>;
    type Block = Block;
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
    type MaxConsumers = frame_support::traits::ConstU32<16>;
    type RuntimeTask = ();
    type SingleBlockMigrations = ();
    type MultiBlockMigrator = ();
    type PreInherents = ();
    type PostInherents = ();
    type PostTransactions = ();
}

parameter_types! {
    pub const EpochDuration: u64 = 10;
    pub const ExpectedBlockTime: u64 = 3000;
    pub const DisabledValidatorCount: u32 = 0;
    pub const MaxAuthorities: u32 = 10;
    pub const ReportLongevity: u64 = 24;
}

impl pallet_babe::Config for Test {
    type EpochDuration = EpochDuration;
    type ExpectedBlockTime = ExpectedBlockTime;
    type EpochChangeTrigger = pallet_babe::ExternalTrigger;
    type DisabledValidators = ();
    type WeightInfo = ();
    type MaxAuthorities = MaxAuthorities;
    type MaxNominators = frame_support::traits::ConstU32<0>;
    type KeyOwnerProof = sp_core::Void;
    type EquivocationReportSystem = ();
}

impl SigningTypes for Test {
    type Public = sr25519::Public;
    type Signature = sr25519::Signature;
}

impl SendTransactionTypes<RuntimeCall> for Test {
    type Extrinsic = UncheckedExtrinsic;
    type OverarchingCall = RuntimeCall;
}

impl frame_system::offchain::CreateSignedTransaction<RuntimeCall> for Test {
    fn create_transaction<C: frame_system::offchain::AppCrypto<Self::Public, Self::Signature>>(
        call: RuntimeCall,
        _public: Self::Public,
        _account: <Test as frame_system::Config>::AccountId,
        _nonce: u64,
    ) -> Option<(RuntimeCall, <UncheckedExtrinsic as sp_runtime::traits::Extrinsic>::SignaturePayload)> {
        Some((call, Default::default()))
    }
}

impl pallet_metagraph::Config for Test {
    type RuntimeEvent = RuntimeEvent;
    type AuthorityId = pallet_metagraph::crypto::MetagraphAuthId;
}

// Build genesis storage according to the mock runtime.
pub fn new_test_ext() -> sp_io::TestExternalities {
    let mut t = system::GenesisConfig::<Test>::default()
        .build_storage()
        .unwrap();

    let keystore = KeyStore::new();
    keystore.sr25519_generate_new(
        crate::KEY_TYPE,
        Some(b"//Alice"),
    )
    .expect("Creating test key failed");

    let mut ext = sp_io::TestExternalities::new(t);
    ext.register_extension(sp_keystore::KeystoreExt(Arc::new(keystore)));
    ext
}

// Helper function to generate a test key pair
pub fn get_test_key_pair() -> sr25519::Pair {
    sr25519::Pair::from_string("//Alice", None).expect("Creates pair")
}

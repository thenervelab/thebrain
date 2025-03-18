// use crate as pallet_ranking;
// use frame_support::{
//     derive_impl, 
//     parameter_types, 
//     traits::{ConstU16, ConstU32, ConstU64},
// };
// use frame_system::EnsureRoot;
// use sp_core::H256;
// use sp_runtime::{
//     testing::TestXt,
//     traits::{BlakeTwo256, IdentityLookup, Zero},
//     BuildStorage,
// };
// use pallet_registration::NodeType;
// use pallet_balances;
// use pallet_registration;
// use pallet_staking;
// use frame_support::traits::KeyOwnerProofSystem;
// use pallet_staking::config_preludes::{BondingDuration, SessionsPerEra};
// use sp_core::offchain::KeyTypeId;
// use frame_system::offchain::SendTransactionTypes;


// use sp_runtime::{
//     traits::{  Verify, IdentifyAccount},
//     MultiSignature, MultiSigner,
// };

// type Block = frame_system::mocking::MockBlock<Test>;
// type UncheckedExtrinsic = frame_system::mocking::MockUncheckedExtrinsic<Test>;
// type AccountId = <MultiSigner as IdentifyAccount>::AccountId;
// type Signature = MultiSignature;

// // Construct a mock runtime
// frame_support::construct_runtime!(
//     pub enum Test 
//     {
//         System: frame_system,
//         Balances: pallet_balances,
//         Registration: pallet_registration,
//         Staking: pallet_staking,
//         Ranking: pallet_ranking,
// 		Utils: pallet_utils,
// 		// Jobs: pallet_jobs,
// 		// Dkg: pallet_dkg,
// 		// ZkSaaS: pallet_zksaas,
// 		// Assets: pallet_assets,
// 		// IpfsPin : pallet_ipfs_pin,
// 		// Registration : pallet_registration,
// 		// ExecutionUnit : pallet_execution_unit,
// 		// Metagraph: pallet_metagraph,
// 		// Babe: pallet_babe
//     }
// );

// parameter_types! {
//     pub const BlockHashCount: u64 = 250;
//     pub const SS58Prefix: u8 = 42;
//     pub const ExistentialDeposit: u128 = 1;
//     pub const MaxLocks: u32 = 50;
//     pub const MaxReserves: u32 = 50;
    
//     // Ranking pallet specific parameters
//     pub const PalletId: frame_support::PalletId = frame_support::PalletId(*b"mrktplce");
//     pub const RelayNodesRewardPercentage: u32 = 40;
//     pub const MinerNodesRewardPercentage: u32 = 60;
//     pub const InstanceID: u16 = 1;
//     pub const RankDistributionLimit: u16 = 10;
// }

// #[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
// impl frame_system::Config for Test {
//     type BaseCallFilter = frame_support::traits::Everything;
//     type BlockWeights = ();
//     type BlockLength = ();
//     type Block = Block;
//     type RuntimeOrigin = RuntimeOrigin;
//     type RuntimeCall = RuntimeCall;
//     type Nonce = u64;
//     type Hash = H256;
//     type Hashing = BlakeTwo256;
//     type AccountId = AccountId;
//     type Lookup = IdentityLookup<Self::AccountId>;
//     type RuntimeEvent = RuntimeEvent;
//     type BlockHashCount = BlockHashCount;
//     type DbWeight = ();
//     type Version = ();
//     type PalletInfo = PalletInfo;
//     type AccountData = ();
//     type OnNewAccount = ();
//     type OnKilledAccount = ();
//     type SystemWeightInfo = ();
//     type SS58Prefix = SS58Prefix;
//     type OnSetCode = ();
//     type MaxConsumers = frame_support::traits::ConstU32<16>;
// }

// impl pallet_balances::Config for Test {
//     type MaxLocks = MaxLocks;
//     type MaxReserves = MaxReserves;
//     type ReserveIdentifier = [u8; 8];
//     type Balance = u128;
//     type RuntimeEvent = RuntimeEvent;
//     type DustRemoval = ();
//     type ExistentialDeposit = ExistentialDeposit;
//     type AccountStore = System;
//     type WeightInfo = ();
//     type FreezeIdentifier = ();
//     type MaxFreezes = ();
//     type RuntimeHoldReason = ();
//     type RuntimeFreezeReason = ();
// }

// parameter_types! {
//     pub const LocalRpcUrl: &'static str = "http://localhost:9944";
//     pub const IpfsPinRpcMethod: &'static str = "extra_peerId";
// }

// impl pallet_utils::Config for Test {
//     type RuntimeEvent = RuntimeEvent;
// 	type LocalRpcUrl = LocalRpcUrl;
//     type RpcMethod = IpfsPinRpcMethod;
// }

// impl pallet_registration::Config for Test {
//     type RuntimeEvent = RuntimeEvent;
//     // Use Pallet instead of the crate name
//     // type MetagraphInfo = pallet_metagraph::Pallet<Test>;
// 	type MinerStakeThreshold = ConstU32<0>;
// 	type ChainDecimals = ConstU32<18>;
// }

// impl frame_system::offchain::SigningTypes for Test {
// 	type Public = <Signature as Verify>::Signer;
// 	type Signature = Signature;
// }

// // Implement SendTransactionTypes for Test
// impl SendTransactionTypes<crate::Call<Test>> for Test {
//     type OverarchingCall = RuntimeCall;
//     type Extrinsic = UncheckedExtrinsic;
// }


// impl pallet_staking::Config for Test {
//     type RuntimeEvent = RuntimeEvent;
//     type Currency = Balances;
//     type UnixTime = ();
//     type CurrencyToVote = ();
//     type RewardRemainder = ();
//     // type RuntimeHoldReason = ();
//     // type RuntimeFreezeReason = ();
//     type WeightInfo = ();
// }

// impl pallet_ranking::Config for Test {
//     type RuntimeEvent = RuntimeEvent;
//     type PalletId = PalletId;
//     type RelayNodesRewardPercentage = RelayNodesRewardPercentage;
//     type MinerNodesRewardPercentage = MinerNodesRewardPercentage;
//     type InstanceID = InstanceID;
//     type AuthorityId = crypto::TestAuthId;
// }

// // Implement mock crypto for offchain workers
// pub mod crypto {
//     use super::*;
//     use sp_core::sr25519::Signature as Sr25519Signature;
//     use sp_runtime::{
//         app_crypto::{app_crypto, sr25519},
//         traits::Verify,
//         MultiSignature, MultiSigner,
//     };
    
//     app_crypto!(sr25519, crate::KEY_TYPE);

//     pub struct TestAuthId;

//     impl frame_system::offchain::AppCrypto<MultiSigner, MultiSignature> for TestAuthId {
//         type RuntimeAppPublic = Public;
//         type GenericSignature = sp_core::sr25519::Signature;
//         type GenericPublic = sp_core::sr25519::Public;
//     }
// }


// // parameter_types! {
// // 	// NOTE: Currently it is not possible to change the epoch duration after the chain has started.
// // 	//       Attempting to do so will brick block production.
// // 	pub const EpochDuration: u64 = 6;
// // 	pub const ExpectedBlockTime: u64 = 12;
// // 	pub const ReportLongevity: u64 =
// // 		BondingDuration::get() as u64 * SessionsPerEra::get() as u64 * EpochDuration::get();
// // 	pub const MaxAuthorities: u32 = 1000;
// // 	pub const MaxNominators: u32 = 1000;
// // }


// // impl pallet_babe::Config for Test {
// // 	type EpochDuration = EpochDuration;
// // 	type ExpectedBlockTime = ExpectedBlockTime;
// // 	type EpochChangeTrigger = pallet_babe::ExternalTrigger;
// // 	type DisabledValidators = Session;
// // 	type WeightInfo = ();
// // 	type MaxAuthorities = MaxAuthorities;
// // 	type MaxNominators = MaxNominatorRewardedPerValidator;
// // 	type KeyOwnerProof =
// // 		<Historical as KeyOwnerProofSystem<(KeyTypeId, pallet_babe::AuthorityId)>>::Proof;
// // 	type EquivocationReportSystem =
// // 		pallet_babe::EquivocationReportSystem<Self, Offences, Historical, ReportLongevity>;
// // }

// // parameter_types! {
// //     pub const FinneyApiUrl: &'static str = "http://loclahost:9945";
// //     pub const FinneyUidsStorageKey: &'static str = "0x658faa385070e074c85bf6b568cf0555aab1b4e78e1ea8305462ee53b3686dc80100";
// //     pub const FinneyDividendsStorageKey: &'static str = "0x658faa385070e074c85bf6b568cf055586752d66f11480ecef37769cdd736b9b0100";
// // 	pub const UidsSubmissionInterval: u32 = 33; // Set your desired value here
// // }

// // impl pallet_metagraph::Config for Test {
// //     type RuntimeEvent = RuntimeEvent;
// //     type FinneyUrl = FinneyApiUrl;
// //     type UidsStorageKey = FinneyUidsStorageKey;
// //     type DividendsStorageKey = FinneyDividendsStorageKey;
// // 	type UidsSubmissionInterval = UidsSubmissionInterval;
// // 	type AuthorityId = pallet_metagraph::crypto::TestAuthId;
// // }

// // Build genesis storage according to the mock runtime
// pub fn new_test_ext() -> sp_io::TestExternalities {
//     let mut t = frame_system::GenesisConfig::<Test>::default()
//         .build_storage()
//         .unwrap();

//     // Add initial balances if needed
//     // pallet_balances::GenesisConfig::<Test> {
//     //     balances: vec![(1, 1000), (2, 2000), (3, 3000)],
//     // }
//     // .assimilate_storage(&mut t)
//     // .unwrap();

//     t.into()
// }
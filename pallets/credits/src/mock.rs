// use crate as pallet_credits;
// use frame_support::{
//     derive_impl,
//     traits::{ConstU16,  ConstU64},
// };
// use sp_core::H256;
// use sp_runtime::{
//     traits::{BlakeTwo256, IdentityLookup},
// };
// use sp_runtime::BuildStorage;
// type Block = frame_system::mocking::MockBlock<Test>;
// type Balance = u64;
// type AccountId = u64;

// // Configure a mock runtime to test the pallet.
// frame_support::construct_runtime!(
//     pub enum Test {
//         System: frame_system,
//         Balances: pallet_balances,
//         Credits: pallet_credits,
//     }
// );

// #[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
// impl frame_system::Config for Test {
//     type BaseCallFilter = frame_support::traits::Everything;
//     type BlockWeights = ();
//     type BlockLength = ();
//     type DbWeight = ();
//     type RuntimeOrigin = RuntimeOrigin;
//     type RuntimeCall = RuntimeCall;
//     type Hash = H256;
//     type Hashing = BlakeTwo256;
//     type AccountId = AccountId;
//     type Lookup = IdentityLookup<Self::AccountId>;
//     type RuntimeEvent = RuntimeEvent;
//     type BlockHashCount = ConstU64<250>;
//     type Version = ();
// 	type Block = Block;
//     type PalletInfo = PalletInfo;
//     type AccountData = pallet_balances::AccountData<Balance>;
//     type OnNewAccount = ();
//     type OnKilledAccount = ();
//     type SystemWeightInfo = ();
//     type SS58Prefix = ConstU16<42>;
//     type OnSetCode = ();
//     type MaxConsumers = frame_support::traits::ConstU32<16>;
// }

// #[derive_impl(pallet_balances::config_preludes::TestDefaultConfig)]
// impl pallet_balances::Config for Test {
// 	type MaxLocks = frame_support::traits::ConstU32<1024>;
// 	type Balance = Balance;
// 	type ExistentialDeposit = ConstU64<1>;
// 	type AccountStore = System;
// }

// impl pallet_credits::Config for Test {
//     type RuntimeEvent = RuntimeEvent;
// }

// // Build genesis storage according to the mock runtime.
// pub fn new_test_ext() -> sp_io::TestExternalities {
//     let mut t = frame_system::GenesisConfig::<Test>::default()
//         .build_storage()
//         .unwrap();

//     pallet_balances::GenesisConfig::<Test> {
//         balances: vec![(1, 100), (2, 200)],
//     }
//     .assimilate_storage(&mut t)
//     .unwrap();

//     let mut ext = sp_io::TestExternalities::new(t);
//     ext.execute_with(|| System::set_block_number(1));
//     ext
// }

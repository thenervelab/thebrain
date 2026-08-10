//! End-to-end: emission deposited into the bank is distributed to ranked
//! storage-miner owners by `pay_storage_miners`, and the emission
//! compartment is invisible to the arion settlement headroom.

use frame_support::traits::Currency;
use hippius_mainnet_runtime::{
	AccountId, ArionPayoutSource, Balances, Hippocampus, Runtime, RuntimeOrigin, System,
};
use pallet_arion::PayoutSource;
use pallet_hippocampus::DepositType;
use sp_core::crypto::Ss58Codec;
use sp_runtime::{AccountId32, BuildStorage};

/// Must match `ExistentialDeposit` in the runtime.
const ED: u128 = 500;

/// The hardcoded `ArionAdminMembers` account (all arion/bank admin origins).
fn admin() -> AccountId {
	AccountId32::from_ss58check("5CVXqxb7mhFTtZVw5BJ8M2ujND9PFymSDxF8bkod6Sm4XJTW").unwrap()
}

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| System::set_block_number(1));
	ext
}

#[test]
fn emission_compartment_invisible_to_arion_settlement() {
	new_test_ext().execute_with(|| {
		let funder = account(1);
		Balances::make_free_balance_be(&funder, 1_000_000);

		// ED cushion + the emission pot.
		assert!(Hippocampus::deposit(
			RuntimeOrigin::signed(funder.clone()),
			ED * 2,
			DepositType::Grant
		)
		.is_ok());
		let arion_headroom_before = ArionPayoutSource::available();
		assert!(Hippocampus::deposit(
			RuntimeOrigin::signed(funder),
			100_000,
			DepositType::Emission
		)
		.is_ok());

		// Wall: emission is invisible to the arion settlement headroom.
		// Arion can only spend what the bank has *minus* emission.
		assert_eq!(ArionPayoutSource::available(), arion_headroom_before);
		assert_eq!(Hippocampus::emission_available(), 100_000);
	});
}

#[test]
fn pay_storage_miners_with_empty_ranking() {
	new_test_ext().execute_with(|| {
		let funder = account(1);
		Balances::make_free_balance_be(&funder, 1_000_000);

		assert!(Hippocampus::deposit(
			RuntimeOrigin::signed(funder.clone()),
			ED * 2,
			DepositType::Grant
		)
		.is_ok());
		assert!(Hippocampus::deposit(
			RuntimeOrigin::signed(funder),
			100_000,
			DepositType::Emission
		)
		.is_ok());

		// With no ranked miners, the call should fail.
		assert!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(admin()), 100_000).is_err());
		// Emission is untouched.
		assert_eq!(Hippocampus::emission_available(), 100_000);
	});
}

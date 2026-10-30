//! compute-scoring per-tier prices (vendored from hippius-compute #1846) in
//! the runtime: the ratios the runtime binds, the v0 → v1 version bump, and
//! the cursor migration converting RU prices with those ratios.

use frame_support::{
	assert_noop, assert_ok,
	traits::{GetStorageVersion, OnRuntimeUpgrade, StorageVersion},
};
use hippius_mainnet_runtime::{AccountId, ComputeScoring, Runtime, RuntimeOrigin, System};
use pallet_compute_scoring::{
	migrations::MigrateV0ToV1, DiskTier, MinerPrice, NodeIdToChild, TierMigrationCursor, TierPrice,
	TierPriceBounds, SPEC_TIER_PRICE_RATIOS,
};
use sp_core::crypto::Ss58Codec;
use sp_runtime::{AccountId32, BuildStorage};

fn authority() -> AccountId {
	AccountId32::from_ss58check("5CP9wzk9G3kMdJmNyAsWGWVWDQE7Goe1dUvKtMv51EoXs563").unwrap()
}

fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| System::set_block_number(1));
	ext
}

#[test]
fn the_runtime_binds_the_published_tier_ratios() {
	let bound = <Runtime as pallet_compute_scoring::Config>::TierPriceRatios::get();
	assert_eq!(bound, SPEC_TIER_PRICE_RATIOS);
	assert_eq!(
		bound,
		[(55_478_250, 45_207_000), (55_478_250, 38_631_000), (55_478_250, 23_918_000)]
	);
}

#[test]
fn v0_to_v1_only_bumps_the_version() {
	new_test_ext().execute_with(|| {
		StorageVersion::new(0).put::<ComputeScoring>();
		let node = [7u8; 32];
		MinerPrice::<Runtime>::insert(node, 10_000);
		MigrateV0ToV1::<Runtime>::on_runtime_upgrade();
		assert_eq!(ComputeScoring::on_chain_storage_version(), 1);
		assert_eq!(MinerPrice::<Runtime>::get(node), Some(10_000));
		assert!(TierPrice::<Runtime>::iter().next().is_none());
		assert!(!pallet_compute_scoring::Pallet::<Runtime>::tier_prices_live());
		// Idempotent.
		MigrateV0ToV1::<Runtime>::on_runtime_upgrade();
		assert_eq!(ComputeScoring::on_chain_storage_version(), 1);
	});
}

#[test]
fn the_cursor_migration_converts_with_the_runtime_ratios() {
	new_test_ext().execute_with(|| {
		let node = [7u8; 32];
		NodeIdToChild::<Runtime>::insert(node, AccountId32::new([8u8; 32]));
		let ru = 10_000u128;
		MinerPrice::<Runtime>::insert(node, ru);

		// Bounds first: the migration refuses to start without all three.
		assert_noop!(
			ComputeScoring::migrate_tier_prices(RuntimeOrigin::signed(authority()), 10),
			pallet_compute_scoring::Error::<Runtime>::PriceBoundsNotConfigured
		);
		for tier in DiskTier::ALL {
			assert_ok!(ComputeScoring::set_tier_price_bounds(
				RuntimeOrigin::signed(authority()),
				tier,
				1,
				1_000_000
			));
			assert!(TierPriceBounds::<Runtime>::contains_key(tier));
		}
		// Admin only.
		assert!(ComputeScoring::migrate_tier_prices(RuntimeOrigin::signed(AccountId32::new([9u8; 32])), 10)
			.is_err());
		assert_ok!(ComputeScoring::migrate_tier_prices(RuntimeOrigin::signed(authority()), 10));
		assert!(pallet_compute_scoring::Pallet::<Runtime>::tier_prices_live());
		assert_eq!(
			TierMigrationCursor::<Runtime>::get(),
			pallet_compute_scoring::TierMigrationStatus::Done
		);

		for (i, tier) in DiskTier::ALL.into_iter().enumerate() {
			let (num, den) = SPEC_TIER_PRICE_RATIOS[i];
			let t = TierPrice::<Runtime>::get(node, tier).expect("converted");
			// floor(ru × k_num / k_den), or for nvme the smallest price whose
			// alias is exactly `ru`: within one of the floor either way.
			let floor = ru * num / den;
			assert!(t == floor || t == floor + 1, "{tier:?}: {t} vs {floor}");
		}
		// The RU alias still reads the original price.
		assert_eq!(MinerPrice::<Runtime>::get(node), Some(ru));
		// A second run is refused.
		assert_noop!(
			ComputeScoring::migrate_tier_prices(RuntimeOrigin::signed(authority()), 10),
			pallet_compute_scoring::Error::<Runtime>::TierMigrationAlreadyDone
		);
	});
}

#[cfg(test)]
mod tests {
	use super::*;
	use frame_support::{assert_noop, assert_ok, traits::OnRuntimeUpgrade};
	use frame_system as system;
	use sp_core::H256;
	use sp_runtime::{
		testing::Header,
		traits::{BlakeTwo256, IdentityLookup},
	};

	type Test = frame_system::mocking::MockRuntime;

	frame_support::construct_runtime!(
		pub enum TestRuntime where
			Block = frame_system::mocking::MockBlock<Test>,
			NodeBlock = frame_system::mocking::MockBlock<Test>,
			UncheckedExtrinsic = frame_system::mocking::MockUncheckedExtrinsic<Test>,
		{
			System: frame_system::{Pallet, Call, Config, Storage, Event<T>},
			SubAccountPallet: pallet::{Pallet, Storage, Config<T>, Event<T>},
		}
	);

	impl system::Config for Test {
		type BaseCallFilter = frame_support::traits::Everything;
		type BlockWeights = ();
		type BlockLength = ();
		type RuntimeDbWeight = ();
		type RuntimeEvent = ();
		type RuntimeOrigin = RuntimeOrigin;
		type Hash = H256;
		type Hashing = BlakeTwo256;
		type AccountId = u64;
		type Lookup = IdentityLookup<u64>;
		type Header = Header;
		type BlockNumber = u64;
		type RuntimeCall = ();
		type RuntimeBlock = ();
		type RuntimeVersion = ();
		type PalletInfo = PalletInfo;
		type OnSetCode = ();
		type OnRuntimeUpgrade = ();
		type MaxConsumers = frame_support::traits::ConstU32<16>;
	}

	impl pallet_subaccount::Config for Test {
		type RuntimeEvent = ();
		type WeightInfo = ();
		type StringLimit = frame_support::traits::ConstU32<300>;
		type OnRuntimeUpgrade = MigrateToNewStorageFormat<Test>;
	}

	/// Helper function to setup the initial storage state
	fn setup_old_storage() {
		SubAccount::<Test>::insert(1, 100);
		SubAccount::<Test>::insert(2, 200);
	}

	#[test]
	fn test_migration_transfers_old_data() {
		new_test_ext().execute_with(|| {
			// Setup old storage state
			setup_old_storage();

			// Ensure initial storage contains values
			assert_eq!(SubAccount::<Test>::get(1), Some(100));
			assert_eq!(SubAccount::<Test>::get(2), Some(200));

			// Run the migration
			let weight = MigrateToNewStorageFormat::<Test>::on_runtime_upgrade();

			// Verify migration result
			assert_eq!(NewSubAccount::<Test>::get(1), Some((100, true)));
			assert_eq!(NewSubAccount::<Test>::get(2), Some((200, true)));

			// Old storage should no longer exist
			assert_eq!(SubAccount::<Test>::get(1), None);
			assert_eq!(SubAccount::<Test>::get(2), None);

			// Ensure a non-zero weight is returned
			assert!(weight.ref_time() > 0);
		});
	}

	#[test]
	fn test_migration_handles_empty_storage() {
		new_test_ext().execute_with(|| {
			// No setup, storage is empty

			// Run the migration
			let weight = MigrateToNewStorageFormat::<Test>::on_runtime_upgrade();

			// Ensure no data was added
			assert_eq!(NewSubAccount::<Test>::iter().count(), 0);

			// Ensure a valid weight is returned
			assert!(weight.ref_time() > 0);
		});
	}
}

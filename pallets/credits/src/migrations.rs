use super::*;
use frame_support::{
    pallet_prelude::*,
    traits::{OnRuntimeUpgrade, StorageVersion},
};
use core::marker::PhantomData;
use frame_system::pallet_prelude::BlockNumberFor;
use sp_runtime::Vec;

pub struct Migrate<T>(PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for Migrate<T> {
    fn on_runtime_upgrade() -> Weight {
        let mut weight = Weight::zero();

        // 🔥 Fix: If StorageVersion is unset, initialize it to 0 first
        let current_version = StorageVersion::get::<Pallet<T>>();
        if current_version == StorageVersion::default() {
            StorageVersion::new(0).put::<Pallet<T>>();
        }

        // ✅ Run migration only if version is 0
        if current_version == 0 {
            weight = Self::migrate_locked_credits_with_migration_flag();
            StorageVersion::new(1).put::<Pallet<T>>();
        } else {
            log::info!("Skipping migration, already migrated.");
        }

        weight
    }
}

impl<T: Config> Migrate<T> {
    fn migrate_locked_credits_with_migration_flag() -> Weight {
        let mut migrated_count = 0;

        for (account, old_credits) in LockedCredits::<T>::iter() {
            let mut updated_credits = old_credits.clone();

            // 🔥 Fix: Handle missing field in older storage
            let credits_modified = updated_credits.iter_mut().fold(false, |modified, credit| {
                if !credit.is_migrated {
                    credit.is_migrated = true;
                    true
                } else {
                    modified
                }
            });

            if credits_modified {
                migrated_count += updated_credits.len();
                LockedCredits::<T>::insert(&account, updated_credits);
            }
        }


        T::DbWeight::get().reads_writes(migrated_count as u64, migrated_count as u64)
    }
}

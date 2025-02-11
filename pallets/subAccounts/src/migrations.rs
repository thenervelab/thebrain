use frame_support::{migration::StorageMapMigration, traits::OnRuntimeUpgrade, Twox64Concat, Blake2_128Concat, pallet_prelude::*};
use frame_system::Config as SystemConfig;
use super::Config;

pub struct MigrateToNewStorageFormat<T: Config>(PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for MigrateToNewStorageFormat<T> {
    fn on_runtime_upgrade() -> frame_support::weights::Weight {
        let module_prefix = b"SubAccounts";
        let storage_prefix = b"SubAccount";
        
        let mut count: u64 = 0;
        StorageMapMigration::<
            Blake2_128Concat,
            T::AccountId,
            T::AccountId,
            (T::AccountId, bool),
            Twox64Concat
        >::migrate_storage(
            module_prefix,
            storage_prefix,
            |key, old_value| {
                count += 1;
                Some((old_value, true))
            }
        );
        
        log::info!(
            "Migrated {} entries in SubAccount storage.",
            count
        );
        
        T::DbWeight::get().reads_writes(count as u64, count as u64)
    }
}

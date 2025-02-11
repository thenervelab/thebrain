use frame_support::{traits::OnRuntimeUpgrade, pallet_prelude::*};
use super::{Config, SubAccount, NewSubAccount};

pub struct MigrateToNewStorageFormat<T: Config>(PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for MigrateToNewStorageFormat<T> {
    fn on_runtime_upgrade() -> frame_support::weights::Weight {
        let mut count: u64 = 0;

        // Drain old storage and insert into new storage
        for (key, old_value) in SubAccount::<T>::drain() {
            NewSubAccount::<T>::insert(&key, (old_value.clone(), true));
            count += 1;
        }

        log::info!(
            "Migrated {} entries from SubAccount to NewSubAccount.",
            count
        );

        // Return the weight
        T::DbWeight::get().reads_writes(count as u64, count as u64)
    }
}

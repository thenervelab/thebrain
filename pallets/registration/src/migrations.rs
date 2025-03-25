use super::*;
use frame_support::{
    pallet_prelude::*, 
    traits::{OnRuntimeUpgrade, StorageInstance},
};
pub struct MigrateNodeRegistration<T>(PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for MigrateNodeRegistration<T> {
    fn on_runtime_upgrade() -> frame_support::weights::Weight {
        let mut count = 0u64;

        // Iterate over all existing node registrations
        for (node_id, node_info) in NodeRegistration::<T>::iter() {
            if let Some(info) = node_info {
                // Copy the data into the new ColdkeyNodeRegistration storage
                ColdkeyNodeRegistration::<T>::insert(node_id.clone(), Some(info));
                count += 1;
            }
        }

        log::info!("Migrated {} records from NodeRegistration to ColdkeyNodeRegistration", count);

        // Return an estimated weight cost
        T::DbWeight::get().reads_writes(count, count)
    }
}
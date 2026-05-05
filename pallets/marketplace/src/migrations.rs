use super::*;
use core::marker::PhantomData;
use frame_support::{
    pallet_prelude::*,
    traits::{OnRuntimeUpgrade, StorageVersion},
};

pub struct Migrate<T>(PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for Migrate<T> {
    fn on_runtime_upgrade() -> Weight {
        let mut weight = Weight::zero();

        // If StorageVersion is unset, initialize it to 0 first.
        let current_version = StorageVersion::get::<Pallet<T>>();
        if current_version == StorageVersion::default() {
            StorageVersion::new(0).put::<Pallet<T>>();
        }

        if current_version == 0 {
            weight = Self::backfill_paid_per_month();
            StorageVersion::new(1).put::<Pallet<T>>();
        } else {
            log::info!("Skipping marketplace migration, already migrated.");
        }

        weight
    }
}

impl<T: Config> Migrate<T> {
    fn backfill_paid_per_month() -> Weight {
        let mut touched: u64 = 0;

        for (who, mut subs) in UserAllSubscriptionPlans::<T>::iter() {
            let mut mutated = false;

            for sub in subs.iter_mut() {
                // Best-effort: historical referral discount cannot be reconstructed.
                // If the field is missing (older storage) it decodes as `package.price`.
                // Still, guard against zero/uninitialized values.
                if sub.paid_per_month == 0 {
                    sub.paid_per_month = sub.package.price;
                    mutated = true;
                }
            }

            if mutated {
                touched = touched.saturating_add(1);
                UserAllSubscriptionPlans::<T>::insert(&who, subs);
            }
        }

        // One read per user entry, plus one write per mutated entry.
        T::DbWeight::get().reads_writes(touched.saturating_add(1), touched)
    }
}


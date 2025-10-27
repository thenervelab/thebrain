// Migration from V0 to V1
//
// This migration removes all old storage items from the previous bridge implementation
// and transitions to the new guardian-based voting system.
//
// Old storage items removed:
// - ProcessedEvents: Map of processed event proofs
// - PendingMints: Map of pending mint requests with operator confirmations
// - PendingBurns: Map of pending burn requests with operator confirmations
// - Authorities: List of authorized operator accounts
// - MinRequiredSignatures: Minimum signatures required for operations
//
// New storage items (initialized empty, configured via governance after migration):
// - ProcessedDeposits: Map of processed deposit IDs
// - PendingDeposits: Map of pending deposit proposals with guardian votes
// - PendingUnlocks: Map of pending unlock requests with guardian votes
// - Guardians: List of authorized guardian accounts
// - ApproveThreshold: Minimum approvals needed
// - DenyThreshold: Minimum denials needed
// - And other configuration storage items

extern crate alloc;

use crate::{Config, Pallet};
use frame_support::{
	pallet_prelude::*,
	storage_alias,
	traits::{Get, UncheckedOnRuntimeUpgrade},
	weights::Weight,
};
use sp_std::{collections::btree_set::BTreeSet, vec::Vec};

#[cfg(feature = "try-runtime")]
use frame_support::traits::StorageVersion;

const LOG_TARGET: &str = "runtime::alpha-bridge::migration::v1";

// Storage aliases for old V0 storage items that need to be removed
#[storage_alias]
type ProcessedEvents<T: Config> = StorageMap<
	Pallet<T>,
	Blake2_128Concat,
	Vec<u8>, // proof (block_hash, event_index)
	bool,
	ValueQuery,
>;

#[storage_alias]
type PendingMints<T: Config> = StorageMap<
	Pallet<T>,
	Blake2_128Concat,
	Vec<u8>, // proof
	(
		<T as frame_system::Config>::AccountId,
		u128,
		BTreeSet<<T as frame_system::Config>::AccountId>,
	), // (user, amount, confirmations)
	OptionQuery,
>;

#[storage_alias]
type PendingBurns<T: Config> = StorageMap<
	Pallet<T>,
	Blake2_128Concat,
	u128, // nonce
	(
		<T as frame_system::Config>::AccountId,           // user
		u128,                                             // amount
		<T as frame_system::Config>::AccountId,           // bittensor_coldkey
		BTreeSet<<T as frame_system::Config>::AccountId>, // confirmations
		Option<(Vec<u8>, Vec<u8>)>,                       // (bittensor_block_hash, extrinsic_id)
	),
	OptionQuery,
>;

#[storage_alias]
type Authorities<T: Config> =
	StorageValue<Pallet<T>, Vec<<T as frame_system::Config>::AccountId>, ValueQuery>;

#[storage_alias]
type MinRequiredSignatures<T: Config> = StorageValue<Pallet<T>, u32, ValueQuery>;

/// Unchecked migration implementation that removes all V0 storage
pub struct UncheckedMigrationToV1<T>(sp_std::marker::PhantomData<T>);

impl<T: Config> UncheckedOnRuntimeUpgrade for UncheckedMigrationToV1<T> {
	#[cfg(feature = "try-runtime")]
	fn pre_upgrade() -> Result<Vec<u8>, sp_runtime::TryRuntimeError> {
		use alloc::vec::Vec;
		// Count existing storage items for verification
		let processed_events_count = ProcessedEvents::<T>::iter().count();
		let pending_mints_count = PendingMints::<T>::iter().count();
		let pending_burns_count = PendingBurns::<T>::iter().count();
		let authorities_exists = Authorities::<T>::exists();
		let min_sigs_exists = MinRequiredSignatures::<T>::exists();

		log::info!(
			target: LOG_TARGET,
			"Pre-upgrade: Found {} processed events, {} pending mints, {} pending burns, authorities exists: {}, min sigs exists: {}",
			processed_events_count,
			pending_mints_count,
			pending_burns_count,
			authorities_exists,
			min_sigs_exists
		);

		// Return counts for post-upgrade verification
		Ok(Vec::new())
	}

	fn on_runtime_upgrade() -> Weight {
		log::info!(target: LOG_TARGET, "Running migration to V1");

		// Clear all old storage items
		let _ = ProcessedEvents::<T>::clear(u32::MAX, None);
		let _ = PendingMints::<T>::clear(u32::MAX, None);
		let _ = PendingBurns::<T>::clear(u32::MAX, None);
		Authorities::<T>::kill();
		MinRequiredSignatures::<T>::kill();

		log::info!(target: LOG_TARGET, "Migration to V1 completed - all old storage cleared");

		T::DbWeight::get().writes(5)
	}

	#[cfg(feature = "try-runtime")]
	fn post_upgrade(_state: Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
		// Verify all old storage is cleared
		let processed_events_count = ProcessedEvents::<T>::iter().count();
		let pending_mints_count = PendingMints::<T>::iter().count();
		let pending_burns_count = PendingBurns::<T>::iter().count();
		let authorities_exists = Authorities::<T>::exists();
		let min_sigs_exists = MinRequiredSignatures::<T>::exists();

		// All should be empty/non-existent
		if processed_events_count != 0 {
			return Err("ProcessedEvents not cleared".into());
		}
		if pending_mints_count != 0 {
			return Err("PendingMints not cleared".into());
		}
		if pending_burns_count != 0 {
			return Err("PendingBurns not cleared".into());
		}
		if authorities_exists {
			return Err("Authorities not cleared".into());
		}
		if min_sigs_exists {
			return Err("MinRequiredSignatures not cleared".into());
		}

		// Verify storage version was updated
		let current_version = StorageVersion::get::<Pallet<T>>();
		if current_version != StorageVersion::new(1) {
			return Err("Storage version not updated to 1".into());
		}

		log::info!(target: LOG_TARGET, "Post-upgrade: All old storage successfully cleared, version updated to 1");

		Ok(())
	}
}

/// Versioned migration from V0 to V1
/// This wraps the unchecked migration with automatic version management
pub type MigrationToV1<T> = frame_support::migrations::VersionedMigration<
	0, // From version
	1, // To version
	UncheckedMigrationToV1<T>,
	Pallet<T>,
	<T as frame_system::Config>::DbWeight,
>;

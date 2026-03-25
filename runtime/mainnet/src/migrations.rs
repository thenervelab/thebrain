use super::*;
use frame_support::traits::OnRuntimeUpgrade;
use parity_scale_codec::{Decode, Encode, MaxEncodedLen};
use sp_runtime::{BoundToRuntimeAppPublic, RuntimeAppPublic, RuntimeDebug};

/// Old session keys structure.
///
/// This struct represents the session keys used in the previous version of the runtime.
/// It includes keys for Grandpa, Babe, ImOnline, and Role.
#[derive(PartialEq, Eq, Clone, RuntimeDebug, Encode, Decode, MaxEncodedLen)]
pub struct OldSessionKeys {
	/// Grandpa key.
	pub grandpa: <Grandpa as BoundToRuntimeAppPublic>::Public,
	/// Babe key.
	pub babe: <Babe as BoundToRuntimeAppPublic>::Public,
	/// ImOnline key.
	pub im_online: pallet_im_online::sr25519::AuthorityId,
	/// Role key.
	pub role: pallet_im_online::sr25519::AuthorityId,
}

impl OpaqueKeys for OldSessionKeys {
	type KeyTypeIdProviders = ();

	/// Return the key IDs of the old session keys.
	fn key_ids() -> &'static [KeyTypeId] {
		&[
			<<Grandpa as BoundToRuntimeAppPublic>::Public>::ID,
			<<Babe as BoundToRuntimeAppPublic>::Public>::ID,
			sp_core::crypto::key_types::IM_ONLINE,
			hippius_crypto_primitives::ROLE_KEY_TYPE,
		]
	}

	/// Get the raw byte representation of a key based on its KeyTypeId.
	fn get_raw(&self, i: KeyTypeId) -> &[u8] {
		match i {
			<<Grandpa as BoundToRuntimeAppPublic>::Public>::ID => self.grandpa.as_ref(),
			<<Babe as BoundToRuntimeAppPublic>::Public>::ID => self.babe.as_ref(),
			sp_core::crypto::key_types::IM_ONLINE => self.im_online.as_ref(),
			hippius_crypto_primitives::ROLE_KEY_TYPE => self.role.as_ref(),
			_ => &[],
		}
	}
}

/// Transform function to convert old session keys to the new session keys structure.
///
/// This function is used during the runtime upgrade to transform the old session keys into
/// the new session keys structure.
fn transform_session_keys(_val: AccountId, old: OldSessionKeys) -> SessionKeys {
	SessionKeys { grandpa: old.grandpa, babe: old.babe, im_online: old.im_online }
}

/// Runtime upgrade for migrating session keys.
///
/// This struct implements the `OnRuntimeUpgrade` trait and performs the migration of session keys
/// from the old structure (`OldSessionKeys`) to the new structure (`SessionKeys`).
pub struct MigrateSessionKeys<T>(sp_std::marker::PhantomData<T>);

impl<T: pallet_session::Config> OnRuntimeUpgrade for MigrateSessionKeys<T> {
	/// Perform the runtime upgrade.
	///
	/// This function upgrades the session keys by transforming them from the old structure to the
	/// new structure using the `transform_session_keys` function. It reads and writes to the
	/// database as needed.
	fn on_runtime_upgrade() -> Weight {
		Session::upgrade_keys::<OldSessionKeys, _>(transform_session_keys);
		T::DbWeight::get().reads_writes(10, 10)
	}
}

/// Migration to remove pallet_ip, pallet_container_registry, and the LinkedNodes
/// storage item from pallet_registration.
///
/// The pallet prefix must match the name used in construct_runtime! exactly:
///   - `PalletIp`          (for `PalletIp: pallet_ip = 74`)
///   - `ContainerRegistry` (for `ContainerRegistry: pallet_container_registry = 69`)
///   - `Registration`      (for `Registration: pallet_registration = 53`)
///     → only the `LinkedNodes` storage item prefix is cleared.
///
/// Run this in the same upgrade where you remove the two pallets and the storage item.
pub struct RemoveIpAndContainerRegistryPallets<T>(sp_std::marker::PhantomData<T>);

impl<T: frame_system::Config> OnRuntimeUpgrade for RemoveIpAndContainerRegistryPallets<T> {
	fn on_runtime_upgrade() -> Weight {
		let mut weight = T::DbWeight::get().reads(0);

		// ── Clear pallet_ip storage (pallet name in construct_runtime: "PalletIp") ──
		// Clears: AvailableHypervisorIps, AvailableClientIps, AvailableStorageMinerIps,
		//         VmAvailableIps, AssignedVmIps, AssignedClientIps,
		//         IpToRole (map), RoleToIp (map), IpReleaseRequests
		let removed_ip =
			frame_support::storage::unhashed::clear_prefix(b"PalletIp", None, None);
		log::info!(
			target: "runtime::migration",
			"RemoveIpAndContainerRegistryPallets: cleared {} keys from PalletIp",
			removed_ip.backend,
		);
		weight = weight.saturating_add(T::DbWeight::get().writes(removed_ip.backend as u64));

		// ── Clear pallet_container_registry storage (pallet name: "ContainerRegistry") ──
		// Clears: NextSpaceId, Spaces, ManifestDigests, DigestInfoStorage, ImageDigestToCid
		let removed_cr =
			frame_support::storage::unhashed::clear_prefix(b"ContainerRegistry", None, None);
		log::info!(
			target: "runtime::migration",
			"RemoveIpAndContainerRegistryPallets: cleared {} keys from ContainerRegistry",
			removed_cr.backend,
		);
		weight = weight.saturating_add(T::DbWeight::get().writes(removed_cr.backend as u64));

		// ── Clear LinkedNodes from pallet_registration (pallet name: "Registration") ──
		// Storage item: LinkedNodes<T> = StorageMap<Blake2_128Concat, Vec<u8>, Vec<Vec<u8>>>
		// The raw key prefix is: twox_128("Registration") ++ twox_128("LinkedNodes")
		// clear_prefix with the full two-part prefix targets only this specific map.
		let reg_linked_nodes_prefix = {
			use frame_support::StorageHasher;
			let mut p = frame_support::Twox128::hash(b"Registration").to_vec();
			p.extend_from_slice(&frame_support::Twox128::hash(b"LinkedNodes"));
			p
		};
		let removed_ln =
			frame_support::storage::unhashed::clear_prefix(&reg_linked_nodes_prefix, None, None);
		log::info!(
			target: "runtime::migration",
			"RemoveIpAndContainerRegistryPallets: cleared {} keys from Registration::LinkedNodes",
			removed_ln.backend,
		);
		weight = weight.saturating_add(T::DbWeight::get().writes(removed_ln.backend as u64));

		weight
	}

	#[cfg(feature = "try-runtime")]
	fn pre_upgrade() -> Result<sp_std::vec::Vec<u8>, sp_runtime::TryRuntimeError> {
		let has_sub_account = sp_io::storage::next_key(b"SubAccount")
			.filter(|k| k.starts_with(b"SubAccount"))
			.is_some();
		let cr_has = sp_io::storage::next_key(b"ContainerRegistry")
			.filter(|k| k.starts_with(b"ContainerRegistry"))
			.is_some();
		let notifications_has = sp_io::storage::next_key(b"Notifications")
			.filter(|k| k.starts_with(b"Notifications"))
			.is_some();
		log::info!(
			target: "runtime::migration",
			"pre_upgrade: SubAccount has_keys={}, ContainerRegistry has_keys={}, Notifications has_keys={}",
			has_sub_account, cr_has, notifications_has
		);
		Ok(sp_std::vec::Vec::new())
	}

	#[cfg(feature = "try-runtime")]
	fn post_upgrade(_state: sp_std::vec::Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
		let has_sub_account_still = sp_io::storage::next_key(b"SubAccount")
			.map(|k| k.starts_with(b"SubAccount"))
			.unwrap_or(false);
		let cr_still = sp_io::storage::next_key(b"ContainerRegistry")
			.map(|k| k.starts_with(b"ContainerRegistry"))
			.unwrap_or(false);
		let notifications_still = sp_io::storage::next_key(b"Notifications")
			.map(|k| k.starts_with(b"Notifications"))
			.unwrap_or(false);

		frame_support::ensure!(!has_sub_account_still, "post_upgrade: SubAccount storage was NOT fully cleared!");
		frame_support::ensure!(
			!cr_still,
			"post_upgrade: ContainerRegistry storage was NOT fully cleared!"
		);
		log::info!(
			target: "runtime::migration",
			"post_upgrade: SubAccount and ContainerRegistry successfully cleared."
		);
		Ok(())
	}
}

use super::*;
use frame_support::{
    traits::{Get, OnRuntimeUpgrade, StorageVersion, GetStorageVersion},
    weights::Weight,
};
use frame_system::pallet_prelude::BlockNumberFor;
use sp_std::vec::Vec;

/// Old NodeInfo structure before adding code signature fields
#[derive(codec::Encode, codec::Decode, Clone, Eq, PartialEq, scale_info::TypeInfo)]
pub struct OldNodeInfo<BlockNumber, AccountId> {
    pub node_id: Vec<u8>,
    pub node_type: NodeType,
    pub ipfs_node_id: Option<Vec<u8>>,
    pub status: Status,
    pub registered_at: BlockNumber,
    pub owner: AccountId,
    pub is_verified: bool,
}

/// Storage version 0: Before code signature verification
pub const STORAGE_VERSION_0: StorageVersion = StorageVersion::new(0);
/// Storage version 1: With code signature verification fields
pub const STORAGE_VERSION_1: StorageVersion = StorageVersion::new(1);

pub mod v1 {
    use super::*;

    /// Migration to add code signature verification fields to existing nodes
    pub struct MigrateToV1<T>(sp_std::marker::PhantomData<T>);

    impl<T: Config> OnRuntimeUpgrade for MigrateToV1<T> {
        fn on_runtime_upgrade() -> Weight {
            let mut weight = T::DbWeight::get().reads(1);
            
            // Check current storage version
            let onchain_version = Pallet::<T>::on_chain_storage_version();
            
            if onchain_version < STORAGE_VERSION_1 {
                log::info!("🔄 Starting migration to v1 - Adding code signature fields");
                
                let mut migrated_coldkey_nodes = 0u64;
                let mut migrated_hotkey_nodes = 0u64;

                // Migrate ColdkeyNodeRegistration
                log::info!("🔄 Migrating ColdkeyNodeRegistration...");
                ColdkeyNodeRegistration::<T>::translate::<
                    Option<OldNodeInfo<BlockNumberFor<T>, T::AccountId>>,
                    _
                >(|node_id, old_node_info_opt| {
                    weight = weight.saturating_add(T::DbWeight::get().reads_writes(1, 1));
                    
                    if let Some(old_info) = old_node_info_opt {
                        migrated_coldkey_nodes += 1;
                        
                        // Create new NodeInfo with code signature fields set to defaults
                        Some(Some(NodeInfo {
                            node_id: old_info.node_id,
                            node_type: old_info.node_type,
                            ipfs_node_id: old_info.ipfs_node_id,
                            status: old_info.status,
                            registered_at: old_info.registered_at,
                            owner: old_info.owner,
                            is_verified: old_info.is_verified,
                            code_signature_verified: false, // Legacy nodes = not code verified
                            code_public_key: None,          // No public key on record
                        }))
                    } else {
                        None
                    }
                });

                log::info!("✅ Migrated {} coldkey nodes", migrated_coldkey_nodes);

                // Migrate NodeRegistration (hotkey nodes)
                log::info!("🔄 Migrating NodeRegistration...");
                NodeRegistration::<T>::translate::<
                    Option<OldNodeInfo<BlockNumberFor<T>, T::AccountId>>,
                    _
                >(|node_id, old_node_info_opt| {
                    weight = weight.saturating_add(T::DbWeight::get().reads_writes(1, 1));
                    
                    if let Some(old_info) = old_node_info_opt {
                        migrated_hotkey_nodes += 1;
                        
                        // Create new NodeInfo with code signature fields set to defaults
                        Some(Some(NodeInfo {
                            node_id: old_info.node_id,
                            node_type: old_info.node_type,
                            ipfs_node_id: old_info.ipfs_node_id,
                            status: old_info.status,
                            registered_at: old_info.registered_at,
                            owner: old_info.owner,
                            is_verified: old_info.is_verified,
                            code_signature_verified: false, // Legacy nodes = not code verified
                            code_public_key: None,          // No public key on record
                        }))
                    } else {
                        None
                    }
                });

                log::info!("✅ Migrated {} hotkey nodes", migrated_hotkey_nodes);

                // Update storage version
                STORAGE_VERSION_1.put::<Pallet<T>>();
                
                log::info!(
                    "✅ Migration to v1 complete! Total nodes migrated: {} (coldkey: {}, hotkey: {})",
                    migrated_coldkey_nodes + migrated_hotkey_nodes,
                    migrated_coldkey_nodes,
                    migrated_hotkey_nodes
                );
            } else {
                log::info!("⏭️  Skipping migration to v1 (already on version {:?})", onchain_version);
            }

            weight
        }

        #[cfg(feature = "try-runtime")]
        fn pre_upgrade() -> Result<Vec<u8>, &'static str> {
            let onchain_version = Pallet::<T>::on_chain_storage_version();
            
            if onchain_version < 1 {
                // Count nodes before migration
                let coldkey_count = ColdkeyNodeRegistration::<T>::iter().count();
                let hotkey_count = NodeRegistration::<T>::iter().count();
                
                log::info!("Pre-upgrade: Found {} coldkey nodes and {} hotkey nodes", 
                    coldkey_count, hotkey_count);
                
                // Encode counts to return
                Ok((coldkey_count as u32, hotkey_count as u32).encode())
            } else {
                Ok(Vec::new())
            }
        }

        #[cfg(feature = "try-runtime")]
        fn post_upgrade(state: Vec<u8>) -> Result<(), &'static str> {
            let onchain_version = Pallet::<T>::on_chain_storage_version();
            
            if !state.is_empty() {
                // Decode the pre-migration counts
                let (pre_coldkey_count, pre_hotkey_count): (u32, u32) = 
                    codec::Decode::decode(&mut &state[..])
                        .map_err(|_| "Failed to decode pre-upgrade state")?;
                
                // Count nodes after migration
                let post_coldkey_count = ColdkeyNodeRegistration::<T>::iter().count() as u32;
                let post_hotkey_count = NodeRegistration::<T>::iter().count() as u32;
                
                // Verify counts match
                if pre_coldkey_count != post_coldkey_count {
                    return Err("Coldkey node count mismatch after migration!");
                }
                
                if pre_hotkey_count != post_hotkey_count {
                    return Err("Hotkey node count mismatch after migration!");
                }
                
                // Verify all nodes have the new fields
                for (_, node_info_opt) in ColdkeyNodeRegistration::<T>::iter() {
                    if let Some(node_info) = node_info_opt {
                        // All migrated nodes should have code_signature_verified = false
                        if node_info.code_signature_verified {
                            return Err("Found coldkey node with code_signature_verified = true after migration!");
                        }
                    }
                }
                
                for (_, node_info_opt) in NodeRegistration::<T>::iter() {
                    if let Some(node_info) = node_info_opt {
                        // All migrated nodes should have code_signature_verified = false
                        if node_info.code_signature_verified {
                            return Err("Found hotkey node with code_signature_verified = true after migration!");
                        }
                    }
                }
                
                log::info!("✅ Post-upgrade checks passed!");
                log::info!("   - Coldkey nodes: {} (unchanged)", post_coldkey_count);
                log::info!("   - Hotkey nodes: {} (unchanged)", post_hotkey_count);
                log::info!("   - All nodes have code_signature_verified = false");
            }
            
            // Verify storage version was updated
            if onchain_version < STORAGE_VERSION_1 {
                return Err("Storage version not updated correctly!");
            }
            
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_node_info_can_decode_to_new() {
        use codec::{Encode, Decode};
        
        // Create an old NodeInfo
        let old_info = OldNodeInfo {
            node_id: vec![1, 2, 3],
            node_type: NodeType::Validator,
            ipfs_node_id: Some(vec![4, 5, 6]),
            status: Status::Online,
            registered_at: 100u32,
            owner: 123u64,
            is_verified: true,
        };
        
        // Encode it
        let encoded = old_info.encode();
        
        // Try to decode as new NodeInfo - this should fail because of missing fields
        // This is expected and why we need migration
        let decode_result = NodeInfo::<u32, u64>::decode(&mut &encoded[..]);
        
        // This should fail without migration
        assert!(decode_result.is_err(), "Old format should not decode to new format directly");
    }
}

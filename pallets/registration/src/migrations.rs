use super::*;
use frame_support::{
    pallet_prelude::*,
    traits::OnRuntimeUpgrade,
};
use sp_std::vec::Vec;
use frame_system::pallet_prelude::BlockNumberFor;
use codec::Encode;

// These storage aliases should now point to the *new* NodeInfo type
// because the translate function is migrating the storage to this new type.
#[frame_support::storage_alias]
pub type NodeRegistration<T: Config> = StorageMap<
    crate::pallet::Pallet<T>,
    frame_support::Blake2_128Concat,
    Vec<u8>,
    Option<types::NodeInfo<BlockNumberFor<T>, <T as frame_system::Config>::AccountId>>, // <-- Changed to types::NodeInfo
>;

#[frame_support::storage_alias]
pub type ColdkeyNodeRegistration<T: Config> = StorageMap<
    crate::pallet::Pallet<T>,
    frame_support::Blake2_128Concat,
    Vec<u8>,
    Option<types::NodeInfo<BlockNumberFor<T>, <T as frame_system::Config>::AccountId>>, // <-- Changed to types::NodeInfo
>;

#[derive(Encode, Decode, Clone, Eq, PartialEq, Debug, TypeInfo)]
pub struct OldNodeInfo<BlockNumber, AccountId> {
    pub node_id: Vec<u8>,
    pub node_type: NodeType,
    pub ipfs_node_id: Option<Vec<u8>>,
    pub status: Status,
    pub registered_at: BlockNumber,
    pub owner: AccountId,
    // No is_verified field here - this matches the old structure
}

pub struct AddIsVerifiedField<T>(sp_std::marker::PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for AddIsVerifiedField<T> {
    fn on_runtime_upgrade() -> frame_support::weights::Weight {
        log::info!("Migrating NodeInfo to add is_verified field...");
        
        // Migrate NodeRegistration
        let mut node_reg_count = 0u64;
        let _ = NodeRegistration::<T>::translate(|node_id: Vec<u8>, old_node_opt: Option<OldNodeInfo<_, _>>| {
            if let Some(old_node) = old_node_opt {
                node_reg_count += 1;
                Some(Some(types::NodeInfo { // Return Option<NodeInfo>
                    node_id: old_node.node_id,
                    node_type: old_node.node_type,
                    ipfs_node_id: old_node.ipfs_node_id,
                    status: old_node.status,
                    registered_at: old_node.registered_at,
                    owner: old_node.owner,
                    is_verified: false, // Set default value for existing nodes
                }))
            } else {
                // Handle case where value is None
                Some(None)
            }
        });

        // Migrate ColdkeyNodeRegistration
        let mut coldkey_reg_count = 0u64;
        let _ = ColdkeyNodeRegistration::<T>::translate(|node_id: Vec<u8>, old_node_opt: Option<OldNodeInfo<_, _>>| {
            if let Some(old_node) = old_node_opt {
                coldkey_reg_count += 1;
                Some(Some(types::NodeInfo { // Return Option<NodeInfo>
                    node_id: old_node.node_id,
                    node_type: old_node.node_type,
                    ipfs_node_id: old_node.ipfs_node_id,
                    status: old_node.status,
                    registered_at: old_node.registered_at,
                    owner: old_node.owner,
                    is_verified: false, // Set default value for existing nodes
                }))
            } else {
                // Handle case where value is None
                Some(None)
            }
        });

        log::info!(
            "Migration complete: Updated {} NodeRegistration and {} ColdkeyNodeRegistration records",
            node_reg_count,
            coldkey_reg_count
        );

        // Return weight
        T::DbWeight::get().reads_writes(
            node_reg_count.saturating_add(coldkey_reg_count),
            node_reg_count.saturating_add(coldkey_reg_count),
        )
    }

    #[cfg(feature = "try-runtime")]
    fn pre_upgrade() -> Result<Vec<u8>, &'static str> {
        let node_reg_count = NodeRegistration::<T>::iter().count() as u64;
        let coldkey_reg_count = ColdkeyNodeRegistration::<T>::iter().count() as u64;
        Ok((node_reg_count, coldkey_reg_count).encode())
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade(state: Vec<u8>) -> Result<(), &'static str> {
        let (old_node_count, old_coldkey_count): (u64, u64) = 
            Decode::decode(&mut &state[..]).map_err(|_| "Failed to decode state")?;
        
        let new_node_count = NodeRegistration::<T>::iter().count() as u64;
        let new_coldkey_count = ColdkeyNodeRegistration::<T>::iter().count() as u64;

        assert_eq!(old_node_count, new_node_count, "Node count changed during migration");
        assert_eq!(old_coldkey_count, new_coldkey_count, "Coldkey count changed during migration");

        Ok(())
    }
}
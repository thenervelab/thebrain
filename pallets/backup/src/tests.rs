use crate::{mock::*, Error, Event};
use frame_support::{assert_noop, assert_ok};
use sp_runtime::AccountId32;

// Helper function to create a test account
fn create_account(seed: u8) -> AccountId32 {
    let mut account_bytes = [0u8; 32];
    account_bytes[31] = seed;
    AccountId32::from(account_bytes)
}

#[test]
fn add_snapshot_works() {
    new_test_ext().execute_with(|| {
        let owner = create_account(1);
        let node_id = b"test-node-1".to_vec();
        let snapshot_cid = b"QmTestCID".to_vec();
        let description = Some(b"Test snapshot".to_vec());
        let request_id = 1;

        // Add a snapshot (unsigned transaction simulation)
        assert_ok!(Backup::add_snapshot(
            RuntimeOrigin::none(),
            node_id.clone(),
            snapshot_cid.clone(),
            description.clone(),
            request_id,
            owner.clone()
        ));

        // Check backup metadata
        let backup = crate::Backups::<Test>::get(&node_id, &owner).expect("Backup should exist");
        assert_eq!(backup.snapshots.len(), 1);
        assert_eq!(backup.last_snapshot, Some(snapshot_cid.clone()));
        
        let snapshot = &backup.snapshots[0];
        assert_eq!(snapshot.cid, snapshot_cid);
        assert_eq!(snapshot.description, description);
        assert_eq!(snapshot.request_id, request_id);

        // Check event
        System::assert_last_event(Event::SnapshotAdded { node_id, snapshot_cid }.into());
    });
}

#[test]
fn set_backup_offchain_worker_frequency_works() {
    new_test_ext().execute_with(|| {
        let owner = create_account(1);
        let new_frequency = 48 * 60 * 60; // 48 hours in blocks

        // Set backup offchain worker frequency
        assert_ok!(Backup::set_backup_offchain_worker_frequency(
            RuntimeOrigin::signed(owner.clone()),
            new_frequency
        ));

        // Check the stored frequency
        let stored_frequency = crate::BackupOffchainWorkerFrequency::<Test>::get(&owner);
        assert_eq!(stored_frequency, new_frequency);

        // Check event
        System::assert_last_event(Event::BackupOffchainWorkerFrequencyUpdated { new_frequency }.into());
    });
}

#[test]
fn delete_backup_metadata_works() {
    new_test_ext().execute_with(|| {
        let owner = create_account(1);
        let node_id = b"test-node-1".to_vec();
        let snapshot_cid1 = b"QmTestCID1".to_vec();
        let snapshot_cid2 = b"QmTestCID2".to_vec();
        let request_id1 = 1;
        let request_id2 = 2;

        // First, add two snapshots
        assert_ok!(Backup::add_snapshot(
            RuntimeOrigin::none(),
            node_id.clone(),
            snapshot_cid1.clone(),
            None,
            request_id1,
            owner.clone()
        ));

        assert_ok!(Backup::add_snapshot(
            RuntimeOrigin::none(),
            node_id.clone(),
            snapshot_cid2.clone(),
            None,
            request_id2,
            owner.clone()
        ));

        // Delete metadata for the first request
        assert_ok!(Backup::delete_backup_metadata(
            node_id.clone(),
            owner.clone(),
            request_id1
        ));

        // Check backup metadata
        let backup = crate::Backups::<Test>::get(&node_id, &owner).expect("Backup should exist");
        assert_eq!(backup.snapshots.len(), 1);
        assert_eq!(backup.last_snapshot, Some(snapshot_cid2.clone()));
        assert_eq!(backup.snapshots[0].cid, snapshot_cid2);
        assert_eq!(backup.snapshots[0].request_id, request_id2);
    });
}

#[test]
fn delete_backup_metadata_removes_entire_metadata() {
    new_test_ext().execute_with(|| {
        let owner = create_account(1);
        let node_id = b"test-node-1".to_vec();
        let snapshot_cid = b"QmTestCID".to_vec();
        let request_id = 1;

        // First, add a snapshot
        assert_ok!(Backup::add_snapshot(
            RuntimeOrigin::none(),
            node_id.clone(),
            snapshot_cid.clone(),
            None,
            request_id,
            owner.clone()
        ));

        // Delete metadata for the request
        assert_ok!(Backup::delete_backup_metadata(
            node_id.clone(),
            owner.clone(),
            request_id
        ));

        // Check backup metadata is removed
        assert!(crate::Backups::<Test>::get(&node_id, &owner).is_none());
    });
}

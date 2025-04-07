use crate::{mock::*, Error, Event};
use frame_support::{assert_noop, assert_ok};
use sp_runtime::AccountId32;

// Helper function to set initial IPs in storage
fn set_initial_ips() {
	let initial_ips =
		vec![b"192.168.1.1".to_vec(), b"192.168.1.2".to_vec(), b"192.168.1.3".to_vec()];
	Compute::initialize_available_ips(initial_ips);
}

#[test]
fn add_available_ip_works() {
	new_test_ext().execute_with(|| {
		let new_ip = b"10.0.0.1".to_vec();

		// Add a new IP address as root
		assert_ok!(Compute::add_available_ip(RuntimeOrigin::root(), new_ip.clone()));

		// Check that the IP was added to available IPs
		let available_ips = Compute::available_ips();
		assert!(available_ips.contains(&new_ip));

		// Check the correct event was emitted
		System::assert_last_event(Event::IpAdded { ip: new_ip }.into());
	});
}

#[test]
fn add_available_ip_fails_with_duplicate() {
	new_test_ext().execute_with(|| {
		// Set initial IPs
		set_initial_ips();

		let duplicate_ip = b"192.168.1.1".to_vec();

		// Attempt to add a duplicate IP
		assert_noop!(
			Compute::add_available_ip(RuntimeOrigin::root(), duplicate_ip),
			Error::<Test>::IpAlreadyExists
		);
	});
}

#[test]
fn assign_ip_works() {
	new_test_ext().execute_with(|| {
		// Set initial IPs
		set_initial_ips();

		let mut account_bytes = [0u8; 32];
		account_bytes[31] = 1; // Set the last byte to 1
		let account = AccountId32::from(account_bytes);

		let vm_uuid = b"test-vm-1".to_vec();
		let initial_available_ips = Compute::available_ips();

		// Assign an IP to a VM
		assert_ok!(Compute::assign_ip(RuntimeOrigin::signed(account.clone()), vm_uuid.clone()));

		// Check that the IP was removed from available IPs
		let updated_available_ips = Compute::available_ips();
		assert_eq!(updated_available_ips.len(), initial_available_ips.len() - 1);

		// Check that the IP is now assigned to the VM
		let assigned_ip = Compute::assigned_ips(&vm_uuid);
		assert!(assigned_ip.is_some());

		// Check the correct event was emitted
		System::assert_last_event(
			Event::IpAssigned { vm_uuid: vm_uuid.clone(), ip: assigned_ip.unwrap() }.into(),
		);
	});
}

#[test]
fn assign_ip_fails_when_vm_already_has_ip() {
	new_test_ext().execute_with(|| {
		// Set initial IPs
		set_initial_ips();

		let mut account_bytes = [0u8; 32];
		account_bytes[31] = 1; // Set the last byte to 1
		let account = AccountId32::from(account_bytes);

		let vm_uuid = b"test-vm-1".to_vec();

		// First assignment should work
		assert_ok!(Compute::assign_ip(RuntimeOrigin::signed(account.clone()), vm_uuid.clone()));

		// Second assignment should fail
		assert_noop!(
			Compute::assign_ip(RuntimeOrigin::signed(account), vm_uuid),
			Error::<Test>::VmAlreadyHasIp
		);
	});
}

#[test]
fn release_ip_works() {
	new_test_ext().execute_with(|| {
		// Set initial IPs
		set_initial_ips();

		let mut account_bytes = [0u8; 32];
		account_bytes[31] = 1; // Set the last byte to 1
		let account = AccountId32::from(account_bytes);

		let vm_uuid = b"test-vm-1".to_vec();
		let initial_available_ips = Compute::available_ips();

		// First assign an IP
		assert_ok!(Compute::assign_ip(RuntimeOrigin::signed(account.clone()), vm_uuid.clone()));

		// Then release the IP
		assert_ok!(Compute::release_ip(RuntimeOrigin::signed(account.clone()), vm_uuid.clone()));

		// Check that the IP is back in available IPs
		let updated_available_ips = Compute::available_ips();
		assert_eq!(updated_available_ips.len(), initial_available_ips.len());

		// Check the correct event was emitted
		System::assert_last_event(
			Event::IpReturned {
				vm_uuid,
				ip: updated_available_ips[updated_available_ips.len() - 1].clone(),
			}
			.into(),
		);
	});
}

#[test]
fn release_ip_fails_when_vm_has_no_ip() {
	new_test_ext().execute_with(|| {
		let mut account_bytes = [0u8; 32];
		account_bytes[31] = 1; // Set the last byte to 1
		let account = AccountId32::from(account_bytes);

		let vm_uuid = b"test-vm-1".to_vec();

		// Attempt to release IP for a VM without an IP
		assert_noop!(
			Compute::release_ip(RuntimeOrigin::signed(account), vm_uuid),
			Error::<Test>::VmHasNoIp
		);
	});
}

#[test]
fn no_available_ip_fails_assignment() {
	new_test_ext().execute_with(|| {
		// Create a valid 32-byte AccountId32
		let mut account_bytes = [0u8; 32];
		account_bytes[31] = 1; // Set the last byte to 1
		let account = AccountId32::from(account_bytes);

		// Consume all available IPs
		let initial_ips = Compute::available_ips();
		for (i, _ip) in initial_ips.into_iter().enumerate() {
			let vm_uuid = format!("test-vm-{}", i).into_bytes();
			assert_ok!(Compute::assign_ip(RuntimeOrigin::signed(account.clone()), vm_uuid));
		}

		// Attempt to assign an IP when none are available
		let final_vm_uuid = b"final-vm".to_vec();
		assert_noop!(
			Compute::assign_ip(RuntimeOrigin::signed(account), final_vm_uuid),
			Error::<Test>::NoAvailableIp
		);
	});
}

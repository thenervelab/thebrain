//! Benchmarking setup for pallet-alpha-bridge
#![cfg(feature = "runtime-benchmarks")]

use super::*;
use crate::Pallet as AlphaBridge;
use frame_benchmarking::v2::*;
use frame_support::{
	assert_ok,
	traits::{fungible::Mutate, Currency, Get},
};
use frame_system::RawOrigin;
use sp_core::{Hasher, H256};
use sp_runtime::traits::{BlakeTwo256, Saturating};
use sp_std::vec;

const SEED: u32 = 0;

/// Create a funded account with the given index and balance
fn create_funded_user<T: Config>(index: u32, balance: u128) -> T::AccountId {
	let user: T::AccountId = account("user", index, SEED);
	let balance_value: BalanceOf<T> =
		balance.try_into().ok().expect("Balance conversion should work");
	<pallet_balances::Pallet<T> as Currency<T::AccountId>>::make_free_balance_be(
		&user,
		balance_value,
	);
	user
}

/// Setup a specified number of guardians and return them
fn setup_guardians<T: Config>(count: u32) -> Vec<T::AccountId> {
	let guardians: Vec<T::AccountId> = (0..count).map(|i| account("guardian", i, SEED)).collect();
	Guardians::<T>::put(guardians.clone());
	guardians
}

/// Create a unique deposit ID from recipient and amount
fn create_deposit_id(recipient: &[u8], amount: u128, nonce: u32) -> H256 {
	let mut data = Vec::new();
	data.extend_from_slice(recipient);
	data.extend_from_slice(&amount.to_le_bytes());
	data.extend_from_slice(&nonce.to_le_bytes());
	BlakeTwo256::hash(&data)
}

/// Setup a pending deposit proposal
fn setup_pending_deposit<T: Config>(
	recipient: T::AccountId,
	amount: u128,
) -> (DepositId, DepositRecord<T::AccountId, BlockNumberFor<T>>) {
	let deposit_id = create_deposit_id(recipient.encode().as_slice(), amount, 0);
	let record = DepositRecord {
		recipient: recipient.clone(),
		amount,
		approve_votes: Default::default(),
		deny_votes: Default::default(),
		proposed_at_block: frame_system::Pallet::<T>::block_number(),
		status: VoteStatus::Pending,
	};
	PendingDeposits::<T>::insert(deposit_id, record.clone());
	(deposit_id, record)
}

/// Setup a pending unlock request with escrowed funds
fn setup_pending_unlock<T: Config>(
	requester: T::AccountId,
	amount: u128,
) -> (BurnId, UnlockRecord<T::AccountId, BlockNumberFor<T>>) {
	let burn_id = NextBurnId::<T>::get();
	NextBurnId::<T>::put(burn_id.saturating_add(1));

	// Transfer funds to pallet escrow
	let balance_value: BalanceOf<T> =
		amount.try_into().ok().expect("Balance conversion should work");
	let pallet_account = Pallet::<T>::account_id();
	<pallet_balances::Pallet<T> as Currency<T::AccountId>>::transfer(
		&requester,
		&pallet_account,
		balance_value,
		frame_support::traits::ExistenceRequirement::AllowDeath,
	)
	.expect("Transfer should succeed");

	let record = UnlockRecord {
		requester: requester.clone(),
		amount,
		bittensor_coldkey: account("coldkey", 0, SEED),
		approve_votes: Default::default(),
		deny_votes: Default::default(),
		requested_at_block: frame_system::Pallet::<T>::block_number(),
		status: VoteStatus::Pending,
	};
	PendingUnlocks::<T>::insert(burn_id, record.clone());
	(burn_id, record)
}

#[benchmarks]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn propose_deposits(d: Linear<1, 100>) {
		// Setup
		let guardians = setup_guardians::<T>(3);
		let guardian = guardians[0].clone();
		ApproveThreshold::<T>::put(2u16);
		DenyThreshold::<T>::put(2u16);
		GlobalMintCap::<T>::put(1_000_000_000u128);
		Paused::<T>::put(false);
		let ttl: BlockNumberFor<T> = 100u32.into();
		SignatureTTLBlocks::<T>::put(ttl);

		// Create d deposit proposals
		let mut deposits = Vec::new();
		for i in 0..d {
			let recipient = create_funded_user::<T>(i, 1000);
			let deposit_id = create_deposit_id(recipient.encode().as_slice(), 100, i);
			deposits.push(DepositProposal { id: deposit_id, recipient, amount: 100 });
		}

		let checkpoint_nonce = 0u64;

		#[extrinsic_call]
		propose_deposits(RawOrigin::Signed(guardian.clone()), deposits.clone(), checkpoint_nonce);

		// Verify first deposit was created
		assert!(PendingDeposits::<T>::contains_key(deposits[0].id));
		assert_eq!(LastCheckpointNonce::<T>::get(), Some(checkpoint_nonce));
	}

	#[benchmark]
	fn attest_deposit() {
		// Setup
		let guardians = setup_guardians::<T>(3);
		ApproveThreshold::<T>::put(2u16);
		DenyThreshold::<T>::put(2u16);
		GlobalMintCap::<T>::put(1_000_000_000u128);
		Paused::<T>::put(false);
		let ttl: BlockNumberFor<T> = 100u32.into();
		SignatureTTLBlocks::<T>::put(ttl);

		let recipient = create_funded_user::<T>(0, 1000);
		let (deposit_id, mut record) = setup_pending_deposit::<T>(recipient.clone(), 500);

		// Add one approval vote (need one more to reach threshold of 2)
		record.approve_votes.insert(guardians[0].clone());
		PendingDeposits::<T>::insert(deposit_id, record);

		let guardian = guardians[1].clone();

		#[extrinsic_call]
		attest_deposit(RawOrigin::Signed(guardian.clone()), deposit_id, true);

		// Should be approved and processed (threshold reached)
		assert!(ProcessedDeposits::<T>::get(deposit_id));
		assert!(!PendingDeposits::<T>::contains_key(deposit_id));
	}

	#[benchmark]
	fn attest_unlock() {
		// Setup
		let guardians = setup_guardians::<T>(3);
		ApproveThreshold::<T>::put(2u16);
		DenyThreshold::<T>::put(2u16);
		Paused::<T>::put(false);
		let ttl: BlockNumberFor<T> = 100u32.into();
		SignatureTTLBlocks::<T>::put(ttl);
		GlobalMintCap::<T>::put(1_000_000_000u128);
		TotalMintedByBridge::<T>::put(1000u128);

		let requester = create_funded_user::<T>(0, 10000);
		let (burn_id, mut record) = setup_pending_unlock::<T>(requester.clone(), 500);

		// Add one approval vote (need one more to reach threshold of 2)
		record.approve_votes.insert(guardians[0].clone());
		PendingUnlocks::<T>::insert(burn_id, record);

		let guardian = guardians[1].clone();

		#[extrinsic_call]
		attest_unlock(RawOrigin::Signed(guardian.clone()), burn_id, true);

		// Should be approved and finalized (threshold reached)
		assert!(!PendingUnlocks::<T>::contains_key(burn_id));
	}

	#[benchmark]
	fn request_unlock() {
		// Setup
		setup_guardians::<T>(3);
		Paused::<T>::put(false);
		let requester = create_funded_user::<T>(0, 10000);
		let amount = 500u128;
		let bittensor_coldkey: T::AccountId = account("coldkey", 0, SEED);

		#[extrinsic_call]
		request_unlock(RawOrigin::Signed(requester.clone()), amount, bittensor_coldkey.clone());

		// Verify unlock request was created
		let burn_id = 0u128; // First burn ID
		assert!(PendingUnlocks::<T>::contains_key(burn_id));
	}

	#[benchmark]
	fn expire_pending_deposit() {
		// Setup
		setup_guardians::<T>(3);
		ApproveThreshold::<T>::put(2u16);
		let ttl: BlockNumberFor<T> = 10u32.into();
		SignatureTTLBlocks::<T>::put(ttl);

		let recipient = create_funded_user::<T>(0, 1000);
		let (deposit_id, _) = setup_pending_deposit::<T>(recipient, 500);

		// Advance blocks past TTL
		let current_block = frame_system::Pallet::<T>::block_number();
		frame_system::Pallet::<T>::set_block_number(current_block + 20u32.into());

		let caller: T::AccountId = whitelisted_caller();

		#[extrinsic_call]
		expire_pending_deposit(RawOrigin::Signed(caller), deposit_id);

		// Verify deposit was removed and marked processed
		assert!(!PendingDeposits::<T>::contains_key(deposit_id));
		assert!(ProcessedDeposits::<T>::get(deposit_id));
	}

	#[benchmark]
	fn expire_pending_unlock() {
		// Setup
		setup_guardians::<T>(3);
		ApproveThreshold::<T>::put(2u16);
		let ttl: BlockNumberFor<T> = 10u32.into();
		SignatureTTLBlocks::<T>::put(ttl);
		Paused::<T>::put(false);

		let requester = create_funded_user::<T>(0, 10000);
		let initial_balance =
			<pallet_balances::Pallet<T> as Currency<T::AccountId>>::free_balance(&requester);
		let (burn_id, _) = setup_pending_unlock::<T>(requester.clone(), 500);

		// Advance blocks past TTL
		let current_block = frame_system::Pallet::<T>::block_number();
		frame_system::Pallet::<T>::set_block_number(current_block + 20u32.into());

		let caller: T::AccountId = whitelisted_caller();

		#[extrinsic_call]
		expire_pending_unlock(RawOrigin::Signed(caller), burn_id);

		// Verify unlock was removed and funds restored
		assert!(!PendingUnlocks::<T>::contains_key(burn_id));
		// Funds should be restored (within existential deposit tolerance)
		let final_balance =
			<pallet_balances::Pallet<T> as Currency<T::AccountId>>::free_balance(&requester);
		assert!(final_balance >= initial_balance - 10u32.into());
	}

	#[benchmark]
	fn add_guardian(g: Linear<0, 100>) {
		// Setup existing guardians
		let existing_guardians = setup_guardians::<T>(g);
		ApproveThreshold::<T>::put(2u16);
		DenyThreshold::<T>::put(2u16);

		let new_guardian: T::AccountId = account("new_guardian", 999, SEED);

		#[extrinsic_call]
		add_guardian(RawOrigin::Root, new_guardian.clone());

		// Verify guardian was added
		let guardians = Guardians::<T>::get();
		assert!(guardians.contains(&new_guardian));
		assert_eq!(guardians.len(), (g + 1) as usize);
	}

	#[benchmark]
	fn remove_guardian(g: Linear<2, 100>) {
		// Setup guardians (at least 2 to maintain valid thresholds)
		let guardians = setup_guardians::<T>(g);
		ApproveThreshold::<T>::put(1u16);
		DenyThreshold::<T>::put(1u16);

		let guardian_to_remove = guardians[0].clone();

		#[extrinsic_call]
		remove_guardian(RawOrigin::Root, guardian_to_remove.clone());

		// Verify guardian was removed
		let remaining_guardians = Guardians::<T>::get();
		assert!(!remaining_guardians.contains(&guardian_to_remove));
		assert_eq!(remaining_guardians.len(), (g - 1) as usize);
	}

	#[benchmark]
	fn set_approve_threshold() {
		// Setup
		setup_guardians::<T>(10);
		let new_threshold = 5u16;

		#[extrinsic_call]
		set_approve_threshold(RawOrigin::Root, new_threshold);

		assert_eq!(ApproveThreshold::<T>::get(), new_threshold);
	}

	#[benchmark]
	fn set_deny_threshold() {
		// Setup
		setup_guardians::<T>(10);
		let new_threshold = 5u16;

		#[extrinsic_call]
		set_deny_threshold(RawOrigin::Root, new_threshold);

		assert_eq!(DenyThreshold::<T>::get(), new_threshold);
	}

	#[benchmark]
	fn set_paused() {
		let paused = true;

		#[extrinsic_call]
		set_paused(RawOrigin::Root, paused);

		assert_eq!(Paused::<T>::get(), paused);
	}

	#[benchmark]
	fn set_global_mint_cap() {
		let new_cap = 1_000_000_000u128;
		TotalMintedByBridge::<T>::put(0u128);

		#[extrinsic_call]
		set_global_mint_cap(RawOrigin::Root, new_cap);

		assert_eq!(GlobalMintCap::<T>::get(), new_cap);
	}

	#[benchmark]
	fn set_signature_ttl() {
		let new_ttl: BlockNumberFor<T> = 200u32.into();

		#[extrinsic_call]
		set_signature_ttl(RawOrigin::Root, new_ttl);

		assert_eq!(SignatureTTLBlocks::<T>::get(), new_ttl);
	}

	impl_benchmark_test_suite!(AlphaBridge, crate::mock::new_test_ext(), crate::mock::Test);
}

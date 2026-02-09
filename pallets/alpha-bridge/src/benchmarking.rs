//! Benchmarking setup for pallet-alpha-bridge
#![cfg(feature = "runtime-benchmarks")]

use super::*;
#[allow(unused_imports)]
use crate::Pallet as AlphaBridge;
use codec::Encode;
use frame_benchmarking::v2::*;
use frame_support::traits::Currency;
use frame_system::RawOrigin;
use sp_core::H256;
use sp_std::{collections::btree_set::BTreeSet, vec::Vec};

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

/// Generate a deposit ID using the same domain separator and hash the pallet verifies.
fn generate_deposit_id<T: Config>(recipient: &T::AccountId, amount: u128, nonce: u64) -> H256 {
	let mut data = Vec::new();
	data.extend_from_slice(DOMAIN_DEPOSIT_REQUEST);
	data.extend_from_slice(&recipient.encode());
	data.extend_from_slice(&amount.to_le_bytes());
	data.extend_from_slice(&nonce.to_le_bytes());
	H256::from(sp_core::hashing::blake2_256(&data))
}

/// Insert a pending deposit directly into storage.
fn insert_pending_deposit<T: Config>(
	deposit_id: pallet::DepositId,
	recipient: T::AccountId,
	amount: u128,
	votes: BTreeSet<T::AccountId>,
	created_at_block: BlockNumberFor<T>,
) {
	let deposit = pallet::Deposit::<T> {
		request_id: deposit_id,
		recipient,
		amount,
		votes,
		status: pallet::DepositStatus::Pending,
		created_at_block,
		finalized_at_block: None,
	};
	Deposits::<T>::insert(deposit_id, deposit);
}

/// Insert a completed deposit directly into storage (for cleanup benchmarks).
fn insert_completed_deposit<T: Config>(
	deposit_id: pallet::DepositId,
	recipient: T::AccountId,
	amount: u128,
	finalized_at_block: BlockNumberFor<T>,
) {
	let deposit = pallet::Deposit::<T> {
		request_id: deposit_id,
		recipient,
		amount,
		votes: BTreeSet::new(),
		status: pallet::DepositStatus::Completed,
		created_at_block: finalized_at_block,
		finalized_at_block: Some(finalized_at_block),
	};
	Deposits::<T>::insert(deposit_id, deposit);
}

/// Insert a withdrawal request directly into storage.
fn insert_withdrawal_request<T: Config>(
	sender: T::AccountId,
	amount: u128,
	nonce: u64,
	created_at_block: BlockNumberFor<T>,
) -> pallet::WithdrawalRequestId {
	let mut data = Vec::new();
	data.extend_from_slice(DOMAIN_WITHDRAWAL_REQUEST);
	data.extend_from_slice(&sender.encode());
	// Use alphaRao amount for hash (amount / HALPHA_RAO_PER_ALPHA_RAO)
	let alpha_amount = amount / HALPHA_RAO_PER_ALPHA_RAO;
	data.extend_from_slice(&alpha_amount.to_le_bytes());
	data.extend_from_slice(&nonce.to_le_bytes());
	let request_id = H256::from(sp_core::hashing::blake2_256(&data));

	let request = pallet::WithdrawalRequest::<T> {
		sender: sender.clone(),
		recipient: sender,
		amount,
		nonce,
		status: pallet::WithdrawalRequestStatus::Requested,
		created_at_block,
	};
	WithdrawalRequests::<T>::insert(request_id, request);
	request_id
}

#[benchmarks]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn withdraw() {
		// Setup
		Paused::<T>::put(false);
		MinWithdrawalAmount::<T>::put(1u128);
		GlobalMintCap::<T>::put(1_000_000_000_000u128);
		// Amount must be divisible by HALPHA_RAO_PER_ALPHA_RAO
		let amount = 1_000_000_000u128;
		// Pre-seed TotalMintedByBridge so the burn accounting doesn't underflow
		TotalMintedByBridge::<T>::put(amount);

		let user = create_funded_user::<T>(0, 10_000_000_000);

		#[extrinsic_call]
		withdraw(RawOrigin::Signed(user.clone()), amount);

		// Verify withdrawal request was created
		assert!(WithdrawalRequests::<T>::iter().count() == 1);
	}

	#[benchmark]
	fn attest_deposit() {
		// Setup 3 guardians, threshold=2 (worst case: this vote triggers finalization)
		let guardians = setup_guardians::<T>(3);
		ApproveThreshold::<T>::put(2u16);
		GlobalMintCap::<T>::put(1_000_000_000_000u128);
		Paused::<T>::put(false);

		let recipient = create_funded_user::<T>(0, 1000);
		let amount = 1_000_000_000u128;
		let nonce = 0u64;
		let deposit_id = generate_deposit_id::<T>(&recipient, amount, nonce);

		// Insert pending deposit with 1 pre-existing vote from guardian[0]
		let mut votes = BTreeSet::new();
		votes.insert(guardians[0].clone());
		insert_pending_deposit::<T>(
			deposit_id,
			recipient.clone(),
			amount,
			votes,
			frame_system::Pallet::<T>::block_number(),
		);

		// Guardian[1] calls attest_deposit (triggers finalization = worst case)
		let guardian = guardians[1].clone();

		#[extrinsic_call]
		attest_deposit(RawOrigin::Signed(guardian), deposit_id, recipient, amount, nonce);

		// Verify deposit was completed
		let deposit = Deposits::<T>::get(deposit_id).expect("Deposit should exist");
		assert_eq!(deposit.status, pallet::DepositStatus::Completed);
	}

	#[benchmark]
	fn cleanup_deposit() {
		// Setup guardians
		let guardians = setup_guardians::<T>(3);

		let recipient = create_funded_user::<T>(0, 1000);
		let deposit_id = generate_deposit_id::<T>(&recipient, 1_000_000_000u128, 0);

		// Insert completed deposit at block 1
		let created_block: BlockNumberFor<T> = 1u32.into();
		insert_completed_deposit::<T>(deposit_id, recipient, 1_000_000_000u128, created_block);

		// Set short TTL
		CleanupTTLBlocks::<T>::put(BlockNumberFor::<T>::from(1u32));

		// Advance to block 10
		frame_system::Pallet::<T>::set_block_number(10u32.into());

		let guardian = guardians[0].clone();

		#[extrinsic_call]
		cleanup_deposit(RawOrigin::Signed(guardian), deposit_id);

		// Verify deposit was removed
		assert!(!Deposits::<T>::contains_key(deposit_id));
	}

	#[benchmark]
	fn cleanup_withdrawal_request() {
		// Setup guardians
		let guardians = setup_guardians::<T>(3);

		let sender = create_funded_user::<T>(0, 1000);

		// Insert withdrawal request at block 1
		frame_system::Pallet::<T>::set_block_number(1u32.into());
		let request_id = insert_withdrawal_request::<T>(
			sender,
			1_000_000_000u128,
			0,
			1u32.into(),
		);

		// Set short TTL
		CleanupTTLBlocks::<T>::put(BlockNumberFor::<T>::from(1u32));

		// Advance to block 10
		frame_system::Pallet::<T>::set_block_number(10u32.into());

		let guardian = guardians[0].clone();

		#[extrinsic_call]
		cleanup_withdrawal_request(RawOrigin::Signed(guardian), request_id);

		// Verify withdrawal request was removed
		assert!(!WithdrawalRequests::<T>::contains_key(request_id));
	}

	#[benchmark]
	fn set_guardians_and_threshold(g: Linear<1, 10>) {
		let guardians: Vec<T::AccountId> = (0..g).map(|i| account("guardian", i, SEED)).collect();
		let threshold = 1u16;

		#[extrinsic_call]
		set_guardians_and_threshold(RawOrigin::Root, guardians.clone(), threshold);

		assert_eq!(Guardians::<T>::get(), guardians);
		assert_eq!(ApproveThreshold::<T>::get(), threshold);
	}

	#[benchmark]
	fn pause() {
		#[extrinsic_call]
		pause(RawOrigin::Root);

		assert_eq!(Paused::<T>::get(), true);
	}

	#[benchmark]
	fn unpause() {
		Paused::<T>::put(true);

		#[extrinsic_call]
		unpause(RawOrigin::Root);

		assert_eq!(Paused::<T>::get(), false);
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
	fn set_cleanup_ttl() {
		let ttl: BlockNumberFor<T> = 50_000u32.into();

		#[extrinsic_call]
		set_cleanup_ttl(RawOrigin::Root, ttl);

		assert_eq!(CleanupTTLBlocks::<T>::get(), ttl);
	}

	#[benchmark]
	fn set_min_withdrawal_amount() {
		let amount = 2_000_000_000u128;

		#[extrinsic_call]
		set_min_withdrawal_amount(RawOrigin::Root, amount);

		assert_eq!(MinWithdrawalAmount::<T>::get(), amount);
	}

	#[benchmark]
	fn admin_cancel_deposit() {
		let recipient = create_funded_user::<T>(0, 1000);
		let deposit_id = generate_deposit_id::<T>(&recipient, 1_000_000_000u128, 0);

		// Insert pending deposit
		insert_pending_deposit::<T>(
			deposit_id,
			recipient,
			1_000_000_000u128,
			BTreeSet::new(),
			frame_system::Pallet::<T>::block_number(),
		);

		#[extrinsic_call]
		admin_cancel_deposit(
			RawOrigin::Root,
			deposit_id,
			pallet::CancelReason::AdminEmergency,
		);

		// Verify deposit was cancelled
		let deposit = Deposits::<T>::get(deposit_id).expect("Deposit should exist");
		assert_eq!(deposit.status, pallet::DepositStatus::Cancelled);
	}

	#[benchmark]
	fn admin_fail_withdrawal_request() {
		// Setup mint cap and tracking
		GlobalMintCap::<T>::put(1_000_000_000_000u128);
		TotalMintedByBridge::<T>::put(0u128);

		let sender = create_funded_user::<T>(0, 10_000_000_000);
		let request_id = insert_withdrawal_request::<T>(
			sender,
			1_000_000_000u128,
			0,
			frame_system::Pallet::<T>::block_number(),
		);

		#[extrinsic_call]
		admin_fail_withdrawal_request(RawOrigin::Root, request_id);

		// Verify withdrawal request was failed
		let request = WithdrawalRequests::<T>::get(request_id).expect("Request should exist");
		assert_eq!(request.status, pallet::WithdrawalRequestStatus::Failed);
	}

	#[benchmark]
	fn admin_manual_mint() {
		// Setup mint cap
		GlobalMintCap::<T>::put(1_000_000_000_000u128);
		TotalMintedByBridge::<T>::put(0u128);

		let recipient = create_funded_user::<T>(0, 1000);
		let amount = 1_000_000_000u128;

		#[extrinsic_call]
		admin_manual_mint(RawOrigin::Root, recipient.clone(), amount, None);

		// Verify total minted was updated
		assert_eq!(TotalMintedByBridge::<T>::get(), amount);
	}

	impl_benchmark_test_suite!(AlphaBridge, crate::mock::new_test_ext(), crate::mock::Test);
}

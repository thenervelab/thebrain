use crate::pallet::{
	BurnId, BurnQueue, BurnQueueItem, Config, DepositId, DepositRecord, ExpiredDeposit,
	ExpiredDeposits, PendingDeposits, PendingUnlocks, UnlockRecord, VoteStatus,
};
use codec::{Decode, Encode};
use frame_support::{traits::Get, weights::Weight};
use frame_system::pallet_prelude::BlockNumberFor;
use scale_info::TypeInfo;
use sp_runtime::RuntimeDebug;
use sp_std::collections::btree_set::BTreeSet;

/// Migrates all amount fields from `u64` to `u128` in storage.
pub fn migrate_to_v2<T>() -> Weight
where
	T: Config,
{
	let mut reads: u64 = 0;
	let mut writes: u64 = 0;

	PendingDeposits::<T>::translate::<DepositRecordV1<T::AccountId, BlockNumberFor<T>>, _>(
		|_, old| {
			reads += 1;
			writes += 1;
			Some(old.into())
		},
	);

	PendingUnlocks::<T>::translate::<UnlockRecordV1<T::AccountId, BlockNumberFor<T>>, _>(
		|_, old| {
			reads += 1;
			writes += 1;
			Some(old.into())
		},
	);

	ExpiredDeposits::<T>::translate::<ExpiredDepositV1<T::AccountId>, _>(|_, old| {
		reads += 1;
		writes += 1;
		Some(old.into())
	});

	BurnQueue::<T>::translate::<BurnQueueItemV1<T::AccountId>, _>(|_, old| {
		reads += 1;
		writes += 1;
		Some(old.into())
	});

	let mut weight = Weight::zero();
	if reads > 0 {
		weight = weight.saturating_add(T::DbWeight::get().reads(reads));
	}
	if writes > 0 {
		weight = weight.saturating_add(T::DbWeight::get().writes(writes));
	}

	weight
}

#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo)]
#[scale_info(skip_type_params(T))]
struct DepositRecordV1<AccountId, BlockNumber> {
	recipient: AccountId,
	amount: u64,
	approve_votes: BTreeSet<AccountId>,
	deny_votes: BTreeSet<AccountId>,
	proposed_at_block: BlockNumber,
	status: VoteStatus,
}

impl<AccountId, BlockNumber> From<DepositRecordV1<AccountId, BlockNumber>>
	for DepositRecord<AccountId, BlockNumber>
{
	fn from(old: DepositRecordV1<AccountId, BlockNumber>) -> Self {
		Self {
			recipient: old.recipient,
			amount: u128::from(old.amount),
			approve_votes: old.approve_votes,
			deny_votes: old.deny_votes,
			proposed_at_block: old.proposed_at_block,
			status: old.status,
		}
	}
}

#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo)]
#[scale_info(skip_type_params(T))]
struct UnlockRecordV1<AccountId, BlockNumber> {
	requester: AccountId,
	amount: u64,
	approve_votes: BTreeSet<AccountId>,
	deny_votes: BTreeSet<AccountId>,
	requested_at_block: BlockNumber,
	status: VoteStatus,
}

impl<AccountId, BlockNumber> From<UnlockRecordV1<AccountId, BlockNumber>>
	for UnlockRecord<AccountId, BlockNumber>
{
	fn from(old: UnlockRecordV1<AccountId, BlockNumber>) -> Self {
		Self {
			requester: old.requester,
			amount: u128::from(old.amount),
			approve_votes: old.approve_votes,
			deny_votes: old.deny_votes,
			requested_at_block: old.requested_at_block,
			status: old.status,
		}
	}
}

#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo)]
struct ExpiredDepositV1<AccountId> {
	deposit_id: DepositId,
	recipient: AccountId,
	amount: u64,
}

impl<AccountId> From<ExpiredDepositV1<AccountId>> for ExpiredDeposit<AccountId> {
	fn from(old: ExpiredDepositV1<AccountId>) -> Self {
		Self {
			deposit_id: old.deposit_id,
			recipient: old.recipient,
			amount: u128::from(old.amount),
		}
	}
}

#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo)]
struct BurnQueueItemV1<AccountId> {
	burn_id: BurnId,
	requester: AccountId,
	amount: u64,
}

impl<AccountId> From<BurnQueueItemV1<AccountId>> for BurnQueueItem<AccountId> {
	fn from(old: BurnQueueItemV1<AccountId>) -> Self {
		Self {
			burn_id: old.burn_id,
			requester: old.requester,
			amount: u128::from(old.amount),
		}
	}
}


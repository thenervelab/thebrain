#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

pub mod weights;

pub mod migrations;

use crate::weights::WeightInfo;
use codec::{Decode, Encode, MaxEncodedLen};
use frame_support::{
	pallet_prelude::*,
	sp_runtime::traits::AccountIdConversion,
	traits::{
		fungible::Mutate,
		tokens::{Fortitude, Precision, Preservation},
		Currency, ExistenceRequirement, StorageVersion,
	},
	PalletId,
};
use frame_system::pallet_prelude::*;
use scale_info::TypeInfo;
use sp_core::H256;
use sp_runtime::traits::{AtLeast32BitUnsigned, Saturating};
use sp_std::{collections::btree_set::BTreeSet, prelude::*};

pub use pallet::*;

/// The current storage version
const STORAGE_VERSION: StorageVersion = StorageVersion::new(1);

#[frame_support::pallet]
pub mod pallet {
	use super::*;

	pub type DepositId = H256;

	pub type BurnId = u128;

	pub type BalanceOf<T> = <T as pallet_balances::Config>::Balance;

	/// Deposit proposal structure submitted by guardians
	#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
	pub struct DepositProposal<AccountId> {
		pub id: DepositId,
		pub recipient: AccountId,
		pub amount: u128,
	}

	/// Status of a vote (deposit or unlock)
	#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
	pub enum VoteStatus {
		Pending,
		Approved,
		Denied,
		Expired,
	}

	impl Default for VoteStatus {
		fn default() -> Self {
			VoteStatus::Pending
		}
	}

	/// Record of a pending deposit proposal with votes
	#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo)]
	#[scale_info(skip_type_params(T))]
	pub struct DepositRecord<AccountId, BlockNumber> {
		pub recipient: AccountId,
		pub amount: u128,
		pub approve_votes: BTreeSet<AccountId>,
		pub deny_votes: BTreeSet<AccountId>,
		pub proposed_at_block: BlockNumber,
		pub status: VoteStatus,
	}

	/// Record of a pending unlock request with votes
	#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo)]
	#[scale_info(skip_type_params(T))]
	pub struct UnlockRecord<AccountId, BlockNumber> {
		pub requester: AccountId,
		pub amount: u128,
		pub bittensor_coldkey: AccountId,
		pub approve_votes: BTreeSet<AccountId>,
		pub deny_votes: BTreeSet<AccountId>,
		pub requested_at_block: BlockNumber,
		pub status: VoteStatus,
	}

	#[pallet::pallet]
	#[pallet::without_storage_info]
	#[pallet::storage_version(STORAGE_VERSION)]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config + pallet_balances::Config {
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
		/// The pallet's id, used for deriving its sovereign account ID.
		#[pallet::constant]
		type PalletId: Get<PalletId>;

		/// The balance type used for this pallet.
		type Balance: Parameter
			+ Member
			+ AtLeast32BitUnsigned
			+ Default
			+ Copy
			+ TryFrom<BalanceOf<Self>>
			+ Into<<Self as pallet_balances::Config>::Balance>;

		/// Weight information for extrinsics in this pallet.
		type WeightInfo: WeightInfo;
	}

	/// Tracks which deposit IDs have been fully processed (prevents replay attacks)
	#[pallet::storage]
	#[pallet::getter(fn processed_deposits)]
	pub type ProcessedDeposits<T: Config> =
		StorageMap<_, Blake2_128Concat, DepositId, bool, ValueQuery>;

	/// Tracks the last processed Bittensor checkpoint nonce to enforce ordering
	#[pallet::storage]
	#[pallet::getter(fn last_checkpoint_nonce)]
	pub type LastCheckpointNonce<T: Config> = StorageValue<_, u64, OptionQuery>;

	/// Running total of all minted halpha (used for mint cap enforcement)
	#[pallet::storage]
	#[pallet::getter(fn total_minted_by_bridge)]
	pub type TotalMintedByBridge<T: Config> = StorageValue<_, u128, ValueQuery>;

	/// Maximum allowed minted halpha (set via governance/sudo)
	#[pallet::storage]
	#[pallet::getter(fn global_mint_cap)]
	pub type GlobalMintCap<T: Config> = StorageValue<_, u128, ValueQuery>;

	/// Emergency pause switch (blocks all deposit/unlock operations when true)
	#[pallet::storage]
	#[pallet::getter(fn paused)]
	pub type Paused<T: Config> = StorageValue<_, bool, ValueQuery>;

	/// Guardian accounts authorized to vote on deposits and unlocks
	#[pallet::storage]
	#[pallet::getter(fn guardians)]
	pub type Guardians<T: Config> = StorageValue<_, Vec<T::AccountId>, ValueQuery>;

	/// Minimum guardian approvals needed to process an action
	#[pallet::storage]
	#[pallet::getter(fn approve_threshold)]
	pub type ApproveThreshold<T: Config> = StorageValue<_, u16, ValueQuery>;

	/// Minimum guardian denials needed to reject an action
	#[pallet::storage]
	#[pallet::getter(fn deny_threshold)]
	pub type DenyThreshold<T: Config> = StorageValue<_, u16, ValueQuery>;

	/// Number of blocks before a pending proposal expires
	#[pallet::storage]
	#[pallet::getter(fn signature_ttl_blocks)]
	pub type SignatureTTLBlocks<T: Config> = StorageValue<_, BlockNumberFor<T>, ValueQuery>;

	/// Pending deposit proposals awaiting guardian votes
	#[pallet::storage]
	#[pallet::getter(fn pending_deposits)]
	pub type PendingDeposits<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		DepositId,
		DepositRecord<T::AccountId, BlockNumberFor<T>>,
		OptionQuery,
	>;

	/// Pending unlock requests awaiting guardian votes
	#[pallet::storage]
	#[pallet::getter(fn pending_unlocks)]
	pub type PendingUnlocks<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		BurnId,
		UnlockRecord<T::AccountId, BlockNumberFor<T>>,
		OptionQuery,
	>;

	/// Counter for generating unique burn IDs
	#[pallet::storage]
	#[pallet::getter(fn next_burn_id)]
	pub type NextBurnId<T: Config> = StorageValue<_, u128, ValueQuery>;

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		/// Batch of deposits proposed by a guardian
		DepositsProposed { deposit_ids: Vec<DepositId>, proposer: T::AccountId },
		/// Guardian voted on a deposit proposal
		DepositAttested { deposit_id: DepositId, guardian: T::AccountId, approved: bool },
		/// Deposit approved and halpha minted to recipient
		BridgeMinted { deposit_id: DepositId, recipient: T::AccountId, amount: u128 },
		/// Deposit denied by guardians
		BridgeDenied { deposit_id: DepositId, recipient: T::AccountId, amount: u128 },
		/// Deposit proposal expired due to timeout
		DepositExpired { deposit_id: DepositId },
		/// User requested unlock (burn halpha to receive alpha on Bittensor)
		UnlockRequested {
			burn_id: BurnId,
			requester: T::AccountId,
			amount: u128,
			bittensor_coldkey: T::AccountId,
		},
		/// Guardian voted on an unlock request
		UnlockAttested { burn_id: BurnId, guardian: T::AccountId, approved: bool },
		/// Unlock approved and halpha burned
		UnlockApproved {
			burn_id: BurnId,
			requester: T::AccountId,
			amount: u128,
			bittensor_coldkey: T::AccountId,
		},
		/// Unlock denied and funds restored to user
		UnlockDenied { burn_id: BurnId, requester: T::AccountId, amount: u128 },
		/// Unlock request expired and funds restored
		UnlockExpired { burn_id: BurnId },
		/// Bridge pause state changed
		BridgePaused { paused: bool },
		/// Global mint cap updated
		GlobalMintCapUpdated { new_cap: u128 },
		/// Approve threshold updated
		ApproveThresholdUpdated { new_threshold: u16 },
		/// Deny threshold updated
		DenyThresholdUpdated { new_threshold: u16 },
		/// Signature TTL updated
		SignatureTTLUpdated { new_ttl: BlockNumberFor<T> },
		/// Guardian added to the set
		GuardianAdded { guardian: T::AccountId },
		/// Guardian removed from the set
		GuardianRemoved { guardian: T::AccountId },
	}

	#[pallet::error]
	pub enum Error<T> {
		/// Caller is not a guardian
		NotGuardian,
		/// Deposit has already been processed (prevents replay attacks)
		DepositAlreadyProcessed,
		/// Deposit already exists in the pending queue
		DepositAlreadyPending,
		/// User has insufficient halpha balance
		InsufficientBalance,
		/// Guardian has already voted on this proposal
		AlreadyVoted,
		/// Not enough votes to finalize proposal
		InsufficientVotes,
		/// Bridge is currently paused
		BridgePaused,
		/// Minting would exceed the global mint cap
		GlobalMintCapExceeded,
		/// Deposit proposal not found in pending state
		DepositNotPending,
		/// Deposit has already been finalized
		DepositAlreadyFinalized,
		/// Unlock request not found in pending state
		UnlockNotPending,
		/// Unlock request has already been finalized
		UnlockAlreadyFinalized,
		/// Proposal has not expired yet
		ProposalNotExpired,
		/// Checkpoint nonce is not sequential
		InvalidCheckpointNonce,
		/// Threshold cannot be zero
		ThresholdTooLow,
		/// Threshold exceeds guardian count
		ThresholdTooHigh,
		/// Cannot propose empty deposit batch
		EmptyDepositBatch,
		/// Guardian already exists in the set
		GuardianAlreadyExists,
		/// Guardian not found in the set
		GuardianNotFound,
		/// Caller not authorized for this action
		NotAuthorized,
		/// Proposal has expired
		ProposalExpired,
		/// Failed to convert between numeric balance types
		AmountConversionFailed,
		/// Failed to mint tokens for approved deposit
		DepositMintFailed,
		/// Escrow account does not hold enough balance (critical: indicates tampering or bug)
		EscrowBalanceInsufficient,
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		/// Guardian proposes a batch of deposit proposals from Bittensor checkpoint
		///
		/// # Arguments
		/// * `origin` - Must be signed by a guardian
		/// * `deposits` - Vector of deposit proposals
		/// * `checkpoint_nonce` - Bittensor checkpoint nonce (must be sequential)
		#[pallet::call_index(0)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::propose_deposits(deposits.len() as u32))]
		pub fn propose_deposits(
			origin: OriginFor<T>,
			deposits: Vec<DepositProposal<T::AccountId>>,
			checkpoint_nonce: u64,
		) -> DispatchResult {
			let guardian = ensure_signed(origin)?;
			Self::ensure_guardian(&guardian)?;
			Self::ensure_not_paused()?;

			ensure!(!deposits.is_empty(), Error::<T>::EmptyDepositBatch);

			match LastCheckpointNonce::<T>::get() {
				Some(last_nonce) => {
					let expected_nonce =
						last_nonce.checked_add(1).ok_or(Error::<T>::InvalidCheckpointNonce)?;
					ensure!(checkpoint_nonce == expected_nonce, Error::<T>::InvalidCheckpointNonce);
				},
				None => ensure!(checkpoint_nonce == 0, Error::<T>::InvalidCheckpointNonce),
			}

			let mut deposit_ids = Vec::new();
			let current_block = frame_system::Pallet::<T>::block_number();

			for deposit in deposits.iter() {
				let deposit_id = deposit.id.clone();

				ensure!(
					!ProcessedDeposits::<T>::get(&deposit_id),
					Error::<T>::DepositAlreadyProcessed
				);
				ensure!(
					!PendingDeposits::<T>::contains_key(&deposit_id),
					Error::<T>::DepositAlreadyPending
				);

				Self::check_mint_cap(deposit.amount)?;

				let mut approve_votes = BTreeSet::new();
				approve_votes.insert(guardian.clone());

				let record = DepositRecord {
					recipient: deposit.recipient.clone(),
					amount: deposit.amount,
					approve_votes,
					deny_votes: BTreeSet::new(),
					proposed_at_block: current_block,
					status: VoteStatus::Pending,
				};

				PendingDeposits::<T>::insert(deposit_id, record.clone());
				deposit_ids.push(deposit_id);

				let vote_status = Self::check_vote_thresholds(1, 0);
				if vote_status == VoteStatus::Approved {
					Self::finalize_deposit(deposit_id, record)?;
				}
			}

			LastCheckpointNonce::<T>::put(checkpoint_nonce);

			Self::deposit_event(Event::DepositsProposed { deposit_ids, proposer: guardian });

			Ok(())
		}

		/// Guardian attests (votes on) a pending deposit proposal
		///
		/// # Arguments
		/// * `origin` - Must be signed by a guardian
		/// * `deposit_id` - The deposit ID to vote on
		/// * `approve` - True to approve, false to deny
		#[pallet::call_index(1)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::attest_deposit())]
		pub fn attest_deposit(
			origin: OriginFor<T>,
			deposit_id: DepositId,
			approve: bool,
		) -> DispatchResult {
			let guardian = ensure_signed(origin)?;
			Self::ensure_guardian(&guardian)?;
			Self::ensure_not_paused()?;

			let mut record =
				PendingDeposits::<T>::get(deposit_id).ok_or(Error::<T>::DepositNotPending)?;

			ensure!(record.status == VoteStatus::Pending, Error::<T>::DepositAlreadyFinalized);

			ensure!(
				!Self::is_proposal_expired(record.proposed_at_block),
				Error::<T>::ProposalExpired
			);

			ensure!(
				!record.approve_votes.contains(&guardian) && !record.deny_votes.contains(&guardian),
				Error::<T>::AlreadyVoted
			);

			if approve {
				record.approve_votes.insert(guardian.clone());
			} else {
				record.deny_votes.insert(guardian.clone());
			}

			Self::deposit_event(Event::DepositAttested { deposit_id, guardian, approved: approve });

			let vote_status =
				Self::check_vote_thresholds(record.approve_votes.len(), record.deny_votes.len());

			match vote_status {
				VoteStatus::Approved => {
					Self::finalize_deposit(deposit_id, record)?;
				},
				VoteStatus::Denied => {
					Self::deny_deposit(deposit_id, record)?;
				},
				_ => {
					PendingDeposits::<T>::insert(deposit_id, record);
				},
			}

			Ok(())
		}

		/// User requests to unlock (burn) halpha to receive alpha on Bittensor
		///
		/// # Arguments
		/// * `origin` - Must be signed by the user
		/// * `amount` - Amount of halpha to burn
		/// * `bittensor_coldkey` - Destination address on Bittensor chain
		#[pallet::call_index(2)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::request_unlock())]
		pub fn request_unlock(
			origin: OriginFor<T>,
			amount: u128,
			bittensor_coldkey: T::AccountId,
		) -> DispatchResult {
			let requester = ensure_signed(origin)?;
			Self::ensure_not_paused()?;

			let balance_value = Self::amount_to_balance(amount)?;
			let pallet_account = Self::account_id();

			<pallet_balances::Pallet<T> as Currency<T::AccountId>>::transfer(
				&requester,
				&pallet_account,
				balance_value,
				ExistenceRequirement::AllowDeath,
			)?;

			let burn_id = Self::generate_burn_id();

			let record = UnlockRecord {
				requester: requester.clone(),
				amount,
				bittensor_coldkey: bittensor_coldkey.clone(),
				approve_votes: BTreeSet::new(),
				deny_votes: BTreeSet::new(),
				requested_at_block: frame_system::Pallet::<T>::block_number(),
				status: VoteStatus::Pending,
			};

			PendingUnlocks::<T>::insert(burn_id, record);

			Self::deposit_event(Event::UnlockRequested {
				burn_id,
				requester,
				amount,
				bittensor_coldkey,
			});

			Ok(())
		}

		/// Guardian attests (votes on) a pending unlock request
		///
		/// # Arguments
		/// * `origin` - Must be signed by a guardian
		/// * `burn_id` - The burn ID to vote on
		/// * `approve` - True to approve, false to deny
		#[pallet::call_index(3)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::attest_unlock())]
		pub fn attest_unlock(
			origin: OriginFor<T>,
			burn_id: BurnId,
			approve: bool,
		) -> DispatchResult {
			let guardian = ensure_signed(origin)?;
			Self::ensure_guardian(&guardian)?;
			Self::ensure_not_paused()?;

			let mut record =
				PendingUnlocks::<T>::get(burn_id).ok_or(Error::<T>::UnlockNotPending)?;

			ensure!(record.status == VoteStatus::Pending, Error::<T>::UnlockAlreadyFinalized);

			ensure!(
				!Self::is_proposal_expired(record.requested_at_block),
				Error::<T>::ProposalExpired
			);

			ensure!(
				!record.approve_votes.contains(&guardian) && !record.deny_votes.contains(&guardian),
				Error::<T>::AlreadyVoted
			);

			if approve {
				record.approve_votes.insert(guardian.clone());
			} else {
				record.deny_votes.insert(guardian.clone());
			}

			Self::deposit_event(Event::UnlockAttested { burn_id, guardian, approved: approve });

			let vote_status =
				Self::check_vote_thresholds(record.approve_votes.len(), record.deny_votes.len());

			match vote_status {
				VoteStatus::Approved => {
					Self::finalize_unlock(burn_id, record)?;
				},
				VoteStatus::Denied => {
					Self::deny_unlock(burn_id, record)?;
				},
				_ => {
					// Still pending, update the record
					PendingUnlocks::<T>::insert(burn_id, record);
				},
			}

			Ok(())
		}

		/// Expire a pending deposit proposal that has exceeded its TTL
		/// Can be called by anyone to clean up stale proposals
		///
		/// # Arguments
		/// * `origin` - Can be any signed account
		/// * `deposit_id` - The deposit ID to expire
		#[pallet::call_index(4)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::expire_pending_deposit())]
		pub fn expire_pending_deposit(
			origin: OriginFor<T>,
			deposit_id: DepositId,
		) -> DispatchResult {
			ensure_signed(origin)?;

			let record =
				PendingDeposits::<T>::get(deposit_id).ok_or(Error::<T>::DepositNotPending)?;

			ensure!(record.status == VoteStatus::Pending, Error::<T>::DepositAlreadyFinalized);

			ensure!(
				Self::is_proposal_expired(record.proposed_at_block),
				Error::<T>::ProposalNotExpired
			);

			PendingDeposits::<T>::remove(deposit_id);

			ProcessedDeposits::<T>::insert(deposit_id, true);

			Self::deposit_event(Event::DepositExpired { deposit_id });

			Ok(())
		}

		/// Expire a pending unlock request that has exceeded its TTL
		/// Restores locked funds to the user. Can be called by anyone.
		///
		/// # Arguments
		/// * `origin` - Can be any signed account
		/// * `burn_id` - The burn ID to expire
		#[pallet::call_index(5)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::expire_pending_unlock())]
		pub fn expire_pending_unlock(origin: OriginFor<T>, burn_id: BurnId) -> DispatchResult {
			ensure_signed(origin)?;

			let record = PendingUnlocks::<T>::get(burn_id).ok_or(Error::<T>::UnlockNotPending)?;

			ensure!(record.status == VoteStatus::Pending, Error::<T>::UnlockAlreadyFinalized);

			ensure!(
				Self::is_proposal_expired(record.requested_at_block),
				Error::<T>::ProposalNotExpired
			);

			let balance_value = Self::amount_to_balance(record.amount)?;
			let pallet_account = Self::account_id();

			<pallet_balances::Pallet<T> as Currency<T::AccountId>>::transfer(
				&pallet_account,
				&record.requester,
				balance_value,
				ExistenceRequirement::AllowDeath,
			)?;

			PendingUnlocks::<T>::remove(burn_id);

			Self::deposit_event(Event::UnlockExpired { burn_id });

			Ok(())
		}

		/// Add a new guardian to the set (sudo/root only)
		///
		/// # Arguments
		/// * `origin` - Must be root
		/// * `guardian` - Account to add as guardian
		#[pallet::call_index(6)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::add_guardian(Guardians::<T>::get().len() as u32))]
		pub fn add_guardian(origin: OriginFor<T>, guardian: T::AccountId) -> DispatchResult {
			ensure_root(origin)?;

			Guardians::<T>::try_mutate(|guardians| -> DispatchResult {
				ensure!(!guardians.contains(&guardian), Error::<T>::GuardianAlreadyExists);
				guardians.push(guardian.clone());
				Self::deposit_event(Event::GuardianAdded { guardian });
				Ok(())
			})
		}

		/// Remove a guardian from the set (sudo/root only)
		///
		/// # Arguments
		/// * `origin` - Must be root
		/// * `guardian` - Account to remove from guardians
		#[pallet::call_index(7)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::remove_guardian(Guardians::<T>::get().len() as u32))]
		pub fn remove_guardian(origin: OriginFor<T>, guardian: T::AccountId) -> DispatchResult {
			ensure_root(origin)?;

			Guardians::<T>::try_mutate(|guardians| -> DispatchResult {
				let position = guardians
					.iter()
					.position(|g| g == &guardian)
					.ok_or(Error::<T>::GuardianNotFound)?;

				guardians.remove(position);

				// Verify thresholds are still achievable
				let guardian_count = guardians.len() as u16;
				let approve_threshold = ApproveThreshold::<T>::get();
				let deny_threshold = DenyThreshold::<T>::get();

				ensure!(
					guardian_count >= approve_threshold && guardian_count >= deny_threshold,
					Error::<T>::ThresholdTooHigh
				);

				Self::deposit_event(Event::GuardianRemoved { guardian });
				Ok(())
			})
		}

		/// Set the approve threshold for proposals (sudo/root only)
		///
		/// # Arguments
		/// * `origin` - Must be root
		/// * `threshold` - Minimum number of approve votes needed
		#[pallet::call_index(8)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::set_approve_threshold())]
		pub fn set_approve_threshold(origin: OriginFor<T>, threshold: u16) -> DispatchResult {
			ensure_root(origin)?;

			ensure!(threshold > 0, Error::<T>::ThresholdTooLow);

			let guardian_count = Guardians::<T>::get().len() as u16;
			ensure!(threshold <= guardian_count, Error::<T>::ThresholdTooHigh);

			ApproveThreshold::<T>::put(threshold);
			Self::deposit_event(Event::ApproveThresholdUpdated { new_threshold: threshold });

			Ok(())
		}

		/// Set the deny threshold for proposals (sudo/root only)
		///
		/// # Arguments
		/// * `origin` - Must be root
		/// * `threshold` - Minimum number of deny votes needed
		#[pallet::call_index(9)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::set_deny_threshold())]
		pub fn set_deny_threshold(origin: OriginFor<T>, threshold: u16) -> DispatchResult {
			ensure_root(origin)?;

			ensure!(threshold > 0, Error::<T>::ThresholdTooLow);

			let guardian_count = Guardians::<T>::get().len() as u16;
			ensure!(threshold <= guardian_count, Error::<T>::ThresholdTooHigh);

			DenyThreshold::<T>::put(threshold);
			Self::deposit_event(Event::DenyThresholdUpdated { new_threshold: threshold });

			Ok(())
		}

		/// Pause or unpause the bridge (sudo/root only)
		///
		/// # Arguments
		/// * `origin` - Must be root
		/// * `paused` - True to pause, false to unpause
		#[pallet::call_index(10)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::set_paused())]
		pub fn set_paused(origin: OriginFor<T>, paused: bool) -> DispatchResult {
			ensure_root(origin)?;

			Paused::<T>::put(paused);
			Self::deposit_event(Event::BridgePaused { paused });

			Ok(())
		}

		/// Set the global mint cap (sudo/root only)
		///
		/// # Arguments
		/// * `origin` - Must be root
		/// * `cap` - Maximum total halpha that can be minted
		#[pallet::call_index(11)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::set_global_mint_cap())]
		pub fn set_global_mint_cap(origin: OriginFor<T>, cap: u128) -> DispatchResult {
			ensure_root(origin)?;

			let total_minted = TotalMintedByBridge::<T>::get();
			ensure!(cap >= total_minted, Error::<T>::GlobalMintCapExceeded);

			GlobalMintCap::<T>::put(cap);
			Self::deposit_event(Event::GlobalMintCapUpdated { new_cap: cap });

			Ok(())
		}

		/// Set the signature TTL (time-to-live) in blocks (sudo/root only)
		///
		/// # Arguments
		/// * `origin` - Must be root
		/// * `blocks` - Number of blocks before proposals expire
		#[pallet::call_index(12)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::set_signature_ttl())]
		pub fn set_signature_ttl(
			origin: OriginFor<T>,
			blocks: BlockNumberFor<T>,
		) -> DispatchResult {
			ensure_root(origin)?;

			ensure!(blocks > 0u32.into(), Error::<T>::ThresholdTooLow);

			SignatureTTLBlocks::<T>::put(blocks);
			Self::deposit_event(Event::SignatureTTLUpdated { new_ttl: blocks });

			Ok(())
		}
	}

	impl<T: Config> Pallet<T> {
		/// Get the pallet's account ID
		pub fn account_id() -> T::AccountId {
			<T as pallet::Config>::PalletId::get().into_account_truncating()
		}

		/// Get the current balance of the Bridge pallet
		pub fn balance() -> BalanceOf<T> {
			pallet_balances::Pallet::<T>::free_balance(&Self::account_id())
		}

		/// Ensure the caller is a guardian
		pub fn ensure_guardian(account: &T::AccountId) -> DispatchResult {
			ensure!(Guardians::<T>::get().contains(account), Error::<T>::NotGuardian);
			Ok(())
		}

		/// Ensure the bridge is not paused
		pub fn ensure_not_paused() -> DispatchResult {
			ensure!(!Paused::<T>::get(), Error::<T>::BridgePaused);
			Ok(())
		}

		/// Check if minting the specified amount would exceed the global mint cap
		pub fn check_mint_cap(amount: u128) -> DispatchResult {
			let total_minted = TotalMintedByBridge::<T>::get();
			let mint_cap = GlobalMintCap::<T>::get();
			ensure!(
				total_minted.saturating_add(amount) <= mint_cap,
				Error::<T>::GlobalMintCapExceeded
			);
			Ok(())
		}

		/// Check if a proposal has expired based on its creation block
		pub fn is_proposal_expired(proposed_at: BlockNumberFor<T>) -> bool {
			let current_block = frame_system::Pallet::<T>::block_number();
			let ttl = SignatureTTLBlocks::<T>::get();
			current_block > proposed_at.saturating_add(ttl)
		}

		/// Generate a unique burn ID by incrementing the counter
		pub fn generate_burn_id() -> BurnId {
			let burn_id = NextBurnId::<T>::get();
			NextBurnId::<T>::put(burn_id.saturating_add(1));
			burn_id
		}

		/// Convert a raw amount into the runtime's balance type.
		pub fn amount_to_balance(amount: u128) -> Result<BalanceOf<T>, DispatchError> {
			amount.try_into().map_err(|_| Error::<T>::AmountConversionFailed.into())
		}

		/// Finalize a deposit by minting halpha to the recipient
		pub fn finalize_deposit(
			deposit_id: DepositId,
			record: DepositRecord<T::AccountId, BlockNumberFor<T>>,
		) -> DispatchResult {
			let recipient = record.recipient.clone();
			let amount = record.amount;

			TotalMintedByBridge::<T>::try_mutate(|total| -> DispatchResult {
				let mint_cap = GlobalMintCap::<T>::get();
				let new_total = total.saturating_add(amount);
				ensure!(new_total <= mint_cap, Error::<T>::GlobalMintCapExceeded);

				let balance_amount = Self::amount_to_balance(amount)?;
				pallet_balances::Pallet::<T>::mint_into(&recipient, balance_amount)
					.map_err(|_| Error::<T>::DepositMintFailed)?;

				*total = new_total;
				Ok(())
			})?;

			ProcessedDeposits::<T>::insert(deposit_id, true);

			PendingDeposits::<T>::remove(deposit_id);

			Self::deposit_event(Event::BridgeMinted {
				deposit_id,
				recipient: record.recipient,
				amount: record.amount,
			});

			Ok(())
		}

		/// Deny a deposit proposal
		pub fn deny_deposit(
			deposit_id: DepositId,
			record: DepositRecord<T::AccountId, BlockNumberFor<T>>,
		) -> DispatchResult {
			ProcessedDeposits::<T>::insert(deposit_id, true);
			PendingDeposits::<T>::remove(deposit_id);

			Self::deposit_event(Event::BridgeDenied {
				deposit_id,
				recipient: record.recipient,
				amount: record.amount,
			});

			Ok(())
		}

		/// Finalize an unlock by burning halpha
		pub fn finalize_unlock(
			burn_id: BurnId,
			record: UnlockRecord<T::AccountId, BlockNumberFor<T>>,
		) -> DispatchResult {
			let balance_value = Self::amount_to_balance(record.amount)?;
			let pallet_account = Self::account_id();

			// Burn tokens from escrow. Should always succeed since funds were locked
			// during request_unlock(). Failure indicates critical bug or tampering.
			let _burned_balance = pallet_balances::Pallet::<T>::burn_from(
				&pallet_account,
				balance_value,
				Preservation::Expendable,
				Precision::Exact,
				Fortitude::Polite,
			)
			.map_err(|_| Error::<T>::EscrowBalanceInsufficient)?;

			TotalMintedByBridge::<T>::mutate(|total| {
				*total = total.saturating_sub(record.amount);
			});

			PendingUnlocks::<T>::remove(burn_id);

			// Emit event
			Self::deposit_event(Event::UnlockApproved {
				burn_id,
				requester: record.requester,
				amount: record.amount,
				bittensor_coldkey: record.bittensor_coldkey,
			});

			Ok(())
		}

		/// Deny an unlock request and restore funds
		pub fn deny_unlock(
			burn_id: BurnId,
			record: UnlockRecord<T::AccountId, BlockNumberFor<T>>,
		) -> DispatchResult {
			let balance_value = Self::amount_to_balance(record.amount)?;

			let pallet_account = Self::account_id();
			<pallet_balances::Pallet<T> as Currency<T::AccountId>>::transfer(
				&pallet_account,
				&record.requester,
				balance_value,
				ExistenceRequirement::AllowDeath,
			)?;

			PendingUnlocks::<T>::remove(burn_id);

			Self::deposit_event(Event::UnlockDenied {
				burn_id,
				requester: record.requester,
				amount: record.amount,
			});

			Ok(())
		}

		/// Check vote thresholds and return the result status
		pub fn check_vote_thresholds(approve_count: usize, deny_count: usize) -> VoteStatus {
			let approve_threshold = ApproveThreshold::<T>::get() as usize;
			let deny_threshold = DenyThreshold::<T>::get() as usize;

			if approve_count >= approve_threshold {
				VoteStatus::Approved
			} else if deny_count >= deny_threshold {
				VoteStatus::Denied
			} else {
				VoteStatus::Pending
			}
		}
	}
}

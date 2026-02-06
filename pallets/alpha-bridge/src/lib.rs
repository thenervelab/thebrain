#![cfg_attr(not(feature = "std"), no_std)]

//! # Alpha Bridge Pallet
//!
//! A minimal viable bridge pallet for bridging Alpha (Bittensor subnet token) to hAlpha (Hippius).
//!
//! ## Design Principles
//! - Stateless guardians — chain is only source of truth
//! - First-attestation-creates-record — no propose/checkpoint races
//! - Symmetric recovery — both directions can refund
//! - Nonce-based unique IDs — no hash collisions
//!
//! ## Storage Model
//! - `Deposits` — Guardian-created records for crediting hAlpha (destination for Alpha deposits)
//! - `WithdrawalRequests` — User-created requests to withdraw hAlpha for Alpha (source for withdrawals)
//!
//! ## Naming Convention
//! - Users create **requests** on the chain they're leaving
//! - Guardians create the matching **record** on the chain they're entering

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

pub mod weights;

use crate::weights::WeightInfo;
use codec::{Decode, Encode, MaxEncodedLen};
use frame_support::{
	pallet_prelude::*,
	sp_runtime::traits::AccountIdConversion,
	traits::{
		fungible::Mutate,
		tokens::{Fortitude, Precision, Preservation},
		StorageVersion,
	},
	PalletId,
};
use frame_system::pallet_prelude::*;
use scale_info::TypeInfo;
use sp_core::H256;
use sp_runtime::traits::{AtLeast32BitUnsigned, Zero};
use sp_std::{collections::btree_set::BTreeSet, prelude::*};

pub use pallet::*;

/// The current storage version
const STORAGE_VERSION: StorageVersion = StorageVersion::new(0);

/// Domain separator for withdrawal request ID generation
const DOMAIN_WITHDRAWAL_REQUEST: &[u8] = b"HIPPIUS-WITHDRAWAL-REQUEST-v1";

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::traits::ReservableCurrency;

	/// Unique identifier for deposits (matches Bittensor deposit_request ID)
	pub type DepositId = H256;

	/// Unique identifier for withdrawal requests
	pub type WithdrawalRequestId = H256;

	/// Balance type from pallet_balances
	pub type BalanceOf<T> = <T as pallet_balances::Config>::Balance;

	/// Default TTL in blocks before finalized records can be cleaned up
	/// ~7 days at 6 second block times
	pub const DEFAULT_CLEANUP_TTL_BLOCKS: u32 = 100_800;

	/// Status of a deposit record (destination side)
	#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen, Default)]
	pub enum DepositStatus {
		#[default]
		Pending,     // Collecting votes for success
		Completed,   // hAlpha credited to recipient
		Cancelled,   // Admin cancelled after stuck
	}

	/// Status of a withdrawal request (source side)
	#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen, Default)]
	pub enum WithdrawalRequestStatus {
		#[default]
		Requested,   // hAlpha burned, awaiting Alpha release on Bittensor
		Failed,      // Admin marked after stuck (hAlpha manually minted back)
	}

	/// Reason for cancellation (for audit trail)
	#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
	pub enum CancelReason {
		AdminEmergency,
	}

	/// Record of a deposit (guardian-created, destination side)
	/// Guardians create this when they observe a deposit_request on Bittensor
	#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo)]
	#[scale_info(skip_type_params(T))]
	pub struct Deposit<T: Config> {
		/// The deposit request ID from Bittensor
		pub request_id: DepositId,
		/// Recipient who will receive hAlpha
		pub recipient: T::AccountId,
		/// Amount of hAlpha to credit (in halphaRao)
		pub amount: u64,
		/// Guardian votes for success
		pub votes: BTreeSet<T::AccountId>,
		/// Current status
		pub status: DepositStatus,
		/// Block when first guardian attested
		pub created_at_block: BlockNumberFor<T>,
		/// Block when deposit was finalized (Completed or Cancelled)
		pub finalized_at_block: Option<BlockNumberFor<T>>,
	}

	/// Record of a withdrawal request (user-created, source side)
	/// User creates this when they want to withdraw hAlpha for Alpha on Bittensor
	#[derive(Clone, Encode, Decode, Eq, PartialEq, RuntimeDebug, TypeInfo)]
	#[scale_info(skip_type_params(T))]
	pub struct WithdrawalRequest<T: Config> {
		/// Sender who burned hAlpha
		pub sender: T::AccountId,
		/// Recipient on Bittensor who will receive Alpha
		pub recipient: T::AccountId,
		/// Amount of hAlpha burned (in halphaRao)
		pub amount: u64,
		/// Nonce used for ID generation
		pub nonce: u64,
		/// Current status
		pub status: WithdrawalRequestStatus,
		/// Block when request was created
		pub created_at_block: BlockNumberFor<T>,
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

		/// The currency mechanism.
		type Currency: ReservableCurrency<Self::AccountId>;
	}

	// ============ Configuration Storage ============

	/// Guardian accounts authorized to attest deposits and withdrawals
	#[pallet::storage]
	#[pallet::getter(fn guardians)]
	pub type Guardians<T: Config> = StorageValue<_, Vec<T::AccountId>, ValueQuery>;

	/// Minimum guardian approvals needed to complete an action
	#[pallet::storage]
	#[pallet::getter(fn approve_threshold)]
	pub type ApproveThreshold<T: Config> = StorageValue<_, u16, ValueQuery>;

	/// Maximum allowed minted hAlpha (set via governance/sudo)
	#[pallet::storage]
	#[pallet::getter(fn global_mint_cap)]
	pub type GlobalMintCap<T: Config> = StorageValue<_, u128, ValueQuery>;

	/// Running total of all minted hAlpha (used for mint cap enforcement)
	#[pallet::storage]
	#[pallet::getter(fn total_minted_by_bridge)]
	pub type TotalMintedByBridge<T: Config> = StorageValue<_, u128, ValueQuery>;

	/// Emergency pause switch (blocks all bridge operations when true)
	#[pallet::storage]
	#[pallet::getter(fn paused)]
	pub type Paused<T: Config> = StorageValue<_, bool, ValueQuery>;

	// ============ Deposit Storage (Destination Side) ============

	/// Deposits created by guardians when they observe deposit_requests on Bittensor
	/// Key: DepositId (same as Bittensor deposit_request ID)
	#[pallet::storage]
	#[pallet::getter(fn deposits)]
	pub type Deposits<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		DepositId,
		Deposit<T>,
		OptionQuery,
	>;

	// ============ Withdrawal Request Storage (Source Side) ============

	/// Withdrawal requests created by users (hAlpha burned immediately)
	/// Key: WithdrawalRequestId
	#[pallet::storage]
	#[pallet::getter(fn withdrawal_requests)]
	pub type WithdrawalRequests<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		WithdrawalRequestId,
		WithdrawalRequest<T>,
		OptionQuery,
	>;

	/// Nonce for generating unique withdrawal request IDs
	#[pallet::storage]
	#[pallet::getter(fn next_withdrawal_request_nonce)]
	pub type NextWithdrawalRequestNonce<T: Config> = StorageValue<_, u64, ValueQuery>;

	/// Default value for cleanup TTL
	#[pallet::type_value]
	pub fn DefaultCleanupTTL<T: Config>() -> BlockNumberFor<T> {
		DEFAULT_CLEANUP_TTL_BLOCKS.into()
	}

	/// TTL in blocks before finalized records can be cleaned up
	/// Default: 100800 blocks (~7 days at 6s blocks)
	#[pallet::storage]
	#[pallet::getter(fn cleanup_ttl_blocks)]
	pub type CleanupTTLBlocks<T: Config> =
		StorageValue<_, BlockNumberFor<T>, ValueQuery, DefaultCleanupTTL<T>>;

	// ============ Events ============

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
	pub enum Event<T: Config> {
		// ============ Deposit Flow Events (Destination) ============

		/// Guardian attested a deposit (vote for success)
		DepositAttested {
			id: DepositId,
			guardian: T::AccountId,
		},

		/// Deposit completed - hAlpha credited to recipient
		DepositCompleted {
			id: DepositId,
			recipient: T::AccountId,
			amount: u64,
		},

		/// Deposit cancelled by admin after stuck
		DepositCancelled {
			id: DepositId,
			reason: CancelReason,
		},

		// ============ Withdrawal Flow Events (Source) ============

		/// User created a withdrawal request (hAlpha burned)
		WithdrawalRequestCreated {
			id: WithdrawalRequestId,
			sender: T::AccountId,
			recipient: T::AccountId,
			amount: u64,
		},

		/// Withdrawal request marked as failed by admin (hAlpha manually minted back)
		WithdrawalRequestFailed {
			id: WithdrawalRequestId,
		},

		// ============ Admin Events ============

		/// Admin manually minted hAlpha to a recipient (for stuck withdrawals)
		AdminManualMint {
			recipient: T::AccountId,
			amount: u64,
			/// Optional deposit ID for audit trail
			deposit_id: Option<H256>,
		},

		/// Bridge pause state changed
		BridgePaused {
			paused: bool,
		},

		/// Global mint cap updated
		GlobalMintCapUpdated {
			new_cap: u128,
		},

		/// Approve threshold updated
		ApproveThresholdUpdated {
			new_threshold: u16,
		},

		/// Guardian added to the set
		GuardianAdded {
			guardian: T::AccountId,
		},

		/// Guardian removed from the set
		GuardianRemoved {
			guardian: T::AccountId,
		},

		// ============ Cleanup Events ============

		/// Deposit record cleaned up after TTL
		DepositCleanedUp {
			id: DepositId,
		},

		/// Withdrawal request record cleaned up after TTL
		WithdrawalRequestCleanedUp {
			id: WithdrawalRequestId,
		},

		/// Cleanup TTL updated
		CleanupTTLUpdated {
			old_ttl: BlockNumberFor<T>,
			new_ttl: BlockNumberFor<T>,
		},
	}

	// ============ Errors ============

	#[pallet::error]
	pub enum Error<T> {
		/// Caller is not a guardian
		NotGuardian,
		/// Guardian has already voted on this deposit
		AlreadyVoted,
		/// User has insufficient hAlpha balance
		InsufficientBalance,
		/// Minting would exceed the global mint cap
		CapExceeded,
		/// Bridge is currently paused
		BridgePaused,
		/// Deposit not found
		DepositNotFound,
		/// Withdrawal request not found
		WithdrawalRequestNotFound,
		/// Invalid status for this operation
		InvalidStatus,
		/// Threshold cannot be zero
		ThresholdTooLow,
		/// Threshold exceeds guardian count
		ThresholdTooHigh,
		/// Guardian already exists in the set
		GuardianAlreadyExists,
		/// Guardian not found in the set
		GuardianNotFound,
		/// Failed to convert between numeric balance types
		AmountConversionFailed,
		/// Failed to mint tokens
		MintFailed,
		/// Arithmetic overflow
		ArithmeticOverflow,
		/// Deposit already completed
		DepositAlreadyCompleted,
		/// Withdrawal request already completed or failed
		WithdrawalRequestAlreadyFinalized,
		/// Deposit details do not match existing record
		InvalidDepositDetails,
		/// Amount must be greater than zero
		AmountTooSmall,
		/// Accounting underflow - indicates a bug
		AccountingUnderflow,
		/// Record is not finalized (not Completed or Cancelled)
		RecordNotFinalized,
		/// TTL has not expired yet
		TTLNotExpired,
		/// TTL must be greater than zero
		InvalidTTL,
	}

	// ============ Extrinsics ============

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		// ============ User Actions ============

		/// User burns hAlpha to initiate a withdrawal to Bittensor
		///
		/// hAlpha is burned immediately - no escrow. If the withdrawal fails,
		/// admin can manually mint hAlpha back via `admin_manual_mint`.
		///
		/// # Arguments
		/// * `origin` - Must be signed by the user
		/// * `amount` - Amount of hAlpha to burn (in halphaRao)
		/// * `recipient` - Recipient account on Bittensor to receive Alpha
		#[pallet::call_index(0)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::request_unlock())]
		pub fn withdraw(
			origin: OriginFor<T>,
			amount: u64,
			recipient: T::AccountId,
		) -> DispatchResult {
			let sender = ensure_signed(origin)?;
			Self::ensure_not_paused()?;

			// Reject zero amount
			ensure!(amount > 0, Error::<T>::AmountTooSmall);

			// Burn hAlpha from user immediately
			let balance_value = Self::amount_to_balance(amount)?;
			pallet_balances::Pallet::<T>::burn_from(
				&sender,
				balance_value,
				Preservation::Expendable,
				Precision::Exact,
				Fortitude::Polite,
			).map_err(|_| Error::<T>::InsufficientBalance)?;

			// Update total minted tracking (use checked_sub to catch accounting bugs)
			TotalMintedByBridge::<T>::try_mutate(|total| -> DispatchResult {
				*total = total.checked_sub(amount as u128).ok_or(Error::<T>::AccountingUnderflow)?;
				Ok(())
			})?;

			// Generate unique withdrawal request ID
			let (request_id, nonce) = Self::generate_withdrawal_request_id(&sender, &recipient, amount);

			// Create withdrawal request
			let request = WithdrawalRequest {
				sender: sender.clone(),
				recipient: recipient.clone(),
				amount,
				nonce,
				status: WithdrawalRequestStatus::Requested,
				created_at_block: frame_system::Pallet::<T>::block_number(),
			};

			WithdrawalRequests::<T>::insert(request_id, request);

			Self::deposit_event(Event::WithdrawalRequestCreated {
				id: request_id,
				sender,
				recipient,
				amount,
			});

			Ok(())
		}

		// ============ Guardian Actions ============

		/// Guardian attests a deposit (first attestation creates the record)
		///
		/// When guardians observe a deposit_request on Bittensor, they call this
		/// to vote for crediting hAlpha. First attestation creates the Deposit record.
		/// When threshold is reached, hAlpha is credited to recipient.
		///
		/// # Arguments
		/// * `origin` - Must be signed by a guardian
		/// * `request_id` - The deposit request ID from Bittensor
		/// * `recipient` - Recipient to credit hAlpha to
		/// * `amount` - Amount to credit (in halphaRao)
		#[pallet::call_index(1)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::attest_deposit())]
		pub fn attest_deposit(
			origin: OriginFor<T>,
			request_id: DepositId,
			recipient: T::AccountId,
			amount: u64,
		) -> DispatchResult {
			let guardian = ensure_signed(origin)?;
			Self::ensure_guardian(&guardian)?;
			Self::ensure_not_paused()?;

			// Check if deposit already exists
			if let Some(mut deposit) = Deposits::<T>::get(request_id) {
				// Deposit exists - verify details match to prevent poisoning
				ensure!(deposit.recipient == recipient, Error::<T>::InvalidDepositDetails);
				ensure!(deposit.amount == amount, Error::<T>::InvalidDepositDetails);

				// Check status and vote
				ensure!(deposit.status == DepositStatus::Pending, Error::<T>::DepositAlreadyCompleted);
				ensure!(!deposit.votes.contains(&guardian), Error::<T>::AlreadyVoted);

				deposit.votes.insert(guardian.clone());

				Self::deposit_event(Event::DepositAttested {
					id: request_id,
					guardian,
				});

				// Check if threshold reached
				if deposit.votes.len() >= ApproveThreshold::<T>::get() as usize {
					Self::finalize_deposit(request_id, deposit)?;
				} else {
					Deposits::<T>::insert(request_id, deposit);
				}
			} else {
				// First attestation - create the deposit record
				let mut votes = BTreeSet::new();
				votes.insert(guardian.clone());

				let deposit = Deposit {
					request_id,
					recipient: recipient.clone(),
					amount,
					votes,
					status: DepositStatus::Pending,
					created_at_block: frame_system::Pallet::<T>::block_number(),
					finalized_at_block: None,
				};

				Self::deposit_event(Event::DepositAttested {
					id: request_id,
					guardian,
				});

				// Check if threshold reached immediately (single guardian setup)
				if deposit.votes.len() >= ApproveThreshold::<T>::get() as usize {
					Self::finalize_deposit(request_id, deposit)?;
				} else {
					Deposits::<T>::insert(request_id, deposit);
				}
			}

			Ok(())
		}

		// ============ Admin Functions ============

		/// Admin cancels a deposit that is stuck (Pending but not reaching threshold)
		///
		/// NOTE: Intentionally does not check pause state — admin must operate during emergencies.
		///
		/// # Arguments
		/// * `origin` - Must be root
		/// * `request_id` - The deposit ID to cancel
		/// * `reason` - Reason for cancellation
		#[pallet::call_index(2)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::expire_pending_deposit())]
		pub fn admin_cancel_deposit(
			origin: OriginFor<T>,
			request_id: DepositId,
			reason: CancelReason,
		) -> DispatchResult {
			ensure_root(origin)?;

			Deposits::<T>::try_mutate(request_id, |maybe_deposit| -> DispatchResult {
				let deposit = maybe_deposit.as_mut().ok_or(Error::<T>::DepositNotFound)?;
				ensure!(deposit.status == DepositStatus::Pending, Error::<T>::InvalidStatus);

				deposit.finalized_at_block = Some(frame_system::Pallet::<T>::block_number());
				deposit.status = DepositStatus::Cancelled;

				Self::deposit_event(Event::DepositCancelled {
					id: request_id,
					reason,
				});

				Ok(())
			})
		}

		/// Admin marks a withdrawal request as failed and manually mints hAlpha back
		///
		/// This restores the hAlpha that was burned during withdraw(). The mint cap
		/// check and TotalMintedByBridge update are performed to maintain accounting.
		///
		/// NOTE: Intentionally does not check pause state — admin must operate during emergencies.
		///
		/// # Arguments
		/// * `origin` - Must be root
		/// * `request_id` - The withdrawal request ID to fail
		#[pallet::call_index(3)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::expire_pending_unlock())]
		pub fn admin_fail_withdrawal_request(
			origin: OriginFor<T>,
			request_id: WithdrawalRequestId,
		) -> DispatchResult {
			ensure_root(origin)?;

			let request = WithdrawalRequests::<T>::get(request_id)
				.ok_or(Error::<T>::WithdrawalRequestNotFound)?;
			ensure!(
				request.status == WithdrawalRequestStatus::Requested,
				Error::<T>::WithdrawalRequestAlreadyFinalized
			);

			// Check and update mint cap (this is restoring burned hAlpha, so it must fit in cap)
			TotalMintedByBridge::<T>::try_mutate(|total| -> DispatchResult {
				let mint_cap = GlobalMintCap::<T>::get();
				let amount_u128 = request.amount as u128;
				let new_total = total.checked_add(amount_u128).ok_or(Error::<T>::ArithmeticOverflow)?;
				ensure!(new_total <= mint_cap, Error::<T>::CapExceeded);
				*total = new_total;
				Ok(())
			})?;

			// Mint hAlpha back to sender
			Self::mint_to_recipient(&request.sender, request.amount)?;

			// Update status
			WithdrawalRequests::<T>::mutate(request_id, |maybe_request| {
				if let Some(req) = maybe_request {
					req.status = WithdrawalRequestStatus::Failed;
				}
			});

			Self::deposit_event(Event::WithdrawalRequestFailed { id: request_id });
			Self::deposit_event(Event::AdminManualMint {
				recipient: request.sender,
				amount: request.amount,
				deposit_id: None,
			});

			Ok(())
		}

		/// Admin manually mints hAlpha to a recipient (for emergency recovery)
		///
		/// WARNING: This mints new hAlpha that wasn't part of a deposit flow.
		/// Only use for emergency recovery. The amount counts toward the mint cap.
		///
		/// NOTE: Intentionally does not check pause state — admin must operate during emergencies.
		///
		/// # Arguments
		/// * `origin` - Must be root
		/// * `recipient` - Account to receive hAlpha
		/// * `amount` - Amount to mint (in halphaRao)
		/// * `deposit_id` - Optional deposit ID for audit trail
		#[pallet::call_index(4)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::set_global_mint_cap())]
		pub fn admin_manual_mint(
			origin: OriginFor<T>,
			recipient: T::AccountId,
			amount: u64,
			deposit_id: Option<H256>,
		) -> DispatchResult {
			ensure_root(origin)?;

			// Check and update mint cap
			TotalMintedByBridge::<T>::try_mutate(|total| -> DispatchResult {
				let mint_cap = GlobalMintCap::<T>::get();
				let amount_u128 = amount as u128;
				let new_total = total.checked_add(amount_u128).ok_or(Error::<T>::ArithmeticOverflow)?;
				ensure!(new_total <= mint_cap, Error::<T>::CapExceeded);
				*total = new_total;
				Ok(())
			})?;

			Self::mint_to_recipient(&recipient, amount)?;

			Self::deposit_event(Event::AdminManualMint { recipient, amount, deposit_id });

			Ok(())
		}

		/// Add a new guardian to the set (sudo/root only)
		///
		/// # Arguments
		/// * `origin` - Must be root
		/// * `guardian` - Account to add as guardian
		#[pallet::call_index(5)]
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
		#[pallet::call_index(6)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::remove_guardian(Guardians::<T>::get().len() as u32))]
		pub fn remove_guardian(origin: OriginFor<T>, guardian: T::AccountId) -> DispatchResult {
			ensure_root(origin)?;

			Guardians::<T>::try_mutate(|guardians| -> DispatchResult {
				let position = guardians
					.iter()
					.position(|g| g == &guardian)
					.ok_or(Error::<T>::GuardianNotFound)?;

				guardians.remove(position);

				// Verify threshold is still achievable
				let guardian_count = guardians.len() as u16;
				let approve_threshold = ApproveThreshold::<T>::get();

				ensure!(
					guardian_count >= approve_threshold,
					Error::<T>::ThresholdTooHigh
				);

				Self::deposit_event(Event::GuardianRemoved { guardian });
				Ok(())
			})
		}

		/// Set the approve threshold (sudo/root only)
		///
		/// # Arguments
		/// * `origin` - Must be root
		/// * `threshold` - Minimum number of guardian votes needed
		#[pallet::call_index(7)]
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

		/// Pause or unpause the bridge (sudo/root only)
		///
		/// # Arguments
		/// * `origin` - Must be root
		/// * `paused` - True to pause, false to unpause
		#[pallet::call_index(8)]
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
		/// * `cap` - Maximum total hAlpha that can be minted
		#[pallet::call_index(9)]
		#[pallet::weight(<T as pallet::Config>::WeightInfo::set_global_mint_cap())]
		pub fn set_global_mint_cap(origin: OriginFor<T>, cap: u128) -> DispatchResult {
			ensure_root(origin)?;

			let total_minted = TotalMintedByBridge::<T>::get();
			ensure!(cap >= total_minted, Error::<T>::CapExceeded);

			GlobalMintCap::<T>::put(cap);
			Self::deposit_event(Event::GlobalMintCapUpdated { new_cap: cap });

			Ok(())
		}

		// ============ Cleanup Functions ============

		/// Anyone can cleanup a finalized deposit after TTL
		///
		/// # Arguments
		/// * `origin` - Must be signed (anyone can call)
		/// * `deposit_id` - The deposit ID to cleanup
		#[pallet::call_index(10)]
		#[pallet::weight(10_000)]
		pub fn cleanup_deposit(
			origin: OriginFor<T>,
			deposit_id: DepositId,
		) -> DispatchResult {
			ensure_signed(origin)?;  // Anyone can call

			let deposit = Deposits::<T>::get(deposit_id)
				.ok_or(Error::<T>::DepositNotFound)?;

			// Must be finalized (Completed or Cancelled)
			ensure!(
				deposit.status == DepositStatus::Completed ||
				deposit.status == DepositStatus::Cancelled,
				Error::<T>::RecordNotFinalized
			);

			// Must have finalized_at_block set
			let finalized_at = deposit.finalized_at_block
				.ok_or(Error::<T>::RecordNotFinalized)?;

			// TTL must have passed since finalization
			let current_block = frame_system::Pallet::<T>::block_number();
			let ttl = CleanupTTLBlocks::<T>::get();
			ensure!(current_block >= finalized_at + ttl, Error::<T>::TTLNotExpired);

			// Remove from storage
			Deposits::<T>::remove(deposit_id);

			Self::deposit_event(Event::DepositCleanedUp { id: deposit_id });

			Ok(())
		}

		/// Anyone can cleanup a withdrawal request after TTL (no status check for source records)
		///
		/// # Arguments
		/// * `origin` - Must be signed (anyone can call)
		/// * `request_id` - The withdrawal request ID to cleanup
		#[pallet::call_index(11)]
		#[pallet::weight(10_000)]
		pub fn cleanup_withdrawal_request(
			origin: OriginFor<T>,
			request_id: WithdrawalRequestId,
		) -> DispatchResult {
			ensure_signed(origin)?;  // Anyone can call

			let request = WithdrawalRequests::<T>::get(request_id)
				.ok_or(Error::<T>::WithdrawalRequestNotFound)?;

			// TTL must have passed since creation (no status check for source records)
			let current_block = frame_system::Pallet::<T>::block_number();
			let ttl = CleanupTTLBlocks::<T>::get();
			ensure!(current_block >= request.created_at_block + ttl, Error::<T>::TTLNotExpired);

			// Remove from storage
			WithdrawalRequests::<T>::remove(request_id);

			Self::deposit_event(Event::WithdrawalRequestCleanedUp { id: request_id });

			Ok(())
		}

		/// Admin sets the cleanup TTL (in blocks)
		///
		/// # Arguments
		/// * `origin` - Must be root
		/// * `ttl_blocks` - TTL in blocks before finalized records can be cleaned up
		#[pallet::call_index(12)]
		#[pallet::weight(10_000)]
		pub fn set_cleanup_ttl(
			origin: OriginFor<T>,
			ttl_blocks: BlockNumberFor<T>,
		) -> DispatchResult {
			ensure_root(origin)?;
			ensure!(ttl_blocks > Zero::zero(), Error::<T>::InvalidTTL);

			let old_ttl = CleanupTTLBlocks::<T>::get();
			CleanupTTLBlocks::<T>::put(ttl_blocks);

			Self::deposit_event(Event::CleanupTTLUpdated { old_ttl, new_ttl: ttl_blocks });

			Ok(())
		}
	}

	// ============ Helper Functions ============

	impl<T: Config> Pallet<T> {
		/// Get the pallet's account ID
		pub fn account_id() -> T::AccountId {
			<T as pallet::Config>::PalletId::get().into_account_truncating()
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

		/// Convert a raw amount into the runtime's balance type
		pub fn amount_to_balance(amount: u64) -> Result<BalanceOf<T>, DispatchError> {
			amount.try_into().map_err(|_| Error::<T>::AmountConversionFailed.into())
		}

		/// Generate a unique withdrawal request ID using nonce
		pub fn generate_withdrawal_request_id(
			sender: &T::AccountId,
			recipient: &T::AccountId,
			amount: u64,
		) -> (WithdrawalRequestId, u64) {
			let nonce = NextWithdrawalRequestNonce::<T>::get();
			NextWithdrawalRequestNonce::<T>::put(nonce.saturating_add(1));

			let mut data = Vec::new();
			data.extend_from_slice(DOMAIN_WITHDRAWAL_REQUEST);
			data.extend_from_slice(&sender.encode());
			data.extend_from_slice(&recipient.encode());
			data.extend_from_slice(&amount.to_le_bytes());
			data.extend_from_slice(&nonce.to_le_bytes());

			(H256::from(sp_core::hashing::blake2_256(&data)), nonce)
		}

		/// Finalize a deposit by minting hAlpha to the recipient
		fn finalize_deposit(
			deposit_id: DepositId,
			mut deposit: Deposit<T>,
		) -> DispatchResult {
			let recipient = deposit.recipient.clone();
			let amount = deposit.amount;

			// Check and update mint cap
			TotalMintedByBridge::<T>::try_mutate(|total| -> DispatchResult {
				let mint_cap = GlobalMintCap::<T>::get();
				let amount_u128 = amount as u128;
				let new_total = total.checked_add(amount_u128).ok_or(Error::<T>::ArithmeticOverflow)?;
				ensure!(new_total <= mint_cap, Error::<T>::CapExceeded);
				*total = new_total;
				Ok(())
			})?;

			// Mint hAlpha to recipient
			Self::mint_to_recipient(&recipient, amount)?;

			// Update deposit status
			deposit.finalized_at_block = Some(frame_system::Pallet::<T>::block_number());
			deposit.status = DepositStatus::Completed;
			Deposits::<T>::insert(deposit_id, deposit);

			Self::deposit_event(Event::DepositCompleted {
				id: deposit_id,
				recipient,
				amount,
			});

			Ok(())
		}

		/// Mint hAlpha to a recipient
		fn mint_to_recipient(recipient: &T::AccountId, amount: u64) -> DispatchResult {
			let balance_amount = Self::amount_to_balance(amount)?;
			pallet_balances::Pallet::<T>::mint_into(recipient, balance_amount)
				.map_err(|_| Error::<T>::MintFailed)?;
			Ok(())
		}
	}
}

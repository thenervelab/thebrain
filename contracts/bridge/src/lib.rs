#![cfg_attr(not(feature = "std"), no_std, no_main)]

//! # Minimal Viable Bridge Contract
//!
//! A simplified bridge contract for bridging Alpha (Bittensor subnet token) to hAlpha (Hippius).
//!
//! ## Design Principles
//! - Stateless guardians — chain is only source of truth
//! - First-attestation-creates-record — no propose/checkpoint races
//! - Symmetric recovery — both directions can refund
//! - Nonce-based unique IDs — no hash collisions
//!
//! ## Storage Model
//! - `deposit_requests` — User-created requests to deposit Alpha for hAlpha (source side)
//! - `withdrawals` — Guardian-created records for releasing Alpha (destination side)
//!
//! ## Naming Convention
//! - Users create **requests** on the chain they're leaving
//! - Guardians create the matching **record** on the chain they're entering

pub mod chain_extension;
pub mod errors;
pub mod events;
pub mod runtime;
pub mod types;

use errors::Error;
use events::*;
use runtime::{AlphaCurrency, NetUid, ProxyCall, RuntimeCall, SubtensorCall};
use sp_runtime::MultiAddress;
use types::*;

#[derive(Clone)]
#[ink::scale_derive(Encode, Decode, TypeInfo)]
pub enum SubtensorEnvironment {}

impl ink::env::Environment for SubtensorEnvironment {
	const MAX_EVENT_TOPICS: usize = 4;
	type AccountId = ink::primitives::AccountId;
	type Balance = u64;
	type Hash = ink::primitives::Hash;
	type Timestamp = u64;
	type BlockNumber = u32;
	type ChainExtension = chain_extension::SubtensorExtension;
}

#[ink::contract(env = crate::SubtensorEnvironment)]
mod bridge {
	use super::*;
	use ink::env::hash::{Blake2x256, CryptoHash, HashOutput};
	use ink::prelude::{boxed::Box, vec, vec::Vec};
	use ink::storage::Mapping;

	#[ink(storage)]
	pub struct Bridge {
		// ============ Configuration ============
		owner: AccountId,
		chain_id: ChainId,
		contract_hotkey: AccountId,
		paused: bool,

		guardians: Vec<AccountId>,
		approve_threshold: u16,

		min_deposit_amount: Balance,

		// ============ Deposit Requests (Source Side) ============
		/// Nonce for generating unique deposit request IDs
		next_deposit_nonce: DepositNonce,
		/// User-created deposit requests (Alpha locked, awaiting hAlpha on Hippius)
		deposit_requests: Mapping<DepositRequestId, DepositRequest>,
		/// Map nonce to deposit request ID for recovery
		nonce_to_deposit_request_id: Mapping<DepositNonce, DepositRequestId>,

		// ============ Withdrawals (Destination Side) ============
		/// Guardian-created withdrawal records (Hippius withdrawal observed, release Alpha)
		withdrawals: Mapping<WithdrawalId, Withdrawal>,

		// ============ Cleanup Configuration ============
		/// TTL in blocks for cleaning up finalized records (~7 days at 6s blocks = 100800)
		cleanup_ttl_blocks: BlockNumber,
	}

	impl Bridge {
		/// Creates a new bridge contract instance
		///
		/// # Arguments
		/// * `owner` - Contract owner with admin privileges
		/// * `chain_id` - Identifier for the Bittensor chain (used in ID generation)
		/// * `hotkey` - Contract's hotkey for stake consolidation
		#[ink(constructor)]
		pub fn new(owner: AccountId, chain_id: ChainId, hotkey: AccountId) -> Self {
			Self {
				owner,
				chain_id,
				contract_hotkey: hotkey,
				paused: false,
				guardians: Vec::new(),
				approve_threshold: 0,
				min_deposit_amount: 1_000_000_000, // 1 Alpha (in alphaRao)
				next_deposit_nonce: 0,
				deposit_requests: Mapping::default(),
				nonce_to_deposit_request_id: Mapping::default(),
				withdrawals: Mapping::default(),
				cleanup_ttl_blocks: 100_800, // ~7 days at 6s blocks
			}
		}

		// ============ User Actions ============

		/// User locks Alpha to create a deposit request
		///
		/// Creates a deposit request that guardians will observe and attest on Hippius.
		/// The caller MUST have added this contract as a proxy on Bittensor.
		///
		/// # Arguments
		/// * `amount` - Amount of Alpha to lock (in alphaRao)
		/// * `recipient` - Recipient on Hippius who will receive hAlpha
		/// * `hotkey` - Hotkey where the stake is currently held
		/// * `netuid` - Network UID (must match chain_id)
		#[ink(message)]
		pub fn deposit(
			&mut self,
			amount: Balance,
			recipient: AccountId,
			hotkey: AccountId,
			netuid: NetUid,
		) -> Result<DepositRequestId, Error> {
			self.ensure_not_paused()?;

			if netuid.as_u16() != self.chain_id {
				return Err(Error::InvalidNetUid);
			}

			if amount < self.min_deposit_amount {
				return Err(Error::AmountTooSmall);
			}

			let sender = self.env().caller();
			let contract_account = self.env().account_id();

			// Verify sender has sufficient stake
			let sender_stake_before = self.get_stake_amount(sender, hotkey, netuid)?;
			if sender_stake_before < amount {
				return Err(Error::InsufficientStake);
			}

			let contract_stake_before = self.get_stake_amount(contract_account, hotkey, netuid)?;

			// Transfer stake from user to contract via proxy
			let transfer_call = RuntimeCall::SubtensorModule(SubtensorCall::TransferStake {
				destination_coldkey: contract_account,
				hotkey,
				origin_netuid: netuid,
				destination_netuid: netuid,
				alpha_amount: AlphaCurrency::from(amount),
			});

			let proxy_call = RuntimeCall::Proxy(ProxyCall::Proxy {
				real: MultiAddress::Id(sender),
				force_proxy_type: None,
				call: Box::new(transfer_call),
			});

			self.env().call_runtime(&proxy_call).map_err(|_| Error::RuntimeCallFailed)?;

			// Verify transfer
			let sender_stake_after = self.get_stake_amount(sender, hotkey, netuid)?;
			let contract_stake_after = self.get_stake_amount(contract_account, hotkey, netuid)?;

			let sender_decrease = sender_stake_before.saturating_sub(sender_stake_after);
			let contract_increase = contract_stake_after.saturating_sub(contract_stake_before);

			let sender_ok = sender_decrease >= amount.saturating_sub(TRANSFER_TOLERANCE)
				&& sender_decrease <= amount;
			let contract_ok = contract_increase >= amount.saturating_sub(TRANSFER_TOLERANCE)
				&& contract_increase <= amount;

			if !sender_ok || !contract_ok {
				return Err(Error::TransferNotVerified);
			}

			// Consolidate stake to contract's hotkey if needed
			let contract_hk = self.contract_hotkey;
			if hotkey != contract_hk {
				let contract_stake_on_user_hotkey =
					self.get_stake_amount(contract_account, hotkey, netuid)?;

				if contract_stake_on_user_hotkey > 0 {
					self.env()
						.extension()
						.move_stake(
							hotkey,
							contract_hk,
							netuid,
							netuid,
							AlphaCurrency::from(contract_stake_on_user_hotkey),
						)
						.map_err(|_| Error::StakeConsolidationFailed)?;
				}
			}

			// Generate unique deposit request ID
			let deposit_nonce = self.next_deposit_nonce;
			self.next_deposit_nonce =
				self.next_deposit_nonce.checked_add(1).ok_or(Error::Overflow)?;

			let mut canonical_bytes = Vec::new();
			canonical_bytes.extend_from_slice(DOMAIN_DEPOSIT_REQUEST);
			canonical_bytes.extend_from_slice(&self.chain_id.to_le_bytes());
			canonical_bytes.extend_from_slice(contract_account.as_ref());
			canonical_bytes.extend_from_slice(&deposit_nonce.to_le_bytes());
			canonical_bytes.extend_from_slice(sender.as_ref());
			canonical_bytes.extend_from_slice(recipient.as_ref());
			canonical_bytes.extend_from_slice(&amount.to_le_bytes());

			let deposit_request_id = {
				let mut hash_output = <<Blake2x256 as HashOutput>::Type as Default>::default();
				<Blake2x256 as CryptoHash>::hash(&canonical_bytes, &mut hash_output);
				Hash::from(hash_output)
			};

			// Create deposit request
			let request = DepositRequest {
				sender,
				recipient,
				amount,
				nonce: deposit_nonce,
				hotkey,
				netuid: netuid.as_u16(),
				status: DepositRequestStatus::Requested,
				created_at_block: self.env().block_number(),
			};

			self.deposit_requests.insert(deposit_request_id, &request);
			self.nonce_to_deposit_request_id.insert(deposit_nonce, &deposit_request_id);

			self.env().emit_event(DepositRequestCreated {
				chain_id: self.chain_id,
				escrow_contract: contract_account,
				deposit_nonce,
				sender,
				recipient,
				amount,
				deposit_request_id,
			});

			Ok(deposit_request_id)
		}

		// ============ Guardian Actions ============

		/// Guardian attests a withdrawal (first attestation creates the record)
		///
		/// When guardians observe a withdrawal_request on Hippius, they call this
		/// to vote for releasing Alpha. First attestation creates the Withdrawal record.
		/// When threshold is reached, Alpha is released to recipient.
		///
		/// # Arguments
		/// * `request_id` - The withdrawal request ID from Hippius
		/// * `recipient` - Recipient to release Alpha to
		/// * `amount` - Amount to release (in alphaRao)
		#[ink(message)]
		#[allow(clippy::cast_possible_truncation)] // vote_count bounded by MAX_GUARDIANS (10)
		pub fn attest_withdrawal(
			&mut self,
			request_id: WithdrawalId,
			recipient: AccountId,
			amount: Balance,
		) -> Result<(), Error> {
			self.ensure_not_paused()?;
			self.ensure_guardian()?;

			let caller = self.env().caller();

			if let Some(mut withdrawal) = self.withdrawals.get(request_id) {
				// Withdrawal exists - verify details match to prevent poisoning
				if withdrawal.recipient != recipient {
					return Err(Error::InvalidWithdrawalDetails);
				}
				if withdrawal.amount != amount {
					return Err(Error::InvalidWithdrawalDetails);
				}

				// Check status and vote
				if withdrawal.status != WithdrawalStatus::Pending {
					return Err(Error::WithdrawalAlreadyFinalized);
				}
				if withdrawal.votes.contains(&caller) {
					return Err(Error::AlreadyVoted);
				}

				withdrawal.votes.push(caller);

				self.env().emit_event(WithdrawalAttested {
					withdrawal_id: request_id,
					guardian: caller,
					vote_count: withdrawal.votes.len() as u16,
				});

				// Check if threshold reached
				if withdrawal.votes.len() >= self.approve_threshold as usize {
					self.finalize_withdrawal(request_id, withdrawal)?;
				} else {
					self.withdrawals.insert(request_id, &withdrawal);
				}
			} else {
				// First attestation - create the withdrawal record
				let withdrawal = Withdrawal {
					request_id,
					recipient,
					amount,
					votes: vec![caller],
					status: WithdrawalStatus::Pending,
					created_at_block: self.env().block_number(),
					finalized_at_block: None,
				};

				self.env().emit_event(WithdrawalAttested {
					withdrawal_id: request_id,
					guardian: caller,
					vote_count: 1,
				});

				// Check if threshold reached immediately (single guardian setup)
				if withdrawal.votes.len() >= self.approve_threshold as usize {
					self.finalize_withdrawal(request_id, withdrawal)?;
				} else {
					self.withdrawals.insert(request_id, &withdrawal);
				}
			}

			Ok(())
		}

		// ============ Admin Functions ============

		/// Admin marks a deposit request as failed
		///
		/// After calling this, admin should manually release Alpha via admin_manual_release.
		///
		/// NOTE: Intentionally does not check pause state — admin must operate during emergencies.
		///
		/// # Arguments
		/// * `request_id` - The deposit request ID to fail
		#[ink(message)]
		pub fn admin_fail_deposit_request(
			&mut self,
			request_id: DepositRequestId,
		) -> Result<(), Error> {
			self.ensure_owner()?;

			let mut request = self
				.deposit_requests
				.get(request_id)
				.ok_or(Error::DepositRequestNotFound)?;

			if request.status != DepositRequestStatus::Requested {
				return Err(Error::DepositRequestAlreadyFinalized);
			}

			request.status = DepositRequestStatus::Failed;
			self.deposit_requests.insert(request_id, &request);

			self.env().emit_event(DepositRequestFailed { deposit_request_id: request_id });

			Ok(())
		}

		/// Admin manually releases Alpha to a recipient (for stuck deposits)
		///
		/// NOTE: Intentionally does not check pause state — admin must operate during emergencies.
		///
		/// # Arguments
		/// * `recipient` - Account to receive Alpha
		/// * `amount` - Amount to release (in alphaRao)
		/// * `deposit_request_id` - Optional deposit request ID for audit trail
		#[ink(message)]
		pub fn admin_manual_release(
			&mut self,
			recipient: AccountId,
			amount: Balance,
			deposit_request_id: Option<DepositRequestId>,
		) -> Result<(), Error> {
			self.ensure_owner()?;

			let contract_hk = self.contract_hotkey;
			let netuid = NetUid::from(self.chain_id);

			self.env()
				.extension()
				.transfer_stake(recipient, contract_hk, netuid, netuid, AlphaCurrency::from(amount))
				.map_err(|_| Error::TransferFailed)?;

			self.env().emit_event(AdminManualRelease { recipient, amount, deposit_request_id });

			Ok(())
		}

		/// Admin cancels a withdrawal that is stuck
		///
		/// NOTE: Intentionally does not check pause state — admin must operate during emergencies.
		///
		/// # Arguments
		/// * `request_id` - The withdrawal ID to cancel
		#[ink(message)]
		pub fn admin_cancel_withdrawal(&mut self, request_id: WithdrawalId) -> Result<(), Error> {
			self.ensure_owner()?;

			let mut withdrawal =
				self.withdrawals.get(request_id).ok_or(Error::WithdrawalNotFound)?;

			if withdrawal.status != WithdrawalStatus::Pending {
				return Err(Error::WithdrawalAlreadyFinalized);
			}

			withdrawal.finalized_at_block = Some(self.env().block_number());
			withdrawal.status = WithdrawalStatus::Cancelled;
			self.withdrawals.insert(request_id, &withdrawal);

			self.env().emit_event(WithdrawalCancelled { withdrawal_id: request_id });

			Ok(())
		}

		/// Configures the guardian set and voting threshold (owner only)
		#[ink(message)]
		pub fn set_guardians_and_threshold(
			&mut self,
			guardians: Vec<AccountId>,
			approve_threshold: u16,
		) -> Result<(), Error> {
			self.ensure_owner()?;

			if guardians.len() > MAX_GUARDIANS {
				return Err(Error::TooManyGuardians);
			}

			let guardians_count =
				u16::try_from(guardians.len()).map_err(|_| Error::TooManyGuardians)?;
			if approve_threshold > guardians_count || approve_threshold == 0 {
				return Err(Error::InvalidThresholds);
			}

			self.guardians = guardians.clone();
			self.approve_threshold = approve_threshold;

			self.env().emit_event(GuardiansUpdated {
				guardians,
				approve_threshold,
				updated_by: self.env().caller(),
			});

			Ok(())
		}

		#[ink(message)]
		pub fn pause(&mut self) -> Result<(), Error> {
			self.ensure_owner()?;
			self.paused = true;
			self.env().emit_event(Paused { paused_by: self.env().caller() });
			Ok(())
		}

		#[ink(message)]
		pub fn unpause(&mut self) -> Result<(), Error> {
			self.ensure_owner()?;
			self.paused = false;
			self.env().emit_event(Unpaused { unpaused_by: self.env().caller() });
			Ok(())
		}

		#[ink(message)]
		pub fn update_owner(&mut self, new_owner: AccountId) -> Result<(), Error> {
			self.ensure_owner()?;
			let old_owner = self.owner;
			self.owner = new_owner;
			self.env().emit_event(OwnerUpdated { old_owner, new_owner });
			Ok(())
		}

		#[ink(message)]
		pub fn set_contract_hotkey(&mut self, hotkey: AccountId) -> Result<(), Error> {
			self.ensure_owner()?;
			let old_hotkey = self.contract_hotkey;
			self.contract_hotkey = hotkey;
			self.env().emit_event(ContractHotkeyUpdated {
				old_hotkey,
				new_hotkey: hotkey,
				updated_by: self.env().caller(),
			});
			Ok(())
		}

		#[ink(message)]
		pub fn set_code(&mut self, code_hash: Hash) -> Result<(), Error> {
			self.ensure_owner()?;
			self.env().set_code_hash(&code_hash).map_err(|_| Error::CodeUpgradeFailed)?;
			self.env()
				.emit_event(CodeUpgraded { code_hash, upgraded_by: self.env().caller() });
			Ok(())
		}

		// ============ Cleanup Functions ============

		/// Anyone can cleanup a deposit request after TTL (no status check for source records)
		#[ink(message)]
		pub fn cleanup_deposit_request(
			&mut self,
			request_id: DepositRequestId,
		) -> Result<(), Error> {
			let request =
				self.deposit_requests.get(request_id).ok_or(Error::DepositRequestNotFound)?;

			// TTL must have passed since creation (no status check for source records)
			let current_block = self.env().block_number();
			if current_block < request.created_at_block.saturating_add(self.cleanup_ttl_blocks) {
				return Err(Error::TTLNotExpired);
			}

			// Remove from storage
			self.deposit_requests.remove(request_id);
			self.nonce_to_deposit_request_id.remove(request.nonce);

			self.env().emit_event(DepositRequestCleanedUp { deposit_request_id: request_id });

			Ok(())
		}

		/// Anyone can cleanup a finalized withdrawal after TTL
		#[ink(message)]
		pub fn cleanup_withdrawal(&mut self, withdrawal_id: WithdrawalId) -> Result<(), Error> {
			let withdrawal =
				self.withdrawals.get(withdrawal_id).ok_or(Error::WithdrawalNotFound)?;

			// Must be finalized (Completed or Cancelled)
			if withdrawal.status != WithdrawalStatus::Completed
				&& withdrawal.status != WithdrawalStatus::Cancelled
			{
				return Err(Error::RecordNotFinalized);
			}

			// Must have finalized_at_block set
			let finalized_at = withdrawal.finalized_at_block.ok_or(Error::RecordNotFinalized)?;

			// TTL must have passed since finalization
			let current_block = self.env().block_number();
			if current_block < finalized_at.saturating_add(self.cleanup_ttl_blocks) {
				return Err(Error::TTLNotExpired);
			}

			// Remove from storage
			self.withdrawals.remove(withdrawal_id);

			self.env().emit_event(WithdrawalCleanedUp { withdrawal_id });

			Ok(())
		}

		/// Admin sets the cleanup TTL (in blocks)
		#[ink(message)]
		pub fn set_cleanup_ttl(&mut self, ttl_blocks: BlockNumber) -> Result<(), Error> {
			self.ensure_owner()?;
			if ttl_blocks == 0 {
				return Err(Error::InvalidTTL);
			}
			let old_ttl = self.cleanup_ttl_blocks;
			self.cleanup_ttl_blocks = ttl_blocks;
			self.env().emit_event(CleanupTTLUpdated { old_ttl, new_ttl: ttl_blocks });
			Ok(())
		}

		/// Admin sets the minimum deposit amount
		#[ink(message)]
		pub fn set_min_deposit_amount(&mut self, amount: Balance) -> Result<(), Error> {
			self.ensure_owner()?;
			let old_amount = self.min_deposit_amount;
			self.min_deposit_amount = amount;
			self.env().emit_event(MinDepositAmountUpdated { old_amount, new_amount: amount });
			Ok(())
		}

		// ============ Query Functions ============

		#[ink(message)]
		pub fn get_deposit_request(
			&self,
			request_id: DepositRequestId,
		) -> Option<DepositRequest> {
			self.deposit_requests.get(request_id)
		}

		#[ink(message)]
		pub fn get_withdrawal(&self, withdrawal_id: WithdrawalId) -> Option<Withdrawal> {
			self.withdrawals.get(withdrawal_id)
		}

		#[ink(message)]
		pub fn get_deposit_request_id_by_nonce(
			&self,
			nonce: DepositNonce,
		) -> Option<DepositRequestId> {
			self.nonce_to_deposit_request_id.get(nonce)
		}

		#[ink(message)]
		pub fn owner(&self) -> AccountId {
			self.owner
		}

		#[ink(message)]
		pub fn chain_id(&self) -> ChainId {
			self.chain_id
		}

		#[ink(message)]
		pub fn contract_hotkey(&self) -> AccountId {
			self.contract_hotkey
		}

		#[ink(message)]
		pub fn next_deposit_nonce(&self) -> DepositNonce {
			self.next_deposit_nonce
		}

		#[ink(message)]
		pub fn guardians(&self) -> Vec<AccountId> {
			self.guardians.clone()
		}

		#[ink(message)]
		pub fn approve_threshold(&self) -> u16 {
			self.approve_threshold
		}

		#[ink(message)]
		pub fn is_paused(&self) -> bool {
			self.paused
		}

		#[ink(message)]
		pub fn min_deposit_amount(&self) -> Balance {
			self.min_deposit_amount
		}

		#[ink(message)]
		pub fn cleanup_ttl(&self) -> BlockNumber {
			self.cleanup_ttl_blocks
		}

		// ============ Helper Functions ============

		fn ensure_owner(&self) -> Result<(), Error> {
			if self.env().caller() != self.owner {
				return Err(Error::Unauthorized);
			}
			Ok(())
		}

		fn ensure_not_paused(&self) -> Result<(), Error> {
			if self.paused {
				return Err(Error::BridgePaused);
			}
			Ok(())
		}

		fn ensure_guardian(&self) -> Result<(), Error> {
			let caller = self.env().caller();
			if !self.guardians.contains(&caller) {
				return Err(Error::NotGuardian);
			}
			Ok(())
		}

		fn get_stake_amount(
			&self,
			coldkey: AccountId,
			hotkey: AccountId,
			netuid: NetUid,
		) -> Result<Balance, Error> {
			let stake_info = self
				.env()
				.extension()
				.get_stake_info(hotkey, coldkey, netuid)
				.map_err(|_| Error::StakeQueryFailed)?;

			match stake_info {
				Some(info) => Ok(info.stake_amount()),
				None => Ok(0),
			}
		}

		fn finalize_withdrawal(
			&mut self,
			withdrawal_id: WithdrawalId,
			mut withdrawal: Withdrawal,
		) -> Result<(), Error> {
			let contract_hk = self.contract_hotkey;
			let netuid = NetUid::from(self.chain_id);
			let contract_account = self.env().account_id();

			// Check contract has sufficient stake before attempting transfer
			let contract_stake = self.get_stake_amount(contract_account, contract_hk, netuid)?;
			if contract_stake < withdrawal.amount {
				return Err(Error::InsufficientContractStake);
			}

			// Transfer stake to recipient
			self.env()
				.extension()
				.transfer_stake(
					withdrawal.recipient,
					contract_hk,
					netuid,
					netuid,
					AlphaCurrency::from(withdrawal.amount),
				)
				.map_err(|_| Error::TransferFailed)?;

			// Update status and set finalized_at_block
			withdrawal.finalized_at_block = Some(self.env().block_number());
			withdrawal.status = WithdrawalStatus::Completed;
			self.withdrawals.insert(withdrawal_id, &withdrawal);

			self.env().emit_event(WithdrawalCompleted {
				withdrawal_id,
				recipient: withdrawal.recipient,
				amount: withdrawal.amount,
			});

			Ok(())
		}
	}

	#[cfg(test)]
	mod tests {
		use super::*;

		#[ink::test]
		fn constructor_works() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let bridge = Bridge::new(accounts.alice, 1, accounts.eve);
			assert_eq!(bridge.owner(), accounts.alice);
			assert_eq!(bridge.chain_id(), 1);
			assert_eq!(bridge.next_deposit_nonce(), 0);
			assert!(!bridge.is_paused());
		}

		#[ink::test]
		fn pause_unpause_works() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			assert!(bridge.pause().is_ok());
			assert!(bridge.is_paused());

			assert!(bridge.unpause().is_ok());
			assert!(!bridge.is_paused());
		}

		#[ink::test]
		fn pause_fails_if_not_owner() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.bob);
			assert_eq!(bridge.pause(), Err(Error::Unauthorized));
		}

		#[ink::test]
		fn set_guardians_works() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			let guardians = vec![accounts.bob, accounts.charlie];
			assert!(bridge.set_guardians_and_threshold(guardians.clone(), 2).is_ok());

			assert_eq!(bridge.guardians(), guardians);
			assert_eq!(bridge.approve_threshold(), 2);
		}

		#[ink::test]
		fn set_guardians_fails_with_invalid_thresholds() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			let guardians = vec![accounts.bob, accounts.charlie];

			// Threshold too high
			assert_eq!(
				bridge.set_guardians_and_threshold(guardians.clone(), 3),
				Err(Error::InvalidThresholds)
			);

			// Threshold zero
			assert_eq!(
				bridge.set_guardians_and_threshold(guardians.clone(), 0),
				Err(Error::InvalidThresholds)
			);
		}

		#[ink::test]
		fn update_owner_works() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			assert!(bridge.update_owner(accounts.bob).is_ok());
			assert_eq!(bridge.owner(), accounts.bob);
		}

		#[ink::test]
		fn update_owner_fails_if_not_owner() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.bob);
			assert_eq!(bridge.update_owner(accounts.charlie), Err(Error::Unauthorized));
		}

		#[ink::test]
		fn attest_withdrawal_creates_record_on_first_attestation() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Set up guardian with threshold of 2 so it doesn't auto-finalize
			let guardians = vec![accounts.bob, accounts.charlie];
			bridge.set_guardians_and_threshold(guardians, 2).unwrap();

			// Call attest_withdrawal as guardian bob
			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.bob);
			let withdrawal_id = Hash::from([1u8; 32]);
			let recipient = accounts.django;
			let amount: Balance = 1_000_000_000;

			let result = bridge.attest_withdrawal(withdrawal_id, recipient, amount);
			assert!(result.is_ok());

			// Verify the withdrawal record was created
			let withdrawal = bridge.get_withdrawal(withdrawal_id);
			assert!(withdrawal.is_some());
			let withdrawal = withdrawal.unwrap();
			assert_eq!(withdrawal.recipient, recipient);
			assert_eq!(withdrawal.amount, amount);
			assert_eq!(withdrawal.votes.len(), 1);
			assert!(withdrawal.votes.contains(&accounts.bob));
			assert_eq!(withdrawal.status, WithdrawalStatus::Pending);
		}

		#[ink::test]
		fn attest_withdrawal_adds_vote_on_subsequent_attestation() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Set up guardians with threshold of 3 so it doesn't auto-finalize
			let guardians = vec![accounts.bob, accounts.charlie, accounts.django];
			bridge.set_guardians_and_threshold(guardians, 3).unwrap();

			let withdrawal_id = Hash::from([1u8; 32]);
			let recipient = accounts.eve;
			let amount: Balance = 1_000_000_000;

			// First attestation by bob
			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.bob);
			bridge.attest_withdrawal(withdrawal_id, recipient, amount).unwrap();

			// Second attestation by charlie
			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.charlie);
			let result = bridge.attest_withdrawal(withdrawal_id, recipient, amount);
			assert!(result.is_ok());

			// Verify the withdrawal record has 2 votes
			let withdrawal = bridge.get_withdrawal(withdrawal_id).unwrap();
			assert_eq!(withdrawal.votes.len(), 2);
			assert!(withdrawal.votes.contains(&accounts.bob));
			assert!(withdrawal.votes.contains(&accounts.charlie));
			assert_eq!(withdrawal.status, WithdrawalStatus::Pending);
		}

		#[ink::test]
		fn attest_withdrawal_fails_with_invalid_recipient() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Set up guardians
			let guardians = vec![accounts.bob, accounts.charlie];
			bridge.set_guardians_and_threshold(guardians, 2).unwrap();

			let withdrawal_id = Hash::from([1u8; 32]);
			let recipient = accounts.eve;
			let wrong_recipient = accounts.django;
			let amount: Balance = 1_000_000_000;

			// First attestation by bob with correct recipient
			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.bob);
			bridge.attest_withdrawal(withdrawal_id, recipient, amount).unwrap();

			// Second attestation by charlie with wrong recipient
			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.charlie);
			let result = bridge.attest_withdrawal(withdrawal_id, wrong_recipient, amount);
			assert_eq!(result, Err(Error::InvalidWithdrawalDetails));
		}

		#[ink::test]
		fn attest_withdrawal_fails_with_invalid_amount() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Set up guardians
			let guardians = vec![accounts.bob, accounts.charlie];
			bridge.set_guardians_and_threshold(guardians, 2).unwrap();

			let withdrawal_id = Hash::from([1u8; 32]);
			let recipient = accounts.eve;
			let amount: Balance = 1_000_000_000;
			let wrong_amount: Balance = 2_000_000_000;

			// First attestation by bob with correct amount
			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.bob);
			bridge.attest_withdrawal(withdrawal_id, recipient, amount).unwrap();

			// Second attestation by charlie with wrong amount
			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.charlie);
			let result = bridge.attest_withdrawal(withdrawal_id, recipient, wrong_amount);
			assert_eq!(result, Err(Error::InvalidWithdrawalDetails));
		}

		#[ink::test]
		fn attest_withdrawal_fails_with_already_voted() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Set up guardians
			let guardians = vec![accounts.bob, accounts.charlie];
			bridge.set_guardians_and_threshold(guardians, 2).unwrap();

			let withdrawal_id = Hash::from([1u8; 32]);
			let recipient = accounts.eve;
			let amount: Balance = 1_000_000_000;

			// First attestation by bob
			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.bob);
			bridge.attest_withdrawal(withdrawal_id, recipient, amount).unwrap();

			// Try to vote again as bob
			let result = bridge.attest_withdrawal(withdrawal_id, recipient, amount);
			assert_eq!(result, Err(Error::AlreadyVoted));
		}

		#[ink::test]
		fn attest_withdrawal_fails_if_not_guardian() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Set up guardians (bob and charlie, but not django)
			let guardians = vec![accounts.bob, accounts.charlie];
			bridge.set_guardians_and_threshold(guardians, 2).unwrap();

			let withdrawal_id = Hash::from([1u8; 32]);
			let recipient = accounts.eve;
			let amount: Balance = 1_000_000_000;

			// Try to attest as django (not a guardian)
			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.django);
			let result = bridge.attest_withdrawal(withdrawal_id, recipient, amount);
			assert_eq!(result, Err(Error::NotGuardian));
		}

		#[ink::test]
		fn admin_fail_deposit_request_works() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Manually create a deposit request in storage for testing
			// Since we can't test the full deposit flow (requires chain extension),
			// we insert a deposit request directly
			let deposit_request_id = Hash::from([2u8; 32]);
			let request = DepositRequest {
				sender: accounts.bob,
				recipient: accounts.charlie,
				amount: 1_000_000_000,
				nonce: 0,
				hotkey: accounts.eve,
				netuid: 1,
				status: DepositRequestStatus::Requested,
				created_at_block: 0,
			};
			bridge.deposit_requests.insert(deposit_request_id, &request);

			// Admin fails the deposit request
			let result = bridge.admin_fail_deposit_request(deposit_request_id);
			assert!(result.is_ok());

			// Verify the status changed
			let updated_request = bridge.get_deposit_request(deposit_request_id).unwrap();
			assert_eq!(updated_request.status, DepositRequestStatus::Failed);
		}

		#[ink::test]
		fn admin_fail_deposit_request_fails_if_not_owner() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			let deposit_request_id = Hash::from([2u8; 32]);
			let request = DepositRequest {
				sender: accounts.bob,
				recipient: accounts.charlie,
				amount: 1_000_000_000,
				nonce: 0,
				hotkey: accounts.eve,
				netuid: 1,
				status: DepositRequestStatus::Requested,
				created_at_block: 0,
			};
			bridge.deposit_requests.insert(deposit_request_id, &request);

			// Try to fail as non-owner
			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.bob);
			let result = bridge.admin_fail_deposit_request(deposit_request_id);
			assert_eq!(result, Err(Error::Unauthorized));
		}

		#[ink::test]
		fn admin_fail_deposit_request_fails_if_not_found() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			let deposit_request_id = Hash::from([2u8; 32]);
			let result = bridge.admin_fail_deposit_request(deposit_request_id);
			assert_eq!(result, Err(Error::DepositRequestNotFound));
		}

		#[ink::test]
		fn admin_fail_deposit_request_fails_if_already_finalized() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			let deposit_request_id = Hash::from([2u8; 32]);
			let request = DepositRequest {
				sender: accounts.bob,
				recipient: accounts.charlie,
				amount: 1_000_000_000,
				nonce: 0,
				hotkey: accounts.eve,
				netuid: 1,
				status: DepositRequestStatus::Failed, // Already failed
				created_at_block: 0,
			};
			bridge.deposit_requests.insert(deposit_request_id, &request);

			let result = bridge.admin_fail_deposit_request(deposit_request_id);
			assert_eq!(result, Err(Error::DepositRequestAlreadyFinalized));
		}

		#[ink::test]
		fn admin_cancel_withdrawal_works() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Set up guardians
			let guardians = vec![accounts.bob, accounts.charlie];
			bridge.set_guardians_and_threshold(guardians, 2).unwrap();

			// Create a withdrawal via attest_withdrawal
			let withdrawal_id = Hash::from([1u8; 32]);
			let recipient = accounts.eve;
			let amount: Balance = 1_000_000_000;

			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.bob);
			bridge.attest_withdrawal(withdrawal_id, recipient, amount).unwrap();

			// Admin cancels the withdrawal
			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.alice);
			let result = bridge.admin_cancel_withdrawal(withdrawal_id);
			assert!(result.is_ok());

			// Verify the status changed
			let withdrawal = bridge.get_withdrawal(withdrawal_id).unwrap();
			assert_eq!(withdrawal.status, WithdrawalStatus::Cancelled);
		}

		#[ink::test]
		fn admin_cancel_withdrawal_fails_if_not_owner() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Set up guardians
			let guardians = vec![accounts.bob, accounts.charlie];
			bridge.set_guardians_and_threshold(guardians, 2).unwrap();

			// Create a withdrawal
			let withdrawal_id = Hash::from([1u8; 32]);
			let recipient = accounts.eve;
			let amount: Balance = 1_000_000_000;

			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.bob);
			bridge.attest_withdrawal(withdrawal_id, recipient, amount).unwrap();

			// Try to cancel as non-owner
			let result = bridge.admin_cancel_withdrawal(withdrawal_id);
			assert_eq!(result, Err(Error::Unauthorized));
		}

		#[ink::test]
		fn admin_cancel_withdrawal_fails_if_not_found() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			let withdrawal_id = Hash::from([1u8; 32]);
			let result = bridge.admin_cancel_withdrawal(withdrawal_id);
			assert_eq!(result, Err(Error::WithdrawalNotFound));
		}

		#[ink::test]
		fn admin_cancel_withdrawal_fails_if_already_finalized() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Insert a withdrawal directly with Cancelled status
			let withdrawal_id = Hash::from([1u8; 32]);
			let withdrawal = Withdrawal {
				request_id: withdrawal_id,
				recipient: accounts.eve,
				amount: 1_000_000_000,
				votes: vec![accounts.bob],
				status: WithdrawalStatus::Cancelled,
				created_at_block: 0,
				finalized_at_block: Some(0),
			};
			bridge.withdrawals.insert(withdrawal_id, &withdrawal);

			let result = bridge.admin_cancel_withdrawal(withdrawal_id);
			assert_eq!(result, Err(Error::WithdrawalAlreadyFinalized));
		}

		#[ink::test]
		fn attest_withdrawal_fails_when_paused() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Set up guardians
			let guardians = vec![accounts.bob, accounts.charlie];
			bridge.set_guardians_and_threshold(guardians, 2).unwrap();

			// Pause the bridge
			bridge.pause().unwrap();

			// Try to attest
			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.bob);
			let withdrawal_id = Hash::from([1u8; 32]);
			let result = bridge.attest_withdrawal(withdrawal_id, accounts.eve, 1_000_000_000);
			assert_eq!(result, Err(Error::BridgePaused));
		}

		#[ink::test]
		fn attest_withdrawal_fails_on_already_finalized() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Set up guardians
			let guardians = vec![accounts.bob, accounts.charlie, accounts.django];
			bridge.set_guardians_and_threshold(guardians, 2).unwrap();

			// Insert a completed withdrawal directly
			let withdrawal_id = Hash::from([1u8; 32]);
			let withdrawal = Withdrawal {
				request_id: withdrawal_id,
				recipient: accounts.eve,
				amount: 1_000_000_000,
				votes: vec![accounts.bob, accounts.charlie],
				status: WithdrawalStatus::Completed,
				created_at_block: 0,
				finalized_at_block: Some(0),
			};
			bridge.withdrawals.insert(withdrawal_id, &withdrawal);

			// Try to attest on a completed withdrawal
			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.django);
			let result = bridge.attest_withdrawal(withdrawal_id, accounts.eve, 1_000_000_000);
			assert_eq!(result, Err(Error::WithdrawalAlreadyFinalized));
		}

		// ============ Cleanup Tests ============

		#[ink::test]
		fn test_cleanup_deposit_request_after_ttl() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Set a small TTL for testing
			bridge.set_cleanup_ttl(10).unwrap();

			// Insert a deposit request
			let deposit_request_id = Hash::from([2u8; 32]);
			let request = DepositRequest {
				sender: accounts.bob,
				recipient: accounts.charlie,
				amount: 1_000_000_000,
				nonce: 0,
				hotkey: accounts.eve,
				netuid: 1,
				status: DepositRequestStatus::Requested,
				created_at_block: 0,
			};
			bridge.deposit_requests.insert(deposit_request_id, &request);
			bridge.nonce_to_deposit_request_id.insert(0, &deposit_request_id);

			// Advance block number past TTL
			ink::env::test::set_block_number::<ink::env::DefaultEnvironment>(11);

			// Cleanup should succeed
			let result = bridge.cleanup_deposit_request(deposit_request_id);
			assert!(result.is_ok());

			// Verify the deposit request was removed
			assert!(bridge.get_deposit_request(deposit_request_id).is_none());
			assert!(bridge.get_deposit_request_id_by_nonce(0).is_none());
		}

		#[ink::test]
		fn test_cleanup_deposit_request_before_ttl_fails() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Set a TTL
			bridge.set_cleanup_ttl(100).unwrap();

			// Insert a deposit request at block 0
			let deposit_request_id = Hash::from([2u8; 32]);
			let request = DepositRequest {
				sender: accounts.bob,
				recipient: accounts.charlie,
				amount: 1_000_000_000,
				nonce: 0,
				hotkey: accounts.eve,
				netuid: 1,
				status: DepositRequestStatus::Requested,
				created_at_block: 0,
			};
			bridge.deposit_requests.insert(deposit_request_id, &request);

			// Try to cleanup before TTL expires (block 50 < 0 + 100)
			ink::env::test::set_block_number::<ink::env::DefaultEnvironment>(50);

			let result = bridge.cleanup_deposit_request(deposit_request_id);
			assert_eq!(result, Err(Error::TTLNotExpired));
		}

		#[ink::test]
		fn test_cleanup_withdrawal_after_ttl() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Set a small TTL for testing
			bridge.set_cleanup_ttl(10).unwrap();

			// Insert a completed withdrawal
			let withdrawal_id = Hash::from([1u8; 32]);
			let withdrawal = Withdrawal {
				request_id: withdrawal_id,
				recipient: accounts.eve,
				amount: 1_000_000_000,
				votes: vec![accounts.bob],
				status: WithdrawalStatus::Completed,
				created_at_block: 0,
				finalized_at_block: Some(5),
			};
			bridge.withdrawals.insert(withdrawal_id, &withdrawal);

			// Advance block number past TTL from finalized_at_block (5 + 10 = 15)
			ink::env::test::set_block_number::<ink::env::DefaultEnvironment>(16);

			// Cleanup should succeed
			let result = bridge.cleanup_withdrawal(withdrawal_id);
			assert!(result.is_ok());

			// Verify the withdrawal was removed
			assert!(bridge.get_withdrawal(withdrawal_id).is_none());
		}

		#[ink::test]
		fn test_cleanup_withdrawal_before_ttl_fails() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Set a TTL
			bridge.set_cleanup_ttl(100).unwrap();

			// Insert a completed withdrawal finalized at block 10
			let withdrawal_id = Hash::from([1u8; 32]);
			let withdrawal = Withdrawal {
				request_id: withdrawal_id,
				recipient: accounts.eve,
				amount: 1_000_000_000,
				votes: vec![accounts.bob],
				status: WithdrawalStatus::Completed,
				created_at_block: 0,
				finalized_at_block: Some(10),
			};
			bridge.withdrawals.insert(withdrawal_id, &withdrawal);

			// Try to cleanup before TTL expires (block 50 < 10 + 100)
			ink::env::test::set_block_number::<ink::env::DefaultEnvironment>(50);

			let result = bridge.cleanup_withdrawal(withdrawal_id);
			assert_eq!(result, Err(Error::TTLNotExpired));
		}

		#[ink::test]
		fn test_cleanup_pending_withdrawal_fails() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Set a small TTL
			bridge.set_cleanup_ttl(10).unwrap();

			// Insert a pending withdrawal (not finalized)
			let withdrawal_id = Hash::from([1u8; 32]);
			let withdrawal = Withdrawal {
				request_id: withdrawal_id,
				recipient: accounts.eve,
				amount: 1_000_000_000,
				votes: vec![accounts.bob],
				status: WithdrawalStatus::Pending,
				created_at_block: 0,
				finalized_at_block: None,
			};
			bridge.withdrawals.insert(withdrawal_id, &withdrawal);

			// Advance block number past any TTL
			ink::env::test::set_block_number::<ink::env::DefaultEnvironment>(1000);

			// Cleanup should fail because withdrawal is not finalized
			let result = bridge.cleanup_withdrawal(withdrawal_id);
			assert_eq!(result, Err(Error::RecordNotFinalized));
		}

		#[ink::test]
		fn test_set_cleanup_ttl() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Default TTL
			assert_eq!(bridge.cleanup_ttl(), 100_800);

			// Update TTL as owner
			let result = bridge.set_cleanup_ttl(50_000);
			assert!(result.is_ok());
			assert_eq!(bridge.cleanup_ttl(), 50_000);
		}

		#[ink::test]
		fn test_set_cleanup_ttl_zero_fails() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			let result = bridge.set_cleanup_ttl(0);
			assert_eq!(result, Err(Error::InvalidTTL));
		}

		#[ink::test]
		fn test_set_cleanup_ttl_non_owner_fails() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Try to update TTL as non-owner
			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.bob);
			let result = bridge.set_cleanup_ttl(50_000);
			assert_eq!(result, Err(Error::Unauthorized));
		}

		#[ink::test]
		fn test_set_min_deposit_amount() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Default min deposit amount
			assert_eq!(bridge.min_deposit_amount(), 1_000_000_000);

			// Update min deposit amount as owner
			let result = bridge.set_min_deposit_amount(2_000_000_000);
			assert!(result.is_ok());
			assert_eq!(bridge.min_deposit_amount(), 2_000_000_000);
		}

		#[ink::test]
		fn test_set_min_deposit_amount_non_owner_fails() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Try to update min deposit amount as non-owner
			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.bob);
			let result = bridge.set_min_deposit_amount(2_000_000_000);
			assert_eq!(result, Err(Error::Unauthorized));
		}
	}
}

#![cfg_attr(not(feature = "std"), no_std, no_main)]

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
		owner: AccountId,
		chain_id: ChainId,
		next_deposit_nonce: DepositNonce,
		last_burns_checkpoint_nonce: CheckpointNonce,
		last_refunds_checkpoint_nonce: CheckpointNonce,

		locked: Mapping<DepositId, Balance>,
		processed_deposits: Mapping<DepositId, bool>,
		denied_deposits: Mapping<DepositId, bool>,
		deposit_metadata: Mapping<DepositId, DepositMetadata>,

		guardians: Vec<AccountId>,
		approve_threshold: u16,
		deny_threshold: u16,
		signature_ttl_blocks: u32,

		pending_burns: Mapping<BurnId, PendingBurn>,
		processed_burns: Mapping<BurnId, bool>,
		pending_refunds: Mapping<DepositId, PendingRefund>,

		user_deposits: Mapping<AccountId, Vec<DepositId>>,
		min_deposit_amount: Balance,
		max_deposits_per_user: u32,

		contract_hotkey: AccountId,
		paused: bool,

		nonce_to_deposit_id: Mapping<DepositNonce, DepositId>,
	}

	impl Bridge {
		/// Creates a new bridge contract instance
		///
		/// Initializes the bridge with default configuration:
		/// - Empty guardian set (use set_guardians_and_thresholds to configure)
		/// - Signature TTL: 100 blocks
		/// - Minimum deposit: 1,000,000,000 rao (1 Alpha)
		/// - Max deposits per user: 25
		/// - Bridge is unpaused
		///
		/// # Arguments
		/// * `owner` - Contract owner with admin privileges
		/// * `chain_id` - Identifier for the Bittensor chain (used in deposit ID generation)
		/// * `hotkey` - Contract's hotkey for stake consolidation
		#[ink(constructor)]
		pub fn new(owner: AccountId, chain_id: ChainId, hotkey: AccountId) -> Self {
			Self {
				owner,
				chain_id,
				next_deposit_nonce: 0,
				last_burns_checkpoint_nonce: 0,
				last_refunds_checkpoint_nonce: 0,
				locked: Mapping::default(),
				contract_hotkey: hotkey,
				processed_deposits: Mapping::default(),
				denied_deposits: Mapping::default(),
				deposit_metadata: Mapping::default(),
				guardians: Vec::new(),
				approve_threshold: 0,
				deny_threshold: 0,
				signature_ttl_blocks: 100,
				pending_burns: Mapping::default(),
				processed_burns: Mapping::default(),
				pending_refunds: Mapping::default(),
				user_deposits: Mapping::default(),
				min_deposit_amount: 1_000_000_000,
				max_deposits_per_user: 25,
				paused: false,
				nonce_to_deposit_id: Mapping::default(),
			}
		}

		/// Locks Alpha tokens on Bittensor to be bridged to Hippius
		///
		/// Users call this function to deposit Alpha stake into the bridge contract.
		/// The locked stake will be available for minting as hAlpha on Hippius after guardian approval.
		///
		/// The caller MUST have added this contract as a proxy on Bittensor via the
		/// Subtensor pallet's add_proxy extrinsic.
		#[ink(message)]
		pub fn lock(
			&mut self,
			amount: Balance,
			hotkey: AccountId,
			netuid: NetUid,
		) -> Result<DepositId, Error> {
			self.ensure_not_paused()?;

			if netuid.as_u16() != self.chain_id {
				return Err(Error::InvalidNetUid);
			}

			if amount < self.min_deposit_amount {
				return Err(Error::AmountTooSmall);
			}

			let sender = self.env().caller();

			let mut user_deposits = self.user_deposits.get(sender).unwrap_or_default();
			if user_deposits.len() >= self.max_deposits_per_user as usize {
				return Err(Error::TooManyDeposits);
			}

			let contract_account = self.env().account_id();

			let sender_stake_before = self.get_stake_amount(sender, hotkey, netuid)?;
			if sender_stake_before < amount {
				return Err(Error::InsufficientStake);
			}

			let contract_stake_before = self.get_stake_amount(contract_account, hotkey, netuid)?;

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

			let deposit_nonce = self.next_deposit_nonce;
			self.next_deposit_nonce =
				self.next_deposit_nonce.checked_add(1).ok_or(Error::Overflow)?;

			let mut canonical_bytes = Vec::new();
			canonical_bytes.extend_from_slice(&self.chain_id.to_le_bytes());
			canonical_bytes.extend_from_slice(contract_account.as_ref());
			canonical_bytes.extend_from_slice(&deposit_nonce.to_le_bytes());
			canonical_bytes.extend_from_slice(sender.as_ref());
			canonical_bytes.extend_from_slice(&amount.to_le_bytes());

			let mut domain_and_canonical = Vec::new();
			domain_and_canonical.extend_from_slice(DOMAIN_DEPOSIT);
			domain_and_canonical.extend_from_slice(&canonical_bytes);

			let deposit_id = {
				let mut hash_output = <<Blake2x256 as HashOutput>::Type as Default>::default();
				<Blake2x256 as CryptoHash>::hash(&domain_and_canonical, &mut hash_output);
				Hash::from(hash_output)
			};

			self.locked.insert(deposit_id, &amount);

			// Store deposit metadata for refunds
			let metadata = DepositMetadata { sender, hotkey, netuid: netuid.as_u16(), amount };
			self.deposit_metadata.insert(deposit_id, &metadata);
			self.nonce_to_deposit_id.insert(deposit_nonce, &deposit_id);

			user_deposits.push(deposit_id);
			self.user_deposits.insert(sender, &user_deposits);

			self.env().emit_event(DepositMade {
				chain_id: self.chain_id,
				escrow_contract: contract_account,
				deposit_nonce,
				sender,
				amount,
				deposit_id,
			});

			Ok(deposit_id)
		}

		/// Proposes burns from Hippius for releasing locked stake
		///
		/// Guardians monitor Hippius UnlockApproved events and create burn proposals.
		/// The checkpoint must have sequential nonce (last_burns_checkpoint_nonce + 1).
		/// The proposing guardian's vote is automatically counted as an approval.
		/// Other guardians then attest via attest_release().
		///
		/// # Security
		/// - Only guardians can propose burns
		/// - Checkpoint nonces must be sequential
		/// - Each burn is stored and voted on individually
		#[ink(message)]
		pub fn propose_releases(
			&mut self,
			burns: Vec<CanonicalBurn>,
			checkpoint_nonce: CheckpointNonce,
		) -> Result<(), Error> {
			self.ensure_not_paused()?;
			self.ensure_guardian()?;

			// Validate sequential checkpoint nonce
			let expected_nonce =
				self.last_burns_checkpoint_nonce.checked_add(1).ok_or(Error::Overflow)?;
			if checkpoint_nonce != expected_nonce {
				return Err(Error::InvalidCheckpointNonce);
			}

			let caller = self.env().caller();
			let current_block = self.env().block_number();
			let mut burn_ids = Vec::new();

			for burn in burns.iter() {
				let burn_id = burn.burn_id;

				// Check not already pending or already finalized
				if self.pending_burns.contains(burn_id)
					|| self.processed_burns.get(burn_id).unwrap_or(false)
				{
					return Err(Error::AlreadyProcessed);
				}

				// Create pending burn with proposer's approval
				let approves = vec![caller];

				let pending = PendingBurn {
					burn_id,
					recipient: burn.recipient,
					hotkey: burn.hotkey,
					netuid: burn.netuid,
					amount: burn.amount,
					approves,
					denies: Vec::new(),
					proposed_at: current_block,
				};

				// Check if threshold reached immediately (like pallet pattern)
				if pending.approves.len() >= self.approve_threshold as usize {
					self.finalize_burn(burn_id, pending)?;
				} else {
					self.pending_burns.insert(burn_id, &pending);
				}

				burn_ids.push(burn_id);
			}

			// Update checkpoint nonce
			self.last_burns_checkpoint_nonce = checkpoint_nonce;

			self.env().emit_event(BurnsProposed {
				checkpoint_nonce,
				proposer: caller,
				burns_count: u32::try_from(burns.len()).map_err(|_| Error::Overflow)?,
			});

			Ok(())
		}

		/// Guardian attests (votes on) a pending burn
		///
		/// Guardians review individual burns and vote to approve or deny.
		/// When approve_threshold is reached and within TTL, the burn is automatically finalized.
		#[ink(message)]
		pub fn attest_release(&mut self, burn_id: BurnId, approve: bool) -> Result<(), Error> {
			self.ensure_not_paused()?;
			self.ensure_guardian()?;

			let mut pending = self.pending_burns.get(burn_id).ok_or(Error::CheckpointNotFound)?;

			let caller = self.env().caller();

			// Check not already voted
			if pending.approves.contains(&caller) || pending.denies.contains(&caller) {
				return Err(Error::AlreadyAttestedBurn);
			}

			// Check not expired
			let current_block = self.env().block_number();
			let blocks_elapsed = current_block.saturating_sub(pending.proposed_at);
			if blocks_elapsed > self.signature_ttl_blocks {
				return Err(Error::CheckpointExpired);
			}

			// Add vote
			if approve {
				pending.approves.push(caller);
			} else {
				pending.denies.push(caller);
			}

			// Check thresholds
			if pending.approves.len() >= self.approve_threshold as usize {
				self.finalize_burn(burn_id, pending)?;
			} else if pending.denies.len() >= self.deny_threshold as usize {
				self.pending_burns.remove(burn_id);
				self.env().emit_event(BurnDenied { burn_id });
			} else {
				self.pending_burns.insert(burn_id, &pending);
			}

			Ok(())
		}

		/// Expires a burn that exceeded its TTL
		///
		/// Anyone can call this to clean up stale burn proposals that did not reach
		/// consensus within signature_ttl_blocks.
		#[ink(message)]
		pub fn expire_release(&mut self, burn_id: BurnId) -> Result<(), Error> {
			let pending = self.pending_burns.get(burn_id).ok_or(Error::CheckpointNotFound)?;

			let current_block = self.env().block_number();
			let blocks_elapsed = current_block.saturating_sub(pending.proposed_at);

			if blocks_elapsed <= self.signature_ttl_blocks {
				return Err(Error::CheckpointNotExpired);
			}

			self.pending_burns.remove(burn_id);

			self.env().emit_event(BurnExpired { burn_id });

			Ok(())
		}

		/// Proposes refunds for deposits that were denied on Hippius
		///
		/// # Guardian Workflow
		/// 1. Monitor Hippius chain for `BridgeDenied` events
		/// 2. Collect denied deposit_ids from events
		/// 3. For each denied deposit, fetch deposit metadata from Bittensor contract
		/// 4. Create RefundItem with deposit_id, original sender, and amount
		/// 5. Call this function with the refund proposals
		/// 6. Other guardians attest the refund via attest_refund()
		/// 7. When threshold reached, refunds are executed automatically
		///
		/// # Arguments
		/// * `refunds` - Vector of refund items (denied deposits to refund)
		/// * `checkpoint_nonce` - Sequential nonce for checkpoint ordering
		///
		/// # Security
		/// - Only guardians can propose refunds
		/// - Checkpoint nonces must be sequential
		/// - Each refund is stored and voted on individually
		/// - **Refund recipient MUST match original deposit sender**
		/// - **Refund amount MUST match full locked amount (no partial refunds)**
		#[ink(message)]
		pub fn propose_refunds(
			&mut self,
			refunds: Vec<RefundItem>,
			checkpoint_nonce: CheckpointNonce,
		) -> Result<(), Error> {
			self.ensure_not_paused()?;
			self.ensure_guardian()?;

			// Validate sequential checkpoint nonce
			let expected_nonce =
				self.last_refunds_checkpoint_nonce.checked_add(1).ok_or(Error::Overflow)?;
			if checkpoint_nonce != expected_nonce {
				return Err(Error::InvalidCheckpointNonce);
			}

			let caller = self.env().caller();
			let current_block = self.env().block_number();
			let mut deposit_ids = Vec::new();

			for refund in refunds.iter() {
				let deposit_id = refund.deposit_id;

				// Check not already pending
				if self.pending_refunds.contains(deposit_id) {
					return Err(Error::AlreadyProcessed);
				}

				// Verify the deposit exists and is locked
				if !self.locked.contains(deposit_id) {
					return Err(Error::CheckpointNotFound);
				}

				// Fetch stored metadata to validate refund parameters
				let metadata =
					self.deposit_metadata.get(deposit_id).ok_or(Error::CheckpointNotFound)?;

				let locked_amount = self.locked.get(deposit_id).ok_or(Error::CheckpointNotFound)?;

				// Validate recipient matches original sender
				if refund.recipient != metadata.sender {
					return Err(Error::InvalidRefundRecipient);
				}

				// Validate amount matches full locked amount (no partial refunds)
				if refund.amount != locked_amount {
					return Err(Error::InvalidRefundAmount);
				}

				// Create pending refund with proposer's approval
				let approves = vec![caller];

				let pending = PendingRefund {
					deposit_id,
					recipient: refund.recipient,
					amount: refund.amount,
					approves,
					denies: Vec::new(),
					proposed_at: current_block,
				};

				// Check if threshold reached immediately (like pallet pattern)
				if pending.approves.len() >= self.approve_threshold as usize {
					self.finalize_refund(deposit_id, pending)?;
				} else {
					self.pending_refunds.insert(deposit_id, &pending);
				}

				deposit_ids.push(deposit_id);
			}

			// Update checkpoint nonce
			self.last_refunds_checkpoint_nonce = checkpoint_nonce;

			self.env().emit_event(RefundsProposed {
				checkpoint_nonce,
				proposer: caller,
				refunds_count: u32::try_from(refunds.len()).map_err(|_| Error::Overflow)?,
			});

			Ok(())
		}

		/// Guardian attests (votes on) a pending refund
		///
		/// Guardians review individual refunds and vote to approve or deny.
		/// When approve_threshold is reached and within TTL, the refund is automatically finalized.
		#[ink(message)]
		pub fn attest_refund(&mut self, deposit_id: DepositId, approve: bool) -> Result<(), Error> {
			self.ensure_not_paused()?;
			self.ensure_guardian()?;

			let mut pending =
				self.pending_refunds.get(deposit_id).ok_or(Error::CheckpointNotFound)?;

			let caller = self.env().caller();

			// Check not already voted
			if pending.approves.contains(&caller) || pending.denies.contains(&caller) {
				return Err(Error::AlreadyAttestedRefund);
			}

			// Check not expired
			let current_block = self.env().block_number();
			let blocks_elapsed = current_block.saturating_sub(pending.proposed_at);
			if blocks_elapsed > self.signature_ttl_blocks {
				return Err(Error::CheckpointExpired);
			}

			// Add vote
			if approve {
				pending.approves.push(caller);
			} else {
				pending.denies.push(caller);
			}

			// Check thresholds
			if pending.approves.len() >= self.approve_threshold as usize {
				self.finalize_refund(deposit_id, pending)?;
			} else if pending.denies.len() >= self.deny_threshold as usize {
				self.pending_refunds.remove(deposit_id);
				self.env().emit_event(RefundDenied { deposit_id });
			} else {
				self.pending_refunds.insert(deposit_id, &pending);
			}

			Ok(())
		}

		/// Expires a refund that exceeded its TTL
		///
		/// Anyone can call this to clean up stale refund proposals that did not reach
		/// consensus within signature_ttl_blocks.
		#[ink(message)]
		pub fn expire_refund(&mut self, deposit_id: DepositId) -> Result<(), Error> {
			let pending = self.pending_refunds.get(deposit_id).ok_or(Error::CheckpointNotFound)?;

			let current_block = self.env().block_number();
			let blocks_elapsed = current_block.saturating_sub(pending.proposed_at);

			if blocks_elapsed <= self.signature_ttl_blocks {
				return Err(Error::CheckpointNotExpired);
			}

			self.pending_refunds.remove(deposit_id);

			self.env().emit_event(RefundExpired { deposit_id });

			Ok(())
		}

		/// Configures the guardian set and voting thresholds (owner only)
		///
		/// Sets the list of authorized guardians and the number of votes required
		/// to approve or deny proposals. Validates that deny_threshold < approve_threshold
		/// and approve_threshold <= guardian_count.
		#[ink(message)]
		pub fn set_guardians_and_thresholds(
			&mut self,
			guardians: Vec<AccountId>,
			approve_threshold: u16,
			deny_threshold: u16,
		) -> Result<(), Error> {
			self.ensure_owner()?;

			if guardians.len() > MAX_GUARDIANS {
				return Err(Error::TooManyGuardians);
			}

			let guardians_count =
				u16::try_from(guardians.len()).map_err(|_| Error::TooManyGuardians)?;
			if deny_threshold >= approve_threshold {
				return Err(Error::InvalidThresholds);
			}
			if approve_threshold > guardians_count {
				return Err(Error::InvalidThresholds);
			}

			self.guardians = guardians.clone();
			self.approve_threshold = approve_threshold;
			self.deny_threshold = deny_threshold;

			self.env().emit_event(GuardiansUpdated {
				guardians,
				approve_threshold,
				deny_threshold,
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

		/// Set the contract's hotkey (admin only)
		/// All deposited stake will be consolidated to this hotkey
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

		/// Replace contract code with new implementation
		/// Only callable by contract owner
		#[ink(message)]
		pub fn set_code(&mut self, code_hash: Hash) -> Result<(), Error> {
			self.ensure_owner()?;

			self.env().set_code_hash(&code_hash).map_err(|_| Error::CodeUpgradeFailed)?;

			self.env()
				.emit_event(CodeUpgraded { code_hash, upgraded_by: self.env().caller() });

			Ok(())
		}

		/// Get the contract's current hotkey
		#[ink(message)]
		pub fn contract_hotkey(&self) -> AccountId {
			self.contract_hotkey
		}

		#[ink(message)]
		pub fn get_locked_amount(&self, deposit_id: DepositId) -> Option<Balance> {
			self.locked.get(deposit_id)
		}

		#[ink(message)]
		pub fn is_processed(&self, deposit_id: DepositId) -> bool {
			self.processed_deposits.get(deposit_id).unwrap_or(false)
		}

		#[ink(message)]
		pub fn is_denied(&self, deposit_id: DepositId) -> bool {
			self.denied_deposits.get(deposit_id).unwrap_or(false)
		}

		#[ink(message)]
		pub fn get_user_deposits(&self, user: AccountId) -> Vec<DepositId> {
			self.user_deposits.get(user).unwrap_or_default()
		}

		#[ink(message)]
		pub fn get_pending_burn(&self, burn_id: BurnId) -> Option<PendingBurn> {
			self.pending_burns.get(burn_id)
		}

		#[ink(message)]
		pub fn get_pending_refund(&self, deposit_id: DepositId) -> Option<PendingRefund> {
			self.pending_refunds.get(deposit_id)
		}

		/// Resolve a deposit ID from its nonce
		#[ink(message)]
		pub fn get_deposit_id_by_nonce(&self, deposit_nonce: DepositNonce) -> Option<DepositId> {
			self.nonce_to_deposit_id.get(deposit_nonce)
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
		pub fn next_deposit_nonce(&self) -> DepositNonce {
			self.next_deposit_nonce
		}

		#[ink(message)]
		pub fn last_burns_checkpoint_nonce(&self) -> CheckpointNonce {
			self.last_burns_checkpoint_nonce
		}

		#[ink(message)]
		pub fn last_refunds_checkpoint_nonce(&self) -> CheckpointNonce {
			self.last_refunds_checkpoint_nonce
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
		pub fn deny_threshold(&self) -> u16 {
			self.deny_threshold
		}

		#[ink(message)]
		pub fn is_paused(&self) -> bool {
			self.paused
		}

		/// Get deposit metadata for a given deposit ID
		#[ink(message)]
		pub fn get_deposit_metadata(&self, deposit_id: DepositId) -> Option<DepositMetadata> {
			self.deposit_metadata.get(deposit_id)
		}

		/// Check if a burn ID has a pending approval
		#[ink(message)]
		pub fn has_pending_burn(&self, burn_id: BurnId) -> bool {
			self.pending_burns.contains(burn_id)
		}

		/// Check if a deposit ID has a pending refund
		#[ink(message)]
		pub fn has_pending_refund(&self, deposit_id: DepositId) -> bool {
			self.pending_refunds.contains(deposit_id)
		}

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

		fn finalize_burn(&mut self, burn_id: BurnId, pending: PendingBurn) -> Result<(), Error> {
			let contract_hk = self.contract_hotkey;

			// Transfer stake for this individual burn
			self.env()
				.extension()
				.transfer_stake(
					pending.recipient,
					contract_hk,
					NetUid::from(pending.netuid),
					NetUid::from(pending.netuid),
					AlphaCurrency::from(pending.amount),
				)
				.map_err(|_| Error::TransferFailed)?;

			// Mark burn as processed to prevent double releases
			self.processed_burns.insert(burn_id, &true);
			// Remove from pending
			self.pending_burns.remove(burn_id);

			// Emit release event
			self.env().emit_event(Released {
				burn_id,
				recipient: pending.recipient,
				amount: pending.amount,
			});

			Ok(())
		}

		fn finalize_refund(
			&mut self,
			deposit_id: DepositId,
			pending: PendingRefund,
		) -> Result<(), Error> {
			let contract_hk = self.contract_hotkey;

			let locked_amount = self.locked.get(deposit_id).ok_or(Error::CheckpointNotFound)?;

			if locked_amount < pending.amount {
				return Err(Error::InsufficientStake);
			}

			// Get stored metadata to know which hotkey/netuid to refund
			let metadata =
				self.deposit_metadata.get(deposit_id).ok_or(Error::CheckpointNotFound)?;

			// Transfer stake back to recipient
			self.env()
				.extension()
				.transfer_stake(
					pending.recipient,
					contract_hk,
					NetUid::from(metadata.netuid),
					NetUid::from(metadata.netuid),
					AlphaCurrency::from(pending.amount),
				)
				.map_err(|_| Error::TransferFailed)?;

			// Clean up storage
			self.locked.remove(deposit_id);
			self.denied_deposits.insert(deposit_id, &true);
			self.deposit_metadata.remove(deposit_id);

			// Remove from user deposits
			let mut user_deposits = self.user_deposits.get(pending.recipient).unwrap_or_default();
			user_deposits.retain(|&id| id != deposit_id);
			if user_deposits.is_empty() {
				self.user_deposits.remove(pending.recipient);
			} else {
				self.user_deposits.insert(pending.recipient, &user_deposits);
			}

			// Remove from pending
			self.pending_refunds.remove(deposit_id);

			// Emit refund event
			self.env().emit_event(Refunded {
				deposit_id,
				recipient: pending.recipient,
				amount: pending.amount,
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
			assert!(bridge.set_guardians_and_thresholds(guardians.clone(), 2, 1).is_ok());

			assert_eq!(bridge.guardians(), guardians);
			assert_eq!(bridge.approve_threshold(), 2);
			assert_eq!(bridge.deny_threshold(), 1);
		}

		#[ink::test]
		fn set_guardians_fails_with_invalid_thresholds() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			let guardians = vec![accounts.bob, accounts.charlie];

			assert_eq!(
				bridge.set_guardians_and_thresholds(guardians.clone(), 1, 2),
				Err(Error::InvalidThresholds)
			);

			assert_eq!(
				bridge.set_guardians_and_thresholds(guardians.clone(), 3, 1),
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
		fn cannot_propose_already_processed_burn() {
			let accounts = ink::env::test::default_accounts::<ink::env::DefaultEnvironment>();
			let mut bridge = Bridge::new(accounts.alice, 1, accounts.eve);

			// Configure a single guardian so propose_releases can be called.
			let guardians = vec![accounts.bob];
			assert!(bridge.set_guardians_and_thresholds(guardians.clone(), 1, 0).is_ok());

			// Guardian is the caller.
			ink::env::test::set_caller::<ink::env::DefaultEnvironment>(accounts.bob);

			// Simulate a burn that has already been finalized.
			let burn_id: BurnId = Default::default();
			bridge.processed_burns.insert(burn_id, &true);

			let burn = CanonicalBurn {
				burn_id,
				recipient: accounts.charlie,
				hotkey: accounts.eve,
				netuid: NetUid::from(1u16),
				amount: 42,
			};

			let result = bridge.propose_releases(vec![burn], 1);
			assert_eq!(result, Err(Error::AlreadyProcessed));
		}
	}
}

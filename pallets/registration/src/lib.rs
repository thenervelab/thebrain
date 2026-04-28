#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;
pub use types::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

mod types;

// Define a trait for proxy type compatibility
pub trait ProxyTypeCompat {
	fn is_non_transfer(&self) -> bool;
	fn is_any(&self) -> bool;
}

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::traits::Currency;
	use frame_support::traits::ExistenceRequirement;
	use frame_support::PalletId;
	use frame_support::{
		dispatch::DispatchResultWithPostInfo,
		pallet_prelude::*,
		traits::Get,
	};
	use frame_system::{offchain::SendTransactionTypes, pallet_prelude::*};
	use pallet_credits::Pallet as CreditsPallet;
	use pallet_proxy::Pallet as ProxyPallet;
	use pallet_utils::{MetagraphInfoProvider, MetricsInfoProvider};
	use scale_info::prelude::string::String;
	use scale_info::prelude::*;
	use sp_core::crypto::Ss58Codec;
	use sp_core::H256;
	use sp_io::hashing::blake2_256;
	use sp_runtime::traits::AccountIdConversion;
	use sp_runtime::traits::CheckedDiv;
	use sp_runtime::traits::Zero;
	use sp_runtime::AccountId32;
	use sp_runtime::Saturating;
	use sp_std::collections::btree_map::BTreeMap;
	use sp_std::{prelude::*, vec::Vec};

	// Define a constant for token decimals (typically at the top of the file)
	pub const DECIMALS: u32 = 18;

	// New pallet_balances balance type
	pub type BalanceOf<T> = <T as pallet_balances::Config>::Balance;

	#[pallet::config]
	pub trait Config:
		frame_system::Config
		+ pallet_babe::Config
		+ pallet_balances::Config
		+ pallet_utils::Config
		+ pallet_credits::Config
		+ SendTransactionTypes<Call<Self>>
		+ pallet_staking::Config
		+ pallet_proxy::Config<ProxyType = Self::ProxyTypeCompatType>
	{
		type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
		type MetagraphInfo: MetagraphInfoProvider<Self>;
		type MetricsInfo: MetricsInfoProvider<Self>;
		// type IpfsInfo: IpfsInfoProvider<Self>;
		/// The minimum amount that must be staked by a miner
		#[pallet::constant]
		type MinerStakeThreshold: Get<u32>;

		#[pallet::constant]
		type ChainDecimals: Get<u32>;

		/// The pallet's id, used for deriving its sovereign account ID.
		#[pallet::constant]
		type PalletId: Get<PalletId>;

		/// Initial fixed fee for each node type
		#[pallet::constant]
		type StorageMinerInitialFee: Get<BalanceOf<Self>>;
		#[pallet::constant]
		type ValidatorInitialFee: Get<BalanceOf<Self>>;
		#[pallet::constant]
		type ComputeMinerInitialFee: Get<BalanceOf<Self>>;

		#[pallet::constant]
		type GpuMinerInitialFee: Get<BalanceOf<Self>>;
		#[pallet::constant]
		type StorageMiners3InitialFee: Get<BalanceOf<Self>>;

		#[pallet::constant]
		type BlocksPerDay: Get<u32>;

		type ProxyTypeCompatType: ProxyTypeCompat;

		#[pallet::constant]
		type NodeCooldownPeriod: Get<BlockNumberFor<Self>>;

		#[pallet::constant]
		type MaxDeregRequestsPerPeriod: Get<u32>;

		#[pallet::constant]
		type ConsensusThreshold: Get<u32>;

		type ConsensusPeriod: Get<BlockNumberFor<Self>>;

		#[pallet::constant]
		type EpochDuration: Get<u32>;

		#[pallet::constant]
		type ReportRequestsClearInterval: Get<u32>;
	}

	#[pallet::pallet]
	#[pallet::without_storage_info]
	pub struct Pallet<T>(_);

	#[pallet::storage]
	#[pallet::getter(fn is_node_type_disabled)]
	pub type DisabledNodeTypes<T: Config> =
		StorageMap<_, Blake2_128Concat, NodeType, bool, ValueQuery>;

	// Saves all the miners who have pinned for that file hash
	#[pallet::storage]
	#[pallet::getter(fn main_node_registration)]
	pub type ColdkeyNodeRegistration<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		Vec<u8>,
		Option<NodeInfo<BlockNumberFor<T>, T::AccountId>>,
		ValueQuery,
	>;

	/// New minimal coldkey/main-node registration storage.
	#[pallet::storage]
	#[pallet::getter(fn main_node_registration_v2)]
	pub type ColdkeyNodeRegistrationV2<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		Vec<u8>,
		Option<ColdkeyNodeInfoLite<BlockNumberFor<T>, T::AccountId>>,
		ValueQuery,
	>;

	/// Storage for banned account IDs
	#[pallet::storage]
	#[pallet::getter(fn banned_accounts)]
	pub type BannedAccounts<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		T::AccountId, // The account ID that is banned
		bool,         // Always true, just using a map for existence check
		ValueQuery,
	>;

	#[pallet::storage]
	#[pallet::getter(fn is_validator_whitelist_enabled)]
	pub type ValidatorWhitelistEnabled<T: Config> = StorageValue<_, bool, ValueQuery>;

	/// Tracks when nodes were deregistered
	#[pallet::storage]
	#[pallet::getter(fn last_deregistered_nodes)]
	pub type NodeLastDeregisteredAt<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		Vec<u8>,           // Node ID
		BlockNumberFor<T>, // Block number of deregistration
		ValueQuery,
	>;

	#[pallet::storage]
	#[pallet::getter(fn whitelisted_validators)]
	pub type WhitelistedValidators<T: Config> =
		StorageValue<_, BoundedVec<T::AccountId, ConstU32<{ 16 * 1024 }>>, ValueQuery>;

	// Saves all the miners who have pinned for that file hash
	#[pallet::storage]
	#[pallet::getter(fn get_node_info)]
	pub type NodeRegistration<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		Vec<u8>,
		Option<NodeInfo<BlockNumberFor<T>, T::AccountId>>,
		ValueQuery,
	>;

	#[pallet::storage]
	#[pallet::getter(fn reports_submission_count)]
	pub type ReportSubmissionCount<T: Config> =
		StorageMap<_, Blake2_128Concat, Vec<u8>, u32, ValueQuery>;

	#[pallet::storage]
	pub type TemporaryDeregistrationReports<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		T::AccountId,
		Vec<DeregistrationReport<BlockNumberFor<T>>>,
		ValueQuery,
	>;

	// Add new storage items
	#[pallet::storage]
	#[pallet::getter(fn fee_charging_enabled)]
	pub type FeeChargingEnabled<T: Config> = StorageValue<_, bool, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn current_node_type_fee)]
	pub type CurrentNodeTypeFee<T: Config> =
		StorageMap<_, Blake2_128Concat, NodeType, BalanceOf<T>, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn last_registration_block)]
	pub type LastRegistrationBlock<T: Config> =
		StorageMap<_, Blake2_128Concat, NodeType, BlockNumberFor<T>, ValueQuery>;

	/// One-shot challenge guard (replay protection)
	#[pallet::storage]
	pub type UsedChallenges<T: Config> =
		StorageMap<_, Blake2_128Concat, H256, BlockNumberFor<T>, ValueQuery>;

	/// Remember the bound libp2p identities (by your node_id)
	#[pallet::storage]
	#[pallet::getter(fn libp2p_main_identity)]
	pub type Libp2pMainIdentity<T: Config> =
		StorageMap<_, Blake2_128Concat, Vec<u8>, (Libp2pKeyType, Vec<u8>), OptionQuery>;

	#[pallet::storage]
	#[pallet::getter(fn libp2p_ipfs_identity)]
	pub type Libp2pIpfsIdentity<T: Config> =
		StorageMap<_, Blake2_128Concat, Vec<u8>, (Libp2pKeyType, Vec<u8>), OptionQuery>;

	#[pallet::storage]
	#[pallet::getter(fn is_deregistration_enabled)]
	pub type DeregistrationEnabled<T: Config> = StorageValue<_, bool, ValueQuery>;

	#[pallet::storage]
	#[pallet::getter(fn owner_to_node)]
	pub type OwnerToNode<T: Config> = StorageMap<_, Blake2_128Concat, T::AccountId, Vec<Vec<u8>>>;

	#[pallet::event]
	#[pallet::generate_deposit(pub fn deposit_event)]
	pub enum Event<T: Config> {
		NodeRegistered {
			node_id: Vec<u8>,
		},
		MainNodeRegistered {
			node_id: Vec<u8>,
		},
		NodeUnregistered {
			node_id: Vec<u8>,
		},
		/// Emitted when multiple nodes are unregistered in a batch
		NodeUnregisteredBatch {
			/// Number of nodes that were unregistered
			count: u32,
		},
		NodeStatusUpdated {
			node_id: Vec<u8>,
			status: Status,
		},
		/// Fee charging status changed
		FeeChargingStatusChanged {
			enabled: bool,
		},
		/// Fee percentage changed
		FeePercentageChanged {
			new_percentage: u16,
		},
		/// Node type fee updated
		NodeTypeFeeUpdated {
			node_type: NodeType,
			fee: BalanceOf<T>,
		},
		NodeTypeDisabledChanged {
			node_type: NodeType,
			disabled: bool,
		},
		NodeOwnerSwapped {
			node_id: Vec<u8>,
			new_owner: T::AccountId,
		},
		DeregistrationConsensusReached {
			node_id: Vec<u8>,
		},
		DeregistrationConsensusFailed {
			node_id: Vec<u8>,
		},
		AccountBanStatusChanged {
			account: T::AccountId,
			banned: bool,
		},
		WhitelistUpdated,
		/// A node was successfully verified
		NodeVerified {
			node_id: Vec<u8>,
			owner: T::AccountId,
		},
		/// A coldkey node was successfully verified
		ColdkeyNodeVerified {
			node_id: Vec<u8>,
			owner: T::AccountId,
		},
		/// Emitted when the de-registration status is changed
		DeregistrationStatusChanged {
			enabled: bool,
		},
	}

	#[pallet::error]
	pub enum Error<T> {
		NoneValue,
		StorageOverflow,
		IpfsNodeIdRequired,
		NodeAlreadyRegistered,
		NodeNotFound,
		NotAminer,
		IpfsNodeIdAlreadyRegistered,
		AddressUidNotFoundOnBittensor,
		InvalidAccountId,
		InsufficientStake,
		InsufficientBalanceForFee,
		InsufficientCreditsForFee,
		FeeTooHigh,
		NodeTypeDisabled,
		NodeTypeMismatch,
		NodeNotRegistered,
		DeregistrationDisabled,
		NotNodeOwner,
		NotAProxyAccount,
		InvalidProxyType,
		AccountNotRegistered,
		NodeNotInUids,
		NodeCooldownPeriodNotExpired,
		OwnerAlreadyRegistered,
		InvalidNodeType,
		NodeNotDegradedStorageMiner,
		TooManyRequests,
		AccountBanned,
		ExceededMaxWhitelistedValidators,
		NodeNotWhitelisted,
		InvalidSignature,
		InvalidKeyType,
		InvalidChallenge,
		InvalidChallengeDomain,
		ChallengeExpired,
		ChallengeReused,
		GenesisMismatch,
		PublicKeyMismatch,
		ChallengeMismatch,
		NodeIdMismatch,
		TooManyUnverifiedNodes,
		NodeAlreadyVerified,
		Unauthorized,
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_initialize(_n: BlockNumberFor<T>) -> Weight {
			let mut reads: u64 = 0;
			let mut writes: u64 = 0;

			// Initialize fees if not already set
			reads = reads.saturating_add(1);
			if CurrentNodeTypeFee::<T>::iter().count() == 0 {
				Self::initialize_node_type_fees();
				writes = writes.saturating_add(5);
			}

			let report_clear_interval = <T as pallet::Config>::ReportRequestsClearInterval::get();

			// Clear entries every 1000 blocks
			if _n % report_clear_interval.into() == 0u32.into() {
				// Iterate through all entries and remove them
				for (node_id, _count) in ReportSubmissionCount::<T>::iter() {
					reads = reads.saturating_add(1);
					ReportSubmissionCount::<T>::remove(&node_id);
					writes = writes.saturating_add(1);
				}
			}

			let consensus_period = <T as pallet::Config>::ConsensusPeriod::get();
			if _n % consensus_period == 0u32.into() {
				// Conservative: consensus touches reports + may unregister nodes.
				reads = reads.saturating_add(10);
				writes = writes.saturating_add(10);
				Self::apply_deregistration_consensus();
			}
			let epoch_clear_interval = <T as pallet::Config>::EpochDuration::get();
			let first_epoch_block = 38u32.into(); // hardcoded or derived
			
			// Use saturating_sub to prevent underflow
			if _n >= first_epoch_block {
				if (_n - first_epoch_block) % epoch_clear_interval.into() == 0u32.into() {
					// Clear deregistration reports
					let _ = TemporaryDeregistrationReports::<T>::clear(u32::MAX, None);
					// Conservative: clear can touch many keys; count at least 1 read.
					reads = reads.saturating_add(1);
					writes = writes.saturating_add(1);
				}
			}

			if _n % 100u32.into() == 0u32.into() {
				// Remove duplicate owners node registrations
				let mut owner_node_counts: BTreeMap<T::AccountId, usize> = BTreeMap::new();

				// Collect all node registrations with their owners
				let mut all_registrations: Vec<(Vec<u8>, T::AccountId, bool)> = Vec::new();

				// Collect from ColdkeyNodeRegistrationV2
				for (node_id, node_info) in ColdkeyNodeRegistrationV2::<T>::iter() {
					reads = reads.saturating_add(1);
					if let Some(info) = node_info {
						all_registrations.push((node_id, info.owner.clone(), true));
						*owner_node_counts.entry(info.owner).or_default() += 1;
					}
				}

				// Group registrations by owner
				let mut owner_registrations: BTreeMap<T::AccountId, Vec<(Vec<u8>, bool)>> =
					BTreeMap::new();
				for (node_id, owner, is_coldkey) in all_registrations {
					owner_registrations.entry(owner).or_default().push((node_id, is_coldkey));
				}

				// Remove registrations for owners with multiple registrations or across different storage types
				for (owner, registrations) in owner_registrations {
					// If an owner has more than one registration total
					if owner_node_counts.get(&owner).copied().unwrap_or(0) > 1
						|| registrations.iter().any(|(_, is_coldkey)| *is_coldkey)
							&& registrations.iter().any(|(_, is_coldkey)| !*is_coldkey)
					{
						// Remove all registrations for this owner
						for (node_id, is_coldkey) in registrations {
							if is_coldkey {
								ColdkeyNodeRegistrationV2::<T>::remove(&node_id);
								writes = writes.saturating_add(1);
							}
						}
					}
				}
			}

			// GC used challenges (keep it light)
			UsedChallenges::<T>::iter()
				.filter(|(_, until)| *until <= _n)
				.map(|(h, _)| h)
				.collect::<Vec<_>>()
				.into_iter()
				.for_each(|h| {
					reads = reads.saturating_add(1);
					UsedChallenges::<T>::remove(h);
					writes = writes.saturating_add(1);
				});

			T::DbWeight::get().reads_writes(reads, writes)
		}
	}

	#[pallet::call]
	impl<T: Config> Pallet<T> {
		#[pallet::call_index(0)]
		#[pallet::weight((0, Pays::No))]
		pub fn force_register_coldkey_node(
			origin: OriginFor<T>,
			owner: T::AccountId,
			node_type: NodeType,
			node_id: Vec<u8>,
		) -> DispatchResultWithPostInfo {
			ensure_root(origin)?;

			// Check if the account is banned
			ensure!(!Self::is_account_banned(&owner), Error::<T>::AccountBanned);

			// Check if the owner already has a registered node
			ensure!(!Self::is_owner_node_registered(&owner), Error::<T>::OwnerAlreadyRegistered);

			// Check if the node is already registered
			ensure!(
				!ColdkeyNodeRegistrationV2::<T>::contains_key(&node_id),
				Error::<T>::NodeAlreadyRegistered
			);

			// Get the current block number
			let current_block_number = <frame_system::Pallet<T>>::block_number();
			let owner_for_index = owner.clone();

			let node_info = ColdkeyNodeInfoLite {
				node_id: node_id.clone(),
				node_type,
				status: Status::Online,
				registered_at: current_block_number,
				owner,
			};
			ColdkeyNodeRegistrationV2::<T>::insert(node_id.clone(), Some(node_info));
			Self::owner_to_node_add(&owner_for_index, &node_id);

			Self::deposit_event(Event::MainNodeRegistered { node_id });
			Ok(().into())
		}

		#[pallet::call_index(1)]
		#[pallet::weight((0, Pays::No))]
		pub fn register_node_with_coldkey(
			origin: OriginFor<T>,
			node_type: NodeType,
			node_id: Vec<u8>,
			pay_in_credits: bool,
			owner: T::AccountId,
			main_key_type: Libp2pKeyType,
			main_public_key: Vec<u8>,
			main_sig: Vec<u8>,
			challenge_bytes: Vec<u8>,
			node_id_hex: Vec<u8>,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// --- Decode & check the challenge ---
			let mut rdr = &challenge_bytes[..];
			let ch: RegisterChallenge<T::AccountId, BlockNumberFor<T>> =
				RegisterChallenge::decode(&mut rdr).map_err(|_| Error::<T>::InvalidChallenge)?;

			const EXPECTED_DOMAIN: [u8; 24] = *b"HIPPIUS::REGISTER::v1\0\0\0";
			ensure!(ch.domain == EXPECTED_DOMAIN, Error::<T>::InvalidChallengeDomain);
			ensure!(ch.account == owner, Error::<T>::InvalidAccountId);
			ensure!(
				ch.expires_at >= <frame_system::Pallet<T>>::block_number(),
				Error::<T>::ChallengeExpired
			);
			ensure!(ch.genesis_hash == Self::genesis_hash_bytes(), Error::<T>::GenesisMismatch);
			ensure!(ch.node_id_hash == Self::blake256(&node_id_hex), Error::<T>::ChallengeMismatch);
	
			let ch_hash = Self::blake256(&challenge_bytes);
			ensure!(!UsedChallenges::<T>::contains_key(ch_hash), Error::<T>::ChallengeReused);

			// --- Verify the libp2p signature (ed25519 now; extend later if needed) ---
			ensure!(matches!(main_key_type, Libp2pKeyType::Ed25519), Error::<T>::InvalidKeyType);

			// Verify signatures
			ensure!(
				Self::verify_ed25519(&challenge_bytes, &main_sig, &main_public_key),
				Error::<T>::InvalidSignature
			);

			// Verify that public keys match the expected peer IDs
			ensure!(
				Self::verify_peer_id(&main_public_key, &node_id_hex, main_key_type),
				Error::<T>::PublicKeyMismatch
			);

			// Mark challenge used (replay protection)
			UsedChallenges::<T>::insert(ch_hash, ch.expires_at);

			// Check if the account is banned
			ensure!(!Self::is_account_banned(&owner), Error::<T>::AccountBanned);

			// Check if the owner already has a registered node
			ensure!(!Self::is_owner_node_registered(&owner), Error::<T>::OwnerAlreadyRegistered);

			// Check cooldown period
			let current_block = <frame_system::Pallet<T>>::block_number();
			let last_deregistered = NodeLastDeregisteredAt::<T>::get(&node_id);
			let cooldown_period = T::NodeCooldownPeriod::get();
			ensure!(
				current_block >= last_deregistered + cooldown_period,
				Error::<T>::NodeCooldownPeriodNotExpired
			);

			// Check if the node is already registered
			ensure!(
				!ColdkeyNodeRegistrationV2::<T>::contains_key(&node_id),
				Error::<T>::NodeAlreadyRegistered
			);

			// Check if the node type is disabled
			ensure!(!Self::is_node_type_disabled(node_type.clone()), Error::<T>::NodeTypeDisabled);

			// If the node type is Validator, ensure minimum stake
			if node_type == NodeType::Validator {
				// You'll need to implement a way to check the staked amount
				// This is a placeholder - replace with actual stake checking logic
				ensure!(
					pallet_staking::Pallet::<T>::ledger(sp_staking::StakingAccount::Stash(
						owner.clone()
					))
					.map(|ledger| ledger.active)
					.unwrap_or_default()
						>= T::MinerStakeThreshold::get().into(),
					Error::<T>::InsufficientStake
				);
			}

			// Get all UIDs using the MetagraphInfo provider
			let uids = T::MetagraphInfo::get_all_uids();

			if let Ok(account_bytes) = owner.clone().encode().try_into() {
				let account = AccountId32::new(account_bytes);
				let owner_ss58 = AccountId32::new(account.encode().try_into().unwrap_or_default())
					.to_ss58check();

				// Check if the caller is in UIDs
				let is_in_uids =
					uids.iter().any(|uid| uid.substrate_address.to_ss58check() == owner_ss58);

				// Check if the caller is not in UIDs and return an error
				ensure!(is_in_uids, Error::<T>::NodeNotInUids);

				// If the caller is in UIDs, check if the node_type matches the role
				if is_in_uids {
					let whitelist = T::MetagraphInfo::get_whitelisted_validators();
					let is_whitelisted = whitelist.iter().any(|validator| validator == &owner);

					if !is_whitelisted {
						if let Some(uid) = uids
							.iter()
							.find(|uid| uid.substrate_address.to_ss58check() == owner_ss58)
						{
							ensure!(uid.role == node_type.to_role(), Error::<T>::NodeTypeMismatch);
						}
					}
				}

				// Check if fee charging is enabled
				if Self::fee_charging_enabled() {
					// Calculate dynamic fee based on node type
					let fee = Self::calculate_dynamic_fee(node_type.clone());

					// Ensure user has sufficient balance
					ensure!(
						<pallet_balances::Pallet<T>>::free_balance(&owner) >= fee,
						Error::<T>::InsufficientBalanceForFee
					);

					if !pay_in_credits {
						// Transfer fee to the pallet's account
						<pallet_balances::Pallet<T>>::transfer(
							&owner.clone(),
							&Self::account_id(),
							fee,
							ExistenceRequirement::AllowDeath,
						)?;
					} else {
						// decrease credits and mint balance
						let fee_u128: u128 = fee.try_into().unwrap_or_default();
						let current_credits = CreditsPallet::<T>::get_free_credits(&owner);
						ensure!(current_credits >= fee_u128, Error::<T>::InsufficientCreditsForFee);
						CreditsPallet::<T>::decrease_user_credits(&owner.clone(), fee_u128);
						// Deposit charge to marketplace account
						let _imbalance = pallet_balances::Pallet::<T>::deposit_creating(
							&Self::account_id(),
							fee,
						);
					}

					// Update fee after successful registration
					Self::update_fee_after_registration(node_type.clone());
				}
			}

			// Get the current block number
			let current_block_number = <frame_system::Pallet<T>>::block_number();
			let owner_for_index = owner.clone();

			let node_info = ColdkeyNodeInfoLite {
				node_id: node_id.clone(),
				node_type,
				status: Status::Online,
				registered_at: current_block_number,
				owner,
			};
			ColdkeyNodeRegistrationV2::<T>::insert(node_id.clone(), Some(node_info));
			Self::owner_to_node_add(&owner_for_index, &node_id);

			// Persist identities (for later audits/liveness checks)
			Libp2pMainIdentity::<T>::insert(node_id.clone(), (main_key_type, main_public_key));

			Self::deposit_event(Event::MainNodeRegistered { node_id });
			Ok(().into())
		}

		#[pallet::call_index(3)]
		#[pallet::weight((10_000, DispatchClass::Normal, Pays::Yes))]
		pub fn set_node_status_to_degraded(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
		) -> DispatchResult {
			// Ensure the caller is root
			ensure_root(origin)?;

			// Try to mutate the node information if it exists
			ColdkeyNodeRegistrationV2::<T>::try_mutate(&node_id, |node_info_opt| -> DispatchResult {
				// Check if the node exists in the storage map
				let node_info = node_info_opt.as_mut().ok_or(Error::<T>::NodeNotFound)?;

				// Ensure the node is of type `miner`
				if node_info.node_type != NodeType::StorageMiner {
					return Err(Error::<T>::NotAminer.into());
				}

				// Update the status to Degraded
				node_info.status = Status::Degraded;

				// Emit an event for the status update
				Self::deposit_event(Event::NodeStatusUpdated {
					node_id: node_id.clone(),
					status: Status::Degraded,
				});

				Ok(())
			})
		}

		/// Sudo function to enable or disable fee charging
		#[pallet::call_index(6)]
		#[pallet::weight((0, Pays::No))]
		pub fn set_fee_charging(origin: OriginFor<T>, enabled: bool) -> DispatchResult {
			// Ensure only root can call this
			ensure_root(origin)?;

			// Update the fee charging status
			FeeChargingEnabled::<T>::put(enabled);

			// Emit an event
			Self::deposit_event(Event::FeeChargingStatusChanged { enabled });

			Ok(())
		}

		/// Sudo function to update the fee for a specific node type
		#[pallet::call_index(7)]
		#[pallet::weight((0, Pays::No))]
		pub fn set_node_type_fee(
			origin: OriginFor<T>,
			node_type: NodeType,
			fee: BalanceOf<T>,
		) -> DispatchResult {
			// Ensure the caller is an authority
			let authority = ensure_signed(origin)?;
			CreditsPallet::<T>::ensure_is_authority(&authority)?;

			// Update the CurrentNodeTypeFee storage map for the specified node type
			<CurrentNodeTypeFee<T>>::insert(node_type.clone(), fee);

			// Deposit an event to notify about the fee update
			Self::deposit_event(Event::<T>::NodeTypeFeeUpdated { node_type, fee });

			Ok(())
		}

		#[pallet::call_index(8)]
		#[pallet::weight((0, Pays::No))]
		pub fn set_node_type_disabled(
			origin: OriginFor<T>,
			node_type: NodeType,
			disabled: bool,
		) -> DispatchResult {
			// Ensure only root can call this
			ensure_root(origin)?;

			// Update the disabled status for the specified node type
			DisabledNodeTypes::<T>::insert(node_type.clone(), disabled);

			// Emit an event
			Self::deposit_event(Event::NodeTypeDisabledChanged { node_type, disabled });

			Ok(())
		}

		#[pallet::call_index(9)]
		#[pallet::weight((0, Pays::No))]
		pub fn force_unregister_coldkey_node(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
		) -> DispatchResultWithPostInfo {
			ensure_root(origin)?;

			Self::do_set_node_status_degraded_or_unregister(node_id);

			Ok(().into())
		}

		#[pallet::call_index(10)]
		#[pallet::weight((0, Pays::No))]
		pub fn unregister_main_node(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Ensure that the node is registered
			ensure!(
				ColdkeyNodeRegistrationV2::<T>::contains_key(&node_id),
				Error::<T>::NodeNotRegistered
			);

			// Retrieve the node info to check ownership
			let node_info = Self::get_coldkey_node_info_v2(&node_id)
				.ok_or(Error::<T>::NodeNotRegistered)?;

			// Ensure the caller is the owner of the node
			ensure!(node_info.owner == who, Error::<T>::NotNodeOwner);

			// Call the existing unregister logic
			Self::do_set_node_status_degraded_or_unregister(node_id);

			Ok(().into())
		}

		#[pallet::call_index(11)]
		#[pallet::weight((0, Pays::No))]
		pub fn swap_node_owner(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			new_owner: T::AccountId,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Retrieve the node information
			let mut node_info =
				ColdkeyNodeRegistrationV2::<T>::get(&node_id).ok_or(Error::<T>::NodeNotRegistered)?;

			// Ensure the caller is the current owner
			ensure!(node_info.owner == who.clone(), Error::<T>::NotNodeOwner);

			// Ensure the new owner is not banned
			ensure!(!Self::is_account_banned(&new_owner), Error::<T>::AccountBanned);

			// Ensure the new owner is not already registered with another node
			ensure!(!Self::is_owner_node_registered(&new_owner), Error::<T>::OwnerAlreadyRegistered);


			// Update the owner
			node_info.owner = new_owner.clone();

			// Save the updated node information back to storage
			ColdkeyNodeRegistrationV2::<T>::insert(node_id.clone(), Some(node_info));

			Self::deposit_event(Event::NodeOwnerSwapped { node_id, new_owner });

			Ok(().into())
		}

		#[pallet::call_index(12)]
		#[pallet::weight((0, Pays::No))]
		pub fn submit_deregistration_report(
			origin: OriginFor<T>,
			node_ids: Vec<Vec<u8>>,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Check if de-registration is enabled
			ensure!(Self::is_deregistration_enabled(), Error::<T>::DeregistrationDisabled);

			let main_account = Self::resolve_deregistration_reporter(&who)?;
			// Check if the node is registered and is a validator
			let node_info = Self::get_registered_node_for_owner(&main_account);
			ensure!(node_info.is_some(), Error::<T>::NodeNotRegistered);
			let node_info = node_info.unwrap();
			ensure!(node_info.node_type == NodeType::Validator, Error::<T>::InvalidNodeType);

			// Rate limit: maximum storage requests per block per user
			let max_requests_per_block = <T as pallet::Config>::MaxDeregRequestsPerPeriod::get();
			let user_requests_count = ReportSubmissionCount::<T>::get(node_info.node_id.clone());
			ensure!(user_requests_count <= max_requests_per_block, Error::<T>::TooManyRequests);

			// Update user's storage requests count
			ReportSubmissionCount::<T>::insert(node_info.node_id.clone(), user_requests_count.saturating_add(1));

			let current_block = <frame_system::Pallet<T>>::block_number();
			let mut existing_reports = TemporaryDeregistrationReports::<T>::get(&main_account);
			for node_id in node_ids {
				let report =
					DeregistrationReport { node_id: node_id.clone(), created_at: current_block };
				log::info!(
					"Submitting deregistration report for node id {}",
					String::from_utf8_lossy(&node_id)
				);
				existing_reports.push(report);
			}
			TemporaryDeregistrationReports::<T>::insert(&main_account, existing_reports);

			Ok(().into())
		}

		/// Ban or unban an account from registering nodes
		#[pallet::call_index(13)]
		#[pallet::weight((0, Pays::No))]
		pub fn set_account_ban_status(
			origin: OriginFor<T>,
			account: T::AccountId,
			banned: bool,
		) -> DispatchResult {
			ensure_root(origin)?;

			if banned {
				BannedAccounts::<T>::insert(&account, true);
			} else {
				BannedAccounts::<T>::remove(&account);
			}

			// Emit an event for the ban status change
			Self::deposit_event(Event::AccountBanStatusChanged { account, banned });

			Ok(())
		}

		/// Set the list of whitelisted validators
		///
		/// Can only be called by root.
		#[pallet::call_index(14)]
		#[pallet::weight((0, Pays::No))]
		pub fn set_whitelisted_validators(
			origin: OriginFor<T>,
			validators: Vec<T::AccountId>,
		) -> DispatchResult {
			ensure_root(origin)?;

			// Convert to bounded vec
			let bounded_validators =
				BoundedVec::<T::AccountId, ConstU32<{ 16 * 1024 }>>::try_from(validators)
					.map_err(|_| Error::<T>::ExceededMaxWhitelistedValidators)?;

			WhitelistedValidators::<T>::put(bounded_validators);

			Self::deposit_event(Event::WhitelistUpdated);
			Ok(())
		}

		/// Toggle the de-registration switch (root only)
		#[pallet::call_index(15)]
		#[pallet::weight((10_000, Pays::No))]
		pub fn set_deregistration_enabled(origin: OriginFor<T>, enabled: bool) -> DispatchResult {
			// Only root can call this function
			ensure_root(origin)?;

			// Update the de-registration enabled state
			DeregistrationEnabled::<T>::put(enabled);

			// Emit an event
			Self::deposit_event(Event::DeregistrationStatusChanged { enabled });

			Ok(())
		}
	}

	impl<T: Config> Pallet<T> {
		fn coldkey_lite_to_node_info(
			lite: ColdkeyNodeInfoLite<BlockNumberFor<T>, T::AccountId>,
		) -> NodeInfo<BlockNumberFor<T>, T::AccountId> {
			NodeInfo {
				node_id: lite.node_id.clone(),
				node_type: lite.node_type,
				ipfs_node_id: None,
				status: lite.status,
				registered_at: lite.registered_at,
				owner: lite.owner,
				is_verified: true,
			}
		}

		/// Get coldkey/main-node registration info (v2 only).
		fn get_coldkey_node_info_v2(
			node_id: &Vec<u8>,
		) -> Option<NodeInfo<BlockNumberFor<T>, T::AccountId>> {
			ColdkeyNodeRegistrationV2::<T>::get(node_id)
				.map(Self::coldkey_lite_to_node_info)
		}

		fn owner_to_node_add(owner: &T::AccountId, node_id: &Vec<u8>) {
			OwnerToNode::<T>::mutate(owner, |nodes_opt| {
				let nodes = nodes_opt.get_or_insert_with(Vec::new);
				if !nodes.iter().any(|n| n == node_id) {
					nodes.push(node_id.clone());
				}
			});
		}

		fn owner_to_node_remove(owner: &T::AccountId, node_id: &Vec<u8>) {
			OwnerToNode::<T>::mutate_exists(owner, |nodes_opt| {
				if let Some(nodes) = nodes_opt.as_mut() {
					nodes.retain(|n| n != node_id);
					if nodes.is_empty() {
						*nodes_opt = None;
					}
				}
			});
		}

		pub fn get_all_degraded_storage_miners() -> Vec<Vec<u8>> {
			let mut degraded_nodes = Vec::new();

			// Iterate over ColdkeyNodeRegistrationV2 storage
			for (node_id, maybe_node_info) in <ColdkeyNodeRegistrationV2<T>>::iter() {
				if let Some(node_info) = maybe_node_info {
					if node_info.node_type == NodeType::StorageMiner
						&& matches!(node_info.status, Status::Degraded)
					{
						degraded_nodes.push(node_id);
					}
				}
			}

			degraded_nodes
		}

		pub fn get_registration_block(node_id: &Vec<u8>) -> Option<BlockNumberFor<T>> {
			// Check ColdkeyNodeRegistrationV2 storage
			if let Some(node_info) = ColdkeyNodeRegistrationV2::<T>::get(node_id) {
				return Some(node_info.registered_at);
			}
			None
		}

		/// Helper function to check if an owner already has a registered node
		pub fn is_owner_node_registered(owner: &T::AccountId) -> bool {
			OwnerToNode::<T>::contains_key(owner)
		}

		/// Fetch all registered miners whose status is not degraded
		pub fn get_all_active_storage_miners() -> Vec<NodeInfo<BlockNumberFor<T>, T::AccountId>> {
			// Vector to store filtered node info
			let mut active_storage_miners = Vec::new();
			let mut seen_node_ids = Vec::new(); // Vec to track unique node IDs

			// Iterate over all registered nodes in the ColdkeyNodeRegistrationV2 storage map
			for (node_id, node_info_opt) in ColdkeyNodeRegistrationV2::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Check if the node is of type miner and its status is not Degraded
					if matches!(node_info.node_type, NodeType::StorageMiner)
						&& !matches!(node_info.status, Status::Degraded)
					{
						// Check for uniqueness
						if !seen_node_ids.contains(&node_id) {
							seen_node_ids.push(node_id.clone());
							active_storage_miners.push(Self::coldkey_lite_to_node_info(node_info));
						}
					}
				}
			}

			active_storage_miners
		}

		/// Fetch all registered miners whose status is not degraded
		pub fn get_all_active_compute_miners() -> Vec<NodeInfo<BlockNumberFor<T>, T::AccountId>> {
			// Vector to store filtered node info
			let mut active_storage_miners = Vec::new();
			let mut seen_node_ids = Vec::new(); // Vec to track unique node IDs

			// Iterate over all registered nodes in the ColdkeyNodeRegistrationV2 storage map
			for (node_id, node_info_opt) in ColdkeyNodeRegistrationV2::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Check if the node is of type miner and its status is not Degraded
					if matches!(node_info.node_type, NodeType::ComputeMiner)
						&& !matches!(node_info.status, Status::Degraded)
					{
						// Check for uniqueness
						if !seen_node_ids.contains(&node_id) {
							seen_node_ids.push(node_id.clone());
							active_storage_miners.push(Self::coldkey_lite_to_node_info(node_info));
						}
					}
				}
			}

			active_storage_miners
		}

		pub fn get_miner_info(account_id: T::AccountId) -> Option<(NodeType, Status)> {
			let mut seen_accounts = Vec::new(); // Vec to track unique account IDs

			// Iterate through the storage map to find the miner associated with the account_id in ColdkeyNodeRegistrationV2
			for (_, node_info) in ColdkeyNodeRegistrationV2::<T>::iter() {
				if let Some(info) = node_info {
					if info.owner == account_id && !seen_accounts.contains(&info.owner) {
						seen_accounts.push(info.owner.clone());
						return Some((info.node_type, info.status));
					}
				}
			}

			// Return None if the account_id is not a miner
			None
		}

		/// Fetch all registered miners who have staked
		pub fn get_all_storage_miners_with_min_staked(
		) -> Vec<NodeInfo<BlockNumberFor<T>, T::AccountId>> {
			// Vector to store filtered node info
			let mut active_storage_miners = Vec::new();
			let mut seen_node_ids = Vec::new(); // Vec to track unique node IDs

			// Iterate over all registered nodes in the ColdkeyNodeRegistrationV2 storage map
			for (node_id, node_info_opt) in ColdkeyNodeRegistrationV2::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Check if the node is of type miner
					if matches!(node_info.node_type, NodeType::StorageMiner)
						&& !matches!(node_info.status, Status::Degraded)
					{
						// Get the staked balance for this miner
						let staked_balance = pallet_staking::Pallet::<T>::ledger(
							sp_staking::StakingAccount::Stash(node_info.owner.clone()),
						)
						.map(|ledger| ledger.active)
						.unwrap_or_default();

						// Convert the threshold to staking currency balance type
						let threshold: <T as pallet_staking::Config>::CurrencyBalance =
							(T::MinerStakeThreshold::get() as u128)
								.saturating_mul(10u128.pow(T::ChainDecimals::get()))
								.try_into()
								.unwrap_or_default();

						// Check if staked balance is greater than threshold
						if staked_balance >= threshold {
							// Check for uniqueness
							if !seen_node_ids.contains(&node_id) {
								seen_node_ids.push(node_id.clone());
								active_storage_miners.push(Self::coldkey_lite_to_node_info(node_info));
							}
						}
					}
				}
			}

			active_storage_miners
		}

		/// Fetch all registered miners who have staked more than 600
		pub fn get_all_nodes_with_min_staked() -> Vec<NodeInfo<BlockNumberFor<T>, T::AccountId>> {
			// Vector to store filtered node info
			let mut active_storage_miners = Vec::new();
			let mut seen_node_ids = Vec::new(); // Vec to track unique node IDs

			// Iterate over all registered nodes in the ColdkeyNodeRegistrationV2 storage map
			for (node_id, node_info_opt) in ColdkeyNodeRegistrationV2::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Get the staked balance for this miner
					let staked_balance = pallet_staking::Pallet::<T>::ledger(
						sp_staking::StakingAccount::Stash(node_info.owner.clone()),
					)
					.map(|ledger| ledger.active)
					.unwrap_or_default();

					// Convert the threshold to staking currency balance type
					let threshold: <T as pallet_staking::Config>::CurrencyBalance =
						(T::MinerStakeThreshold::get() as u128)
							.saturating_mul(10u128.pow(T::ChainDecimals::get()))
							.try_into()
							.unwrap_or_default();

					// Check if staked balance is greater than threshold
					if staked_balance >= threshold {
						// Check for uniqueness
						if !seen_node_ids.contains(&node_id) {
							seen_node_ids.push(node_id.clone());
							active_storage_miners.push(Self::coldkey_lite_to_node_info(node_info));
						}
					}
				}
			}

			active_storage_miners
		}

		pub fn get_primary_account(
			proxy: &T::AccountId,
		) -> Result<Option<T::AccountId>, DispatchError> {
			// First check if this is actually a main account
			let (proxy_definitions, _) = pallet_proxy::Pallet::<T>::proxies(proxy);
			if !proxy_definitions.is_empty() {
				return Ok(Some(proxy.clone()));
			}

			// Get all validator nodes and their owners
			let validator_nodes = Self::get_all_nodes_by_node_type(NodeType::Validator);

			let mut main_account = None;

			// Iterate through validator owners
			for node_info in validator_nodes {
				let (proxies, _) = pallet_proxy::Pallet::<T>::proxies(&node_info.owner);

				for p in proxies.iter() {
					if &p.delegate == proxy {
						main_account = Some(node_info.owner.clone());
						break;
					}
				}

				if main_account.is_some() {
					break;
				}
			}

			Ok(main_account)
		}

		/// Validator owner used for deregistration report storage and consensus (one identity per validator).
		fn resolve_deregistration_reporter(
			issuer: &T::AccountId,
		) -> Result<T::AccountId, DispatchError> {
			Ok(match Self::get_primary_account(issuer)? {
				Some(owner) => owner,
				None => issuer.clone(),
			})
		}

		pub fn get_chain_decimals() -> u32 {
			T::ChainDecimals::get()
		}

		/// Fetch all registered miners whose status is not degraded
		pub fn get_all_nodes_by_node_type(
			node_type: NodeType,
		) -> Vec<NodeInfo<BlockNumberFor<T>, T::AccountId>> {
			// Vector to store filtered node info
			let mut active_nodes = Vec::new();
			let mut seen_node_ids = Vec::new(); 

			// Iterate over all registered nodes in the ColdkeyNodeRegistrationV2 storage map
			for (node_id, node_info_opt) in ColdkeyNodeRegistrationV2::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Check if the node is of the specified type and its status is not Degraded
					if node_info.node_type == node_type
						&& !matches!(node_info.status, Status::Degraded)
					{
						// Check for uniqueness
						if !seen_node_ids.contains(&node_id) {
							seen_node_ids.push(node_id.clone());
							active_nodes.push(Self::coldkey_lite_to_node_info(node_info));
						}
					}
				}
			}

			active_nodes
		}

		/// Fetch all registered miners whose status is not degraded
		pub fn get_all_coldkey_active_nodes() -> Vec<NodeInfo<BlockNumberFor<T>, T::AccountId>> {
			// Vector to store filtered node info
			let mut active_nodes = Vec::new();
			let mut seen_node_ids = Vec::new(); // Vec to track unique node IDs

			// Iterate over all registered nodes in the ColdkeyNodeRegistrationV2 storage map
			for (node_id, node_info_opt) in ColdkeyNodeRegistrationV2::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Check if the node's status is not Degraded
					if !matches!(node_info.status, Status::Degraded)
						&& matches!(node_info.node_type, NodeType::StorageMiner)
					{
						// Check for uniqueness
						if !seen_node_ids.contains(&node_id) {
							seen_node_ids.push(node_id.clone());
							active_nodes.push(Self::coldkey_lite_to_node_info(node_info));
						}
					}
				}
			}

			active_nodes
		}

		/// Fetch all registered miners whose status is not degraded
		pub fn get_all_active_nodes() -> Vec<NodeInfo<BlockNumberFor<T>, T::AccountId>> {
			// Vector to store filtered node info
			let mut active_nodes = Vec::new();
			let mut seen_node_ids = Vec::new(); // Vec to track unique node IDs

			// Iterate over all registered nodes in the ColdkeyNodeRegistrationV2 storage map
			for (node_id, node_info_opt) in ColdkeyNodeRegistrationV2::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Check if the node's status is not Degraded
					if !matches!(node_info.status, Status::Degraded) {
						// Check for uniqueness
						if !seen_node_ids.contains(&node_id) {
							seen_node_ids.push(node_id.clone());
							active_nodes.push(Self::coldkey_lite_to_node_info(node_info));
						}
					}
				}
			}

			active_nodes
		}

		pub fn account_id() -> T::AccountId {
			<T as pallet::Config>::PalletId::get().into_account_truncating()
		}

		/// Get the current balance of the registration pallet
		pub fn balance() -> BalanceOf<T> {
			pallet_balances::Pallet::<T>::free_balance(&Self::account_id())
		}

		pub fn get_registered_node(
			node_id: Vec<u8>,
		) -> Result<NodeInfo<BlockNumberFor<T>, T::AccountId>, &'static str> {
			// Use the helper function to retrieve node info
			match Self::get_node_registration_info(node_id) {
				Some(node_info) => Ok(node_info),
				None => Err("Node ID not registered or invalid data"),
			}
		}

		// Helper function to get node info from both registrations
		pub fn get_node_registration_info(
			node_id: Vec<u8>,
		) -> Option<NodeInfo<BlockNumberFor<T>, T::AccountId>> {
			if let Some(coldkey_info) = Self::get_coldkey_node_info_v2(&node_id) {
				if coldkey_info.status != Status::Degraded {
					return Some(coldkey_info);
				}
			} 
			None
		}

		fn blake256(data: &[u8]) -> H256 {
			H256::from(blake2_256(data))
		}

		fn genesis_hash_bytes() -> [u8; 32] {
			// block 0 hash
			let zero_block: BlockNumberFor<T> = Zero::zero();
			let h = <frame_system::Pallet<T>>::block_hash(zero_block);
			let mut out = [0u8; 32];
			out.copy_from_slice(h.as_ref());
			out
		}

		fn verify_ed25519(msg: &[u8], sig: &[u8], pk: &[u8]) -> bool {
			if sig.len() != 64 || pk.len() != 32 {
				return false;
			}
			sp_io::crypto::ed25519_verify(
				&sp_core::ed25519::Signature::from_raw(
					<[u8; 64]>::try_from(sig).unwrap_or([0u8; 64]),
				),
				msg,
				&sp_core::ed25519::Public::from_raw(<[u8; 32]>::try_from(pk).unwrap_or([0u8; 32])),
			)
		}

		// Add this function to verify that a public key corresponds to a peer ID
		fn verify_peer_id(public_key: &[u8], peer_id: &[u8], key_type: Libp2pKeyType) -> bool {
			match key_type {
				Libp2pKeyType::Ed25519 => {
					// For Ed25519, the peer ID should be the multihash of the public key
					// Format: \x00\x24\x08\x01\x12\x20 + public_key (32 bytes)
					if peer_id.len() != 38 {
						return false;
					}

					let expected_prefix = [0x00, 0x24, 0x08, 0x01, 0x12, 0x20];
					if &peer_id[0..6] != &expected_prefix {
						return false;
					}

					&peer_id[6..38] == public_key
				},
				// Add other key types as needed
				_ => false,
			}
		}

		fn apply_deregistration_consensus() {
			// Collect all reports from all validators
			let all_reports: Vec<(T::AccountId, Vec<DeregistrationReport<BlockNumberFor<T>>>)> =
				TemporaryDeregistrationReports::<T>::iter().collect();

			// Get threshold from registration config
			let threshold = T::ConsensusThreshold::get();

			// Group reports by node_id
			let mut reports_by_node: sp_std::collections::btree_map::BTreeMap<
				Vec<u8>,
				Vec<(T::AccountId, DeregistrationReport<BlockNumberFor<T>>)>,
			> = sp_std::collections::btree_map::BTreeMap::new();

			for (validator_id, reports) in all_reports {
				for report in reports {
					reports_by_node
						.entry(report.node_id.clone())
						.or_insert_with(Vec::new)
						.push((validator_id.clone(), report));
				}
			}

			for (node_id, reports) in reports_by_node {
				if reports.is_empty() {
					continue;
				}
			
				// One vote per validator owner (normalize proxy keys for legacy rows too).
				let unique_validators: sp_std::collections::btree_set::BTreeSet<T::AccountId> =
					reports
						.iter()
						.map(|(validator_id, _)| {
							Self::resolve_deregistration_reporter(validator_id).unwrap_or_else(
								|_| validator_id.clone(),
							)
						})
						.collect();
			
				let agreeing_validators = unique_validators.len() as u32;
			
				log::info!(
					"Node: {:?}, Total Reports: {}, Agreeing Validators: {}, Threshold: {}",
					node_id,
					reports.len(),
					agreeing_validators,
					threshold
				);
			
				if agreeing_validators >= threshold {
					// Consensus reached, unregister the node
					Self::do_unregister_main_node(node_id.clone());
					Self::deposit_event(Event::<T>::DeregistrationConsensusReached {
						node_id: node_id.clone(),
					});
				} else {
					Self::deposit_event(Event::<T>::DeregistrationConsensusFailed {
						node_id: node_id.clone(),
					});
				}
			}
		}

		/// Initialize fees for node types
		fn initialize_node_type_fees() {
			CurrentNodeTypeFee::<T>::insert(
				NodeType::StorageMiner,
				T::StorageMinerInitialFee::get(),
			);
			CurrentNodeTypeFee::<T>::insert(NodeType::Validator, T::ValidatorInitialFee::get());
			CurrentNodeTypeFee::<T>::insert(
				NodeType::ComputeMiner,
				T::ComputeMinerInitialFee::get(),
			);
			CurrentNodeTypeFee::<T>::insert(
				NodeType::StorageS3,
				T::StorageMiners3InitialFee::get(),
			);
			CurrentNodeTypeFee::<T>::insert(NodeType::GpuMiner, T::GpuMinerInitialFee::get());
		}

		/// Check if an account is banned
		fn is_account_banned(account: &T::AccountId) -> bool {
			BannedAccounts::<T>::contains_key(account)
		}

		/// Calculate dynamic fee for a node type
		fn calculate_dynamic_fee(node_type: NodeType) -> BalanceOf<T> {
			let current_block = frame_system::Pallet::<T>::block_number();
			let last_registration_block = Self::last_registration_block(node_type.clone());
			let blocks_per_day = T::BlocksPerDay::get();

			// If no registration for a day, halve the fee
			if current_block.saturating_sub(last_registration_block) > blocks_per_day.into() {
				let current_fee = Self::current_node_type_fee(node_type.clone());

				// Safely divide the fee by 2
				let halved_fee =
					current_fee.checked_div(&2u32.into()).unwrap_or_else(|| Zero::zero());

				// Update the stored fee
				CurrentNodeTypeFee::<T>::mutate(node_type.clone(), |fee| {
					*fee = halved_fee;
				});

				halved_fee
			} else {
				// Return current fee
				Self::current_node_type_fee(node_type)
			}
		}

		/// Update fee after registration
		fn update_fee_after_registration(node_type: NodeType) {
			// Double the fee
			CurrentNodeTypeFee::<T>::mutate(node_type.clone(), |fee| {
				*fee = fee.saturating_mul(2u32.into());
			});

			// Update last registration block
			let current_block = frame_system::Pallet::<T>::block_number();
			LastRegistrationBlock::<T>::insert(node_type, current_block);
		}

		/// Fetch all registered miners of a specific type whose status is not degraded
		pub fn get_active_nodes_by_type(node_type: NodeType) -> Vec<Vec<u8>> {
			// Vector to store filtered node info
			let mut active_nodes = Vec::new();
			let mut seen_node_ids = Vec::new(); // Vec to track unique node IDs

			// Iterate over all registered nodes in the ColdkeyNodeRegistrationV2 storage map
			for (node_id, node_info_opt) in ColdkeyNodeRegistrationV2::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Check if the node is of the specified type and its status is not Degraded
					if node_info.node_type == node_type
						&& !matches!(node_info.status, Status::Degraded)
					{
						// Check for uniqueness
						if !seen_node_ids.contains(&node_id) {
							seen_node_ids.push(node_id.clone());
							active_nodes.push(node_id.clone());
						}
					}
				}
			}

			active_nodes
		}

		pub fn do_unregister_main_node(node_id: Vec<u8>) {
			let owner = ColdkeyNodeRegistrationV2::<T>::get(&node_id).map(|info| info.owner);

			// Remove the main node's registration
			// Note: LinkedNodes has been removed; sub-node cleanup is handled separately
			ColdkeyNodeRegistrationV2::<T>::remove(node_id.clone());

			// Keep OwnerToNode in sync
			if let Some(owner) = owner {
				Self::owner_to_node_remove(&owner, &node_id);
			}

			T::MetricsInfo::remove_metrics(node_id.clone());
			// T::IpfsInfo::remove_miner_profile_info(node_id.clone());

			// Store the deregistration time
			NodeLastDeregisteredAt::<T>::insert(&node_id, <frame_system::Pallet<T>>::block_number());

			Self::deposit_event(Event::NodeUnregistered { node_id });
		}

		/// Helper function to get the registered node for a specific owner
		pub fn get_registered_node_for_owner(
			owner: &T::AccountId,
		) -> Option<NodeInfo<BlockNumberFor<T>, T::AccountId>> {
			// First, check ColdkeyNodeRegistrationV2
			let coldkey_node =
				ColdkeyNodeRegistrationV2::<T>::iter().find_map(|(node_id, node_info)| {
					node_info
						.filter(|info| info.owner == *owner && info.status != Status::Degraded)
						.map(Self::coldkey_lite_to_node_info)
				});

			coldkey_node
		}

		pub fn do_set_node_status_degraded_or_unregister(node_id: Vec<u8>) {
			if ColdkeyNodeRegistrationV2::<T>::contains_key(&node_id) {
				// Handle main node (ColdkeyNodeRegistrationV2)
				if let Some(mut node_info) = ColdkeyNodeRegistrationV2::<T>::get(&node_id) {
					if node_info.node_type == NodeType::StorageMiner {
						// Update status to Degraded for StorageMiner
						node_info.status = Status::Degraded;
						ColdkeyNodeRegistrationV2::<T>::insert(&node_id, Some(node_info));

						// Emit event for status update
						Self::deposit_event(Event::NodeStatusUpdated {
							node_id,
							status: Status::Degraded,
						});
					} else {
						// Deregister non-StorageMiner main nodes
						Self::do_unregister_main_node(node_id.clone());
					}
				}
			}
		}
	}
}

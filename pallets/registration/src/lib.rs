#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;
pub use types::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

mod types;

// #[cfg(feature = "runtime-benchmarks")]
// mod benchmarking;

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
		traits::{
			// tokens::ExistenceRequirement,
			Get,
			// One,
		},
	};
	use frame_system::{offchain::SendTransactionTypes, pallet_prelude::*};
	use pallet_credits::Pallet as CreditsPallet;
	use pallet_proxy::Pallet as ProxyPallet;
	use pallet_utils::IpfsInfoProvider;
	use pallet_utils::{MetagraphInfoProvider, MetricsInfoProvider};
	use scale_info::prelude::string::String;
	use scale_info::prelude::*;
	use scale_info::scale;
	use sp_core::crypto::Ss58Codec;
	use sp_core::H256;
	use sp_io::hashing::blake2_256;
	use sp_runtime::traits::AccountIdConversion;
	use sp_runtime::traits::CheckedDiv;
	use sp_runtime::traits::Zero;
	use sp_runtime::AccountId32;
	use sp_runtime::Saturating;
	use sp_std::collections::btree_map::BTreeMap;
	use sp_std::collections::btree_set::BTreeSet;
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
		type IpfsInfo: IpfsInfoProvider<Self>;
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

	/// Stores the linked node IDs for each main node
	#[pallet::storage]
	#[pallet::getter(fn linked_nodes)]
	pub type LinkedNodes<T: Config> = StorageMap<
		_,
		Blake2_128Concat,
		Vec<u8>,      // The node_id of the main node
		Vec<Vec<u8>>, // The vector of linked node IDs
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
		FeeTooHigh,
		NodeTypeDisabled,
		NodeTypeMismatch,
		NodeNotRegistered,
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
		/// Batch unregistration failed due to too many nodes
		TooManyUnverifiedNodes,
		NodeAlreadyVerified,
		Unauthorized,
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_initialize(_n: BlockNumberFor<T>) -> Weight {
			// Initialize fees if not already set
			if CurrentNodeTypeFee::<T>::iter().count() == 0 {
				Self::initialize_node_type_fees();
			}

			let report_clear_interval = <T as pallet::Config>::ReportRequestsClearInterval::get();

			// Clear entries every 1000 blocks
			if _n % report_clear_interval.into() == 0u32.into() {
				// Iterate through all entries in HardwareRequestsCount
				ReportSubmissionCount::<T>::iter().for_each(|(node_id, _count)| {
					ReportSubmissionCount::<T>::remove(&node_id);
				});
			}

			let consensus_period = <T as pallet::Config>::ConsensusPeriod::get();
			if _n % consensus_period == 0u32.into() {
				Self::apply_deregistration_consensus();
			}

			let epoch_clear_interval = <T as pallet::Config>::EpochDuration::get();
			let first_epoch_block = 38u32.into(); // hardcoded or derived

			if (_n - first_epoch_block) % epoch_clear_interval.into() == 0u32.into() {
				// Clear deregistration reports
				let _ = TemporaryDeregistrationReports::<T>::clear(u32::MAX, None);
			}

			if _n % 100u32.into() == 0u32.into() {
				// Perform periodic cleanup tasks
				Self::remove_duplicate_linked_nodes();

				// Remove LinkedNodes entries for main nodes without corresponding ColdkeyNodeRegistration
				LinkedNodes::<T>::iter_keys().collect::<Vec<_>>().into_iter().for_each(
					|main_node_id| {
						if ColdkeyNodeRegistration::<T>::get(&main_node_id).is_none() {
							T::MetricsInfo::remove_metrics(main_node_id.clone());
							T::IpfsInfo::remove_miner_profile_info(main_node_id.clone());
							LinkedNodes::<T>::remove(&main_node_id);
						}
					},
				);

				// Remove duplicate owners node registrations
				let mut owner_node_counts: BTreeMap<T::AccountId, usize> = BTreeMap::new();

				// Collect all node registrations with their owners
				let mut all_registrations: Vec<(Vec<u8>, T::AccountId, bool)> = Vec::new();

				// Collect from ColdkeyNodeRegistration
				ColdkeyNodeRegistration::<T>::iter().for_each(|(node_id, node_info)| {
					if let Some(info) = node_info {
						all_registrations.push((node_id, info.owner.clone(), true));
						*owner_node_counts.entry(info.owner).or_default() += 1;
					}
				});

				// Collect from NodeRegistration
				NodeRegistration::<T>::iter().for_each(|(node_id, node_info)| {
					if let Some(info) = node_info {
						all_registrations.push((node_id, info.owner.clone(), false));
						*owner_node_counts.entry(info.owner).or_default() += 1;
					}
				});

				// Group registrations by owner
				let mut owner_registrations: BTreeMap<T::AccountId, Vec<(Vec<u8>, bool)>> =
					BTreeMap::new();
				for (node_id, owner, is_coldkey) in all_registrations {
					owner_registrations.entry(owner).or_default().push((node_id, is_coldkey));
				}

				// Remove registrations for owners with multiple registrations or across different storage types
				for (owner, registrations) in owner_registrations {
					// If an owner has more than one registration total
					// Or has registrations in both ColdkeyNodeRegistration and NodeRegistration
					if owner_node_counts.get(&owner).copied().unwrap_or(0) > 1
						|| registrations.iter().any(|(_, is_coldkey)| *is_coldkey)
							&& registrations.iter().any(|(_, is_coldkey)| !*is_coldkey)
					{
						// Remove all registrations for this owner
						for (node_id, is_coldkey) in registrations {
							if is_coldkey {
								ColdkeyNodeRegistration::<T>::remove(&node_id);
							} else {
								NodeRegistration::<T>::remove(&node_id);
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
					UsedChallenges::<T>::remove(h);
				});

			Weight::zero()
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
			ipfs_node_id: Option<Vec<u8>>,
		) -> DispatchResultWithPostInfo {
			ensure_root(origin)?;

			// Check if the account is banned
			ensure!(!Self::is_account_banned(&owner), Error::<T>::AccountBanned);

			// Check if the owner already has a registered node
			ensure!(!Self::is_owner_node_registered(&owner), Error::<T>::OwnerAlreadyRegistered);

			// Check if the node is already registered
			ensure!(
				!ColdkeyNodeRegistration::<T>::contains_key(&node_id),
				Error::<T>::NodeAlreadyRegistered
			);
			// Check if the node is already registered
			ensure!(
				!NodeRegistration::<T>::contains_key(&node_id),
				Error::<T>::NodeAlreadyRegistered
			);

			// Get the current block number
			let current_block_number = <frame_system::Pallet<T>>::block_number();

			let node_info = NodeInfo {
				node_id: node_id.clone(),
				node_type,
				ipfs_node_id,
				status: Status::Online,
				is_verified: true,
				registered_at: current_block_number,
				owner,
			};
			ColdkeyNodeRegistration::<T>::insert(node_id.clone(), Some(node_info));

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
			ipfs_node_id: Option<Vec<u8>>,
			owner: T::AccountId,
			ipfs_peer_id: Vec<u8>,
			main_key_type: Libp2pKeyType,
			main_public_key: Vec<u8>,
			main_sig: Vec<u8>,
			ipfs_key_type: Libp2pKeyType,
			ipfs_public_key: Vec<u8>,
			ipfs_sig: Vec<u8>,
			challenge_bytes: Vec<u8>,
			ipfs_id_hex: Vec<u8>,
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
			ensure!(
				ch.ipfs_peer_id_hash == Self::blake256(&ipfs_id_hex),
				Error::<T>::ChallengeMismatch
			);

			let ch_hash = Self::blake256(&challenge_bytes);
			ensure!(!UsedChallenges::<T>::contains_key(ch_hash), Error::<T>::ChallengeReused);

			// --- Verify the two libp2p signatures (ed25519 now; extend later if needed) ---
			ensure!(matches!(main_key_type, Libp2pKeyType::Ed25519), Error::<T>::InvalidKeyType);
			ensure!(matches!(ipfs_key_type, Libp2pKeyType::Ed25519), Error::<T>::InvalidKeyType);

			// Verify signatures
			ensure!(
				Self::verify_ed25519(&challenge_bytes, &main_sig, &main_public_key),
				Error::<T>::InvalidSignature
			);
			ensure!(
				Self::verify_ed25519(&challenge_bytes, &ipfs_sig, &ipfs_public_key),
				Error::<T>::InvalidSignature
			);

			// NEW: Verify that public keys match the expected peer IDs
			ensure!(
				Self::verify_peer_id(&main_public_key, &node_id_hex, main_key_type),
				Error::<T>::PublicKeyMismatch
			);

			ensure!(
				Self::verify_peer_id(&ipfs_public_key, &ipfs_id_hex, ipfs_key_type),
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

			// Ensure that if the node type is `StorageMiner`, the `ipfs_node_id` is not `None`
			match node_type {
				NodeType::StorageMiner => {
					ensure!(ipfs_node_id.is_some(), Error::<T>::IpfsNodeIdRequired)
				},
				NodeType::Validator => {
					ensure!(ipfs_node_id.is_some(), Error::<T>::IpfsNodeIdRequired)
				},
				_ => {},
			}

			// Check if the node is already registered
			ensure!(
				!ColdkeyNodeRegistration::<T>::contains_key(&node_id),
				Error::<T>::NodeAlreadyRegistered
			);
			// Check if the node is already registered
			ensure!(
				!NodeRegistration::<T>::contains_key(&node_id),
				Error::<T>::NodeAlreadyRegistered
			);

			// Check if the ipfs_node_id is already registered
			if let Some(ref ipfs_id) = ipfs_node_id {
				// Iterate through all registered nodes to check for the ipfs_node_id
				for (_registered_node_id, registered_node_info) in NodeRegistration::<T>::iter() {
					if let Some(info) = registered_node_info {
						if info.ipfs_node_id.as_ref() == Some(ipfs_id) {
							return Err(Error::<T>::IpfsNodeIdAlreadyRegistered.into());
						}
					}
				}
			}

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
				if Self::fee_charging_enabled() && !is_in_uids {
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
						CreditsPallet::<T>::decrease_user_credits(&owner.clone(), fee_u128);
						// Deposit charge to marketplace account
						let _ = pallet_balances::Pallet::<T>::deposit_creating(
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

			let node_info = NodeInfo {
				node_id: node_id.clone(),
				node_type,
				ipfs_node_id,
				status: Status::Online,
				is_verified: true,
				registered_at: current_block_number,
				owner,
			};
			ColdkeyNodeRegistration::<T>::insert(node_id.clone(), Some(node_info));

			// Persist identities (for later audits/liveness checks)
			Libp2pMainIdentity::<T>::insert(node_id.clone(), (main_key_type, main_public_key));
			Libp2pIpfsIdentity::<T>::insert(node_id.clone(), (ipfs_key_type, ipfs_public_key));

			Self::deposit_event(Event::MainNodeRegistered { node_id });
			Ok(().into())
		}

		#[pallet::call_index(2)]
		#[pallet::weight((0, Pays::No))]
		pub fn register_node_with_hotkey(
			origin: OriginFor<T>,
			coldkey: T::AccountId,
			node_type: NodeType,
			node_id: Vec<u8>,
			pay_in_credits: bool,
			ipfs_node_id: Option<Vec<u8>>,
			ipfs_peer_id: Vec<u8>,
			owner: T::AccountId,
			main_key_type: Libp2pKeyType,
			main_public_key: Vec<u8>,
			main_sig: Vec<u8>,
			ipfs_key_type: Libp2pKeyType,
			ipfs_public_key: Vec<u8>,
			ipfs_sig: Vec<u8>,
			challenge_bytes: Vec<u8>,
			ipfs_id_hex: Vec<u8>,
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
			ensure!(
				ch.ipfs_peer_id_hash == Self::blake256(&ipfs_id_hex),
				Error::<T>::ChallengeMismatch
			);

			let ch_hash = Self::blake256(&challenge_bytes);
			ensure!(!UsedChallenges::<T>::contains_key(ch_hash), Error::<T>::ChallengeReused);

			// --- Verify the two libp2p signatures (ed25519 now; extend later if needed) ---
			ensure!(matches!(main_key_type, Libp2pKeyType::Ed25519), Error::<T>::InvalidKeyType);
			ensure!(matches!(ipfs_key_type, Libp2pKeyType::Ed25519), Error::<T>::InvalidKeyType);

			// Verify signatures
			ensure!(
				Self::verify_ed25519(&challenge_bytes, &main_sig, &main_public_key),
				Error::<T>::InvalidSignature
			);
			ensure!(
				Self::verify_ed25519(&challenge_bytes, &ipfs_sig, &ipfs_public_key),
				Error::<T>::InvalidSignature
			);

			// NEW: Verify that public keys match the expected peer IDs
			ensure!(
				Self::verify_peer_id(&main_public_key, &node_id_hex, main_key_type),
				Error::<T>::PublicKeyMismatch
			);

			ensure!(
				Self::verify_peer_id(&ipfs_public_key, &ipfs_id_hex, ipfs_key_type),
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

			// Check if the caller is a proxy and retrieve the real (main) account
			let proxy_def = ProxyPallet::<T>::find_proxy(
				&coldkey, &owner, None, // Accept any proxy type for now
			)
			.map_err(|_| Error::<T>::NotAProxyAccount)?;

			let proxy_type = proxy_def.proxy_type;

			// Validate the proxy type using ProxyTypeCompat
			ensure!(proxy_type.is_non_transfer(), Error::<T>::InvalidProxyType);

			// Check if the coldkey is registered in ColdkeyNodeRegistration
			let is_registered =
				ColdkeyNodeRegistration::<T>::iter().any(|(_node_id, node_info)| {
					if let Some(info) = node_info {
						info.owner == coldkey
					} else {
						false
					}
				});

			ensure!(is_registered, Error::<T>::AccountNotRegistered);

			// Ensure that if the node type is `StorageMiner`, the `ipfs_node_id` is not `None`
			match node_type {
				NodeType::StorageMiner => {
					ensure!(ipfs_node_id.is_some(), Error::<T>::IpfsNodeIdRequired)
				},
				NodeType::Validator => {
					ensure!(ipfs_node_id.is_some(), Error::<T>::IpfsNodeIdRequired)
				},
				_ => {},
			}

			// Check if the node is already registered
			ensure!(
				!ColdkeyNodeRegistration::<T>::contains_key(&node_id),
				Error::<T>::NodeAlreadyRegistered
			);
			// Check if the node is already registered
			ensure!(
				!NodeRegistration::<T>::contains_key(&node_id),
				Error::<T>::NodeAlreadyRegistered
			);

			// Check if the account ID already has a registered node
			let existing_node = NodeRegistration::<T>::iter().find(
				|(_registered_node_id, registered_node_info)| {
					if let Some(info) = registered_node_info {
						info.owner == owner
					} else {
						false
					}
				},
			);

			ensure!(existing_node.is_none(), Error::<T>::NodeAlreadyRegistered); // You can define a more specific error if needed

			// Check if the ipfs_node_id is already registered
			if let Some(ref ipfs_id) = ipfs_node_id {
				// Iterate through all registered nodes to check for the ipfs_node_id
				for (_registered_node_id, registered_node_info) in NodeRegistration::<T>::iter() {
					if let Some(info) = registered_node_info {
						if info.ipfs_node_id.as_ref() == Some(ipfs_id) {
							return Err(Error::<T>::IpfsNodeIdAlreadyRegistered.into());
						}
					}
				}
			}

			// Check if the node type is disabled
			ensure!(!Self::is_node_type_disabled(node_type.clone()), Error::<T>::NodeTypeDisabled);

			// If the node type is Validator, ensure minimum stake
			if node_type == NodeType::Validator {
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
				let who_ss58 = AccountId32::new(account.encode().try_into().unwrap_or_default())
					.to_ss58check();

				// Check if the caller is in UIDs
				let is_in_uids =
					uids.iter().any(|uid| uid.substrate_address.to_ss58check() == who_ss58);

				// If the caller is in UIDs, check if the node_type matches the role
				if is_in_uids {
					let whitelist = T::MetagraphInfo::get_whitelisted_validators();
					let is_whitelisted = whitelist.iter().any(|validator| validator == &owner);

					if !is_whitelisted {
						if let Some(uid) =
							uids.iter().find(|uid| uid.substrate_address.to_ss58check() == who_ss58)
						{
							ensure!(uid.role == node_type.to_role(), Error::<T>::NodeTypeMismatch);
						}
					}
				}

				// Check if fee charging is enabled
				if Self::fee_charging_enabled() && !is_in_uids {
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
						CreditsPallet::<T>::decrease_user_credits(&owner.clone(), fee_u128);
						// Deposit charge to marketplace account
						let _ = pallet_balances::Pallet::<T>::deposit_creating(
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

			let node_info = NodeInfo {
				node_id: node_id.clone(),
				node_type,
				ipfs_node_id,
				status: Status::Online,
				is_verified: true,
				registered_at: current_block_number,
				owner,
			};

			NodeRegistration::<T>::insert(node_id.clone(), Some(node_info));

			// Iterate through ColdkeyNodeRegistration to find the node_info for the given coldkey
			let coldkeynode_info = ColdkeyNodeRegistration::<T>::iter()
				.find_map(|(_, node_info_opt)| {
					node_info_opt.as_ref().and_then(|node_info| {
						if node_info.owner == coldkey {
							Some(node_info.clone())
						} else {
							None
						}
					})
				})
				.ok_or(Error::<T>::NodeNotFound)?;

			// Update the linked nodes count
			LinkedNodes::<T>::try_mutate(&coldkeynode_info.node_id.clone(), |linked_node_ids| {
				if !linked_node_ids.contains(&node_id) {
					linked_node_ids.push(node_id.clone());
				}
				Ok::<(), DispatchError>(())
			})?;

			// Persist identities (for later audits/liveness checks)
			Libp2pMainIdentity::<T>::insert(node_id.clone(), (main_key_type, main_public_key));
			Libp2pIpfsIdentity::<T>::insert(node_id.clone(), (ipfs_key_type, ipfs_public_key));

			Self::deposit_event(Event::NodeRegistered { node_id });
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
			NodeRegistration::<T>::try_mutate(&node_id, |node_info_opt| -> DispatchResult {
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

		#[pallet::call_index(5)]
		#[pallet::weight((0, Pays::No))]
		pub fn force_register_node_with_hotkey(
			origin: OriginFor<T>,
			owner: T::AccountId,
			coldkey: T::AccountId,
			node_type: NodeType,
			node_id: Vec<u8>,
			ipfs_node_id: Option<Vec<u8>>,
		) -> DispatchResultWithPostInfo {
			ensure_root(origin)?;

			// Check if the account is banned
			ensure!(!Self::is_account_banned(&owner), Error::<T>::AccountBanned);

			// Check if the owner already has a registered node
			ensure!(!Self::is_owner_node_registered(&owner), Error::<T>::OwnerAlreadyRegistered);

			let node_id = node_id.clone();
			// Check if the node is already registered
			ensure!(
				!ColdkeyNodeRegistration::<T>::contains_key(&node_id),
				Error::<T>::NodeAlreadyRegistered
			);
			// Check if the node is already registered
			ensure!(
				!NodeRegistration::<T>::contains_key(&node_id),
				Error::<T>::NodeAlreadyRegistered
			);

			// Get the current block number
			let current_block_number = <frame_system::Pallet<T>>::block_number();

			let node_info = NodeInfo {
				node_id: node_id.clone(),
				node_type,
				ipfs_node_id,
				status: Status::Online,
				is_verified: true,
				registered_at: current_block_number,
				owner,
			};
			NodeRegistration::<T>::insert(node_id.clone(), Some(node_info));

			// Iterate through ColdkeyNodeRegistration to find the node_info for the given coldkey
			let node_info = ColdkeyNodeRegistration::<T>::iter()
				.find_map(|(_, node_info_opt)| {
					node_info_opt.as_ref().and_then(|node_info| {
						if node_info.owner == coldkey {
							Some(node_info.clone())
						} else {
							None
						}
					})
				})
				.ok_or(Error::<T>::NodeNotFound)?;

			// Update the linked nodes count
			LinkedNodes::<T>::try_mutate(&node_info.node_id.clone(), |linked_node_ids| {
				if !linked_node_ids.contains(&node_id) {
					linked_node_ids.push(node_id.clone());
				}
				Ok::<(), DispatchError>(()) // Specify the type here
			})?;

			Self::deposit_event(Event::NodeRegistered { node_id });
			Ok(().into())
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
		pub fn force_unregister_hotkey_node(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
		) -> DispatchResultWithPostInfo {
			ensure_root(origin)?;

			Self::do_set_node_status_degraded_or_unregister(node_id);

			Ok(().into())
		}

		#[pallet::call_index(10)]
		#[pallet::weight((0, Pays::No))]
		pub fn force_unregister_coldkey_node(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
		) -> DispatchResultWithPostInfo {
			ensure_root(origin)?;

			Self::do_set_node_status_degraded_or_unregister(node_id);

			Ok(().into())
		}

		#[pallet::call_index(11)]
		#[pallet::weight((0, Pays::No))]
		pub fn unregister_node(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Ensure that the node is registered
			ensure!(NodeRegistration::<T>::contains_key(&node_id), Error::<T>::NodeNotRegistered);

			// Retrieve the node info to check ownership
			let node_info =
				NodeRegistration::<T>::get(&node_id).ok_or(Error::<T>::NodeNotRegistered)?;

			// Ensure the caller is the owner of the node
			ensure!(node_info.owner == who, Error::<T>::NotNodeOwner);

			// Call the existing unregister logic
			Self::do_set_node_status_degraded_or_unregister(node_id);

			Ok(().into())
		}

		#[pallet::call_index(12)]
		#[pallet::weight((0, Pays::No))]
		pub fn unregister_main_node(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Ensure that the node is registered
			ensure!(
				ColdkeyNodeRegistration::<T>::contains_key(&node_id),
				Error::<T>::NodeNotRegistered
			);

			// Retrieve the node info to check ownership
			let node_info =
				ColdkeyNodeRegistration::<T>::get(&node_id).ok_or(Error::<T>::NodeNotRegistered)?;

			// Ensure the caller is the owner of the node
			ensure!(node_info.owner == who, Error::<T>::NotNodeOwner);

			// Call the existing unregister logic
			Self::do_set_node_status_degraded_or_unregister(node_id);

			Ok(().into())
		}

		#[pallet::call_index(13)]
		#[pallet::weight((0, Pays::No))]
		pub fn swap_node_owner(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			new_owner: T::AccountId,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Retrieve the node information
			let mut node_info =
				NodeRegistration::<T>::get(&node_id).ok_or(Error::<T>::NodeNotRegistered)?;

			// Ensure the caller is the current owner
			ensure!(node_info.owner == who.clone(), Error::<T>::NotNodeOwner);

			// Update the owner
			node_info.owner = new_owner.clone();

			// Save the updated node information back to storage
			NodeRegistration::<T>::insert(node_id.clone(), Some(node_info)); // Wrap node_info in Some

			Self::deposit_event(Event::NodeOwnerSwapped { node_id, new_owner });

			Ok(().into())
		}

		/// Sudo call to unregister all nodes with is_verified = false
		/// This will iterate through all registered nodes and unregister those that are not verified
		#[pallet::call_index(17)]
		#[pallet::weight((T::DbWeight::get().reads_writes(2, 1), Pays::No))]
		pub fn sudo_unregister_unverified_nodes(
			origin: OriginFor<T>,
		) -> DispatchResultWithPostInfo {
			// Ensure the caller is root (sudo)
			ensure_root(origin)?;

			let mut unregistered_count = 0u32;

			// Unregister unverified coldkey nodes
			let coldkey_nodes: Vec<_> = ColdkeyNodeRegistration::<T>::iter()
				.filter_map(|(node_id, node_info_opt)| {
					node_info_opt.filter(|info| !info.is_verified).map(|_| node_id)
				})
				.collect();

			for node_id in coldkey_nodes {
				// Use the existing unregister logic
				Self::do_unregister_main_node(node_id.clone());
				unregistered_count += 1;
			}

			// Unregister unverified hotkey nodes
			let hotkey_nodes: Vec<_> = NodeRegistration::<T>::iter()
				.filter_map(|(node_id, node_info_opt)| {
					node_info_opt.filter(|info| !info.is_verified).map(|_| node_id)
				})
				.collect();

			for node_id in hotkey_nodes {
				// Use the existing unregister logic
				Self::do_unregister_main_node(node_id);
				unregistered_count += 1;
			}

			// Emit an event with the count of unregistered nodes
			Self::deposit_event(Event::<T>::NodeUnregisteredBatch { count: unregistered_count });

			Ok(().into())
		}

		#[pallet::call_index(14)]
		#[pallet::weight((0, Pays::No))]
		pub fn submit_deregistration_report(
			origin: OriginFor<T>,
			node_ids: Vec<Vec<u8>>,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Check if this is a proxy account and get the main account
			let main_account = if let Some(primary) = Self::get_primary_account(&who)? {
				primary
			} else {
				who.clone() // If not a proxy, use the account itself
			};
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
			ReportSubmissionCount::<T>::insert(node_info.node_id.clone(), user_requests_count + 1);

			// Store deregistration reports with current block number
			let current_block = <frame_system::Pallet<T>>::block_number();
			let mut existing_reports = TemporaryDeregistrationReports::<T>::get(&who);
			for node_id in node_ids {
				let report =
					DeregistrationReport { node_id: node_id.clone(), created_at: current_block };
				log::info!(
					"Submitting deregistration report for node id {}",
					String::from_utf8_lossy(&node_id)
				);
				existing_reports.push(report);
			}
			TemporaryDeregistrationReports::<T>::insert(&who, existing_reports);

			Ok(().into())
		}

		/// Ban or unban an account from registering nodes
		#[pallet::call_index(15)]
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
		#[pallet::call_index(16)]
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

		#[pallet::call_index(18)]
		#[pallet::weight((10_000, Pays::No))]
		pub fn verify_existing_node(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			challenge_bytes: Vec<u8>,
			main_key_type: Libp2pKeyType,
			main_public_key: Vec<u8>,
			main_sig: Vec<u8>,
			ipfs_key_type: Libp2pKeyType,
			ipfs_public_key: Vec<u8>,
			ipfs_sig: Vec<u8>,
			ipfs_id_hex: Vec<u8>,
			node_id_hex: Vec<u8>,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Get the node information
			let mut node_info = NodeRegistration::<T>::get(&node_id)
				.and_then(|opt| Some(opt))
				.ok_or(Error::<T>::NodeNotFound)?;

			// Check if node is already verified
			ensure!(!node_info.is_verified, Error::<T>::NodeAlreadyVerified);

			// Verify that the caller owns this node
			ensure!(node_info.owner == who, Error::<T>::Unauthorized);

			// --- Decode & check the challenge ---
			let mut rdr = &challenge_bytes[..];
			let ch: RegisterChallenge<T::AccountId, BlockNumberFor<T>> =
				RegisterChallenge::decode(&mut rdr).map_err(|_| Error::<T>::InvalidChallenge)?;

			const EXPECTED_DOMAIN: [u8; 24] = *b"HIPPIUS::REGISTER::v1\0\0\0";
			ensure!(ch.domain == EXPECTED_DOMAIN, Error::<T>::InvalidChallengeDomain);
			ensure!(ch.account == who, Error::<T>::InvalidAccountId);
			ensure!(
				ch.expires_at >= <frame_system::Pallet<T>>::block_number(),
				Error::<T>::ChallengeExpired
			);
			ensure!(ch.genesis_hash == Self::genesis_hash_bytes(), Error::<T>::GenesisMismatch);
			ensure!(ch.node_id_hash == Self::blake256(&node_id_hex), Error::<T>::ChallengeMismatch);
			ensure!(
				ch.ipfs_peer_id_hash == Self::blake256(&ipfs_id_hex),
				Error::<T>::ChallengeMismatch
			);

			let ch_hash = Self::blake256(&challenge_bytes);
			ensure!(!UsedChallenges::<T>::contains_key(ch_hash), Error::<T>::ChallengeReused);

			// --- Verify the two libp2p signatures ---
			ensure!(matches!(main_key_type, Libp2pKeyType::Ed25519), Error::<T>::InvalidKeyType);
			ensure!(matches!(ipfs_key_type, Libp2pKeyType::Ed25519), Error::<T>::InvalidKeyType);

			// Verify signatures
			ensure!(
				Self::verify_ed25519(&challenge_bytes, &main_sig, &main_public_key),
				Error::<T>::InvalidSignature
			);
			ensure!(
				Self::verify_ed25519(&challenge_bytes, &ipfs_sig, &ipfs_public_key),
				Error::<T>::InvalidSignature
			);

			// Verify that public keys match the expected peer IDs
			ensure!(
				Self::verify_peer_id(&main_public_key, &node_id_hex, main_key_type),
				Error::<T>::PublicKeyMismatch
			);

			ensure!(
				Self::verify_peer_id(&ipfs_public_key, &ipfs_id_hex, ipfs_key_type),
				Error::<T>::PublicKeyMismatch
			);

			// Mark challenge used (replay protection)
			UsedChallenges::<T>::insert(ch_hash, ch.expires_at);

			// Update the node to verified
			node_info.is_verified = true;
			NodeRegistration::<T>::insert(&node_id, Some(node_info.clone()));

			// Store the identities for future use (liveness checks, etc.)
			Libp2pMainIdentity::<T>::insert(&node_id, (main_key_type, main_public_key.clone()));
			Libp2pIpfsIdentity::<T>::insert(&node_id, (ipfs_key_type, ipfs_public_key.clone()));

			Self::deposit_event(Event::NodeVerified {
				node_id: node_id.clone(),
				owner: who.clone(),
			});

			Ok(().into())
		}

		#[pallet::call_index(19)]
		#[pallet::weight((10_000, Pays::No))]
		pub fn verify_existing_coldkey_node(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
			challenge_bytes: Vec<u8>,
			main_key_type: Libp2pKeyType,
			main_public_key: Vec<u8>,
			main_sig: Vec<u8>,
			ipfs_key_type: Libp2pKeyType,
			ipfs_public_key: Vec<u8>,
			ipfs_sig: Vec<u8>,
			ipfs_id_hex: Vec<u8>,
			node_id_hex: Vec<u8>,
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Get the coldkey node information
			let mut node_info = ColdkeyNodeRegistration::<T>::get(&node_id)
				.and_then(|opt| Some(opt))
				.ok_or(Error::<T>::NodeNotFound)?;

			// Check if node is already verified
			ensure!(!node_info.is_verified, Error::<T>::NodeAlreadyVerified);

			// Verify that the caller owns this node
			ensure!(node_info.owner == who, Error::<T>::Unauthorized);

			// --- Decode & check the challenge ---
			let mut rdr = &challenge_bytes[..];
			let ch: RegisterChallenge<T::AccountId, BlockNumberFor<T>> =
				RegisterChallenge::decode(&mut rdr).map_err(|_| Error::<T>::InvalidChallenge)?;

			const EXPECTED_DOMAIN: [u8; 24] = *b"HIPPIUS::REGISTER::v1\0\0\0";
			ensure!(ch.domain == EXPECTED_DOMAIN, Error::<T>::InvalidChallengeDomain);
			ensure!(ch.account == who, Error::<T>::InvalidAccountId);
			ensure!(
				ch.expires_at >= <frame_system::Pallet<T>>::block_number(),
				Error::<T>::ChallengeExpired
			);
			ensure!(ch.genesis_hash == Self::genesis_hash_bytes(), Error::<T>::GenesisMismatch);
			ensure!(ch.node_id_hash == Self::blake256(&node_id_hex), Error::<T>::ChallengeMismatch);
			ensure!(
				ch.ipfs_peer_id_hash == Self::blake256(&ipfs_id_hex),
				Error::<T>::ChallengeMismatch
			);

			let ch_hash = Self::blake256(&challenge_bytes);
			ensure!(!UsedChallenges::<T>::contains_key(ch_hash), Error::<T>::ChallengeReused);

			// --- Verify the two libp2p signatures ---
			ensure!(matches!(main_key_type, Libp2pKeyType::Ed25519), Error::<T>::InvalidKeyType);
			ensure!(matches!(ipfs_key_type, Libp2pKeyType::Ed25519), Error::<T>::InvalidKeyType);

			// Verify signatures
			ensure!(
				Self::verify_ed25519(&challenge_bytes, &main_sig, &main_public_key),
				Error::<T>::InvalidSignature
			);
			ensure!(
				Self::verify_ed25519(&challenge_bytes, &ipfs_sig, &ipfs_public_key),
				Error::<T>::InvalidSignature
			);

			// Verify that public keys match the expected peer IDs
			ensure!(
				Self::verify_peer_id(&main_public_key, &node_id_hex, main_key_type),
				Error::<T>::PublicKeyMismatch
			);

			ensure!(
				Self::verify_peer_id(&ipfs_public_key, &ipfs_id_hex, ipfs_key_type),
				Error::<T>::PublicKeyMismatch
			);

			// Mark challenge used (replay protection)
			UsedChallenges::<T>::insert(ch_hash, ch.expires_at);

			// Update the node to verified
			node_info.is_verified = true;
			ColdkeyNodeRegistration::<T>::insert(&node_id, Some(node_info.clone()));

			// Store the identities for future use
			Libp2pMainIdentity::<T>::insert(&node_id, (main_key_type, main_public_key.clone()));
			Libp2pIpfsIdentity::<T>::insert(&node_id, (ipfs_key_type, ipfs_public_key.clone()));

			Self::deposit_event(Event::ColdkeyNodeVerified {
				node_id: node_id.clone(),
				owner: who.clone(),
			});

			Ok(().into())
		}
	}

	impl<T: Config> Pallet<T> {
		pub fn get_all_degraded_storage_miners() -> Vec<Vec<u8>> {
			let mut degraded_nodes = Vec::new();

			// Iterate over NodeRegistration storage
			for (_, maybe_node_info) in <NodeRegistration<T>>::iter() {
				if let Some(node_info) = maybe_node_info {
					if node_info.node_type == NodeType::StorageMiner
						&& matches!(node_info.status, Status::Degraded)
					{
						degraded_nodes.push(node_info.node_id.clone());
					}
				}
			}

			// Iterate over ColdkeyNodeRegistration storage
			for (_, maybe_node_info) in <ColdkeyNodeRegistration<T>>::iter() {
				if let Some(node_info) = maybe_node_info {
					if node_info.node_type == NodeType::StorageMiner
						&& matches!(node_info.status, Status::Degraded)
					{
						degraded_nodes.push(node_info.node_id.clone());
					}
				}
			}

			degraded_nodes
		}

		pub fn get_registration_block(node_id: &Vec<u8>) -> Option<BlockNumberFor<T>> {
			// Check NodeRegistration storage
			if let Some(node_info) = NodeRegistration::<T>::get(node_id) {
				return Some(node_info.registered_at);
			}
			// Check ColdkeyNodeRegistration storage
			if let Some(node_info) = ColdkeyNodeRegistration::<T>::get(node_id) {
				return Some(node_info.registered_at);
			}
			None
		}

		pub fn try_unregister_storage_miner(node_to_deregister: Vec<u8>) -> Result<(), Error<T>> {
			// Check if the node to deregister is a degraded StorageMiner
			ensure!(
				Self::is_storage_miner_degraded(node_to_deregister.clone()),
				Error::<T>::NodeNotDegradedStorageMiner
			);

			// Perform deregistration
			Self::do_unregister_node(node_to_deregister);
			Ok(())
		}

		/// Helper function to check if an owner already has a registered node
		fn is_owner_node_registered(owner: &T::AccountId) -> bool {
			// Check both ColdkeyNodeRegistration and NodeRegistration storage maps
			ColdkeyNodeRegistration::<T>::iter()
				.any(|(_, node_info)| node_info.map_or(false, |info| info.owner == *owner))
				|| NodeRegistration::<T>::iter()
					.any(|(_, node_info)| node_info.map_or(false, |info| info.owner == *owner))
		}

		/// Fetch all registered miners whose status is not degraded
		pub fn get_all_active_storage_miners() -> Vec<NodeInfo<BlockNumberFor<T>, T::AccountId>> {
			// Vector to store filtered node info
			let mut active_storage_miners = Vec::new();
			let mut seen_node_ids = Vec::new(); // Vec to track unique node IDs

			// Iterate over all registered nodes in the NodeRegistration storage map
			for (_, node_info_opt) in NodeRegistration::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Check if the node is of type miner and its status is not Degraded
					if matches!(node_info.node_type, NodeType::StorageMiner)
						&& !matches!(node_info.status, Status::Degraded)
					{
						// Check for uniqueness
						if !seen_node_ids.contains(&node_info.node_id) {
							seen_node_ids.push(node_info.node_id.clone());
							active_storage_miners.push(node_info);
						}
					}
				}
			}

			// Iterate over all registered nodes in the ColdkeyNodeRegistration storage map
			for (_, node_info_opt) in ColdkeyNodeRegistration::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Check if the node is of type miner and its status is not Degraded
					if matches!(node_info.node_type, NodeType::StorageMiner)
						&& !matches!(node_info.status, Status::Degraded)
					{
						// Check for uniqueness
						if !seen_node_ids.contains(&node_info.node_id) {
							seen_node_ids.push(node_info.node_id.clone());
							active_storage_miners.push(node_info);
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

			// Iterate over all registered nodes in the NodeRegistration storage map
			for (_, node_info_opt) in NodeRegistration::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Check if the node is of type miner and its status is not Degraded
					if matches!(node_info.node_type, NodeType::ComputeMiner)
						&& !matches!(node_info.status, Status::Degraded)
					{
						// Check for uniqueness
						if !seen_node_ids.contains(&node_info.node_id) {
							seen_node_ids.push(node_info.node_id.clone());
							active_storage_miners.push(node_info);
						}
					}
				}
			}

			// Iterate over all registered nodes in the ColdkeyNodeRegistration storage map
			for (_, node_info_opt) in ColdkeyNodeRegistration::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Check if the node is of type miner and its status is not Degraded
					if matches!(node_info.node_type, NodeType::ComputeMiner)
						&& !matches!(node_info.status, Status::Degraded)
					{
						// Check for uniqueness
						if !seen_node_ids.contains(&node_info.node_id) {
							seen_node_ids.push(node_info.node_id.clone());
							active_storage_miners.push(node_info);
						}
					}
				}
			}

			active_storage_miners
		}

		pub fn get_miner_info(account_id: T::AccountId) -> Option<(NodeType, Status)> {
			let mut seen_accounts = Vec::new(); // Vec to track unique account IDs

			// Iterate through the storage map to find the miner associated with the account_id in NodeRegistration
			for (_, node_info) in NodeRegistration::<T>::iter() {
				if let Some(info) = node_info {
					// Check if the owner of the node matches the provided account_id
					if info.owner == account_id && !seen_accounts.contains(&info.owner) {
						seen_accounts.push(info.owner.clone()); // Track this account ID
											  // Return the miner type and status
						return Some((info.node_type, info.status));
					}
				}
			}

			// Iterate through the storage map to find the miner associated with the account_id in ColdkeyNodeRegistration
			for (_, node_info) in ColdkeyNodeRegistration::<T>::iter() {
				if let Some(info) = node_info {
					// Check if the owner of the node matches the provided account_id
					if info.owner == account_id && !seen_accounts.contains(&info.owner) {
						seen_accounts.push(info.owner.clone()); // Track this account ID
											  // Return the miner type and status
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

			// Iterate over all registered nodes in the NodeRegistration storage map
			for (_, node_info_opt) in NodeRegistration::<T>::iter() {
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
							if !seen_node_ids.contains(&node_info.node_id) {
								seen_node_ids.push(node_info.node_id.clone());
								active_storage_miners.push(node_info);
							}
						}
					}
				}
			}

			// Iterate over all registered nodes in the ColdkeyNodeRegistration storage map
			for (_, node_info_opt) in ColdkeyNodeRegistration::<T>::iter() {
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
							if !seen_node_ids.contains(&node_info.node_id) {
								seen_node_ids.push(node_info.node_id.clone());
								active_storage_miners.push(node_info);
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

			// Iterate over all registered nodes in the ColdkeyNodeRegistration storage map
			for (_, node_info_opt) in ColdkeyNodeRegistration::<T>::iter() {
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
						if !seen_node_ids.contains(&node_info.node_id) {
							seen_node_ids.push(node_info.node_id.clone());
							active_storage_miners.push(node_info);
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

		pub fn get_chain_decimals() -> u32 {
			T::ChainDecimals::get()
		}

		/// Fetch all registered miners whose status is not degraded
		pub fn get_all_nodes_by_node_type(
			node_type: NodeType,
		) -> Vec<NodeInfo<BlockNumberFor<T>, T::AccountId>> {
			// Vector to store filtered node info
			let mut active_nodes = Vec::new();
			let mut seen_node_ids = Vec::new(); // Vec to track unique node IDs

			// Iterate over all registered nodes in the NodeRegistration storage map
			for (_, node_info_opt) in NodeRegistration::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Check if the node is of the specified type and its status is not Degraded
					if node_info.node_type == node_type
						&& !matches!(node_info.status, Status::Degraded)
					{
						// Check for uniqueness
						if !seen_node_ids.contains(&node_info.node_id) {
							seen_node_ids.push(node_info.node_id.clone());
							active_nodes.push(node_info);
						}
					}
				}
			}

			// Iterate over all registered nodes in the ColdkeyNodeRegistration storage map
			for (_, node_info_opt) in ColdkeyNodeRegistration::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Check if the node is of the specified type and its status is not Degraded
					if node_info.node_type == node_type
						&& !matches!(node_info.status, Status::Degraded)
					{
						// Check for uniqueness
						if !seen_node_ids.contains(&node_info.node_id) {
							seen_node_ids.push(node_info.node_id.clone());
							active_nodes.push(node_info);
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

			// Iterate over all registered nodes in the ColdkeyNodeRegistration storage map
			for (_, node_info_opt) in ColdkeyNodeRegistration::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Check if the node's status is not Degraded
					if !matches!(node_info.status, Status::Degraded)
						&& matches!(node_info.node_type, NodeType::StorageMiner)
					{
						// Check for uniqueness
						if !seen_node_ids.contains(&node_info.node_id) {
							seen_node_ids.push(node_info.node_id.clone());
							active_nodes.push(node_info);
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

			// Iterate over all registered nodes in the NodeRegistration storage map
			for (_, node_info_opt) in NodeRegistration::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Check if the node's status is not Degraded
					if !matches!(node_info.status, Status::Degraded) {
						// Check for uniqueness
						if !seen_node_ids.contains(&node_info.node_id) {
							seen_node_ids.push(node_info.node_id.clone());
							active_nodes.push(node_info);
						}
					}
				}
			}

			// Iterate over all registered nodes in the ColdkeyNodeRegistration storage map
			for (_, node_info_opt) in ColdkeyNodeRegistration::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Check if the node's status is not Degraded
					if !matches!(node_info.status, Status::Degraded) {
						// Check for uniqueness
						if !seen_node_ids.contains(&node_info.node_id) {
							seen_node_ids.push(node_info.node_id.clone());
							active_nodes.push(node_info);
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
			if let Some(coldkey_info) = ColdkeyNodeRegistration::<T>::get(&node_id) {
				if coldkey_info.status != Status::Degraded {
					return Some(coldkey_info);
				}
			} else if let Some(node_info) = NodeRegistration::<T>::get(&node_id) {
				if node_info.status != Status::Degraded {
					return Some(node_info);
				}
			}
			None
		}

		pub fn is_storage_miner_degraded(node_id: Vec<u8>) -> bool {
			if let Some(coldkey_info) = ColdkeyNodeRegistration::<T>::get(&node_id) {
				return coldkey_info.node_type == NodeType::StorageMiner
					&& coldkey_info.status == Status::Degraded;
			}
			if let Some(node_info) = NodeRegistration::<T>::get(&node_id) {
				return node_info.node_type == NodeType::StorageMiner
					&& node_info.status == Status::Degraded;
			}
			false
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

				let total_reports = reports.len() as u32;
				if total_reports >= threshold {
					// Count unique validators
					let unique_validators: sp_std::collections::btree_set::BTreeSet<T::AccountId> =
						reports.iter().map(|(validator_id, _)| validator_id.clone()).collect();

					let agreeing_validators = unique_validators.len() as u32;

					log::info!(
						"Node: {:?}, Total Reports: {}, Agreeing Validators: {}, Threshold: {}",
						node_id,
						total_reports,
						agreeing_validators,
						threshold
					);

					if agreeing_validators >= threshold {
						// Consensus reached, unregister the node
						Self::do_unregister_main_node(node_id.clone());

						// Use proper event emission
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

			// Iterate over all registered nodes in the NodeRegistration storage map
			for (_, node_info_opt) in NodeRegistration::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Check if the node is of the specified type and its status is not Degraded
					if node_info.node_type == node_type
						&& !matches!(node_info.status, Status::Degraded)
					{
						// Check for uniqueness
						if !seen_node_ids.contains(&node_info.node_id) {
							seen_node_ids.push(node_info.node_id.clone());
							active_nodes.push(node_info.node_id.clone());
						}
					}
				}
			}

			// Iterate over all registered nodes in the ColdkeyNodeRegistration storage map
			for (_, node_info_opt) in ColdkeyNodeRegistration::<T>::iter() {
				if let Some(node_info) = node_info_opt {
					// Check if the node is of the specified type and its status is not Degraded
					if node_info.node_type == node_type
						&& !matches!(node_info.status, Status::Degraded)
					{
						// Check for uniqueness
						if !seen_node_ids.contains(&node_info.node_id) {
							seen_node_ids.push(node_info.node_id.clone());
							active_nodes.push(node_info.node_id.clone());
						}
					}
				}
			}

			active_nodes
		}

		pub fn do_unregister_node(node_id: Vec<u8>) {
			if !NodeRegistration::<T>::contains_key(&node_id) {
				if ColdkeyNodeRegistration::<T>::contains_key(&node_id) {
					Self::do_unregister_main_node(node_id.clone());
				}
				return;
			}

			if let Some(_node_info) = NodeRegistration::<T>::get(&node_id) {
				// Find the main node by searching LinkedNodes
				let main_node_id =
					LinkedNodes::<T>::iter()
						.find_map(|(main_node, linked_nodes)| {
							if linked_nodes.contains(&node_id) {
								Some(main_node)
							} else {
								None
							}
						})
						.unwrap_or(node_id.clone());

				// Remove the node registration
				NodeRegistration::<T>::remove(&node_id);

				// Remove metrics for the main node
				T::MetricsInfo::remove_metrics(main_node_id.clone());
				T::IpfsInfo::remove_miner_profile_info(node_id.clone());

				// Remove the node from LinkedNodes if it exists
				let main_node_linked_nodes = LinkedNodes::<T>::get(&main_node_id);
				let updated_linked_nodes: Vec<Vec<u8>> =
					main_node_linked_nodes.into_iter().filter(|n| n != &node_id).collect();

				if !updated_linked_nodes.is_empty() {
					LinkedNodes::<T>::insert(&main_node_id, updated_linked_nodes);
				} else {
					LinkedNodes::<T>::remove(&main_node_id);
				}

				Self::deposit_event(Event::NodeUnregistered { node_id });
			}
		}

		pub fn do_unregister_main_node(node_id: Vec<u8>) {
			// Remove all node registrations linked to this coldkey node
			let linked_node_ids = Self::linked_nodes(&node_id);
			for linked_node_vec in linked_node_ids {
				NodeRegistration::<T>::remove(linked_node_vec.clone());
			}

			// Remove the main node's registration and linked nodes
			ColdkeyNodeRegistration::<T>::remove(node_id.clone());
			LinkedNodes::<T>::remove(node_id.clone());
			T::MetricsInfo::remove_metrics(node_id.clone());
			T::IpfsInfo::remove_miner_profile_info(node_id.clone());
			Self::deposit_event(Event::NodeUnregistered { node_id });
		}

		/// Helper function to remove duplicate linked node IDs for all main nodes.
		pub fn remove_duplicate_linked_nodes() {
			LinkedNodes::<T>::iter_keys().for_each(|main_node_id| {
				let linked_nodes = LinkedNodes::<T>::get(&main_node_id);
				let unique_nodes: BTreeSet<Vec<u8>> = linked_nodes.into_iter().collect();
				let unique_nodes_vec: Vec<Vec<u8>> = unique_nodes.into_iter().collect();
				LinkedNodes::<T>::insert(&main_node_id, unique_nodes_vec);
			});
		}

		/// Helper function to get the registered node for a specific owner
		pub fn get_registered_node_for_owner(
			owner: &T::AccountId,
		) -> Option<NodeInfo<BlockNumberFor<T>, T::AccountId>> {
			// First, check ColdkeyNodeRegistration
			let coldkey_node = ColdkeyNodeRegistration::<T>::iter().find_map(|(_, node_info)| {
				node_info.filter(|info| info.owner == *owner && info.status != Status::Degraded)
			});

			if coldkey_node.is_some() {
				return coldkey_node;
			}

			// If not found, check NodeRegistration
			let node_reg_node = NodeRegistration::<T>::iter().find_map(|(_, node_info)| {
				node_info.filter(|info| info.owner == *owner && info.status != Status::Degraded)
			});

			node_reg_node
		}

		pub fn do_set_node_status_degraded_or_unregister(node_id: Vec<u8>) {
			// Check if the node exists in NodeRegistration
			if NodeRegistration::<T>::contains_key(&node_id) {
				if let Some(mut node_info) = NodeRegistration::<T>::get(&node_id) {
					if node_info.node_type == NodeType::StorageMiner {
						// Update status to Degraded for StorageMiner
						node_info.status = Status::Degraded;
						NodeRegistration::<T>::insert(&node_id, Some(node_info.clone()));

						// Emit event for status update
						Self::deposit_event(Event::NodeStatusUpdated {
							node_id,
							status: Status::Degraded,
						});
					} else {
						// Deregister non-StorageMiner nodes
						Self::do_unregister_node(node_id.clone());
					}
				}
			} else if ColdkeyNodeRegistration::<T>::contains_key(&node_id) {
				// Handle main node (ColdkeyNodeRegistration)
				if let Some(mut node_info) = ColdkeyNodeRegistration::<T>::get(&node_id) {
					if node_info.node_type == NodeType::StorageMiner {
						// Update status to Degraded for StorageMiner
						node_info.status = Status::Degraded;
						ColdkeyNodeRegistration::<T>::insert(&node_id, Some(node_info.clone()));

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

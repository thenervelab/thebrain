#![cfg_attr(not(feature = "std"), no_std)]
pub use pallet::*;
pub use types::*;
// pub mod migrations; // Add this line
#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

mod types;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;

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
	use frame_system::{
		offchain::{SendTransactionTypes, SubmitTransaction},
		pallet_prelude::*,
	};
	use pallet_credits::Pallet as CreditsPallet;
	use pallet_proxy::Pallet as ProxyPallet;
	use pallet_utils::{MetagraphInfoProvider,MetricsInfoProvider, Pallet as UtilsPallet};
	use scale_info::prelude::*;
	use sp_core::crypto::Ss58Codec;
	// use sp_core::offchain::Duration;
	// use sp_runtime::offchain::storage_lock::{BlockAndTime, StorageLock};
	use sp_runtime::traits::AccountIdConversion;
	use sp_runtime::traits::CheckedDiv;
	use sp_runtime::traits::Zero;
	use sp_runtime::AccountId32;
	use sp_runtime::Saturating;
	use sp_std::{prelude::*, vec::Vec};
	use sp_std::collections::btree_set::BTreeSet;
	use sp_std::collections::btree_map::BTreeMap;
	use pallet_proxy::Proxies;

	// const LOCK_BLOCK_EXPIRATION: u32 = 1;
	// const LOCK_TIMEOUT_EXPIRATION: u32 = 3000;

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

	#[pallet::event]
	#[pallet::generate_deposit(pub(super) fn deposit_event)]
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
		}, // New event
		NodeOwnerSwapped {
			node_id: Vec<u8>,
			new_owner: T::AccountId,
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
	}

	#[pallet::hooks]
	impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {
		fn on_initialize(_n: BlockNumberFor<T>) -> Weight {
			// Initialize fees if not already set
			if CurrentNodeTypeFee::<T>::iter().count() == 0 {
				Self::initialize_node_type_fees();
			}

			if _n % 100u32.into() == 0u32.into() {	

				Self::remove_duplicate_linked_nodes();

				// Remove LinkedNodes entries for main nodes without corresponding ColdkeyNodeRegistration
				LinkedNodes::<T>::iter_keys()
					.collect::<Vec<_>>()
					.into_iter()
					.for_each(|main_node_id| {
						if ColdkeyNodeRegistration::<T>::get(&main_node_id).is_none() {
							LinkedNodes::<T>::remove(&main_node_id);
						}
					});

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
				let mut owner_registrations: BTreeMap<T::AccountId, Vec<(Vec<u8>, bool)>> = BTreeMap::new();
				for (node_id, owner, is_coldkey) in all_registrations {
					owner_registrations
						.entry(owner)
						.or_default()
						.push((node_id, is_coldkey));
				}
				
				// Remove registrations for owners with multiple registrations or across different storage types
				for (owner, registrations) in owner_registrations {
					// If an owner has more than one registration total
					// Or has registrations in both ColdkeyNodeRegistration and NodeRegistration
					if owner_node_counts.get(&owner).copied().unwrap_or(0) > 1 ||
					   registrations.iter().any(|(_, is_coldkey)| *is_coldkey) && 
					   registrations.iter().any(|(_, is_coldkey)| !*is_coldkey) {
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
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Check if the owner already has a registered node
			ensure!(!Self::is_owner_node_registered(&who), Error::<T>::OwnerAlreadyRegistered);

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
						who.clone()
					))
					.map(|ledger| ledger.active)
					.unwrap_or_default()
						>= T::MinerStakeThreshold::get().into(),
					Error::<T>::InsufficientStake
				);
			}

			// Get all UIDs using the MetagraphInfo provider
			let uids = T::MetagraphInfo::get_all_uids();

			if let Ok(account_bytes) = who.clone().encode().try_into() {
				let account = AccountId32::new(account_bytes);
				let who_ss58 = AccountId32::new(account.encode().try_into().unwrap_or_default())
					.to_ss58check();

				// Check if the caller is in UIDs
				let is_in_uids =
					uids.iter().any(|uid| uid.substrate_address.to_ss58check() == who_ss58);

				// Check if the caller is not in UIDs and return an error
				ensure!(is_in_uids, Error::<T>::NodeNotInUids);

				// If the caller is in UIDs, check if the node_type matches the role
				if is_in_uids {
					let whitelist = T::MetagraphInfo::get_whitelisted_validators();
					let is_whitelisted = whitelist.iter().any(|validator| validator == &who);

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
						<pallet_balances::Pallet<T>>::free_balance(&who) >= fee,
						Error::<T>::InsufficientBalanceForFee
					);

					if !pay_in_credits {
						// Transfer fee to the pallet's account
						<pallet_balances::Pallet<T>>::transfer(
							&who.clone(),
							&Self::account_id(),
							fee,
							ExistenceRequirement::AllowDeath,
						)?;
					} else {
						// decrease credits and mint balance
						let fee_u128: u128 = fee.try_into().unwrap_or_default();
						CreditsPallet::<T>::decrease_user_credits(&who.clone(), fee_u128);
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
				registered_at: current_block_number,
				owner: who,
			};
			ColdkeyNodeRegistration::<T>::insert(node_id.clone(), Some(node_info));

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
		) -> DispatchResultWithPostInfo {
			let who = ensure_signed(origin)?;

			// Check if the owner already has a registered node
			ensure!(!Self::is_owner_node_registered(&who), Error::<T>::OwnerAlreadyRegistered);

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
				&coldkey, &who, None, // Accept any proxy type for now
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
						info.owner == who
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
				// You'll need to implement a way to check the staked amount
				// This is a placeholder - replace with actual stake checking logic
				ensure!(
					pallet_staking::Pallet::<T>::ledger(sp_staking::StakingAccount::Stash(
						who.clone()
					))
					.map(|ledger| ledger.active)
					.unwrap_or_default()
						>= T::MinerStakeThreshold::get().into(),
					Error::<T>::InsufficientStake
				);
			}

			// Get all UIDs using the MetagraphInfo provider
			let uids = T::MetagraphInfo::get_all_uids();

			if let Ok(account_bytes) = who.clone().encode().try_into() {
				let account = AccountId32::new(account_bytes);
				let who_ss58 = AccountId32::new(account.encode().try_into().unwrap_or_default())
					.to_ss58check();

				// Check if the caller is in UIDs
				let is_in_uids =
					uids.iter().any(|uid| uid.substrate_address.to_ss58check() == who_ss58);

				// If the caller is in UIDs, check if the node_type matches the role
				if is_in_uids {
					let whitelist = T::MetagraphInfo::get_whitelisted_validators();
					let is_whitelisted = whitelist.iter().any(|validator| validator == &who);

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
						<pallet_balances::Pallet<T>>::free_balance(&who) >= fee,
						Error::<T>::InsufficientBalanceForFee
					);

					if !pay_in_credits {
						// Transfer fee to the pallet's account
						<pallet_balances::Pallet<T>>::transfer(
							&who.clone(),
							&Self::account_id(),
							fee,
							ExistenceRequirement::AllowDeath,
						)?;
					} else {
						// decrease credits and mint balance
						let fee_u128: u128 = fee.try_into().unwrap_or_default();
						CreditsPallet::<T>::decrease_user_credits(&who.clone(), fee_u128);
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
				registered_at: current_block_number,
				owner: who,
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

			Self::do_unregister_node(node_id);

			Ok(().into())
		}

		#[pallet::call_index(10)]
		#[pallet::weight((0, Pays::No))]
		pub fn force_unregister_coldkey_node(
			origin: OriginFor<T>,
			node_id: Vec<u8>,
		) -> DispatchResultWithPostInfo {
			ensure_root(origin)?;

			Self::do_unregister_main_node(node_id);

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
			Self::do_unregister_node(node_id);

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
			Self::do_unregister_main_node(node_id);

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
	}

	impl<T: Config> Pallet<T> {
		/// Helper function to check if an owner already has a registered node
		fn is_owner_node_registered(owner: &T::AccountId) -> bool {
			// Check both ColdkeyNodeRegistration and NodeRegistration storage maps
			ColdkeyNodeRegistration::<T>::iter().any(|(_, node_info)| 
				node_info.map_or(false, |info| info.owner == *owner)
			) || 
			NodeRegistration::<T>::iter().any(|(_, node_info)| 
				node_info.map_or(false, |info| info.owner == *owner)
			)
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
				return Some(coldkey_info);
			}
			NodeRegistration::<T>::get(&node_id)
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
			
			if let Some(node_info) = NodeRegistration::<T>::get(&node_id) {
				// Find the main node by searching LinkedNodes
				let main_node_id = LinkedNodes::<T>::iter()
					.find_map(|(main_node, linked_nodes)| 
						if linked_nodes.contains(&node_id) {
							Some(main_node)
						} else {
							None
						}
					)
					.unwrap_or(node_id.clone());

				// Remove the node registration
				NodeRegistration::<T>::remove(&node_id);
				
				// Remove metrics for the main node
				T::MetricsInfo::remove_metrics(main_node_id.clone());
				
				// Remove the node from LinkedNodes if it exists
				let main_node_linked_nodes = LinkedNodes::<T>::get(&main_node_id);
				let updated_linked_nodes: Vec<Vec<u8>> = main_node_linked_nodes
					.into_iter()
					.filter(|n| n != &node_id)
					.collect();
					
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
			let coldkey_node = ColdkeyNodeRegistration::<T>::iter()
				.find_map(|(_, node_info)| {
					node_info
						.filter(|info| info.owner == *owner)
				});

			if coldkey_node.is_some() {
				return coldkey_node;
			}

			// If not found, check NodeRegistration
			let node_reg_node = NodeRegistration::<T>::iter()
				.find_map(|(_, node_info)| {
					node_info
						.filter(|info| info.owner == *owner)
				});

			node_reg_node
		}

	}
}
// We make sure this pallet uses `no_std` for compiling to Wasm.
#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

#[cfg(feature = "runtime-benchmarks")]
mod benchmarking;
pub mod weights;
pub use weights::*;
mod types;
pub use types::*;
use frame_support::PalletId;
use sp_runtime::{SaturatedConversion,KeyTypeId};
use pallet_balances;
use core::marker::PhantomData;
#[cfg(feature = "std")]
use scale_info::prelude::vec;
use sp_io::hashing::blake2_128;

type BalanceOf<T> = <T as pallet_balances::Config>::Balance;

/// Defines application identifier for crypto keys of this module.
///
/// Every module that deals with signatures needs to declare its unique identifier for
/// its crypto keys.
/// When offchain worker is signing transactions it's going to request keys of type
/// `KeyTypeId` from the keystore and use the ones it finds to sign the transaction.
/// The keys can be inserted manually via RPC (see `author_insertKey`).
pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"hips");

/// Based on the above `KeyTypeId` we need to generate a pallet-specific crypto type wrappers.
/// We can use from supported crypto kinds (`sr25519`, `ed25519` and `ecdsa`) and augment
/// the types with this pallet-specific identifier.
pub mod crypto {
	use super::KEY_TYPE;
	use sp_core::sr25519::Signature as Sr25519Signature;
	use sp_runtime::{
		app_crypto::{app_crypto, sr25519},
		traits::Verify,
		MultiSignature, MultiSigner,
	};
	app_crypto!(sr25519, KEY_TYPE);

	pub struct TestAuthId;

	impl frame_system::offchain::AppCrypto<MultiSigner, MultiSignature> for TestAuthId {
		type RuntimeAppPublic = Public;
		type GenericSignature = sp_core::sr25519::Signature;
		type GenericPublic = sp_core::sr25519::Public;
	}

	impl frame_system::offchain::AppCrypto<<Sr25519Signature as Verify>::Signer, Sr25519Signature>
		for TestAuthId
	{
		type RuntimeAppPublic = Public;
		type GenericSignature = sp_core::sr25519::Signature;
		type GenericPublic = sp_core::sr25519::Public;
	}
}


pub const PALLET_ID: PalletId = PalletId(*b"mrktplce");

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use frame_support::pallet_prelude::*;
	use sp_std::vec::Vec;
	use frame_system::{pallet_prelude::*, offchain::{Signer, SendTransactionTypes, SendUnsignedTransaction,AppCrypto}};
	use sp_runtime::{
		traits::{AccountIdConversion, Zero, Get},
		offchain::{Duration, storage_lock::{BlockAndTime,StorageLock}}
	};
	use pallet_registration::NodeType;
	use sp_std::collections::btree_map::BTreeMap;

	const LOCK_BLOCK_EXPIRATION: u32 = 3;
    const LOCK_TIMEOUT_EXPIRATION: u32 = 10000;

	#[pallet::pallet]
	#[pallet::without_storage_info]
	pub struct Pallet<T, I = ()>(_); // Use `I` for the instance type
	
	#[pallet::config]
	pub trait Config<I: 'static = ()>: frame_system::Config + 
		pallet_metagraph::Config +
		pallet_staking::Config + 
		SendTransactionTypes<Call<Self, I>> + 
		pallet_registration::Config + 
		pallet_balances::Config +
		frame_system::offchain::SigningTypes // Add this line
		{
		// type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
		type RuntimeEvent: From<Event<Self, I>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;
		
		/// The identifier type for an offchain worker.
		type AuthorityId: AppCrypto<Self::Public, Self::Signature>;

        /// The PalletId for this pallet
        #[pallet::constant]
        type PalletId: Get<frame_support::PalletId>;
		
	   /// Percentage of total rewards allocated to compute nodes (0-100)
	   #[pallet::constant]
	   type ComputeNodesRewardPercentage: Get<u32>;
   
	   /// Percentage of total rewards allocated to miner nodes (0-100)
	   #[pallet::constant]
	   type MinerNodesRewardPercentage: Get<u32>;

	   	/// Percentage of total rewards allocated to miner nodes (0-100)
		#[pallet::constant]
		type InstanceID: Get<u16>;

		#[pallet::constant]
        type BlocksPerEra: Get<u32>;
   }

   #[pallet::storage]
   #[pallet::getter(fn rank_distribution_limit)]
   pub type RankDistributionLimit<T: Config<I>, I: 'static = ()> = StorageValue<_, u16, ValueQuery>;   

	#[pallet::event]
	#[pallet::generate_deposit(pub fn deposit_event)]
	pub enum Event<T: Config<I>, I: 'static = ()> {
		SomethingStored {
			something: u32,
			who: T::AccountId,
		},
		RankingsUpdated { count: u32 },
		RewardDistributed {
			account: T::AccountId,
			amount: BalanceOf<T>,
		},
		RankDistributionLimitUpdated { new_limit: u16 },
	}

	#[pallet::error]
	pub enum Error<T, I = ()> {
		/// Value is None.
		NoneValue,
		/// Storage overflow occurred.
		StorageOverflow,
		/// Input provided is invalid.
		InvalidInput,
		/// Error during conversion.
		ConversionError,
		/// No signer was available to submit the transaction
		NoSignerAvailable,
		/// Could not acquire the lock for updating rankings
		CannotAcquireLock,
	}

	// rankings of the nodes
	#[pallet::storage]
	pub type Rankings<T: Config<I>, I: 'static = ()> = StorageMap<_, Blake2_128Concat,Vec<u8>, NodeRankings<BlockNumberFor<T>>>;

	#[pallet::storage]
	pub type RankedList<T: Config<I>, I: 'static = ()> = StorageValue<_, Vec<NodeRankings<BlockNumberFor<T>>>, ValueQuery>; // Sorted list of nodes by weight

	#[pallet::storage]
	pub type LastGlobalUpdate<T: Config<I>, I: 'static = ()> = StorageValue<_, BlockNumberFor<T>, ValueQuery>;

	// hitorical tracking
	#[pallet::storage]
	pub type RewardsRecord<T: Config<I>, I: 'static = ()> = StorageMap<
		_,
		Blake2_128Concat,
		Vec<u8>, // node id
		Vec<RewardsRecordDetails<T::AccountId, BlockNumberFor<T>>>, // Vec of (timestamp, weight, amount)
		ValueQuery,
	>;
	
	#[derive(Encode, Decode, Clone, TypeInfo)]
	pub struct RewardsRecordDetails<AccountId, BlockNumberFor> {
		pub node_types: NodeType,
		pub weight: u16,
		pub amount: u128,
		pub account: AccountId,
		pub block_number: BlockNumberFor,
	}

	#[derive(Encode, Decode, Clone, TypeInfo)]
	pub struct MinerRewardSummary<AccountId> {
		pub account: AccountId,
		pub reward: u128,
	}
	
	// Validate unsigned transactions implementation
	#[pallet::validate_unsigned]
	impl<T: Config<I>, I: 'static> ValidateUnsigned for Pallet<T, I> {
		type Call = Call<T, I>;

		fn validate_unsigned(_source: TransactionSource, call: &Self::Call) -> TransactionValidity {
			match call {
                Call::update_rankings { 
                    all_nodes_ss58, 
                    weights, 
                    node_ids, 
                    node_types ,
					block_number,
					ranking_instance_id, 
                } => {
                    // Create a unique hash based on the input parameters
                    let unique_hash = blake2_128(
                        &[
                            all_nodes_ss58.concat(), 
                            weights.iter().flat_map(|w| w.to_le_bytes()).collect::<Vec<_>>(), 
                            node_ids.concat(), 
                            node_types.iter().map(|nt| nt.encode()).collect::<Vec<_>>().concat(),
							block_number.encode(),
							ranking_instance_id.encode(),
                        ].concat()
                    );

                    ValidTransaction::with_tag_prefix("RankingOffchain")
                        .priority(TransactionPriority::max_value())
                        .longevity(5)
                        .propagate(true)
                        // Add the unique hash to ensure transaction uniqueness
                        .and_provides(unique_hash)
                        .build()
                },
				_ => InvalidTransaction::Call.into(),
			}
		}
	}

	#[pallet::call]
	impl<T: Config<I>, I: 'static> Pallet<T, I> {
		#[pallet::call_index(0)]
        #[pallet::weight(Weight::from_parts(10_000, 0) + T::DbWeight::get().writes(1))]
		pub fn update_rankings(origin: OriginFor<T>, all_nodes_ss58 : Vec<Vec<u8>>,  weights: Vec<u16>, node_ids: Vec<Vec<u8>>, node_types: Vec<NodeType> , _block_number: BlockNumberFor<T>, _ranking_instance_id: u32) -> DispatchResult {
			// Check that the extrinsic was signed and get the signer.
			ensure_none(origin)?;

			let _ = Self::do_update_rankings( weights, all_nodes_ss58,  node_ids, node_types);
			// Return a successful `DispatchResult`
			Ok(())
		}

		#[pallet::call_index(1)]
        #[pallet::weight(T::DbWeight::get().writes(1))]
        pub fn update_rank_distribution_limit(
            origin: OriginFor<T>,
            new_limit: u16,
        ) -> DispatchResult {
            // Ensure the call is from sudo
            ensure_root(origin)?;
            
            // Update the storage
            RankDistributionLimit::<T,I>::put(new_limit);
            
            // Emit an event
            Self::deposit_event(Event::RankDistributionLimitUpdated { new_limit });
            
            Ok(())
        }
	}

	impl<T: Config<I>, I: 'static> Pallet<T, I> {
		// Your save_rankings_update function
		pub fn save_rankings_update(weights: Vec<u16>, all_nodes_ss58: Vec<Vec<u8>>, node_ids: Vec<Vec<u8>>, node_types: Vec<NodeType>, block_number: BlockNumberFor<T>, ranking_instance_id: u32) {
			let mut lock = StorageLock::<BlockAndTime<frame_system::Pallet<T>>>::with_block_and_time_deadline(
				b"Ranking::update_rankings_lock",
				LOCK_BLOCK_EXPIRATION,
				Duration::from_millis(LOCK_TIMEOUT_EXPIRATION.into()),
			);

			if let Ok(_guard) = lock.try_lock() {
				let signer = Signer::<T, <T as pallet::Config<I>>::AuthorityId>::all_accounts();

				// Check if there are any accounts and log them
				if signer.can_sign() {
					log::info!("Signer has accounts available for signing.");
				} else {
					log::warn!("No accounts available for signing in signer.");
				}

				let results = signer.send_unsigned_transaction(
					|account| {
						// Create a payload with all necessary data for the update_rankings call
						UpdateRankingsPayload {
							all_nodes_ss58: all_nodes_ss58.clone(),
							weights: weights.clone(),
							node_ids: node_ids.clone(),
							node_types: node_types.clone(),
							ranking_instance_id: ranking_instance_id.clone(),
							public: account.public.clone(),
							block_number,
							_marker: PhantomData, // Add this line
						}
					},
					|payload, _signature| {
						// Construct the call with the payload and signature
						Call::update_rankings {
							all_nodes_ss58: payload.all_nodes_ss58,
							weights: payload.weights,
							node_ids: payload.node_ids,
							node_types: payload.node_types,
							ranking_instance_id: payload.ranking_instance_id,
							block_number: payload.block_number
						}
					},
				);

				// Restore error processing with more comprehensive logging
				for (acc, res) in &results {
					match res {
						Ok(_) => log::info!("[{:?}] Successfully submitted rankings update", acc.id),
						Err(e) => log::error!("[{:?}] Failed to submit rankings update: {:?}", acc.id, e),
					}
				}
			} else {
				log::error!("❌ Could not acquire lock for saving Rankings Update");
			};
		}


		/// Update rankings based on node ids and their corresponding weights
		pub fn do_update_rankings( weights: Vec<u16>, all_nodes_ss58 : Vec<Vec<u8>>, node_ids: Vec<Vec<u8>>, node_types: Vec<NodeType>) -> DispatchResult {
			// Ensure vectors have same length
			ensure!(node_ids.len() == weights.len(), Error::<T,I>::InvalidInput);
			let current_block = frame_system::Pallet::<T>::block_number();
			let timestamp: u64 = current_block.try_into().map_err(|_| Error::<T,I>::ConversionError)?;
			
			// Create a vector to store all rankings
			let mut all_rankings: Vec<NodeRankings<BlockNumberFor<T>>> = Vec::new();
			
			// Process each UID and its weight
			for (index, &ref node_id) in node_ids.iter().enumerate() {
				let weight = weights[index];
				let node_type = node_types[index].clone();
				let node_ss58 = all_nodes_ss58[index].clone();
				
				// Create or update ranking entry
				let node_ranking = NodeRankings {
					rank: 0, // Will be updated after sorting
					node_id: node_id.clone(),
					node_ss58_address: node_ss58,
					node_type,
					weight,
					last_updated: current_block,
					is_active: true,
				};
					
				// Store in Rankings map
				Rankings::<T,I>::insert(node_id.clone() , node_ranking.clone());
					
				// Add to vector for sorting
				all_rankings.push(node_ranking);
			}
			
			// Sort rankings by weight (descending)
			all_rankings.sort_by(|a, b| b.weight.cmp(&a.weight));
			
			// Update ranks
			for (index, ranking) in all_rankings.iter_mut().enumerate() {
				ranking.rank = (index + 1) as u32;
			}
			
			// Store sorted list
			RankedList::<T,I>::put(all_rankings);
			
			// Update last global update
			let time : BlockNumberFor<T> = (timestamp as u32).into();
			LastGlobalUpdate::<T,I>::put(time);
			
			Self::deposit_event(Event::RankingsUpdated { count: node_ids.len() as u32 });
		
			Ok(())
		}

		// Helper function to get total rewards for a specific account across all nodes
		pub fn get_total_node_rewards(account: T::AccountId) -> u128 {
			// Iterate through all nodes' reward records
			RewardsRecord::<T, I>::iter()
				.map(|(_, records)| {
					// Sum up all the reward amounts for the specific account
					records.iter()
						.filter(|record| record.account == account.clone())
						.map(|record| record.amount)
						.sum::<u128>()
				})
				.sum()
		}

		// Helper function to get total distributed rewards for miners of a specific node type
		pub fn get_miners_total_rewards(node_type: NodeType) -> Vec<MinerRewardSummary<T::AccountId>> {
			// Use a BTreeMap to aggregate rewards by account
			let mut reward_map: BTreeMap<T::AccountId, u128> = BTreeMap::new();

			// Iterate through all nodes' reward records
			RewardsRecord::<T, I>::iter()
				.for_each(|(node_id, records)| {
					// Check if the node type matches the requested type
					if let Ok(node_info) = pallet_registration::Pallet::<T>::get_registered_node(node_id.clone()) {
						if node_info.node_type == node_type {
							// Aggregate rewards for each account
							records.iter()
								.for_each(|record| {
									*reward_map.entry(record.account.clone()).or_insert(0) += record.amount;
								});
						}
					}
				});

			// Convert the BTreeMap to a Vec of MinerRewardSummary
			reward_map.into_iter()
				.map(|(account, reward)| MinerRewardSummary { account, reward })
				.collect()
		}

		// Helper function to get total distributed rewards for a specific node type
		pub fn get_total_distributed_rewards_by_node_type(node_type: NodeType) -> u128 {
			// Iterate through all nodes' reward records
			RewardsRecord::<T, I>::iter()
				.filter_map(|(node_id, records)| {
					// Get the registered node information
					pallet_registration::Pallet::<T>::get_registered_node(node_id.clone())
						.ok()
						.filter(|node_info| node_info.node_type == node_type)
						.map(|_| {
							// Sum total rewards for this node type
							records.iter() 
								.map(|record| record.amount)
								.sum::<u128>()
						})
				})
				.sum()
		}

	    /// The account ID of the marketplace pallet
		pub fn account_id() -> T::AccountId {
			<T as pallet::Config<I>>::PalletId::get().into_account_truncating()
		}

		// Helper function to get pending rewards for a specific account
		pub fn get_account_pending_rewards(account: T::AccountId) -> Vec<MinerRewardSummary<T::AccountId>> {
			// Get the sorted list of rankings
			let ranked_list = RankedList::<T,I>::get();
			
			// Get pallet's account balance
			let pallet_account = Self::account_id();
			let total_balance = pallet_balances::Pallet::<T>::free_balance(&pallet_account);

			// Separate nodes by type
			let mut compute_miner_node: Vec<(T::AccountId, u16, Vec<u8>)> = Vec::new();
			let mut storage_miner_node: Vec<(T::AccountId, u16, Vec<u8>)> = Vec::new();
			let mut gpu_miner_node: Vec<(T::AccountId, u16, Vec<u8>)> = Vec::new();
			let mut storage_s3_miner_node: Vec<(T::AccountId, u16, Vec<u8>)> = Vec::new();

			for ranking in ranked_list.iter() {
				if let Ok(node_info) = pallet_registration::Pallet::<T>::get_registered_node(ranking.node_id.clone()) {
					if node_info.owner == account {
						match node_info.node_type {
							NodeType::ComputeMiner => compute_miner_node.push((node_info.owner, ranking.weight, node_info.node_id)),
							NodeType::StorageMiner => storage_miner_node.push((node_info.owner, ranking.weight, node_info.node_id)),
							NodeType::GpuMiner => gpu_miner_node.push((node_info.owner, ranking.weight, node_info.node_id)),
							NodeType::StorageS3 => storage_s3_miner_node.push((node_info.owner, ranking.weight, node_info.node_id)),
							_ => {} // Ignore validator nodes
						}
					}
				}
			}

			let mut pending_rewards = Vec::new();
			
			// Calculate pending rewards for each node type
			if T::InstanceID::get() == 2 && !compute_miner_node.is_empty() {
				let compute_miner_total_weight: u128 = compute_miner_node.iter()
					.map(|(_, weight, _)| *weight as u128)
					.sum();

				for (owner, weight, _) in compute_miner_node {
					let weight_u128 = weight as u128;
					let reward = if let Some(ratio) = weight_u128
						.checked_mul(total_balance.saturated_into())
						.and_then(|r| r.checked_div(compute_miner_total_weight)) 
					{
						let decimal_factor: u128 = 10_u128.pow(pallet_registration::Pallet::<T>::get_chain_decimals());
						let reward_with_decimals = ratio.checked_mul(decimal_factor)
							.unwrap_or_default();
						reward_with_decimals
					} else {
						0
					};

					pending_rewards.push(MinerRewardSummary {
						account: owner,
						reward,
					});
				}
			} else if T::InstanceID::get() == 4 && !gpu_miner_node.is_empty() {
				let gpu_miner_total_weight: u128 = gpu_miner_node.iter()
					.map(|(_, weight, _)| *weight as u128)
					.sum();

				for (owner, weight, _) in gpu_miner_node {
					let weight_u128 = weight as u128;
					let reward = if let Some(ratio) = weight_u128
						.checked_mul(total_balance.saturated_into())
						.and_then(|r| r.checked_div(gpu_miner_total_weight)) 
					{
						let decimal_factor: u128 = 10_u128.pow(pallet_registration::Pallet::<T>::get_chain_decimals());
						let reward_with_decimals = ratio.checked_mul(decimal_factor)
							.unwrap_or_default();
						reward_with_decimals
					} else {
						0
					};

					pending_rewards.push(MinerRewardSummary {
						account: owner,
						reward,
					});
				}
			} else if T::InstanceID::get() == 5 && !storage_s3_miner_node.is_empty() {
				let s3_miner_total_weight: u128 = storage_s3_miner_node.iter()
					.map(|(_, weight, _)| *weight as u128)
					.sum();

				for (owner, weight, _) in storage_s3_miner_node {
					let weight_u128 = weight as u128;
					let reward = if let Some(ratio) = weight_u128
						.checked_mul(total_balance.saturated_into())
						.and_then(|r| r.checked_div(s3_miner_total_weight)) 
					{
						let decimal_factor: u128 = 10_u128.pow(pallet_registration::Pallet::<T>::get_chain_decimals());
						let reward_with_decimals = ratio.checked_mul(decimal_factor)
							.unwrap_or_default();
						reward_with_decimals
					} else {
						0
					};

					pending_rewards.push(MinerRewardSummary {
						account: owner,
						reward,
					});
				}
			} else if T::InstanceID::get() == 1 && !storage_miner_node.is_empty() {
				let storage_miner_total_weight: u128 = storage_miner_node.iter()
					.map(|(_, weight, _)| *weight as u128)
					.sum();

				for (owner, weight, _) in storage_miner_node {
					let weight_u128 = weight as u128;
					let reward = if let Some(ratio) = weight_u128
						.checked_mul(total_balance.saturated_into())
						.and_then(|r| r.checked_div(storage_miner_total_weight)) 
					{
						let decimal_factor: u128 = 10_u128.pow(pallet_registration::Pallet::<T>::get_chain_decimals());
						let reward_with_decimals = ratio.checked_mul(decimal_factor)
							.unwrap_or_default();
						reward_with_decimals
					} else {
						0
					};

					pending_rewards.push(MinerRewardSummary {
						account: owner,
						reward,
					});
				}
			}

			pending_rewards
		}

		/// Helper function to retrieve the ranked list of nodes.
		pub fn get_ranked_list() -> Vec<NodeRankings<BlockNumberFor<T>>> {
			RankedList::<T,I>::get() // Access the storage item
		}

		// Helper function to get list of miners with their pending rewards for a specific node type
		pub fn get_miners_pending_rewards(node_type: NodeType) -> Vec<MinerRewardSummary<T::AccountId>> {
			// Get the sorted list of rankings
			let ranked_list = RankedList::<T,I>::get(); 
			
			// Get pallet's account balance
			let pallet_account = Self::account_id();
			let total_balance = pallet_balances::Pallet::<T>::free_balance(&pallet_account);

			// Separate nodes by type
			let mut target_nodes: Vec<(T::AccountId, u16, Vec<u8>)> = Vec::new();

			for ranking in ranked_list.iter() {
				if let Ok(node_info) = pallet_registration::Pallet::<T>::get_registered_node(ranking.node_id.clone()) {
					if node_info.node_type == node_type {
						target_nodes.push((node_info.owner, ranking.weight, node_info.node_id));
					}
				}
			}

			// Use sp_std::collections::btree_map::BTreeMap for WASM compatibility
			let mut pending_reward_map: BTreeMap<T::AccountId, u128> = 
				BTreeMap::new();

			// Calculate total weight
			let total_weight: u128 = target_nodes.iter()
				.map(|(_, weight, _)| *weight as u128)
				.sum();

			// Calculate rewards for each account
			for (account, weight, _) in target_nodes {
				let weight_u128 = weight as u128;
				let reward = if let Some(ratio) = weight_u128
					.checked_mul(total_balance.saturated_into())
					.and_then(|r| r.checked_div(total_weight)) 
				{
					let decimal_factor: u128 = 10_u128.pow(pallet_registration::Pallet::<T>::get_chain_decimals());
					let reward_with_decimals = ratio.checked_mul(decimal_factor)
						.unwrap_or_default();
					reward_with_decimals
				} else {
					0
				};

				*pending_reward_map.entry(account).or_insert(0) += reward;
			}

			// Convert the BTreeMap to a Vec of MinerRewardSummary
			pending_reward_map.into_iter()
				.map(|(account, reward)| MinerRewardSummary { account, reward })
				.collect()
		}
	}

	#[derive(Encode, Decode, Clone, TypeInfo)]
	pub struct PendingRewardDetails<T: Config<I>, I: 'static = ()> {
		pub node_type: NodeType,
		pub node_id: Vec<u8>,
		pub weight: u16,
		pub pending_amount: u128,
		pub block_number: BlockNumberFor<T>,
		pub _marker: PhantomData<I>,
	}

	#[pallet::hooks]
	impl<T: Config<I>, I: 'static> Hooks<BlockNumberFor<T>> for Pallet<T, I> {
		fn on_initialize(n: BlockNumberFor<T>) -> Weight {
			let mut weight_used = Weight::zero();
			if n % T::BlocksPerEra::get().into() == Zero::zero() {
				let mut distribution_count: u16 = 0;
	
				// Get the sorted list of rankings
				let ranked_list = RankedList::<T,I>::get();
				// Get pallet's account balance
				let pallet_account = Self::account_id();
	
				let total_balance = pallet_balances::Pallet::<T>::free_balance(&pallet_account);
					
				// Only proceed if we have balance to distribute
				if !total_balance.is_zero() {
					// Separate nodes by type and take only up to the limit
					let mut compute_miner_node: Vec<(T::AccountId, u16, Vec<u8>)> = Vec::new();
					let mut storage_miner_node: Vec<(T::AccountId, u16, Vec<u8>)> = Vec::new();
					let mut gpu_miner_node: Vec<(T::AccountId, u16, Vec<u8>)> = Vec::new();
					let mut storage_s3_miner_node: Vec<(T::AccountId, u16, Vec<u8>)> = Vec::new();
	
					for ranking in ranked_list.iter() {
						if distribution_count >= Self::rank_distribution_limit() {
							break;
						}
						
						// Get UID info to check role and get owner address
						if let Ok(node_info) = pallet_registration::Pallet::<T>::get_registered_node(ranking.node_id.clone()) {
							distribution_count += 1;
							match node_info.node_type {
								NodeType::ComputeMiner => compute_miner_node.push((node_info.owner, ranking.weight, node_info.node_id)),
								NodeType::StorageMiner => storage_miner_node.push((node_info.owner, ranking.weight, node_info.node_id)),
								NodeType::GpuMiner => gpu_miner_node.push((node_info.owner, ranking.weight, node_info.node_id)),
								NodeType::StorageS3 => storage_s3_miner_node.push((node_info.owner, ranking.weight, node_info.node_id)),
								_ => {} // Ignore validator nodes
							}
						}
					}

					// if instance id is 2 the distribute to compute miner nodes
					if T::InstanceID::get() == 2 {
						// Calculate total weights for each type
						let compute_miner_total_weight: u128 = compute_miner_node.iter()
							.map(|(_, weight, _node_id)| *weight as u128)
							.sum();

						// Distribute to compute miner nodes
						if !compute_miner_total_weight.is_zero() {
							for (account, weight, node_id) in compute_miner_node {
								let weight_u128 = weight as u128;
								let reward = if let Some(ratio) = weight_u128
									.checked_mul(total_balance.saturated_into())
									.and_then(|r| r.checked_div(compute_miner_total_weight)) 
								{
									// Convert to proper decimal representation
									// If your chain uses 18 decimals, the reward should be multiplied by 10^18
									let decimal_factor: u128 = 10_u128.pow(pallet_registration::Pallet::<T>::get_chain_decimals());
									let reward_with_decimals = ratio.checked_mul(decimal_factor)
										.unwrap_or_default();
									BalanceOf::<T>::saturated_from(reward_with_decimals)
								} else {
									BalanceOf::<T>::zero()
								};
		
								if !reward.is_zero() {
									// Burn the equivalent amount from their free balance
									let _ = pallet_balances::Pallet::<T>::burn(
										frame_system::RawOrigin::Signed(pallet_account.clone()).into(),
										reward,
										false, // keep_alive set to false to allow burning entire balance
									);
		
									// Convert reward to u128 first
									let reward_u128: u128 = reward.saturated_into();
										
									// Try to convert to staking balance type
									if let Ok(_ledger) = pallet_staking::Pallet::<T>::ledger(sp_staking::StakingAccount::Stash(account.clone())) {
										// Account is already bonded, so we can use bond_extra
										if let Ok(staking_reward) = reward_u128.try_into() {
											let _ = pallet_staking::Pallet::<T>::bond_extra(
												frame_system::RawOrigin::Signed(account.clone()).into(),
												staking_reward,
											);
											Self::deposit_event(Event::RewardDistributed {
												account: account.clone(),
												amount: reward,
											});
										}
									} else {
										// Account is not bonded yet, so we bond it first
										if let Ok(staking_reward) = reward_u128.try_into() {
											let _ = pallet_staking::Pallet::<T>::bond(
												frame_system::RawOrigin::Signed(account.clone()).into(),
												staking_reward, // Initial stake
												pallet_staking::RewardDestination::Staked, // Reward destination
											);
											Self::deposit_event(Event::RewardDistributed {
												account: account.clone(),
												amount: reward,
											});
										}
									}
											
									// Get the current records for this node_id
									let mut records = RewardsRecord::<T, I>::get(node_id.clone());

									records.push(RewardsRecordDetails {
										node_types: NodeType::StorageS3,
										weight: reward_u128 as u16,
										amount: reward_u128,
										account: account.clone(),
										block_number: n
									});
									
									// Optionally: Keep only last 100 entries
									if records.len() > 100 {
										records.remove(0);
									}
									
									// Put the updated records back into storage
									RewardsRecord::<T, I>::insert(node_id, records);
								}
								weight_used = weight_used.saturating_add(T::DbWeight::get().reads_writes(3, 1));
							}
						}					
					}
					// if instance id is 4 the distribute to compute miner nodes
					else if T::InstanceID::get() == 4 {
						// Calculate total weights for each type
						let gpu_miner_total_weight: u128 = gpu_miner_node.iter()
							.map(|(_, weight, _node_id)| *weight as u128)
							.sum();

						// Distribute to compute miner nodes
						if !gpu_miner_total_weight.is_zero() {
							for (account, weight, node_id) in gpu_miner_node {
								let weight_u128 = weight as u128;
								let reward = if let Some(ratio) = weight_u128
									.checked_mul(total_balance.saturated_into())
									.and_then(|r| r.checked_div(gpu_miner_total_weight)) 
								{
									// Convert to proper decimal representation
									// If your chain uses 18 decimals, the reward should be multiplied by 10^18
									let decimal_factor: u128 = 10_u128.pow(pallet_registration::Pallet::<T>::get_chain_decimals());
									let reward_with_decimals = ratio.checked_mul(decimal_factor)
										.unwrap_or_default();
									BalanceOf::<T>::saturated_from(reward_with_decimals)
								} else {
									BalanceOf::<T>::zero()
								};
		
								if !reward.is_zero() {
									// Burn the equivalent amount from their free balance
									let _ = pallet_balances::Pallet::<T>::burn(
										frame_system::RawOrigin::Signed(pallet_account.clone()).into(),
										reward,
										false, // keep_alive set to false to allow burning entire balance
									);
		
									// Convert reward to u128 first
									let reward_u128: u128 = reward.saturated_into();
										
									// Try to convert to staking balance type
									if let Ok(_ledger) = pallet_staking::Pallet::<T>::ledger(sp_staking::StakingAccount::Stash(account.clone())) {
										// Account is already bonded, so we can use bond_extra
										if let Ok(staking_reward) = reward_u128.try_into() {
											let _ = pallet_staking::Pallet::<T>::bond_extra(
												frame_system::RawOrigin::Signed(account.clone()).into(),
												staking_reward,
											);
											Self::deposit_event(Event::RewardDistributed {
												account: account.clone(),
												amount: reward,
											});
										}
									} else {
										// Account is not bonded yet, so we bond it first
										if let Ok(staking_reward) = reward_u128.try_into() {
											let _ = pallet_staking::Pallet::<T>::bond(
												frame_system::RawOrigin::Signed(account.clone()).into(),
												staking_reward, // Initial stake
												pallet_staking::RewardDestination::Staked, // Reward destination
											);
											Self::deposit_event(Event::RewardDistributed {
												account: account.clone(),
												amount: reward,
											});
										}
									}
											
									// Get the current records for this node_id
									let mut records = RewardsRecord::<T, I>::get(node_id.clone());

									records.push(RewardsRecordDetails {
										node_types: NodeType::StorageS3,
										weight: reward_u128 as u16,
										amount: reward_u128,
										account: account.clone(),
										block_number: n
									});
									
									// Optionally: Keep only last 100 entries
									if records.len() > 100 {
										records.remove(0);
									}
									
									// Put the updated records back into storage
									RewardsRecord::<T, I>::insert(node_id, records);
								}
								weight_used = weight_used.saturating_add(T::DbWeight::get().reads_writes(3, 1));
							}
						}				
					}
					// if instance id is 5 the distribute to s3 miner nodes
					else if T::InstanceID::get() == 5 {
						// Calculate total weights for each type
						let s3_miner_total_weight: u128 = storage_s3_miner_node.iter()
							.map(|(_, weight, _node_id)| *weight as u128)
							.sum();

						// Distribute to s3 miner nodes
						if !s3_miner_total_weight.is_zero() {
							for (account, weight, node_id) in storage_s3_miner_node {
								let weight_u128 = weight as u128;
								let reward = if let Some(ratio) = weight_u128
									.checked_mul(total_balance.saturated_into())
									.and_then(|r| r.checked_div(s3_miner_total_weight)) 
								{
									// Convert to proper decimal representation
									// If your chain uses 18 decimals, the reward should be multiplied by 10^18
									let decimal_factor: u128 = 10_u128.pow(pallet_registration::Pallet::<T>::get_chain_decimals());
									let reward_with_decimals = ratio.checked_mul(decimal_factor)
										.unwrap_or_default();
									BalanceOf::<T>::saturated_from(reward_with_decimals)
								} else {
									BalanceOf::<T>::zero()
								};
		
								if !reward.is_zero() {
									// Burn the equivalent amount from their free balance
									let _ = pallet_balances::Pallet::<T>::burn(
										frame_system::RawOrigin::Signed(pallet_account.clone()).into(),
										reward,
										false, // keep_alive set to false to allow burning entire balance
									);
		
									// Convert reward to u128 first
									let reward_u128: u128 = reward.saturated_into();
										
									// Try to convert to staking balance type
									if let Ok(_ledger) = pallet_staking::Pallet::<T>::ledger(sp_staking::StakingAccount::Stash(account.clone())) {
										// Account is already bonded, so we can use bond_extra
										if let Ok(staking_reward) = reward_u128.try_into() {
											let _ = pallet_staking::Pallet::<T>::bond_extra(
												frame_system::RawOrigin::Signed(account.clone()).into(),
												staking_reward,
											);
											Self::deposit_event(Event::RewardDistributed {
												account: account.clone(),
												amount: reward,
											});
										}
									} else {
										// Account is not bonded yet, so we bond it first
										if let Ok(staking_reward) = reward_u128.try_into() {
											let _ = pallet_staking::Pallet::<T>::bond(
												frame_system::RawOrigin::Signed(account.clone()).into(),
												staking_reward, // Initial stake
												pallet_staking::RewardDestination::Staked, // Reward destination
											);
											Self::deposit_event(Event::RewardDistributed {
												account: account.clone(),
												amount: reward,
											});
										}
									}
		
									// Get the current records for this node_id
									let mut records = RewardsRecord::<T, I>::get(node_id.clone());

									records.push(RewardsRecordDetails {
										node_types: NodeType::StorageS3,
										weight: reward_u128 as u16,
										amount: reward_u128,
										account: account.clone(),
										block_number: n
									});
									
									// Optionally: Keep only last 100 entries
									if records.len() > 100 {
										records.remove(0);
									}
									
									// Put the updated records back into storage
									RewardsRecord::<T, I>::insert(node_id, records);
								}
								weight_used = weight_used.saturating_add(T::DbWeight::get().reads_writes(3, 1));
							}
						}				
					}
					// if instance id is 1 the distribute to storage miner nodes
					else if T::InstanceID::get() == 1{
						
						let storage_miner_total_weight: u128 = storage_miner_node.iter()
						.map(|(_, weight, _node_id)| *weight as u128)
						.sum();
	
						// Distribute to storage miner nodes
						if !storage_miner_total_weight.is_zero() {
							for (account, weight, node_id) in storage_miner_node {
								let weight_u128 = weight as u128;
								let reward = if let Some(ratio) = weight_u128
									.checked_mul(total_balance.saturated_into())
									.and_then(|r| r.checked_div(storage_miner_total_weight)) 
								{
									// Convert to proper decimal representation
									let decimal_factor: u128 = 10_u128.pow(pallet_registration::Pallet::<T>::get_chain_decimals());
									let reward_with_decimals = ratio.checked_mul(decimal_factor)
										.unwrap_or_default();
									BalanceOf::<T>::saturated_from(reward_with_decimals)
								} else {
									BalanceOf::<T>::zero()
								};
		
								if !reward.is_zero() {
									// Burn the equivalent amount from their free balance
									let _ = pallet_balances::Pallet::<T>::burn(
										frame_system::RawOrigin::Signed(pallet_account.clone()).into(),
										reward,
										false, // keep_alive set to false to allow burning entire balance
									);
		
									// Convert reward to u128 first
									let reward_u128: u128 = reward.saturated_into();
			
									// Try to convert to staking balance type
									if let Ok(_ledger) = pallet_staking::Pallet::<T>::ledger(sp_staking::StakingAccount::Stash(account.clone())) {
										// Account is already bonded, so we can use bond_extra
										if let Ok(staking_reward) = reward_u128.try_into() {
											let _ = pallet_staking::Pallet::<T>::bond_extra(
												frame_system::RawOrigin::Signed(account.clone()).into(),
												staking_reward,
											);
											Self::deposit_event(Event::RewardDistributed {
												account: account.clone(),
												amount: reward,
											});
										}
									} else {
										// Account is not bonded yet, so we bond it first
										if let Ok(staking_reward) = reward_u128.try_into() {
											let _ = pallet_staking::Pallet::<T>::bond(
												frame_system::RawOrigin::Signed(account.clone()).into(),
												staking_reward, // Initial stake
												pallet_staking::RewardDestination::Staked, // Reward destination
											);
											Self::deposit_event(Event::RewardDistributed {
												account: account.clone(),
												amount: reward,
											});
										}
									}

									// Get the current records for this node_id
									let mut records = RewardsRecord::<T, I>::get(node_id.clone());

									records.push(RewardsRecordDetails {
										node_types: NodeType::StorageS3,
										weight: reward_u128 as u16,
										amount: reward_u128,
										account: account.clone(),
										block_number: n
									});
									
									// Optionally: Keep only last 100 entries
									if records.len() > 100 {
										records.remove(0);
									}
									
									// Put the updated records back into storage
									RewardsRecord::<T, I>::insert(node_id, records);
								
								}

								weight_used = weight_used.saturating_add(T::DbWeight::get().reads_writes(3, 1));
							}
						}		
					}				
				}

			}
			weight_used
		}
	}
}
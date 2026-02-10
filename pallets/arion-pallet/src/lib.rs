#![cfg_attr(not(feature = "std"), no_std)]

pub mod weights;
pub use weights::WeightInfo;

// `pallet-arion`
//
// Minimal scaffold for Arion’s **on-chain CRUSH map** + **periodic miner stats**.
// Goals:
// - Keep placement-critical state on-chain (epoch → CRUSH input map)
// - Allow periodic aggregated stats updates without per-heartbeat writes
// - Avoid epoch regressions
// - Provide **node registration** primitives to:
//   - prevent spoofed node ids (node_id must prove ownership via signature)
//   - enforce global adaptive registration cost (anti-sybil / anti-yoyo)
//   - support deregistration with cooldown + unbonding (anti lock/unlock spam)

use codec::{Decode, Encode, MaxEncodedLen};
use frame_support::{
    dispatch::DispatchResult,
    pallet_prelude::*,
    traits::{Currency, EnsureOrigin, Get, ReservableCurrency},
    BoundedVec,
};
use sp_runtime::{
    traits::{Hash, Saturating, Zero, Verify},
    RuntimeDebug,
};
use frame_system::pallet_prelude::*;
use scale_info::TypeInfo;
use sp_core::{ed25519, H256};
use sp_std::prelude::*;

pub use pallet::*;

#[frame_support::pallet]
pub mod pallet {
    use super::*;

    /// Domain separator for attestation signing.
    ///
    /// IMPORTANT: This must be typed as `&[u8]` (slice) to match off-chain SCALE encoding.
    /// Using `b"..."` directly would be `&[u8; N]` (fixed array) which encodes differently.
    const ATTESTATION_DOMAIN_SEPARATOR: &[u8] = b"ARION_ATTESTATION_V1";

    type BalanceOf<T> =
        <<T as Config>::DepositCurrency as Currency<<T as frame_system::Config>::AccountId>>::Balance;

    /// Trait to verify if a family is registered in the registration pallet.
    pub trait FamilyRegistry<AccountId> {
        fn is_registered_family(family: &AccountId) -> bool;
    }

    impl<T: Config> FamilyRegistry<T::AccountId> for pallet_registration::Pallet<T> {
        fn is_registered_family(family: &T::AccountId) -> bool {
            pallet_registration::Pallet::<T>::is_owner_node_registered(family)
        }
    }

    impl<AccountId> FamilyRegistry<AccountId> for () {
        fn is_registered_family(_family: &AccountId) -> bool {
            false
        }
    }


    pub trait ProxyVerifier<AccountId> {
        fn can_register_child(family: &AccountId, child: &AccountId) -> bool;
    }

    impl<AccountId> ProxyVerifier<AccountId> for () {
        fn can_register_child(_family: &AccountId, _child: &AccountId) -> bool {
            false
        }
    }

    impl<T: Config> ProxyVerifier<T::AccountId> for pallet_proxy::Pallet<T> {
        fn can_register_child(family: &T::AccountId, child: &T::AccountId) -> bool {
            let (proxy_definitions, _) = pallet_proxy::Pallet::<T>::proxies(family);
            
            // You should also enforce a specific proxy type here (e.g., NonTransfer)
            // to prevent using "Any" proxies which could be too permissive
            proxy_definitions.iter().any(|p| &p.delegate == child)
            
            // Optional: Enforce specific proxy type
            // proxy_definitions.iter().any(|p| {
            //     &p.delegate == child && 
            //     matches!(p.proxy_type, ProxyType::NonTransfer) 
            // })
        }
    }

    /// CRUSH map parameters that affect deterministic placement.
    #[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
    pub struct CrushParams {
        pub pg_count: u32,
        pub ec_k: u16,
        pub ec_m: u16,
    }

    /// A miner entry in the on-chain CRUSH map.
    ///
    /// **Important:** only include fields that are required to:
    /// - compute placement deterministically
    /// - connect to the miner (addressing)
    #[derive(Encode, Decode, TypeInfo, MaxEncodedLen)]
    #[scale_info(skip_type_params(MaxEndpoint, MaxHttp))]
    pub struct MinerRecord<AccountId, MaxEndpoint, MaxHttp> 
    where
        MaxEndpoint: Get<u32> + 'static,
        MaxHttp: Get<u32> + 'static,
        AccountId: Clone,
    {
        /// Stable uid used by CRUSH placement (can be derived from pubkey off-chain; stored for speed).
        pub uid: u32,
        /// Miner node identity (ed25519 public key, 32 bytes). Encoding is chain-defined.
        pub node_id: [u8; 32],
        /// CRUSH weight.
        pub weight: u32,
        /// Failure domain (“family”) identifier.
        ///
        /// Stored as the raw `AccountId` bytes on-chain (SS58 is just the human encoding).
        pub family_id: AccountId,
        /// Miner endpoint info (your canonical encoding; e.g. SCALE-encoded EndpointAddr or compact multiaddr).
        pub endpoint: BoundedVec<u8, MaxEndpoint>,
        /// Optional HTTP address (legacy/debug). Can be empty.
        pub http_addr: BoundedVec<u8, MaxHttp>,
    }

    impl<AccountId: Clone, MaxEndpoint, MaxHttp> Clone for MinerRecord<AccountId, MaxEndpoint, MaxHttp>
    where
        MaxEndpoint: Get<u32> + 'static,
        MaxHttp: Get<u32> + 'static,
    {
        fn clone(&self) -> Self {
            Self {
                uid: self.uid,
                node_id: self.node_id,
                weight: self.weight,
                family_id: self.family_id.clone(),
                endpoint: self.endpoint.clone(),
                http_addr: self.http_addr.clone(),
            }
        }
    }

    impl<AccountId: PartialEq + Clone, MaxEndpoint, MaxHttp> PartialEq for MinerRecord<AccountId, MaxEndpoint, MaxHttp>
    where
        MaxEndpoint: Get<u32> + 'static,
        MaxHttp: Get<u32> + 'static,
    {
        fn eq(&self, other: &Self) -> bool {
            self.uid == other.uid &&
            self.node_id == other.node_id &&
            self.weight == other.weight &&
            self.family_id == other.family_id &&
            self.endpoint == other.endpoint &&
            self.http_addr == other.http_addr
        }
    }

    impl<AccountId: Eq + Clone, MaxEndpoint, MaxHttp> Eq for MinerRecord<AccountId, MaxEndpoint, MaxHttp>
    where
        MaxEndpoint: Get<u32> + 'static,
        MaxHttp: Get<u32> + 'static,
    {}

    impl<AccountId: sp_std::fmt::Debug + Clone, MaxEndpoint, MaxHttp> sp_std::fmt::Debug for MinerRecord<AccountId, MaxEndpoint, MaxHttp>
    where
        MaxEndpoint: Get<u32> + 'static,
        MaxHttp: Get<u32> + 'static,
    {
        fn fmt(&self, f: &mut sp_std::fmt::Formatter<'_>) -> sp_std::fmt::Result {
            f.debug_struct("MinerRecord")
                .field("uid", &self.uid)
                .field("node_id", &self.node_id)
                .field("weight", &self.weight)
                .field("family_id", &self.family_id)
                .field("endpoint", &self.endpoint)
                .field("http_addr", &self.http_addr)
                .finish()
        }
    }

    /// Aggregated, non-placement-critical miner stats submitted periodically.
    #[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen, Default)]
    pub struct MinerStats {
        /// Total shard/blobs stored by this miner (validator-observed).
        pub shard_count: u64,
        /// Total bytes stored for shards/blobs on this miner (validator-observed).
        pub shard_data_bytes: u128,
        /// Total strikes (or delta applied by off-chain aggregator).
        pub strikes: u32,
        /// Last seen bucket (e.g. block_number / N).
        pub last_seen_bucket: u32,
        /// Optional bandwidth aggregate (bytes) in last reporting window.
        pub bandwidth_bytes: u64,
        /// Optional integrity failure counter.
        pub integrity_fails: u32,
    }

    /// A batch stats update item.
    #[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
    pub struct MinerStatsUpdate {
        pub uid: u32,
        pub stats: MinerStats,
    }

    /// Audit result from warden proof-of-storage verification.
    #[derive(Clone, Copy, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
    pub enum AuditResult {
        /// Miner provided valid proof for the challenged data
        Passed,
        /// Miner provided incorrect proof
        Failed,
        /// Miner did not respond within timeout
        Timeout,
        /// Miner's proof was malformed or cryptographically invalid
        InvalidProof,
    }

    impl AuditResult {
        /// Convert to u8 for SCALE encoding consistency with off-chain components.
        ///
        /// This must match the encoding used by warden, validator, and chain-submitter
        /// when signing/verifying attestations.
        pub fn as_u8(&self) -> u8 {
            match self {
                AuditResult::Passed => 0,
                AuditResult::Failed => 1,
                AuditResult::Timeout => 2,
                AuditResult::InvalidProof => 3,
            }
        }
    }

    /// Attestation record (signed by warden after proof-of-storage audit).
    #[derive(Encode, Decode, TypeInfo, MaxEncodedLen)]
    #[scale_info(skip_type_params(MaxShardHash, MaxWardenPubkey, MaxSignature, MaxMerkleProof, MaxWardenId))]
    pub struct AttestationRecord<MaxShardHash, MaxWardenPubkey, MaxSignature, MaxMerkleProof, MaxWardenId>
    where
        MaxShardHash: Get<u32> + 'static,
        MaxWardenPubkey: Get<u32> + 'static,
        MaxSignature: Get<u32> + 'static,
        MaxMerkleProof: Get<u32> + 'static,
        MaxWardenId: Get<u32> + 'static,
    {
        /// BLAKE3 hash of the shard that was audited (hex string bytes)
        pub shard_hash: BoundedVec<u8, MaxShardHash>,
        /// UID of the miner being audited
        pub miner_uid: u32,
        /// Result of the audit
        pub result: AuditResult,
        /// Random seed used to generate the challenge
        pub challenge_seed: [u8; 32],
        /// Block number when challenge was issued
        pub block_number: u64,
        /// Unix timestamp when challenge was issued
        pub timestamp: u64,
        /// Warden's Ed25519 public key (32 bytes)
        pub warden_pubkey: BoundedVec<u8, MaxWardenPubkey>,
        /// Ed25519 signature over attestation (64 bytes)
        pub signature: BoundedVec<u8, MaxSignature>,
        /// Merkle proof signature hash for proof-of-storage verification
        pub merkle_proof_sig_hash: BoundedVec<u8, MaxMerkleProof>,
        /// Warden identifier (unique warden ID)
        pub warden_id: BoundedVec<u8, MaxWardenId>,
    }

    impl<MaxShardHash, MaxWardenPubkey, MaxSignature, MaxMerkleProof, MaxWardenId> Clone 
        for AttestationRecord<MaxShardHash, MaxWardenPubkey, MaxSignature, MaxMerkleProof, MaxWardenId>
    where
        MaxShardHash: Get<u32> + 'static,
        MaxWardenPubkey: Get<u32> + 'static,
        MaxSignature: Get<u32> + 'static,
        MaxMerkleProof: Get<u32> + 'static,
        MaxWardenId: Get<u32> + 'static,
    {
        fn clone(&self) -> Self {
            Self {
                shard_hash: self.shard_hash.clone(),
                miner_uid: self.miner_uid,
                result: self.result,
                challenge_seed: self.challenge_seed,
                block_number: self.block_number,
                timestamp: self.timestamp,
                warden_pubkey: self.warden_pubkey.clone(),
                signature: self.signature.clone(),
                merkle_proof_sig_hash: self.merkle_proof_sig_hash.clone(),
                warden_id: self.warden_id.clone(),
            }
        }
    }

    impl<MaxShardHash, MaxWardenPubkey, MaxSignature, MaxMerkleProof, MaxWardenId> PartialEq 
        for AttestationRecord<MaxShardHash, MaxWardenPubkey, MaxSignature, MaxMerkleProof, MaxWardenId>
    where
        MaxShardHash: Get<u32> + 'static,
        MaxWardenPubkey: Get<u32> + 'static,
        MaxSignature: Get<u32> + 'static,
        MaxMerkleProof: Get<u32> + 'static,
        MaxWardenId: Get<u32> + 'static,
    {
        fn eq(&self, other: &Self) -> bool {
            self.shard_hash == other.shard_hash &&
            self.miner_uid == other.miner_uid &&
            self.result == other.result &&
            self.challenge_seed == other.challenge_seed &&
            self.block_number == other.block_number &&
            self.timestamp == other.timestamp &&
            self.warden_pubkey == other.warden_pubkey &&
            self.signature == other.signature &&
            self.merkle_proof_sig_hash == other.merkle_proof_sig_hash &&
            self.warden_id == other.warden_id
        }
    }

    impl<MaxShardHash, MaxWardenPubkey, MaxSignature, MaxMerkleProof, MaxWardenId> Eq 
        for AttestationRecord<MaxShardHash, MaxWardenPubkey, MaxSignature, MaxMerkleProof, MaxWardenId>
    where
        MaxShardHash: Get<u32> + 'static,
        MaxWardenPubkey: Get<u32> + 'static,
        MaxSignature: Get<u32> + 'static,
        MaxMerkleProof: Get<u32> + 'static,
        MaxWardenId: Get<u32> + 'static,
    {}

    impl<MaxShardHash, MaxWardenPubkey, MaxSignature, MaxMerkleProof, MaxWardenId> sp_std::fmt::Debug
        for AttestationRecord<MaxShardHash, MaxWardenPubkey, MaxSignature, MaxMerkleProof, MaxWardenId>
    where
        MaxShardHash: Get<u32> + 'static,
        MaxWardenPubkey: Get<u32> + 'static,
        MaxSignature: Get<u32> + 'static,
        MaxMerkleProof: Get<u32> + 'static,
        MaxWardenId: Get<u32> + 'static,
    {
        fn fmt(&self, f: &mut sp_std::fmt::Formatter<'_>) -> sp_std::fmt::Result {
            f.debug_struct("AttestationRecord")
                .field("shard_hash", &self.shard_hash)
                .field("miner_uid", &self.miner_uid)
                .field("result", &self.result)
                .field("challenge_seed", &self.challenge_seed)
                .field("block_number", &self.block_number)
                .field("timestamp", &self.timestamp)
                .field("warden_pubkey", &self.warden_pubkey)
                .field("signature", &self.signature)
                .field("merkle_proof_sig_hash", &self.merkle_proof_sig_hash)
                .field("warden_id", &self.warden_id)
                .finish()
        }
    }

    /// Compact commitment for epoch attestation bundles stored on-chain.
    ///
    /// This enables third-party verification:
    /// 1. Query chain: `EpochAttestationCommitments[epoch]`
    /// 2. Download bundle from Arion: `GET /download/{arion_content_hash}`
    /// 3. Verify `BLAKE3(bundle_bytes) == arion_content_hash`
    /// 4. Recompute merkle roots and compare against commitment
    /// 5. Verify individual attestation signatures
    #[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
    #[scale_info(skip_type_params(MaxContentHash))]
    pub struct EpochAttestationCommitment<MaxContentHash>
    where
        MaxContentHash: Get<u32> + 'static,
    {
        /// Epoch this commitment covers
        pub epoch: u64,
        /// BLAKE3 hash of the SCALE-encoded AttestationBundle (for Arion retrieval)
        pub arion_content_hash: BoundedVec<u8, MaxContentHash>,
        /// Merkle root of all attestation leaves
        pub attestation_merkle_root: [u8; 32],
        /// Merkle root of unique warden public keys
        pub warden_pubkey_merkle_root: [u8; 32],
        /// Number of attestations in the bundle
        pub attestation_count: u32,
        /// Block number when commitment was submitted
        pub submitted_at_block: u64,
    }

    /// Warden registration status.
    #[derive(Clone, Copy, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
    pub enum WardenStatus {
        /// Actively registered and authorized to submit attestations
        Active,
        /// Deregistered, attestations from this warden will be rejected
        Deregistered,
    }

    /// Warden registration information.
    ///
    /// Tracks authorized wardens for third-party verification of attestation authorization.
    #[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
    pub struct WardenInfo<BlockNumber> {
        /// Current registration status
        pub status: WardenStatus,
        /// Block number when warden was registered
        pub registered_at: BlockNumber,
        /// Block number when warden was deregistered (if applicable)
        pub deregistered_at: Option<BlockNumber>,
    }

    /// Network-wide totals (validator-observed).
    #[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen, Default)]
    pub struct NetworkTotals {
        pub total_shards: u64,
        pub total_shard_data_bytes: u128,
        pub total_bandwidth_bytes: u128,
    }

    /// Validator-reported, non-cheatable per-node quality inputs used for **on-chain** weight computation.
    ///
    /// The pallet deterministically computes `node_weight_u16` from these inputs.
    #[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen, Default)]
    pub struct NodeQuality {
        /// Total stored bytes on this node (validator-observed).
        pub shard_data_bytes: u128,
        /// Served bandwidth bytes for this reporting window (validator-observed).
        pub bandwidth_bytes: u128,
        /// Availability in permille for this reporting window (0..1000).
        pub uptime_permille: u16,
        /// Strike count (or strike delta accumulated by validator).
        pub strikes: u32,
        /// Integrity failures (or delta accumulated by validator).
        pub integrity_fails: u32,
    }

    #[derive(Clone, Copy, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
    pub enum ChildStatus {
        /// Actively registered and eligible for inclusion in CRUSH maps.
        Active,
        /// Deregistered but deposit is still bonded; can be claimed after `unbonding_end`.
        Unbonding,
    }

    #[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
    pub struct ChildRegistration<AccountId, BlockNumber, Balance> {
        pub family: AccountId,
        pub node_id: [u8; 32],
        pub status: ChildStatus,
        /// Amount reserved from `family` for this child (0 for the free slot).
        pub deposit: Balance,
        /// If `status == Unbonding`, the earliest block when `claim_unbonded` can unreserve funds.
        pub unbonding_end: BlockNumber,
    }

    #[pallet::config]
    pub trait Config: frame_system::Config + pallet_proxy::Config + pallet_registration::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Sudo/admin origin for pallet parameters.
        type ArionAdminOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// Who is allowed to publish a new CRUSH map epoch.
        type MapAuthorityOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// Who is allowed to submit periodic miner stats aggregates.
        type StatsAuthorityOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// Who is allowed to submit warden attestations.
        type AttestationAuthorityOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// Currency used to reserve deposits for child registrations.
        type DepositCurrency: ReservableCurrency<Self::AccountId>;

        /// Family registry hook (TODO: wire to pallet).
        type FamilyRegistry: FamilyRegistry<Self::AccountId>;

        type ProxyVerifier: ProxyVerifier<Self::AccountId>;

        /// Enforce that miners in submitted CRUSH maps must be present in this pallet’s registry.
        /// Recommended: `true` once validator fully relies on on-chain registration.
        #[pallet::constant]
        type EnforceRegisteredMinersInMap: Get<bool>;

        /// Max number of miners allowed in a single epoch map.
        #[pallet::constant]
        type MaxMiners: Get<u32>;

        /// Max bytes for `endpoint`.
        #[pallet::constant]
        type MaxEndpointLen: Get<u32>;

        /// Max bytes for `http_addr`.
        #[pallet::constant]
        type MaxHttpAddrLen: Get<u32>;

        /// Max number of miner stats updates per submission.
        #[pallet::constant]
        type MaxStatsUpdates: Get<u32>;

        /// Max number of attestations per submission.
        #[pallet::constant]
        type MaxAttestations: Get<u32>;

        /// Max bytes for shard hash in attestation.
        #[pallet::constant]
        type MaxShardHashLen: Get<u32>;

        /// Max bytes for warden pubkey in attestation.
        #[pallet::constant]
        type MaxWardenPubkeyLen: Get<u32>;

        /// Max bytes for signature in attestation.
        #[pallet::constant]
        type MaxSignatureLen: Get<u32>;

        /// Max bytes for Merkle proof signature hash in attestation.
        #[pallet::constant]
        type MaxMerkleProofLen: Get<u32>;

        /// Max bytes for warden ID in attestation.
        #[pallet::constant]
        type MaxWardenIdLen: Get<u32>;

        /// Max bytes for content hash in attestation commitment (BLAKE3 = 32 bytes).
        #[pallet::constant]
        type MaxContentHashLen: Get<u32>;

        /// Number of attestation buckets to retain before pruning.
        /// Older buckets can be pruned to prevent unbounded storage growth.
        /// Default recommendation: 1000 buckets (~7 days at 300 blocks/bucket, 6s blocks)
        #[pallet::constant]
        type AttestationRetentionBuckets: Get<u32>;

        // --- Registration economics / safety controls ---

        /// Max distinct families that can ever claim their first “free” child slot.
        /// (You mentioned 256).
        #[pallet::constant]
        type MaxFamilies: Get<u32>;

        /// Max total number of active children across all families.
        /// (You mentioned ~2K).
        #[pallet::constant]
        type MaxChildrenTotal: Get<u32>;

        /// Max number of active children within one family.
        #[pallet::constant]
        type MaxChildrenPerFamily: Get<u32>;

        /// Base deposit used to initialize / floor the global deposit curve.
        ///
        /// NOTE: This acts as a default. The actual runtime-configurable value is stored
        /// in `BaseChildDepositValue` and can be changed via the sudo/admin extrinsic.
        #[pallet::constant]
        type BaseChildDeposit: Get<BalanceOf<Self>>;

        /// Number of blocks after which the global next deposit halves (lazy decay).
        /// Use ~24h: 24*60*60/6 = 14_400 blocks at 6s.
        #[pallet::constant]
        type GlobalDepositHalvingPeriodBlocks: Get<BlockNumberFor<Self>>;

        /// Cooldown after deregistration: child/node cannot be re-registered until this passes.
        #[pallet::constant]
        type UnregisterCooldownBlocks: Get<BlockNumberFor<Self>>;

        /// Unbonding period: deposit stays reserved until this passes, then `claim_unbonded` releases it.
        #[pallet::constant]
        type UnbondingPeriodBlocks: Get<BlockNumberFor<Self>>;

        // --- Weight / incentives (validator-reported) ---

        /// Who is allowed to submit weight updates (typically the validator set / a council origin).
        type WeightAuthorityOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// Max number of node-weight updates per submission.
        #[pallet::constant]
        type MaxNodeWeightUpdates: Get<u32>;

        /// Max per-node weight (caps what can be stored / counted).
        #[pallet::constant]
        type MaxNodeWeight: Get<u16>;

        /// Max per-family weight (caps what can be stored / submitted to external systems like Bittensor).
        #[pallet::constant]
        type MaxFamilyWeight: Get<u16>;

        /// How many top node weights to count per family (anti “just add infinite children”).
        #[pallet::constant]
        type FamilyTopN: Get<u32>;

        /// Decay factor per rank (permille). Example: 800 → each next node contributes 0.8x the previous.
        #[pallet::constant]
        type FamilyRankDecayPermille: Get<u32>;

        /// EMA alpha (permille) for smoothing family weight over time.
        /// Example: 300 → 30% new, 70% previous.
        #[pallet::constant]
        type FamilyWeightEmaAlphaPermille: Get<u32>;

        /// Max change allowed per bucket for family weights (safety against transient spikes).
        #[pallet::constant]
        type MaxFamilyWeightDeltaPerBucket: Get<u16>;

        /// Newcomer grace buckets: apply a small floor (only if computed weight > 0) for early buckets.
        #[pallet::constant]
        type NewcomerGraceBuckets: Get<u32>;

        /// Newcomer floor weight (applied during grace window if computed weight > 0).
        #[pallet::constant]
        type NewcomerFloorWeight: Get<u16>;

        // --- Deterministic on-chain node weight computation ---

        /// Bandwidth contribution weight (permille). Recommended: 700..900.
        #[pallet::constant]
        type NodeBandwidthWeightPermille: Get<u32>;

        /// Storage (stored bytes) contribution weight (permille). Recommended: 100..300.
        #[pallet::constant]
        type NodeStorageWeightPermille: Get<u32>;

        /// Scale applied after combining log-scores to map into u16 range. Recommended: 512.
        #[pallet::constant]
        type NodeScoreScale: Get<u16>;

        /// Strike penalty per strike (in u16 weight units).
        #[pallet::constant]
        type StrikePenalty: Get<u16>;

        /// Integrity failure penalty per fail (in u16 weight units).
        #[pallet::constant]
        type IntegrityFailPenalty: Get<u16>;

        /// Weight information for extrinsics in this pallet.
        type WeightInfo: crate::weights::WeightInfo;
    }

	#[pallet::pallet]
	#[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    /// Current CRUSH epoch.
    #[pallet::storage]
    pub type CurrentEpoch<T> = StorageValue<_, u64, ValueQuery>;

    /// CRUSH params per epoch.
    #[pallet::storage]
    pub type EpochParams<T> = StorageMap<_, Blake2_128Concat, u64, CrushParams, OptionQuery>;

    /// Miners for a given epoch (bounded).
    #[pallet::storage]
    pub type EpochMiners<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        u64,
        BoundedVec<
            MinerRecord<T::AccountId, T::MaxEndpointLen, T::MaxHttpAddrLen>,
            T::MaxMiners,
        >,
        OptionQuery,
    >;

    /// Root hash (commitment) of the canonical epoch map encoding (miners sorted by uid).
    #[pallet::storage]
    pub type EpochRoot<T: Config> = StorageMap<_, Blake2_128Concat, u64, H256, OptionQuery>;

    /// Latest stats bucket id (e.g. block_number / N).
    #[pallet::storage]
    pub type CurrentStatsBucket<T> = StorageValue<_, u32, ValueQuery>;

    /// Latest network totals for `CurrentStatsBucket`.
    #[pallet::storage]
    pub type CurrentNetworkTotals<T> = StorageValue<_, NetworkTotals, ValueQuery>;

    /// Stats by miner uid (latest).
    #[pallet::storage]
    pub type MinerStatsByUid<T> = StorageMap<_, Blake2_128Concat, u32, MinerStats, OptionQuery>;

    // -------------------------
    // Attestation state
    // -------------------------

    /// Current attestation bucket (monotonic).
    #[pallet::storage]
    pub type CurrentAttestationBucket<T> = StorageValue<_, u32, ValueQuery>;

    /// Attestations by bucket (bounded list of attestation records).
    #[pallet::storage]
    pub type AttestationsByBucket<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        u32,
        BoundedVec<
            AttestationRecord<T::MaxShardHashLen, T::MaxWardenPubkeyLen, T::MaxSignatureLen, T::MaxMerkleProofLen, T::MaxWardenIdLen>,
            T::MaxAttestations,
        >,
        ValueQuery,
    >;

    /// Epoch attestation commitments for third-party verification.
    ///
    /// Maps epoch → commitment containing merkle roots and Arion content hash.
    /// The full attestation bundle can be retrieved from Arion using the content hash.
    #[pallet::storage]
    pub type EpochAttestationCommitments<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        u64,
        EpochAttestationCommitment<T::MaxContentHashLen>,
        OptionQuery,
    >;

    /// Registered wardens authorized to submit attestations.
    ///
    /// Maps warden Ed25519 public key (32 bytes) → registration info.
    /// This enables third-party verification of warden authorization.
    #[pallet::storage]
    pub type RegisteredWardens<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        [u8; 32],
        WardenInfo<BlockNumberFor<T>>,
        OptionQuery,
    >;

    /// Total count of active (not deregistered) wardens.
    #[pallet::storage]
    pub type ActiveWardenCount<T> = StorageValue<_, u32, ValueQuery>;

    // -------------------------
    // Registration state
    // -------------------------

    /// Whether registration lockup (reserve/unbond) is enabled.
    ///
    /// If disabled:
    /// - `register_child` does not reserve deposits (deposit = 0)
    /// - global fee curve does not advance
    /// - `deregister_child` unbonding becomes immediate (still enforces cooldown)
    #[pallet::storage]
    pub type LockupEnabled<T> = StorageValue<_, bool, ValueQuery>;

    /// Runtime-configurable base deposit (floor for the global fee curve).
    #[pallet::storage]
    pub type BaseChildDepositValue<T: Config> = StorageValue<_, BalanceOf<T>, ValueQuery>;

    /// Number of families that have claimed the “first child free” slot.
    #[pallet::storage]
    pub type FamilyCount<T> = StorageValue<_, u32, ValueQuery>;

    /// Whether a family has already used its one-time free registration.
    #[pallet::storage]
    pub type FamilyUsedFreeSlot<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, bool, ValueQuery>;

    /// Active children count per family.
    #[pallet::storage]
    pub type FamilyActiveChildren<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

    /// Total active children across all families.
    #[pallet::storage]
    pub type TotalActiveChildren<T> = StorageValue<_, u32, ValueQuery>;

    /// Global next required deposit for paid child registration (network-wide adaptive fee).
    #[pallet::storage]
    pub type GlobalNextDeposit<T: Config> = StorageValue<_, BalanceOf<T>, ValueQuery>;

    /// Last block when a paid child registration happened (for lazy halving).
    #[pallet::storage]
    pub type GlobalLastPaidRegistrationBlock<T: Config> =
        StorageValue<_, BlockNumberFor<T>, ValueQuery>;

    /// Child registration record (only while Active/Unbonding).
    #[pallet::storage]
    pub type ChildRegistrations<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        ChildRegistration<T::AccountId, BlockNumberFor<T>, BalanceOf<T>>,
        OptionQuery,
    >;

    /// Node id -> current child (active only).
    #[pallet::storage]
    pub type NodeIdToChild<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 32], T::AccountId, OptionQuery>;

    /// Prevent replay across (de)registration cycles: nonce per node id.
    #[pallet::storage]
    pub type NodeIdNonce<T: Config> = StorageMap<_, Blake2_128Concat, [u8; 32], u64, ValueQuery>;

    /// Cooldown until (block number) for child accounts (after deregistration).
    #[pallet::storage]
    pub type ChildCooldownUntil<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, BlockNumberFor<T>, ValueQuery>;

    /// Cooldown until for node ids (after deregistration).
    #[pallet::storage]
    pub type NodeIdCooldownUntil<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 32], BlockNumberFor<T>, ValueQuery>;

    /// Active children list per family (needed to aggregate per-family weights safely).
    #[pallet::storage]
    pub type FamilyChildren<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        BoundedVec<T::AccountId, T::MaxChildrenPerFamily>,
        ValueQuery,
    >;

    #[pallet::genesis_config]
    #[derive(frame_support::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        pub base_child_deposit: Option<BalanceOf<T>>,
        pub lockup_enabled: bool,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            if let Some(deposit) = &self.base_child_deposit {
                BaseChildDepositValue::<T>::put(deposit);
            }
            LockupEnabled::<T>::put(self.lockup_enabled);
        }
    }

    // -------------------------
    // Incentive weights (validator-reported)
    // -------------------------

    /// Current weight bucket (monotonic).
    #[pallet::storage]
    pub type CurrentWeightBucket<T> = StorageValue<_, u32, ValueQuery>;

    /// Latest per-node (child) weight.
    #[pallet::storage]
    pub type NodeWeightByChild<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u16, ValueQuery>;

    /// Last bucket when a node (child) weight was updated.
    #[pallet::storage]
    pub type NodeWeightLastBucket<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

    /// Latest validator-reported quality inputs per node (child).
    #[pallet::storage]
    pub type NodeQualityByChild<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, NodeQuality, OptionQuery>;

    /// Latest raw (unsmoothed) family weight (derived from node weights).
    #[pallet::storage]
    pub type FamilyWeightRaw<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u16, ValueQuery>;

    /// Latest smoothed family weight (EMA + delta clamp).
    #[pallet::storage]
    pub type FamilyWeight<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u16, ValueQuery>;

    /// First bucket a family became active (for newcomer grace).
    #[pallet::storage]
    pub type FamilyFirstSeenBucket<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u32, OptionQuery>;

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        /// A new CRUSH epoch was published.
        CrushMapPublished { epoch: u64, miners: u32, root: H256 },
        /// Miner stats were updated for a bucket.
        MinerStatsUpdated { bucket: u32, updates: u32 },
        /// Attestations were submitted for a bucket.
        AttestationsSubmitted { bucket: u32, count: u32 },
        /// Attestation commitment was submitted for an epoch.
        AttestationCommitmentSubmitted {
            epoch: u64,
            attestation_count: u32,
            attestation_merkle_root: [u8; 32],
            warden_pubkey_merkle_root: [u8; 32],
        },
        /// A child node was registered under a family.
        ChildRegistered {
            family: T::AccountId,
            child: T::AccountId,
            node_id: [u8; 32],
            deposit: BalanceOf<T>,
        },
        /// A child node was deregistered and entered unbonding.
        ChildDeregistered {
            family: T::AccountId,
            child: T::AccountId,
            node_id: [u8; 32],
            unbonding_end: BlockNumberFor<T>,
            cooldown_end: BlockNumberFor<T>,
        },
        /// A child’s deposit was unbonded and released.
        ChildUnbonded {
            family: T::AccountId,
            child: T::AccountId,
            node_id: [u8; 32],
            amount: BalanceOf<T>,
        },
        /// Node weights were updated for a bucket.
        NodeWeightsUpdated { bucket: u32, updates: u32 },
        /// Family weights were recomputed for a bucket.
        FamilyWeightsComputed { bucket: u32, families: u32 },
        /// Registration lockup was enabled/disabled by admin.
        LockupEnabledSet { enabled: bool },
        /// Base child deposit floor was set by admin.
        BaseChildDepositSet { deposit: BalanceOf<T> },
        /// A warden was registered and authorized to submit attestations.
        WardenRegistered {
            warden_pubkey: [u8; 32],
            registered_at: BlockNumberFor<T>,
        },
        /// A warden was deregistered and can no longer submit attestations.
        WardenDeregistered {
            warden_pubkey: [u8; 32],
            deregistered_at: BlockNumberFor<T>,
        },
        /// Old attestation buckets were pruned.
        AttestationBucketsPruned {
            pruned_count: u32,
            oldest_remaining: u32,
        },
    }

    #[pallet::error]
    pub enum Error<T> {
        /// Epoch is not strictly increasing.
        EpochRegression,
        /// Epoch already exists.
        EpochAlreadyExists,
        /// Miner list must be sorted by uid and unique.
        MinerListNotSortedOrNotUnique,
        /// Too many miners.
        TooManyMiners,
        /// Too many stats updates in one call.
        TooManyStatsUpdates,
        /// Stats bucket regression.
        StatsBucketRegression,
        /// Family is not registered (per `FamilyRegistry` hook).
        FamilyNotRegistered,
        /// Proxy verification failed (per `ProxyVerifier` hook).
        ProxyVerificationFailed,
        /// Too many families.
        TooManyFamilies,
        /// Too many active children total.
        TooManyChildrenTotal,
        /// Too many active children in this family.
        TooManyChildrenInFamily,
        /// Child is already registered.
        ChildAlreadyRegistered,
        /// Child is not registered.
        ChildNotRegistered,
        /// Child is in cooldown.
        ChildInCooldown,
        /// Node id is already registered.
        NodeIdAlreadyRegistered,
        /// Node id is in cooldown.
        NodeIdInCooldown,
        /// Invalid node signature.
        InvalidNodeSignature,
        /// Child is not currently active (cannot be deregistered).
        ChildNotActive,
        /// Child is not in unbonding state.
        NotUnbonding,
        /// Unbonding not finished yet.
        UnbondingNotReady,
        /// Failed to reserve required deposit.
        InsufficientDeposit,
        /// CRUSH map includes a miner that is not registered (when enforcement is enabled).
        MinerNotRegistered,
        /// Weight bucket regression.
        WeightBucketRegression,
        /// Too many node weight updates.
        TooManyNodeWeightUpdates,
        /// Attestation bucket regression.
        AttestationBucketRegression,
        /// Too many attestations in one call.
        TooManyAttestations,
        /// Attestation list is full for this bucket.
        AttestationBucketFull,
        /// Invalid attestation signature.
        InvalidAttestationSignature,
        /// Attestation commitment already exists for this epoch.
        AttestationCommitmentAlreadyExists,
        /// Invalid content hash length (expected 32 bytes for BLAKE3).
        InvalidContentHashLength,
        /// Warden is already registered.
        WardenAlreadyRegistered,
        /// Warden is not registered.
        WardenNotRegistered,
        /// Attestation submitted by unregistered warden.
        UnregisteredWarden,
        /// Cannot prune buckets within retention period.
        PruningWithinRetentionPeriod,
    }

    impl<T: Config> Pallet<T> {
        fn now() -> BlockNumberFor<T> {
            <frame_system::Pallet<T>>::block_number()
        }

        fn base_child_deposit() -> BalanceOf<T> {
            let cur = BaseChildDepositValue::<T>::get();
            if cur.is_zero() {
                let d = T::BaseChildDeposit::get();
                BaseChildDepositValue::<T>::put(d);
                d
            } else {
                cur
            }
        }

        fn global_next_deposit_floor_init() -> BalanceOf<T> {
            let base = Self::base_child_deposit();
            let cur = GlobalNextDeposit::<T>::get();
            if cur.is_zero() {
                GlobalNextDeposit::<T>::put(base);
                base
            } else if cur < base {
                GlobalNextDeposit::<T>::put(base);
                base
            } else {
                cur
            }
        }

        /// Lazily halves the global next deposit based on time since last paid registration.
        /// Never goes below `BaseChildDeposit`.
        fn apply_lazy_global_halving() -> BalanceOf<T> {
            let base = Self::base_child_deposit();
            let mut next = Self::global_next_deposit_floor_init();
            let period = T::GlobalDepositHalvingPeriodBlocks::get();
            if period.is_zero() {
                return next;
            }

            let last = GlobalLastPaidRegistrationBlock::<T>::get();
            let elapsed = Self::now().saturating_sub(last);

            // Compute number of full periods elapsed (best-effort conversion).
            let periods: u32 = {
                let e: u128 = TryInto::<u128>::try_into(elapsed).ok().unwrap_or(0);
                let p: u128 = TryInto::<u128>::try_into(period).ok().unwrap_or(0);
                if p == 0 {
                    0
                } else {
                    (e / p) as u32
                }
            };

            for _ in 0..periods {
                next /= 2u32.into();
                if next < base {
                    next = base;
                    break;
                }
            }
            GlobalNextDeposit::<T>::put(next);
            next
        }

        fn registration_message(
            family: &T::AccountId,
            child: &T::AccountId,
            node_id: &[u8; 32],
            nonce: u64,
        ) -> Vec<u8> {
            // Domain-separated payload: SCALE("ARION_NODE_REG_V1", family, child, node_id, nonce)
            (b"ARION_NODE_REG_V1", family, child, node_id, nonce).encode()
        }

        fn verify_node_sig(node_id: [u8; 32], message: &[u8], sig: [u8; 64]) -> bool {
            let pk = ed25519::Public::from_raw(node_id);
            let sig = ed25519::Signature::from_raw(sig);
            sig.verify(message, &pk)
        }

        /// Verify attestation signature.
        ///
        /// Constructs the canonical signing message from attestation fields and verifies
        /// the Ed25519 signature using the warden's public key.
        fn verify_attestation_sig(
            attestation: &AttestationRecord<T::MaxShardHashLen, T::MaxWardenPubkeyLen, T::MaxSignatureLen, T::MaxMerkleProofLen, T::MaxWardenIdLen>,
        ) -> bool {
            // Ensure warden_pubkey is exactly 32 bytes (Ed25519)
            let pubkey_bytes: [u8; 32] = match attestation.warden_pubkey.as_slice().try_into() {
                Ok(b) => b,
                Err(_) => return false,
            };

            // Ensure signature is exactly 64 bytes (Ed25519)
            let sig_bytes: [u8; 64] = match attestation.signature.as_slice().try_into() {
                Ok(b) => b,
                Err(_) => return false,
            };

            // Construct canonical message for signature verification
            // Domain-separated payload: SCALE(domain_sep, fields...)
            // IMPORTANT: Uses ATTESTATION_DOMAIN_SEPARATOR (&[u8] slice) which SCALE-encodes
            // with a length prefix, matching off-chain warden/validator encoding.
            let message = (
                ATTESTATION_DOMAIN_SEPARATOR,
                attestation.shard_hash.as_slice(),
                attestation.miner_uid,
                attestation.result.as_u8(),
                attestation.challenge_seed,
                attestation.block_number,
                attestation.timestamp,
                attestation.merkle_proof_sig_hash.as_slice(),
                attestation.warden_id.as_slice(),
            ).encode();

            // Verify Ed25519 signature
            let pk = ed25519::Public::from_raw(pubkey_bytes);
            let sig = ed25519::Signature::from_raw(sig_bytes);
            sig.verify(&message[..], &pk)
        }

        fn ensure_miner_records_registered(
            miners: &BoundedVec<
                MinerRecord<T::AccountId, T::MaxEndpointLen, T::MaxHttpAddrLen>,
                T::MaxMiners,
            >,
        ) -> DispatchResult {
            if !T::EnforceRegisteredMinersInMap::get() {
                return Ok(());
            }
            for m in miners.iter() {
                let child = NodeIdToChild::<T>::get(m.node_id).ok_or(Error::<T>::MinerNotRegistered)?;
                let reg =
                    ChildRegistrations::<T>::get(&child).ok_or(Error::<T>::MinerNotRegistered)?;
                ensure!(reg.status == ChildStatus::Active, Error::<T>::MinerNotRegistered);
                ensure!(reg.family == m.family_id, Error::<T>::MinerNotRegistered);
            }
            Ok(())
        }

        fn clamp_u16(x: u16, lo: u16, hi: u16) -> u16 {
            if x < lo {
                lo
            } else if x > hi {
                hi
            } else {
                x
            }
        }

        fn ema_permille_u16(prev: u16, next: u16, alpha_permille: u32) -> u16 {
            let a = alpha_permille.min(1000) as u128;
            let inv = (1000u128).saturating_sub(a);
            let v: u128 = (prev as u128)
                .saturating_mul(inv)
                .saturating_add((next as u128).saturating_mul(a))
                / 1000u128;
            v.min(u16::MAX as u128) as u16
        }

        fn compute_family_weight_from_nodes(family: &T::AccountId) -> u16 {
            let max_node = T::MaxNodeWeight::get();
            let max_family = T::MaxFamilyWeight::get();
            let top_n = T::FamilyTopN::get().max(1) as usize;
            let decay = T::FamilyRankDecayPermille::get().min(1000);

            let children = FamilyChildren::<T>::get(family);
            if children.is_empty() {
                return 0;
            }

            // Collect node weights for this family.
            let mut ws: sp_std::vec::Vec<u16> = sp_std::vec::Vec::with_capacity(children.len());
            for c in children.iter() {
                let w = NodeWeightByChild::<T>::get(c).min(max_node);
                if w > 0 {
                    ws.push(w);
                }
            }
            if ws.is_empty() {
                return 0;
            }
            ws.sort_unstable_by(|a, b| b.cmp(a)); // desc

            // Top-N with rank decay: w0 + w1*decay + w2*decay^2 ...
            let mut factor_permille: u128 = 1000;
            let mut sum: u128 = 0;
            for (i, w) in ws.iter().take(top_n).enumerate() {
                if i == 0 {
                    // factor=1000 for first term
                } else {
                    factor_permille = factor_permille.saturating_mul(decay as u128) / 1000u128;
                }
                let term = (*w as u128).saturating_mul(factor_permille) / 1000u128;
                sum = sum.saturating_add(term);
                if sum >= max_family as u128 {
                    return max_family;
                }
            }
            sum.min(max_family as u128) as u16
        }

        pub fn get_total_family_weight() -> u128 {
            FamilyWeight::<T>::iter()
                .map(|(_, w)| w as u128)
                .sum()
        }

        fn ilog2_u128(x: u128) -> u32 {
            if x == 0 {
                0
            } else {
                127u32.saturating_sub(x.leading_zeros())
            }
        }

        /// Deterministically compute a `u16` node weight from validator-reported metrics.
        ///
        /// Uses concave `log2(1+x)` scoring to avoid “rich get richer” runaway.
        fn compute_node_weight_from_quality(q: &NodeQuality) -> u16 {
            // Concave scores in [0..127]
            let bw_s = Self::ilog2_u128(q.bandwidth_bytes.saturating_add(1));
            let st_s = Self::ilog2_u128(q.shard_data_bytes.saturating_add(1));

            // Weighted blend in [0..127] (approx), using permille weights.
            let bw_w = T::NodeBandwidthWeightPermille::get().min(1000) as u128;
            let st_w = T::NodeStorageWeightPermille::get().min(1000) as u128;
            let denom = (bw_w + st_w).max(1);
            let blended = ((bw_s as u128).saturating_mul(bw_w)
                .saturating_add((st_s as u128).saturating_mul(st_w)))
                / denom; // 0..127-ish

            // Scale to u16 range.
            let mut score: u128 = blended.saturating_mul(T::NodeScoreScale::get() as u128);
            if score > u16::MAX as u128 {
                score = u16::MAX as u128;
            }
            let mut score_u16 = score as u16;

            // Uptime multiplier (permille)
            let up = q.uptime_permille.min(1000) as u128;
            score_u16 = (((score_u16 as u128).saturating_mul(up)) / 1000u128)
                .min(u16::MAX as u128) as u16;

            // Penalties
            let strike_pen = (q.strikes as u128)
                .saturating_mul(T::StrikePenalty::get() as u128);
            let integ_pen = (q.integrity_fails as u128)
                .saturating_mul(T::IntegrityFailPenalty::get() as u128);
            let pen = strike_pen.saturating_add(integ_pen).min(u16::MAX as u128) as u16;
            score_u16.saturating_sub(pen)
        }

        fn apply_node_weights_and_recompute(
            bucket: u32,
            updates: &BoundedVec<(T::AccountId, u16), T::MaxNodeWeightUpdates>,
        ) -> DispatchResult {
            // Track which families are impacted so we only recompute those.
            let mut touched_families: sp_std::vec::Vec<T::AccountId> = sp_std::vec::Vec::new();

            for (child, w) in updates.iter() {
                let reg =
                    ChildRegistrations::<T>::get(child).ok_or(Error::<T>::ChildNotRegistered)?;
                ensure!(reg.status == ChildStatus::Active, Error::<T>::ChildNotActive);

                let capped = (*w).min(T::MaxNodeWeight::get());
                NodeWeightByChild::<T>::insert(child, capped);
                NodeWeightLastBucket::<T>::insert(child, bucket);

                if !touched_families.iter().any(|f| f == &reg.family) {
                    touched_families.push(reg.family);
                }
            }

            // Bucket advances once per call (monotonic), so newcomers are measured relative to that.
            CurrentWeightBucket::<T>::put(bucket);
            Self::deposit_event(Event::NodeWeightsUpdated {
                bucket,
                updates: updates.len() as u32,
            });

            // Recompute family weights for touched families.
            let mut computed: u32 = 0;
            for family in touched_families.iter() {
                let raw = Self::compute_family_weight_from_nodes(family);

                // Newcomer floor, only if raw > 0 and within grace
                let raw = if raw > 0 {
                    if let Some(first) = FamilyFirstSeenBucket::<T>::get(family) {
                        let age = bucket.saturating_sub(first);
                        if age <= T::NewcomerGraceBuckets::get() {
                            raw.max(T::NewcomerFloorWeight::get().min(T::MaxFamilyWeight::get()))
                        } else {
                            raw
                        }
                    } else {
                        raw
                    }
                } else {
                    0
                };

                FamilyWeightRaw::<T>::insert(family, raw);

                // Smooth + clamp delta
                let prev = FamilyWeight::<T>::get(family);
                let ema =
                    Self::ema_permille_u16(prev, raw, T::FamilyWeightEmaAlphaPermille::get());
                let d = T::MaxFamilyWeightDeltaPerBucket::get();
                let lo = prev.saturating_sub(d);
                let hi = prev.saturating_add(d).min(T::MaxFamilyWeight::get());
                let final_w = Self::clamp_u16(ema, lo, hi).min(T::MaxFamilyWeight::get());
                FamilyWeight::<T>::insert(family, final_w);
                computed = computed.saturating_add(1);
            }

            Self::deposit_event(Event::FamilyWeightsComputed {
                bucket,
                families: computed,
            });
            Ok(())
        }
    }

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        /// Publish a new CRUSH map for a specific epoch.
        ///
        /// Expected usage:
        /// - Called only when epoch changes.
        /// - Miner list MUST be sorted by `uid` ascending and have unique uids.
        ///
        /// Stores:
        /// - `EpochParams[epoch]`
        /// - `EpochMiners[epoch]`
        /// - `EpochRoot[epoch]` (hash of canonical SCALE encoding)
        /// Updates:
        /// - `CurrentEpoch`
        #[pallet::call_index(0)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::submit_crush_map(miners.len() as u32), Pays::No))]
        pub fn submit_crush_map(
            origin: OriginFor<T>,
            epoch: u64,
            params: CrushParams,
            miners: BoundedVec<
                MinerRecord<T::AccountId, T::MaxEndpointLen, T::MaxHttpAddrLen>,
                T::MaxMiners,
            >,
        ) -> DispatchResult {
            T::MapAuthorityOrigin::ensure_origin(origin)?;

            let current = CurrentEpoch::<T>::get();
            ensure!(epoch > current, Error::<T>::EpochRegression);
            ensure!(!EpochParams::<T>::contains_key(epoch), Error::<T>::EpochAlreadyExists);

            // Ensure sorted + unique by uid
            let mut last_uid: Option<u32> = None;
            for m in miners.iter() {
                if let Some(prev) = last_uid {
                    ensure!(m.uid > prev, Error::<T>::MinerListNotSortedOrNotUnique);
                }
                last_uid = Some(m.uid);
            }

            // Optional hardening: ensure miners are registered (validator can enable once ready).
            Self::ensure_miner_records_registered(&miners)?;

            // Commitment: hash SCALE encoding of (params, miners)
            let mut bytes = params.encode();
            bytes.extend_from_slice(&miners.encode());
            let root = T::Hashing::hash(&bytes);
            let root_h256 = H256::from_slice(root.as_ref());

            EpochParams::<T>::insert(epoch, params);
            EpochMiners::<T>::insert(epoch, miners.clone());
            EpochRoot::<T>::insert(epoch, root_h256);
            CurrentEpoch::<T>::put(epoch);

            Self::deposit_event(Event::CrushMapPublished {
                epoch,
                miners: miners.len() as u32,
                root: root_h256,
            });
            Ok(())
        }

        /// Submit aggregated miner stats updates for the current reporting bucket.
        ///
        /// Suggested: call every N blocks (e.g. 300) with aggregates.
        #[pallet::call_index(1)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::submit_miner_stats(updates.len() as u32), Pays::No))]
        pub fn submit_miner_stats(
            origin: OriginFor<T>,
            bucket: u32,
            updates: BoundedVec<MinerStatsUpdate, T::MaxStatsUpdates>,
            network_totals: Option<NetworkTotals>,
        ) -> DispatchResult {
            T::StatsAuthorityOrigin::ensure_origin(origin)?;

            let cur = CurrentStatsBucket::<T>::get();
            ensure!(bucket >= cur, Error::<T>::StatsBucketRegression);

            for u in updates.iter() {
                MinerStatsByUid::<T>::insert(u.uid, u.stats.clone());
            }
            CurrentStatsBucket::<T>::put(bucket);
            if let Some(t) = network_totals {
                CurrentNetworkTotals::<T>::put(t);
            }

            Self::deposit_event(Event::MinerStatsUpdated {
                bucket,
                updates: updates.len() as u32,
            });
            Ok(())
        }

        /// Register a child node under a family.
        ///
        /// - **First child free per family (one-time)**.
        /// - After that, the required deposit is **global** (network-wide) and **doubles** after each paid registration.
        /// - Global deposit **halves** after each `GlobalDepositHalvingPeriodBlocks` of inactivity (lazy, computed on registration).
        /// - Requires `node_id` (ed25519 pubkey) to sign a domain-separated payload including a per-node nonce.
        ///
        /// Signature payload (domain-separated, SCALE-encoded):
        /// - ("ARION_NODE_REG_V1", family, child, node_id, nonce)
        #[pallet::call_index(10)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::register_child(), Pays::No))]
        pub fn register_child(
            origin: OriginFor<T>,
            family: T::AccountId,
            child: T::AccountId,
            node_id: [u8; 32],
            node_sig: [u8; 64],
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(who == family, DispatchError::BadOrigin);

            ensure!(
                T::FamilyRegistry::is_registered_family(&family),
                Error::<T>::FamilyNotRegistered
            );
            ensure!(
                T::ProxyVerifier::can_register_child(&family, &child),
                Error::<T>::ProxyVerificationFailed
            );

            // Cooldowns
            let now = Self::now();
            let child_cd = ChildCooldownUntil::<T>::get(&child);
            ensure!(now >= child_cd, Error::<T>::ChildInCooldown);
            let node_cd = NodeIdCooldownUntil::<T>::get(node_id);
            ensure!(now >= node_cd, Error::<T>::NodeIdInCooldown);

            // Uniqueness
            ensure!(
                !ChildRegistrations::<T>::contains_key(&child),
                Error::<T>::ChildAlreadyRegistered
            );
            ensure!(
                !NodeIdToChild::<T>::contains_key(node_id),
                Error::<T>::NodeIdAlreadyRegistered
            );

            // Caps
            let total = TotalActiveChildren::<T>::get();
            ensure!(total < T::MaxChildrenTotal::get(), Error::<T>::TooManyChildrenTotal);
            let fam_count = FamilyActiveChildren::<T>::get(&family);
            ensure!(
                fam_count < T::MaxChildrenPerFamily::get(),
                Error::<T>::TooManyChildrenInFamily
            );

            // Enforce MaxFamilies the moment a family claims the free slot for the first time.
            let used_free = FamilyUsedFreeSlot::<T>::get(&family);
            if !used_free {
                let families = FamilyCount::<T>::get();
                ensure!(families < T::MaxFamilies::get(), Error::<T>::TooManyFamilies);
            }

            // Verify node signature
            let nonce = NodeIdNonce::<T>::get(node_id);
            let msg = Self::registration_message(&family, &child, &node_id, nonce);
            ensure!(
                Self::verify_node_sig(node_id, &msg, node_sig),
                Error::<T>::InvalidNodeSignature
            );

            // Determine deposit requirement
            let lockup_enabled = LockupEnabled::<T>::get();
            let deposit = if !lockup_enabled {
                BalanceOf::<T>::zero()
            } else if !used_free {
                BalanceOf::<T>::zero()
            } else {
                Self::apply_lazy_global_halving()
            };

            if lockup_enabled && !deposit.is_zero() {
                T::DepositCurrency::reserve(&family, deposit).map_err(|_| Error::<T>::InsufficientDeposit)?;
            }

            // Persist registration
            ChildRegistrations::<T>::insert(
                &child,
                ChildRegistration {
                    family: family.clone(),
                    node_id,
                    status: ChildStatus::Active,
                    deposit,
                    unbonding_end: now, // unused unless Unbonding
                },
            );
            NodeIdToChild::<T>::insert(node_id, &child);
            NodeIdNonce::<T>::insert(node_id, nonce.saturating_add(1));

            // Update counts
            TotalActiveChildren::<T>::put(total.saturating_add(1));
            FamilyActiveChildren::<T>::insert(&family, fam_count.saturating_add(1));
            // Maintain family children index
            FamilyChildren::<T>::try_mutate(&family, |v| v.try_push(child.clone()))
                .map_err(|_| Error::<T>::TooManyChildrenInFamily)?;

            // Set first-seen bucket (for newcomer grace) when family becomes active.
            if FamilyFirstSeenBucket::<T>::get(&family).is_none() {
                FamilyFirstSeenBucket::<T>::insert(&family, CurrentWeightBucket::<T>::get());
            }

            // Mark free slot as used and bump family count
            if !used_free {
                FamilyUsedFreeSlot::<T>::insert(&family, true);
                FamilyCount::<T>::put(FamilyCount::<T>::get().saturating_add(1));
            } else if lockup_enabled {
                // Paid registration: update global fee curve and timestamp
                let next = Self::global_next_deposit_floor_init();
                GlobalNextDeposit::<T>::put(next.saturating_add(next));
                GlobalLastPaidRegistrationBlock::<T>::put(now);
            }

            Self::deposit_event(Event::ChildRegistered {
                family,
                child,
                node_id,
                deposit,
            });
            Ok(())
        }

        /// Deregister a child node.
        ///
        /// Effects:
        /// - Child becomes `Unbonding`, removed from active counts
        /// - Node id is released from the active registry, but put in cooldown
        /// - Deposit remains reserved until `claim_unbonded`
        #[pallet::call_index(11)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::deregister_child(), Pays::No))]
        pub fn deregister_child(origin: OriginFor<T>, child: T::AccountId) -> DispatchResult {
            let who = ensure_signed(origin)?;

            let mut reg =
                ChildRegistrations::<T>::get(&child).ok_or(Error::<T>::ChildNotRegistered)?;
            ensure!(who == reg.family, DispatchError::BadOrigin);
            ensure!(reg.status == ChildStatus::Active, Error::<T>::ChildNotActive);

            let now = Self::now();
            let lockup_enabled = LockupEnabled::<T>::get();
            let unbonding_end = if !lockup_enabled || reg.deposit.is_zero() {
                now
            } else {
                now.saturating_add(T::UnbondingPeriodBlocks::get())
            };
            let cooldown_end = now.saturating_add(T::UnregisterCooldownBlocks::get());

            // Remove from active set
            NodeIdToChild::<T>::remove(reg.node_id);
            TotalActiveChildren::<T>::put(TotalActiveChildren::<T>::get().saturating_sub(1));
            let fam_active = FamilyActiveChildren::<T>::get(&reg.family);
            FamilyActiveChildren::<T>::insert(&reg.family, fam_active.saturating_sub(1));
            // Maintain family children index
            FamilyChildren::<T>::mutate(&reg.family, |v| {
                if let Some(pos) = v.iter().position(|c| c == &child) {
                    v.swap_remove(pos);
                }
            });

            // Cooldowns
            ChildCooldownUntil::<T>::insert(&child, cooldown_end);
            NodeIdCooldownUntil::<T>::insert(reg.node_id, cooldown_end);

            // Flip status to Unbonding
            reg.status = ChildStatus::Unbonding;
            reg.unbonding_end = unbonding_end;
            ChildRegistrations::<T>::insert(&child, &reg);

            // Prevent replay with old signatures after deregistration.
            let nonce = NodeIdNonce::<T>::get(reg.node_id);
            NodeIdNonce::<T>::insert(reg.node_id, nonce.saturating_add(1));

            Self::deposit_event(Event::ChildDeregistered {
                family: reg.family,
                child,
                node_id: reg.node_id,
                unbonding_end,
                cooldown_end,
            });
            Ok(())
        }

        /// Claim (unbond) the deposit for a deregistered child after the unbonding period.
        ///
        /// Note: this does NOT bypass cooldown; cooldown is enforced on `register_child`.
        #[pallet::call_index(12)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::claim_unbonded(), Pays::No))]
        pub fn claim_unbonded(origin: OriginFor<T>, child: T::AccountId) -> DispatchResult {
            let who = ensure_signed(origin)?;

            let reg =
                ChildRegistrations::<T>::get(&child).ok_or(Error::<T>::ChildNotRegistered)?;
            ensure!(who == reg.family, DispatchError::BadOrigin);
            ensure!(reg.status == ChildStatus::Unbonding, Error::<T>::NotUnbonding);
            ensure!(Self::now() >= reg.unbonding_end, Error::<T>::UnbondingNotReady);

            let amount = reg.deposit;
            if LockupEnabled::<T>::get() && !amount.is_zero() {
                let _ = T::DepositCurrency::unreserve(&reg.family, amount);
            }

            // Remove record; cooldown tombstones remain in separate storage.
            ChildRegistrations::<T>::remove(&child);

            Self::deposit_event(Event::ChildUnbonded {
                family: reg.family,
                child,
                node_id: reg.node_id,
                amount,
            });
            Ok(())
        }

        /// Submit validator-observed per-node quality metrics and let the pallet compute the final node + family weights.
        ///
        /// This is the **recommended** path (deterministic on-chain weight calculation).
        #[pallet::call_index(20)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::submit_node_quality(updates.len() as u32), Pays::No))]
        pub fn submit_node_quality(
            origin: OriginFor<T>,
            bucket: u32,
            updates: BoundedVec<(T::AccountId, NodeQuality), T::MaxNodeWeightUpdates>,
        ) -> DispatchResult {
            T::WeightAuthorityOrigin::ensure_origin(origin)?;

            let cur = CurrentWeightBucket::<T>::get();
            ensure!(bucket >= cur, Error::<T>::WeightBucketRegression);

            // Compute node weights from quality inputs.
            let mut node_updates: BoundedVec<(T::AccountId, u16), T::MaxNodeWeightUpdates> =
                BoundedVec::default();
            for (child, q) in updates.iter() {
                NodeQualityByChild::<T>::insert(child, q.clone());
                let w = Self::compute_node_weight_from_quality(q);
                node_updates
                    .try_push((child.clone(), w))
                    .map_err(|_| Error::<T>::TooManyNodeWeightUpdates)?;
            }
            Self::apply_node_weights_and_recompute(bucket, &node_updates)
        }

        /// Submit warden proof-of-storage attestations.
        ///
        /// Attestations are signed audit results from wardens that verify miners
        /// are storing the data they claim to store. These are used for:
        /// - Reputation scoring
        /// - Slashing for failed audits
        /// - Rewarding successful storage proofs
        ///
        /// Expected usage:
        /// - Called periodically by the chain-submitter service
        /// - Warden signs attestations with Ed25519 keypair
        /// - Signature verification is performed on-chain for each attestation
        ///
        /// # Security
        /// - Each attestation signature is verified using Ed25519
        /// - Invalid signatures are rejected with InvalidAttestationSignature error
        #[pallet::call_index(2)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::submit_attestations(attestations.len() as u32), Pays::No))]
        pub fn submit_attestations(
            origin: OriginFor<T>,
            bucket: u32,
            attestations: BoundedVec<
                AttestationRecord<T::MaxShardHashLen, T::MaxWardenPubkeyLen, T::MaxSignatureLen, T::MaxMerkleProofLen, T::MaxWardenIdLen>,
                T::MaxAttestations,
            >,
        ) -> DispatchResult {
            T::AttestationAuthorityOrigin::ensure_origin(origin)?;

            let cur = CurrentAttestationBucket::<T>::get();
            ensure!(bucket >= cur, Error::<T>::AttestationBucketRegression);

            let count = attestations.len() as u32;
            ensure!(count > 0, Error::<T>::TooManyAttestations);

            // Verify all attestation signatures before storing
            for attestation in attestations.iter() {
                ensure!(
                    Self::verify_attestation_sig(attestation),
                    Error::<T>::InvalidAttestationSignature
                );
            }

            // Append to existing attestations for this bucket (or create new vec)
            AttestationsByBucket::<T>::try_mutate(bucket, |existing| {
                for attestation in attestations.iter() {
                    existing
                        .try_push(attestation.clone())
                        .map_err(|_| Error::<T>::AttestationBucketFull)?;
                }
                Ok::<(), DispatchError>(())
            })?;

            // Update bucket cursor
            CurrentAttestationBucket::<T>::put(bucket);

            Self::deposit_event(Event::AttestationsSubmitted { bucket, count });
            Ok(())
        }

        /// Submit an attestation commitment for third-party verification.
        ///
        /// This stores a compact commitment containing merkle roots and the Arion
        /// content hash. Third parties can:
        /// 1. Query this commitment from the chain
        /// 2. Download the full bundle from Arion using `arion_content_hash`
        /// 3. Verify the bundle hash matches
        /// 4. Verify attestations against the merkle roots
        ///
        /// # Parameters
        /// - `epoch`: The epoch this commitment covers
        /// - `arion_content_hash`: BLAKE3 hash of the SCALE-encoded AttestationBundle (32 bytes)
        /// - `attestation_merkle_root`: Merkle root of all attestation leaves
        /// - `warden_pubkey_merkle_root`: Merkle root of unique warden public keys
        /// - `attestation_count`: Number of attestations in the bundle
        #[pallet::call_index(3)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::submit_attestation_commitment(), Pays::No))]
        pub fn submit_attestation_commitment(
            origin: OriginFor<T>,
            epoch: u64,
            arion_content_hash: BoundedVec<u8, T::MaxContentHashLen>,
            attestation_merkle_root: [u8; 32],
            warden_pubkey_merkle_root: [u8; 32],
            attestation_count: u32,
        ) -> DispatchResult {
            T::AttestationAuthorityOrigin::ensure_origin(origin)?;

            // Ensure commitment doesn't already exist for this epoch
            ensure!(
                !EpochAttestationCommitments::<T>::contains_key(epoch),
                Error::<T>::AttestationCommitmentAlreadyExists
            );

            // Validate content hash is 32 bytes (BLAKE3)
            ensure!(
                arion_content_hash.len() == 32,
                Error::<T>::InvalidContentHashLength
            );

            let now = <frame_system::Pallet<T>>::block_number();
            let submitted_at_block: u64 = TryInto::<u64>::try_into(now).ok().unwrap_or(0);

            let commitment = EpochAttestationCommitment {
                epoch,
                arion_content_hash,
                attestation_merkle_root,
                warden_pubkey_merkle_root,
                attestation_count,
                submitted_at_block,
            };

            EpochAttestationCommitments::<T>::insert(epoch, commitment);

            Self::deposit_event(Event::AttestationCommitmentSubmitted {
                epoch,
                attestation_count,
                attestation_merkle_root,
                warden_pubkey_merkle_root,
            });

            Ok(())
        }

        /// Admin: enable/disable registration lockup (reserve/unbond).
        ///
        /// Configure `AdminOrigin` as `EnsureRoot` to make this a sudo-only extrinsic.
        #[pallet::call_index(30)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::set_lockup_enabled(), Pays::No))]
        pub fn set_lockup_enabled(origin: OriginFor<T>, enabled: bool) -> DispatchResult {
            T::ArionAdminOrigin::ensure_origin(origin)?;
            LockupEnabled::<T>::put(enabled);
            Self::deposit_event(Event::LockupEnabledSet { enabled });
            Ok(())
        }

        /// Admin: set the base deposit price (floor for the global fee curve).
        ///
        /// Configure `AdminOrigin` as `EnsureRoot` to make this a sudo-only extrinsic.
        ///
        /// Notes:
        /// - This does not overwrite `GlobalNextDeposit` unless it is below the new floor;
        ///   the next time registration runs, `global_next_deposit_floor_init` will raise it.
        #[pallet::call_index(31)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::set_base_child_deposit(), Pays::No))]
        pub fn set_base_child_deposit(origin: OriginFor<T>, deposit: BalanceOf<T>) -> DispatchResult {
            T::ArionAdminOrigin::ensure_origin(origin)?;
            BaseChildDepositValue::<T>::put(deposit);
            // Ensure fee floor is applied immediately.
            let _ = Self::global_next_deposit_floor_init();
            Self::deposit_event(Event::BaseChildDepositSet { deposit });
            Ok(())
        }

        /// Admin: Register a warden authorized to submit attestations.
        ///
        /// Once registered, attestations from this warden's public key will be accepted.
        /// Third parties can query `RegisteredWardens[pubkey]` to verify authorization.
        ///
        /// # Parameters
        /// - `warden_pubkey`: The warden's Ed25519 public key (32 bytes)
        #[pallet::call_index(32)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::register_warden(), Pays::No))]
        pub fn register_warden(origin: OriginFor<T>, warden_pubkey: [u8; 32]) -> DispatchResult {
            T::ArionAdminOrigin::ensure_origin(origin)?;

            // Check if already registered
            ensure!(
                !RegisteredWardens::<T>::contains_key(&warden_pubkey),
                Error::<T>::WardenAlreadyRegistered
            );

            let now = Self::now();
            let info = WardenInfo {
                status: WardenStatus::Active,
                registered_at: now,
                deregistered_at: None,
            };

            RegisteredWardens::<T>::insert(&warden_pubkey, info);
            ActiveWardenCount::<T>::put(ActiveWardenCount::<T>::get().saturating_add(1));

            Self::deposit_event(Event::WardenRegistered {
                warden_pubkey,
                registered_at: now,
            });

            Ok(())
        }

        /// Admin: Deregister a warden, preventing future attestation submissions.
        ///
        /// The warden's registration record is kept for audit purposes but marked as deregistered.
        /// Attestations from deregistered wardens will be rejected.
        ///
        /// # Parameters
        /// - `warden_pubkey`: The warden's Ed25519 public key (32 bytes)
        #[pallet::call_index(33)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::deregister_warden(), Pays::No))]
        pub fn deregister_warden(origin: OriginFor<T>, warden_pubkey: [u8; 32]) -> DispatchResult {
            T::ArionAdminOrigin::ensure_origin(origin)?;

            // Check if registered
            let mut info = RegisteredWardens::<T>::get(&warden_pubkey)
                .ok_or(Error::<T>::WardenNotRegistered)?;

            // Already deregistered?
            ensure!(
                info.status == WardenStatus::Active,
                Error::<T>::WardenNotRegistered
            );

            let now = Self::now();
            info.status = WardenStatus::Deregistered;
            info.deregistered_at = Some(now);

            RegisteredWardens::<T>::insert(&warden_pubkey, info);
            ActiveWardenCount::<T>::put(ActiveWardenCount::<T>::get().saturating_sub(1));

            Self::deposit_event(Event::WardenDeregistered {
                warden_pubkey,
                deregistered_at: now,
            });

            Ok(())
        }

        /// Prune old attestation buckets to prevent unbounded storage growth.
        ///
        /// Removes attestation data for buckets older than `before_bucket`.
        /// The `before_bucket` must be at least `AttestationRetentionBuckets` behind
        /// the current bucket to prevent accidental pruning of recent data.
        ///
        /// This is a permissionless operation - anyone can call it to help clean up
        /// old attestation data. The retention period ensures recent data is protected.
        ///
        /// # Parameters
        /// - `before_bucket`: Prune all buckets with ID less than this value
        /// - `max_buckets`: Maximum number of buckets to prune in this call (for weight limiting)
        #[pallet::call_index(34)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::prune_attestation_buckets(*max_buckets), Pays::No))]
        pub fn prune_attestation_buckets(
            origin: OriginFor<T>,
            before_bucket: u32,
            max_buckets: u32,
        ) -> DispatchResult {
            // Permissionless - anyone can prune old data
            let _who = ensure_signed(origin)?;

            let current = CurrentAttestationBucket::<T>::get();
            let retention = T::AttestationRetentionBuckets::get();

            // Ensure we're not pruning within the retention period
            ensure!(
                before_bucket <= current.saturating_sub(retention),
                Error::<T>::PruningWithinRetentionPeriod
            );

            // Prune buckets, up to max_buckets
            let mut pruned: u32 = 0;
            let mut bucket = before_bucket.saturating_sub(max_buckets);

            while bucket < before_bucket && pruned < max_buckets {
                if AttestationsByBucket::<T>::contains_key(bucket) {
                    AttestationsByBucket::<T>::remove(bucket);
                    pruned = pruned.saturating_add(1);
                }
                bucket = bucket.saturating_add(1);
            }

            if pruned > 0 {
                Self::deposit_event(Event::AttestationBucketsPruned {
                    pruned_count: pruned,
                    oldest_remaining: before_bucket,
                });
            }

            Ok(())
        }
    }
}
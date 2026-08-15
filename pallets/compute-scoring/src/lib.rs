#![cfg_attr(not(feature = "std"), no_std)]
//! # `pallet-compute-scoring`
//!
//! On-chain scoring for the Hippius DePIN compute family. Owns the
//! §23 reward inputs (per-node served-units via audit-VM signature
//! chain) + the §13 status transitions (Active ↔ Quarantined) + the
//! §13 anti-Sybil registration. Ranking lives elsewhere (off-chain
//! ranking pallet pulls from [`EpochWeights`]); CRUSH placement is
//! a vali responsibility (off-chain) — neither belongs in this
//! pallet.
//!
//! ## Anti-pollution discipline (§I, locked 2026-05-20)
//!
//! State / events on-chain MUST serve the **reward**,
//! **anti-Sybil**, or **slashing** path. Everything else goes
//! off-chain (vali audit log, KBS audit log #36). PR-I4 enforced
//! this by ripping ~1300 LOC of vendored arion-pallet code that
//! tracked storage-shard CRUSH maps, warden audit attestations,
//! file-size aggregates, and per-node-quality weight blending —
//! none of those layers apply to confidential compute. See
//! `CHANGES.md` PR-I4 section for the line-by-line trace.
//!
//! ## Surface (post-PR-I4)
//!
//! Extrinsics:
//! - [`Pallet::submit_audit_stats`] — audit-VM-signed aggregate
//!   submission (PR-I3). Verifies the Ed25519 chain + §23 replay
//!   domain (chain_genesis, pallet_instance, epoch, expiry,
//!   prev_aggregate_hash). Chains the body SHA-256 forward. **Does
//!   NOT credit anything to per-node storage** — the validator
//!   aggregates served-units off-chain and writes them at epoch
//!   close.
//! - [`Pallet::vali_submit_epoch_close`] — root-only (locked Q13
//!   single-operator v1). Writes [`EpochWeights`] for the closing
//!   epoch + transitions [`MinerStatus`] rows. Emits events ONLY
//!   on status transitions; a heartbeat that does not change the
//!   stored status is silent.
//! - [`Pallet::register_child`] / [`Pallet::deregister_child`] /
//!   [`Pallet::force_deregister_child`] /
//!   [`Pallet::claim_unbonded`] — §13 anti-Sybil registration with
//!   capped doubling deposit + cooldown + nonce-bumped replay
//!   protection.
//! - [`Pallet::set_audit_vm_pubkey`] — admin placeholder until
//!   PR-I6 wires the KBS-issued certificate flow.
//! - [`Pallet::set_lockup_enabled`] /
//!   [`Pallet::set_base_child_deposit`] /
//!   [`Pallet::set_free_child_slots_per_family`] — admin config.

#![allow(
    // PR-I3: `submit_audit_stats` carries `AggregateView` + a
    // bounded body, which clippy flags as a large-vs-small variant
    // imbalance against the bare `set_lockup_enabled(bool)`
    // neighbours. Boxing the call argument would change the
    // metadata's visible argument shape (and the auto-generated
    // client interfaces off-chain submitters rely on) for no real
    // saving; SCALE wire bytes are unaffected either way.
    clippy::large_enum_variant,
)]

extern crate alloc;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

pub mod weights;
pub use weights::WeightInfo;

pub use pallet::*;

use alloc::collections::BTreeSet;
use alloc::vec::Vec;
use codec::{Decode, Encode, MaxEncodedLen};
use frame_support::{
    dispatch::DispatchResult,
    pallet_prelude::*,
    traits::{Currency, EnsureOrigin, Get, ReservableCurrency},
    BoundedVec,
};
use frame_system::pallet_prelude::*;
use scale_info::TypeInfo;
use sp_core::ed25519;
use sp_runtime::{
    traits::{Saturating, Verify, Zero},
    RuntimeDebug, SaturatedConversion,
};

#[frame_support::pallet]
pub mod pallet {
    use super::*;
    // PR-I5: the local stubs are gone — these resolve to the real
    // `thenervelab/thebrain` crates pinned in `Cargo.toml` at commit
    // `11226860c66e51bd7092c67263ef59b658f1d7f4` (same as
    // `PROVENANCE.md`'s arion-pallet pin). The scoring pallet's own
    // `Config` does NOT inherit `pallet_registration::Config` /
    // `pallet_proxy::Config` as supertraits — instead the
    // `FamilyRegistry` / `ProxyVerifier` / `NodeRegistrationProvider`
    // trait abstractions decouple the scoring pallet's tests from
    // the heavy 8-supertrait wiring on the real registration crate.
    // The runtime binds the abstractions to
    // `pallet_registration::Pallet<Self>` / `pallet_proxy::Pallet<Self>`.
    // The trait impls reach `pallet_registration::Pallet<T>` /
    // `pallet_proxy::Pallet<T>` via their fully-qualified paths in
    // this module — no top-level `use` needed.

    /// Domain separator for attestation signing.
    ///
    /// IMPORTANT: This must be typed as `&[u8]` (slice) to match
    /// off-chain SCALE encoding. Using `b"..."` directly would be
    /// `&[u8; N]` (fixed array) which encodes differently.
    #[allow(dead_code)]
    const ATTESTATION_DOMAIN_SEPARATOR: &[u8] = b"HIPPIUS_COMPUTE_ATTESTATION_V1";

    type BalanceOf<T> = <<T as Config>::DepositCurrency as Currency<
        <T as frame_system::Config>::AccountId,
    >>::Balance;

    // =====================================================================
    // Trait hooks (runtime-wired) — anti-Sybil registration / proxy gates
    // =====================================================================

    /// Trait to query the runtime's family / owner registration
    /// layer. Production binds this to `pallet_registration::Pallet<Self>`;
    /// tests use `()` (fail-closed) or a stand-in.
    pub trait FamilyRegistry<AccountId> {
        /// Anti-Sybil check: `true` iff the runtime considers
        /// `family` a registered owner with at least one node.
        fn is_registered_family(family: &AccountId) -> bool;
        /// `force_deregister_child` gate: `true` iff the runtime
        /// considers `owner` a registered owner whose primary node
        /// has type `Validator` (only validators can force-
        /// deregister others' children, per the upstream
        /// `pallet-registration` model). Returns `false` for any
        /// non-validator owner OR if `owner` has no registered
        /// node at all.
        fn owner_has_validator_node(owner: &AccountId) -> bool;
    }

    impl<AccountId> FamilyRegistry<AccountId> for () {
        fn is_registered_family(_family: &AccountId) -> bool {
            false
        }
        fn owner_has_validator_node(_owner: &AccountId) -> bool {
            false
        }
    }

    /// `pallet_registration::Pallet<T>` satisfies `FamilyRegistry`
    /// when the runtime `T` implements `pallet_registration::Config`.
    /// The bound is on the impl (NOT on the scoring pallet's own
    /// `Config` supertrait), so test runtimes that bind
    /// `T::FamilyRegistry = ()` don't need to wire all 8 of
    /// `pallet-registration`'s supertraits.
    impl<T: pallet_registration::Config> FamilyRegistry<T::AccountId>
        for pallet_registration::Pallet<T>
    {
        fn is_registered_family(family: &T::AccountId) -> bool {
            pallet_registration::Pallet::<T>::is_owner_node_registered(family)
        }
        fn owner_has_validator_node(owner: &T::AccountId) -> bool {
            match pallet_registration::Pallet::<T>::get_registered_node_for_owner(owner) {
                Some(info) => info.node_type == pallet_registration::NodeType::Validator,
                None => false,
            }
        }
    }

    pub trait ProxyVerifier<AccountId> {
        /// `register_child` gate: `true` iff `family` has authorised
        /// `child` as a proxy.
        fn can_register_child(family: &AccountId, child: &AccountId) -> bool;
    }

    impl<AccountId> ProxyVerifier<AccountId> for () {
        fn can_register_child(_family: &AccountId, _child: &AccountId) -> bool {
            false
        }
    }

    /// `pallet_proxy::Pallet<T>` satisfies `ProxyVerifier` when the
    /// runtime `T` implements `pallet_proxy::Config`. Same impl-bound
    /// pattern as [`FamilyRegistry`] above.
    impl<T: pallet_proxy::Config> ProxyVerifier<T::AccountId> for pallet_proxy::Pallet<T> {
        fn can_register_child(family: &T::AccountId, child: &T::AccountId) -> bool {
            let (proxy_definitions, _) = pallet_proxy::Pallet::<T>::proxies(family);
            proxy_definitions.iter().any(|p| p.delegate == *child)
        }
    }

    /// PR-I5: query whether a `node_id` is currently registered in
    /// the real `pallet-registration` crate (or any future
    /// registration backend). `submit_audit_stats` gates on this so
    /// the on-chain audit-VM signature chain only advances for
    /// nodes the registration layer knows about — an unregistered
    /// node trying to submit aggregates is rejected before the
    /// expensive Ed25519 verify.
    pub trait NodeRegistrationProvider {
        /// `true` iff the runtime's registration layer has an
        /// active registration record for `node_id`. Implementations
        /// may exclude `Degraded` / `Offline` statuses (see the
        /// `pallet_registration::Pallet<T>` impl below).
        fn is_node_registered(node_id: &[u8; 32]) -> bool;
    }

    impl NodeRegistrationProvider for () {
        /// Fail-closed default — tests using `Registration = ()`
        /// will see every `submit_audit_stats` rejected with
        /// `NodeNotRegistered` until a real backend (or a test
        /// stand-in) is wired.
        fn is_node_registered(_node_id: &[u8; 32]) -> bool {
            false
        }
    }

    /// `pallet_registration::Pallet<T>` impl. `node_id` is
    /// converted to `Vec<u8>` because the upstream
    /// `get_node_registration_info` takes that shape (the storage
    /// key is variable-length per upstream). `is_some()` excludes
    /// `Degraded` nodes per the upstream filter — "actively
    /// registered" is the right gate for reward eligibility.
    impl<T: pallet_registration::Config> NodeRegistrationProvider for pallet_registration::Pallet<T> {
        fn is_node_registered(node_id: &[u8; 32]) -> bool {
            pallet_registration::Pallet::<T>::get_node_registration_info(node_id.to_vec()).is_some()
        }
    }

    /// PR-I6: one `(node_id, weight)` entry as written to
    /// [`EpochWeights`] by [`Pallet::vali_submit_epoch_close`].
    /// Aggregated into a `Vec<EpochWeightEntry>` and pushed to
    /// `T::RankingsSink` at the end of an epoch-close so a
    /// downstream consumer (e.g. an in-runtime adapter that
    /// re-dispatches into the thebrain `pallet-rankings` extrinsic,
    /// OR a tally / event sink for off-chain validators) can react
    /// to the closed epoch in one hook.
    #[derive(Clone, Encode, Decode, RuntimeDebug, TypeInfo, PartialEq, Eq, MaxEncodedLen)]
    pub struct EpochWeightEntry {
        pub node_id: [u8; 32],
        pub weight: u128,
    }

    /// PR-I6: downstream consumer of the per-epoch weight snapshot
    /// produced by [`Pallet::vali_submit_epoch_close`].
    ///
    /// **Production wiring** (typical): the validator's off-chain
    /// worker reads [`Pallet::epoch_weights_for`] directly + posts
    /// a `pallet_rankings::update_rankings` extrinsic — `RankingsSink`
    /// stays `()`. The trait exists for runtimes that want to push
    /// in-runtime (e.g. an adapter that calls `update_rankings`
    /// synchronously from `vali_submit_epoch_close`), and as the
    /// **integration boundary** the PR-I6 bridge test exercises:
    /// the mock sink records what `vali_submit_epoch_close`
    /// produces, the test asserts the payload matches the
    /// `EpochWeights` written that epoch.
    ///
    /// The trait stays decoupled from `pallet-rankings`: building
    /// a mini `construct_runtime!` with the real pallet would
    /// require ~1000 LOC of polkadot-sdk relay-chain scaffolding
    /// (`pallet-staking` + `pallet-babe` + `pallet-metagraph`
    /// (circular with `pallet-registration`) + Session/Election/
    /// Offences/Historical). See `CHANGES.md` PR-I6 for the
    /// scope decision + the production wiring path.
    pub trait RankingsSink {
        /// Called once per `vali_submit_epoch_close` after every
        /// `EpochWeights[epoch][*]` write completes + after
        /// `CurrentEpoch` advances. `entries` is the full snapshot
        /// for `epoch` — even nodes whose status was a heartbeat
        /// (no transition) appear if a weight was written.
        ///
        /// **Best-effort, no-fail contract**: implementations MUST
        /// NOT return errors that would roll back the epoch close —
        /// the close has already advanced `CurrentEpoch` and written
        /// `EpochWeights`, so the sink push is a notification
        /// downstream, not a transactional dependency. Production
        /// adapters that need fallible delivery (e.g. dispatching
        /// into another pallet's extrinsic synchronously) MUST do
        /// the fallible work via an off-chain worker triggered by
        /// the `EpochClosed` event instead.
        ///
        /// **Weight contract**: `vali_submit_epoch_close`'s weight is
        /// parameterised by the batch size `n` (bounded by
        /// `MaxMinerStatusUpdatesPerCall`), and `push_rankings` runs
        /// INSIDE that call with `entries.len() == n`. An implementation
        /// MUST therefore be `O(n)` and bounded per entry, and the
        /// runtime's `WeightInfo::vali_submit_epoch_close` benchmark MUST
        /// exercise the real bound sink — otherwise a heavy sink could
        /// push the close past the block weight limit. The recommended
        /// production binding is `()` (no-op): off-chain validators read
        /// [`Pallet::epoch_weights_for`] and dispatch the ranking update
        /// themselves, so the in-runtime push costs nothing.
        fn push_rankings(epoch: u64, entries: &[EpochWeightEntry]);
    }

    impl RankingsSink for () {
        /// Recommended production binding — the off-chain validator
        /// OCW reads our storage and dispatches `update_rankings`
        /// itself; no in-runtime push needed. Test runtimes substitute a
        /// recording sink (see `mock::MockRankingsSink`).
        fn push_rankings(_epoch: u64, _entries: &[EpochWeightEntry]) {}
    }

    // =====================================================================
    // Types
    // =====================================================================

    /// §13 lifecycle status for a registered compute node.
    ///
    /// **Active** = eligible for reward weight; **Quarantined** =
    /// failed an attestation / liveness check, no reward but still
    /// registered; **Decommissioned** = retired per §24.
    /// Transitions are written by [`Pallet::vali_submit_epoch_close`]
    /// — the root-only privileged submitter.
    #[derive(Clone, Copy, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
    pub enum MinerStatus {
        Active,
        Quarantined,
        Decommissioned,
    }

    /// One row per registered compute node (NOT per epoch — the
    /// status is a single-row state machine; per-epoch history lives
    /// in the audit-log off-chain, not on-chain).
    #[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
    pub struct MinerStatusEntry<BlockNumber> {
        pub status: MinerStatus,
        /// Block number when the status was last transitioned (not
        /// updated on heartbeats — only on transitions).
        pub last_transition_block: BlockNumber,
        /// Epoch when the status was last transitioned.
        pub last_transition_epoch: u64,
    }

    /// One row in the batched `vali_submit_epoch_close` payload —
    /// the validator's verdict on a node's status + weight for the
    /// closing epoch.
    #[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
    pub struct MinerStatusUpdate {
        pub node_id: [u8; 32],
        pub new_status: MinerStatus,
        /// Compute-served reward weight for this node this epoch.
        /// Validator computes off-chain from the audit-VM-signed
        /// aggregates submitted via [`Pallet::submit_audit_stats`].
        /// `0` means the node was active but earned no reward this
        /// window (vs `Quarantined` which means actively penalised).
        pub weight: u128,
    }

    /// §13 child lifecycle status.
    #[derive(Clone, Copy, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
    pub enum ChildStatus {
        /// Actively registered and eligible for status updates.
        Active,
        /// Deregistered but deposit is still bonded; can be claimed
        /// after `unbonding_end`.
        Unbonding,
    }

    #[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
    pub struct ChildRegistration<AccountId, BlockNumber, Balance> {
        pub family: AccountId,
        pub node_id: [u8; 32],
        pub status: ChildStatus,
        /// Amount reserved from `family` for this child (0 for the
        /// free slot).
        pub deposit: Balance,
        /// If `status == Unbonding`, the earliest block when
        /// `claim_unbonded` can unreserve funds.
        pub unbonding_end: BlockNumber,
    }

    // =====================================================================
    // PR-I3 — Audit-VM signed compute-served aggregate (kept)
    // =====================================================================

    /// SCALE projection of the audit-VM-signed
    /// `hippius_types::audit_vm::ServedDeliveryAggregate` fields.
    /// The validator extracts these from the canonical CBOR body
    /// before submitting; the pallet enforces the replay-domain
    /// checks against on-chain state.
    ///
    /// `Clone` / `PartialEq` / `Eq` / `Debug` are manually
    /// implemented below (the `Get<u32>` marker types aren't `Clone`
    /// themselves, so `#[derive(Clone, …)]` would require an
    /// unhelpful `MaxX: Clone` bound at every use site).
    #[derive(Encode, Decode, TypeInfo, MaxEncodedLen)]
    #[scale_info(skip_type_params(MaxValidatorId, MaxFamilyId, MaxAuditVmKeyId))]
    pub struct AggregateView<MaxValidatorId, MaxFamilyId, MaxAuditVmKeyId>
    where
        MaxValidatorId: Get<u32> + 'static,
        MaxFamilyId: Get<u32> + 'static,
        MaxAuditVmKeyId: Get<u32> + 'static,
    {
        pub chain_genesis: [u8; 32],
        pub pallet_instance: [u8; 32],
        pub validator_id: BoundedVec<u8, MaxValidatorId>,
        pub family_id: BoundedVec<u8, MaxFamilyId>,
        pub node_id: [u8; 32],
        pub audit_vm_key_id: BoundedVec<u8, MaxAuditVmKeyId>,
        pub epoch: u64,
        pub challenge_nonce: [u8; 32],
        pub interval_start: u64,
        pub interval_end: u64,
        pub map_root: [u8; 32],
        pub totals_root: [u8; 32],
        pub prev_aggregate_hash: [u8; 32],
        pub expiry: u64,
        /// Sum of `served_units` across every resource class.
        /// `submit_audit_stats` does NOT store this — it's preserved
        /// on the wire so a future PR-I3.1 can rebind it via a
        /// canonical-CBOR `view_hash` schema change. Today the
        /// validator's off-chain aggregator is the authoritative
        /// source; the on-chain projection lives in
        /// [`EpochWeights`] via `vali_submit_epoch_close`.
        pub served_units: u128,
    }

    impl<MaxValidatorId, MaxFamilyId, MaxAuditVmKeyId> Clone
        for AggregateView<MaxValidatorId, MaxFamilyId, MaxAuditVmKeyId>
    where
        MaxValidatorId: Get<u32> + 'static,
        MaxFamilyId: Get<u32> + 'static,
        MaxAuditVmKeyId: Get<u32> + 'static,
    {
        fn clone(&self) -> Self {
            Self {
                chain_genesis: self.chain_genesis,
                pallet_instance: self.pallet_instance,
                validator_id: self.validator_id.clone(),
                family_id: self.family_id.clone(),
                node_id: self.node_id,
                audit_vm_key_id: self.audit_vm_key_id.clone(),
                epoch: self.epoch,
                challenge_nonce: self.challenge_nonce,
                interval_start: self.interval_start,
                interval_end: self.interval_end,
                map_root: self.map_root,
                totals_root: self.totals_root,
                prev_aggregate_hash: self.prev_aggregate_hash,
                expiry: self.expiry,
                served_units: self.served_units,
            }
        }
    }

    impl<MaxValidatorId, MaxFamilyId, MaxAuditVmKeyId> PartialEq
        for AggregateView<MaxValidatorId, MaxFamilyId, MaxAuditVmKeyId>
    where
        MaxValidatorId: Get<u32> + 'static,
        MaxFamilyId: Get<u32> + 'static,
        MaxAuditVmKeyId: Get<u32> + 'static,
    {
        fn eq(&self, other: &Self) -> bool {
            self.chain_genesis == other.chain_genesis
                && self.pallet_instance == other.pallet_instance
                && self.validator_id == other.validator_id
                && self.family_id == other.family_id
                && self.node_id == other.node_id
                && self.audit_vm_key_id == other.audit_vm_key_id
                && self.epoch == other.epoch
                && self.challenge_nonce == other.challenge_nonce
                && self.interval_start == other.interval_start
                && self.interval_end == other.interval_end
                && self.map_root == other.map_root
                && self.totals_root == other.totals_root
                && self.prev_aggregate_hash == other.prev_aggregate_hash
                && self.expiry == other.expiry
                && self.served_units == other.served_units
        }
    }

    impl<MaxValidatorId, MaxFamilyId, MaxAuditVmKeyId> Eq
        for AggregateView<MaxValidatorId, MaxFamilyId, MaxAuditVmKeyId>
    where
        MaxValidatorId: Get<u32> + 'static,
        MaxFamilyId: Get<u32> + 'static,
        MaxAuditVmKeyId: Get<u32> + 'static,
    {
    }

    impl<MaxValidatorId, MaxFamilyId, MaxAuditVmKeyId> sp_std::fmt::Debug
        for AggregateView<MaxValidatorId, MaxFamilyId, MaxAuditVmKeyId>
    where
        MaxValidatorId: Get<u32> + 'static,
        MaxFamilyId: Get<u32> + 'static,
        MaxAuditVmKeyId: Get<u32> + 'static,
    {
        fn fmt(&self, f: &mut sp_std::fmt::Formatter<'_>) -> sp_std::fmt::Result {
            f.debug_struct("AggregateView")
                .field("node_id", &self.node_id)
                .field("epoch", &self.epoch)
                .field("interval_start", &self.interval_start)
                .field("interval_end", &self.interval_end)
                .field("served_units", &self.served_units)
                .finish()
        }
    }

    /// Signed envelope mirror of
    /// `hippius_types::audit_vm::SignedServedDeliveryAggregate` —
    /// `{body, sig}` where body is the canonical CBOR of the
    /// [`AggregateView`] fields and sig is the audit-VM Ed25519
    /// signature over body.
    #[derive(Encode, Decode, TypeInfo, MaxEncodedLen)]
    #[scale_info(skip_type_params(MaxAggregateBody))]
    pub struct SignedAggregateWire<MaxAggregateBody>
    where
        MaxAggregateBody: Get<u32> + 'static,
    {
        pub body: BoundedVec<u8, MaxAggregateBody>,
        pub sig: [u8; 64],
    }

    impl<MaxAggregateBody> Clone for SignedAggregateWire<MaxAggregateBody>
    where
        MaxAggregateBody: Get<u32> + 'static,
    {
        fn clone(&self) -> Self {
            Self {
                body: self.body.clone(),
                sig: self.sig,
            }
        }
    }

    impl<MaxAggregateBody> PartialEq for SignedAggregateWire<MaxAggregateBody>
    where
        MaxAggregateBody: Get<u32> + 'static,
    {
        fn eq(&self, other: &Self) -> bool {
            self.body == other.body && self.sig == other.sig
        }
    }

    impl<MaxAggregateBody> Eq for SignedAggregateWire<MaxAggregateBody> where
        MaxAggregateBody: Get<u32> + 'static
    {
    }

    impl<MaxAggregateBody> sp_std::fmt::Debug for SignedAggregateWire<MaxAggregateBody>
    where
        MaxAggregateBody: Get<u32> + 'static,
    {
        fn fmt(&self, f: &mut sp_std::fmt::Formatter<'_>) -> sp_std::fmt::Result {
            f.debug_struct("SignedAggregateWire")
                .field("body_len", &self.body.len())
                .finish()
        }
    }

    // ---------------------------------------------------------------------
    // #322 live-attestation wire types
    // ---------------------------------------------------------------------

    /// Branching-relevant projection of a
    /// `hippius_types::live_attestation::LiveAttestation` body. The
    /// raw canonical-CBOR bytes travel in
    /// [`SignedLiveAttestationWire::body`] and stay the
    /// signature's pre-image; this `view` carries the same fields in
    /// SCALE-friendly form so the extrinsic can branch on them
    /// without paying for a CBOR decode in the runtime. The
    /// extrinsic still re-hashes `signed.body` for chain continuity
    /// and re-runs Ed25519 against it, so a `view` that disagrees
    /// with the body cannot grant any state mutation — at worst the
    /// pre-flight gates reject before the verify, at best the verify
    /// itself fails.
    #[derive(Encode, Decode, TypeInfo, MaxEncodedLen)]
    #[scale_info(skip_type_params(MaxVmIdLen))]
    pub struct LiveAttestationView<MaxVmIdLen>
    where
        MaxVmIdLen: Get<u32> + 'static,
    {
        pub chain_genesis: [u8; 32],
        pub pallet_instance: [u8; 32],
        pub vm_id: BoundedVec<u8, MaxVmIdLen>,
        pub node_id: [u8; 32],
        pub attestation_seq: u64,
        pub epoch: u64,
        pub observed_at_unix: u64,
        pub verified_at_unix: u64,
        pub prev_attestation_hash: [u8; 32],
        pub expiry_unix: u64,
        pub signer_pubkey: [u8; 32],
    }

    impl<MaxVmIdLen> Clone for LiveAttestationView<MaxVmIdLen>
    where
        MaxVmIdLen: Get<u32> + 'static,
    {
        fn clone(&self) -> Self {
            Self {
                chain_genesis: self.chain_genesis,
                pallet_instance: self.pallet_instance,
                vm_id: self.vm_id.clone(),
                node_id: self.node_id,
                attestation_seq: self.attestation_seq,
                epoch: self.epoch,
                observed_at_unix: self.observed_at_unix,
                verified_at_unix: self.verified_at_unix,
                prev_attestation_hash: self.prev_attestation_hash,
                expiry_unix: self.expiry_unix,
                signer_pubkey: self.signer_pubkey,
            }
        }
    }

    impl<MaxVmIdLen> PartialEq for LiveAttestationView<MaxVmIdLen>
    where
        MaxVmIdLen: Get<u32> + 'static,
    {
        fn eq(&self, other: &Self) -> bool {
            self.chain_genesis == other.chain_genesis
                && self.pallet_instance == other.pallet_instance
                && self.vm_id == other.vm_id
                && self.node_id == other.node_id
                && self.attestation_seq == other.attestation_seq
                && self.epoch == other.epoch
                && self.observed_at_unix == other.observed_at_unix
                && self.verified_at_unix == other.verified_at_unix
                && self.prev_attestation_hash == other.prev_attestation_hash
                && self.expiry_unix == other.expiry_unix
                && self.signer_pubkey == other.signer_pubkey
        }
    }

    impl<MaxVmIdLen> Eq for LiveAttestationView<MaxVmIdLen> where MaxVmIdLen: Get<u32> + 'static {}

    impl<MaxVmIdLen> sp_std::fmt::Debug for LiveAttestationView<MaxVmIdLen>
    where
        MaxVmIdLen: Get<u32> + 'static,
    {
        fn fmt(&self, f: &mut sp_std::fmt::Formatter<'_>) -> sp_std::fmt::Result {
            f.debug_struct("LiveAttestationView")
                .field("vm_id_len", &self.vm_id.len())
                .field("node_id", &self.node_id)
                .field("attestation_seq", &self.attestation_seq)
                .field("epoch", &self.epoch)
                .finish()
        }
    }

    /// Signed envelope mirror of
    /// `hippius_types::live_attestation::SignedLiveAttestation` —
    /// `{body, sig}` where body is the canonical CBOR of the
    /// [`LiveAttestationView`] fields and sig is the KBS L0 Ed25519
    /// signature over body.
    #[derive(Encode, Decode, TypeInfo, MaxEncodedLen)]
    #[scale_info(skip_type_params(MaxLiveAttestationBody))]
    pub struct SignedLiveAttestationWire<MaxLiveAttestationBody>
    where
        MaxLiveAttestationBody: Get<u32> + 'static,
    {
        pub body: BoundedVec<u8, MaxLiveAttestationBody>,
        pub sig: [u8; 64],
    }

    impl<MaxLiveAttestationBody> Clone for SignedLiveAttestationWire<MaxLiveAttestationBody>
    where
        MaxLiveAttestationBody: Get<u32> + 'static,
    {
        fn clone(&self) -> Self {
            Self {
                body: self.body.clone(),
                sig: self.sig,
            }
        }
    }

    impl<MaxLiveAttestationBody> PartialEq for SignedLiveAttestationWire<MaxLiveAttestationBody>
    where
        MaxLiveAttestationBody: Get<u32> + 'static,
    {
        fn eq(&self, other: &Self) -> bool {
            self.body == other.body && self.sig == other.sig
        }
    }

    impl<MaxLiveAttestationBody> Eq for SignedLiveAttestationWire<MaxLiveAttestationBody> where
        MaxLiveAttestationBody: Get<u32> + 'static
    {
    }

    impl<MaxLiveAttestationBody> sp_std::fmt::Debug
        for SignedLiveAttestationWire<MaxLiveAttestationBody>
    where
        MaxLiveAttestationBody: Get<u32> + 'static,
    {
        fn fmt(&self, f: &mut sp_std::fmt::Formatter<'_>) -> sp_std::fmt::Result {
            f.debug_struct("SignedLiveAttestationWire")
                .field("body_len", &self.body.len())
                .finish()
        }
    }

    /// Per-VM live-attestation row — the on-chain replay state for
    /// the `prev_attestation_hash` chain + the most recent
    /// observation used as the "is the VM alive?" signal at epoch
    /// close.
    #[derive(Encode, Decode, TypeInfo, MaxEncodedLen, Clone, PartialEq, Eq, Debug)]
    pub struct LiveAttestationRecord {
        /// Miner host the latest attestation said the VM was on.
        pub node_id: [u8; 32],
        /// Monotonic per-VM counter — the next valid attestation
        /// MUST present `attestation_seq + 1`.
        pub attestation_seq: u64,
        /// SHA-256 of the canonical `signed.body` bytes — the next
        /// valid attestation MUST present this as its
        /// `prev_attestation_hash`.
        pub body_hash: [u8; 32],
        /// Epoch the latest attestation was observed under.
        pub epoch: u64,
        /// `observed_at_unix` of the latest attestation — used by
        /// the off-chain ranker to estimate the staleness gap when
        /// the chain advances past the epoch boundary.
        pub observed_at_unix: u64,
    }

    // =====================================================================
    // Config
    // =====================================================================

    #[pallet::config]
    // PR-I5: `pallet_registration::Config` + `pallet_proxy::Config`
    // are NOT added as supertraits here on purpose — the real
    // upstream Configs pull in ~8 transitive pallet Config
    // requirements (`pallet_babe`, `pallet_balances`,
    // `pallet_credits`, `pallet_staking`, `pallet_proxy`,
    // `pallet_utils` + custom providers / `ProxyTypeCompat` impls),
    // which would force every consuming runtime + every test mock
    // to wire all 8. The scoring pallet stays decoupled via the
    // `FamilyRegistry` / `ProxyVerifier` / `NodeRegistrationProvider`
    // trait abstractions below; the production runtime binds them
    // to `pallet_registration::Pallet<Self>` /
    // `pallet_proxy::Pallet<Self>` (where the heavy supertraits ARE
    // satisfied), while test runtimes use `()` or a lightweight
    // stand-in.
    pub trait Config: frame_system::Config {
        type RuntimeEvent: From<Event<Self>> + IsType<<Self as frame_system::Config>::RuntimeEvent>;

        /// Sudo/admin origin for pallet parameters.
        type ComputeScoringAdminOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// Currency used to reserve deposits for child registrations.
        type DepositCurrency: ReservableCurrency<Self::AccountId>;

        /// Family registry hook — production runtime binds this to
        /// `pallet_registration::Pallet<Self>`; tests use `()` (fail-
        /// closed) or a stand-in.
        type FamilyRegistry: FamilyRegistry<Self::AccountId>;

        /// Proxy gate — production runtime binds this to
        /// `pallet_proxy::Pallet<Self>`; tests use `()` (fail-closed)
        /// or a stand-in.
        type ProxyVerifier: ProxyVerifier<Self::AccountId>;

        /// PR-I5: node-registration query. Used by
        /// [`Pallet::submit_audit_stats`] to gate audit-VM aggregate
        /// submissions on "is this `node_id` actually registered in
        /// the registration layer". Production runtime binds this to
        /// `pallet_registration::Pallet<Self>`; tests use `()`
        /// (fail-closed) or a lightweight stand-in.
        type Registration: NodeRegistrationProvider;

        /// PR-I6: downstream consumer of the per-epoch weight
        /// snapshot produced by [`Pallet::vali_submit_epoch_close`].
        /// **Recommended production binding**: `()` — the off-chain
        /// validator OCW reads our storage and dispatches
        /// `pallet_rankings::update_rankings` itself. The runtime
        /// must still bind `type RankingsSink = ();` explicitly;
        /// there is no FRAME default. Tests bind a recording sink
        /// (`MockRankingsSink`) to pin the integration-boundary
        /// contract. See [`RankingsSink`] + `CHANGES.md` PR-I6.
        type RankingsSink: RankingsSink;

        // --- Registration economics / safety controls ---

        /// Max distinct families that can ever claim their first
        /// "free" child slot.
        #[pallet::constant]
        type MaxFamilies: Get<u32>;

        /// Max total number of active children across all families.
        #[pallet::constant]
        type MaxChildrenTotal: Get<u32>;

        /// Max number of active children within one family.
        #[pallet::constant]
        type MaxChildrenPerFamily: Get<u32>;

        /// Base deposit used to initialize / floor the global
        /// deposit curve.
        #[pallet::constant]
        type BaseChildDeposit: Get<BalanceOf<Self>>;

        /// Number of blocks after which the global next deposit
        /// halves (lazy decay).
        #[pallet::constant]
        type GlobalDepositHalvingPeriodBlocks: Get<BlockNumberFor<Self>>;

        /// Cooldown after deregistration: child/node cannot be
        /// re-registered until this passes.
        #[pallet::constant]
        type UnregisterCooldownBlocks: Get<BlockNumberFor<Self>>;

        /// Unbonding period: deposit stays reserved until this
        /// passes, then `claim_unbonded` releases it.
        #[pallet::constant]
        type UnbondingPeriodBlocks: Get<BlockNumberFor<Self>>;

        /// Weight information for extrinsics in this pallet.
        type WeightInfo: crate::weights::WeightInfo;

        // --- PR-I3 audit-stats config ---

        /// Privileged origin allowed to submit audit-VM-signed
        /// aggregates ([`Pallet::submit_audit_stats`]). v1 trust
        /// posture: the vali cluster is the single authoritative
        /// submitter (issue #25 §G).
        type AuditAuthorityOrigin: EnsureOrigin<Self::RuntimeOrigin>;

        /// Max byte length of the canonical-CBOR `body` carried in
        /// [`SignedAggregateWire`]. Recommended 8 KiB.
        #[pallet::constant]
        type MaxAggregateBody: Get<u32>;

        /// Max byte length of `validator_id` inside
        /// [`AggregateView`].
        #[pallet::constant]
        type MaxValidatorIdLen: Get<u32>;

        /// Max byte length of `family_id` inside [`AggregateView`].
        #[pallet::constant]
        type MaxFamilyIdLen: Get<u32>;

        /// Max byte length of `audit_vm_key_id` inside
        /// [`AggregateView`].
        #[pallet::constant]
        type MaxAuditVmKeyIdLen: Get<u32>;

        /// Pallet-instance discriminator pinned by the runtime.
        #[pallet::constant]
        type ComputePalletInstance: Get<[u8; 32]>;

        /// Chain genesis discriminator pinned by the runtime —
        /// every [`AggregateView::chain_genesis`] must equal this
        /// value. The runtime MUST wire this to the actual chain
        /// genesis hash (NOT `frame_system::block_hash(0)`, which
        /// gets pruned by `BlockHashCount`).
        #[pallet::constant]
        type ComputeChainGenesis: Get<[u8; 32]>;

        /// Unix-time source used to enforce
        /// [`AggregateView::expiry`]. Production runtimes wire this
        /// to `pallet_timestamp`'s `UnixTime` impl.
        type NowUnix: frame_support::traits::UnixTime;

        // --- PR-I4 vali_submit_epoch_close config ---

        /// Max number of [`MinerStatusUpdate`] rows per
        /// [`Pallet::vali_submit_epoch_close`] call. Bounds the
        /// extrinsic's weight + the storage write fan-out.
        #[pallet::constant]
        type MaxMinerStatusUpdatesPerCall: Get<u32>;

        // --- #322 live attestation config ---

        /// Max byte length of the canonical-CBOR `body` carried in
        /// [`SignedLiveAttestationWire`]. The `hippius_types::
        /// live_attestation::LiveAttestation` body is ~360 B
        /// canonical-encoded; 1024 leaves headroom for schema growth
        /// without busting the mempool's per-extrinsic limit.
        #[pallet::constant]
        type MaxLiveAttestationBody: Get<u32>;

        /// Max byte length of `vm_id` inside
        /// [`LiveAttestationView`]. Production VMs use ~10-char
        /// tenant-prefixed ids (`tnXxxxxx…`); 64 leaves headroom for
        /// long-lived ids without growing the storage key size.
        #[pallet::constant]
        type MaxVmIdLen: Get<u32>;

        /// Max number of KBS L0 verifying keys the runtime root may
        /// allowlist via [`Pallet::set_kbs_attestation_pubkey`]. A
        /// small bound (recommended 4) is enough for the single
        /// production KBS deployment + 1-2 rollover keys; growing
        /// past this is a config change, not an on-chain limit.
        #[pallet::constant]
        type MaxKbsAttestationPubkeys: Get<u32>;
    }

    // =====================================================================
    // Pallet + Storage
    // =====================================================================

    #[pallet::pallet]
    #[pallet::without_storage_info]
    pub struct Pallet<T>(_);

    /// Current epoch — written by
    /// [`Pallet::vali_submit_epoch_close`] (must be strictly
    /// increasing). Read by [`Pallet::submit_audit_stats`] to
    /// enforce `view.epoch == CurrentEpoch::get()`.
    #[pallet::storage]
    pub type CurrentEpoch<T> = StorageValue<_, u64, ValueQuery>;

    // --- PR-I4 (NEW) ---

    /// §13 status row per node. **Single row** — no per-epoch
    /// history on-chain (off-chain audit log is the source of
    /// truth for transitions over time).
    #[pallet::storage]
    pub type MinerStatuses<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 32], MinerStatusEntry<BlockNumberFor<T>>, OptionQuery>;

    /// §23 reward weight per node per epoch. **Authoritative**
    /// reward input that an off-chain ranking pallet pulls from
    /// to compute `f`. Indexed as `(epoch, node_id) → Some(u128)`.
    /// Written only by [`Pallet::vali_submit_epoch_close`].
    ///
    /// TODO: `EpochWeights` grows indefinitely. A future PR should
    /// introduce a pruning strategy (e.g. retaining only the last
    /// N epochs) to prevent state bloat.
    ///
    /// `OptionQuery` (not `ValueQuery`) so the ranking reader can
    /// distinguish:
    /// - `None` → the validator did NOT report this node this
    ///   epoch (off-chain accounting decides: skipped? offline?
    ///   anti-Sybil suspended?),
    /// - `Some(0)` → the validator DID report and the node earned
    ///   zero reward weight this window (e.g. status was
    ///   Quarantined; an explicit "no reward" verdict).
    #[pallet::storage]
    pub type EpochWeights<T> =
        StorageDoubleMap<_, Blake2_128Concat, u64, Blake2_128Concat, [u8; 32], u128, OptionQuery>;

    // --- PR-I3 (kept) ---

    /// Registered audit-VM Ed25519 public key for each node. Set
    /// by the admin via [`Pallet::set_audit_vm_pubkey`] (PR-I3
    /// placeholder; PR-I6 wires the KBS-issued certificate flow).
    #[pallet::storage]
    pub type AuditVmPubkeyByNode<T> =
        StorageMap<_, Blake2_128Concat, [u8; 32], [u8; 32], OptionQuery>;

    /// SHA-256 of the latest accepted aggregate body for each
    /// node — chained: every new aggregate MUST carry this as its
    /// `prev_aggregate_hash`. Empty / missing means "no aggregate
    /// accepted yet" (the genesis aggregate uses
    /// `prev_aggregate_hash = [0u8; 32]`).
    #[pallet::storage]
    pub type LastAggregateHashByNode<T> =
        StorageMap<_, Blake2_128Concat, [u8; 32], [u8; 32], OptionQuery>;

    // --- #322 live attestation ---

    /// Root-allowlisted KBS L0 verifying keys. A
    /// [`Pallet::submit_live_attestation`] call MUST carry a
    /// `view.signer_pubkey` present in this list — closes the
    /// "untrusted submitter posts a self-signed attestation" hole.
    /// Maintained by [`Pallet::set_kbs_attestation_pubkey`] +
    /// [`Pallet::remove_kbs_attestation_pubkey`] (admin origin).
    #[pallet::storage]
    pub type KbsAttestationPubkeys<T: Config> =
        StorageValue<_, BoundedVec<[u8; 32], T::MaxKbsAttestationPubkeys>, ValueQuery>;

    /// Per-VM replay-chain state + uptime probe — see
    /// [`LiveAttestationRecord`]. Key is the Blake2_128Concat hash
    /// of the `vm_id` bytes (NOT the raw `vm_id`, which is
    /// variable-length and not a fixed-size key).
    #[pallet::storage]
    pub type LastLiveAttestation<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 32], LiveAttestationRecord, OptionQuery>;

    /// Per-epoch live-attestation count, indexed by
    /// `(epoch, vm_id_hash) → count`. Drives the uptime ratio at
    /// epoch close: the §322 reward function reads
    /// `LiveAttestationCount(epoch, vm) / expected_attestations(epoch)`
    /// per VM, and scales the per-miner weight accordingly. Written
    /// by [`Pallet::submit_live_attestation`], read by the future
    /// epoch-close weight scaler (#322 PR 4).
    ///
    /// `ValueQuery` (default = 0) so a missing entry reads as "no
    /// attestation observed", which is the right uptime signal — a
    /// VM that never reported is treated identically to one that
    /// reported zero times.
    #[pallet::storage]
    pub type LiveAttestationCount<T: Config> =
        StorageDoubleMap<_, Blake2_128Concat, u64, Blake2_128Concat, [u8; 32], u32, ValueQuery>;

    // --- §13 anti-Sybil registration ---

    /// Whether registration lockup (reserve/unbond) is enabled.
    #[pallet::storage]
    pub type LockupEnabled<T> = StorageValue<_, bool, ValueQuery>;

    /// Runtime-configurable base deposit (floor for the global
    /// fee curve).
    #[pallet::storage]
    pub type BaseChildDepositValue<T: Config> = StorageValue<_, BalanceOf<T>, ValueQuery>;

    /// Number of families that have claimed the "first child
    /// free" slot.
    #[pallet::storage]
    pub type FamilyCount<T> = StorageValue<_, u32, ValueQuery>;

    /// Whether a family has already used its one-time free
    /// registration.
    #[pallet::storage]
    pub type FamilyUsedFreeSlot<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, bool, ValueQuery>;

    /// How many fee-free child registrations this family has
    /// consumed (lifetime).
    #[pallet::storage]
    pub type FamilyFreeSlotsUsed<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u32, OptionQuery>;

    /// Admin-configurable number of fee-free registrations per
    /// family before the global paid curve.
    #[pallet::storage]
    pub type FreeChildSlotsPerFamily<T> = StorageValue<_, u32, OptionQuery>;

    /// Active children count per family.
    #[pallet::storage]
    pub type FamilyActiveChildren<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, u32, ValueQuery>;

    /// Total active children across all families.
    #[pallet::storage]
    pub type TotalActiveChildren<T> = StorageValue<_, u32, ValueQuery>;

    /// Global next required deposit for paid child registration
    /// (network-wide adaptive fee).
    #[pallet::storage]
    pub type GlobalNextDeposit<T: Config> = StorageValue<_, BalanceOf<T>, ValueQuery>;

    /// Last block when a paid child registration happened (for
    /// lazy halving).
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

    /// Node id → current child (active only).
    #[pallet::storage]
    pub type NodeIdToChild<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 32], T::AccountId, OptionQuery>;

    /// Prevent replay across (de)registration cycles: nonce per
    /// node id.
    #[pallet::storage]
    pub type NodeIdNonce<T: Config> = StorageMap<_, Blake2_128Concat, [u8; 32], u64, ValueQuery>;

    /// Cooldown until (block number) for child accounts (after
    /// deregistration).
    #[pallet::storage]
    pub type ChildCooldownUntil<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, BlockNumberFor<T>, ValueQuery>;

    /// Cooldown until for node ids (after deregistration).
    #[pallet::storage]
    pub type NodeIdCooldownUntil<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 32], BlockNumberFor<T>, ValueQuery>;

    /// Active children list per family.
    #[pallet::storage]
    pub type FamilyChildren<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        BoundedVec<T::AccountId, T::MaxChildrenPerFamily>,
        ValueQuery,
    >;

    // =====================================================================
    // PR-1 — $-denominated, hosting-proportional STAKE
    //
    // A miner posts native-coin collateral sized in USD value-at-risk, so
    // its skin-in-the-game tracks what it ACTUALLY hosts (not advertised
    // capacity). The price is admin-posted (a trusted relayer off a DEX
    // TWAP, phase-1) and smoothed by an ASYMMETRIC EMA: fast to react when
    // the coin dumps (required stake climbs quickly → top-up), slow when it
    // pumps (unlockable stake grows only gradually). This gives the
    // conservative "lock more fast, release slow" posture WITHOUT a hard
    // ratchet that would freeze a miner's capital forever.
    //
    // Default-OFF (`StakeEnabled = false`): the oracle/EMA track and
    // staking works, but [`Pallet::is_stake_sufficient`] never blocks until
    // ops flips the switch — mainnet behaviour is unchanged on upgrade.
    // See docs/design/marketplace-stake-slashing.md.
    // =====================================================================

    /// EMA smoothing (per-mille) when `alpha_per_usd` RISES (coin dumps).
    #[pallet::type_value]
    pub fn DefaultEmaDownPermille() -> u32 {
        300
    }
    /// EMA smoothing (per-mille) when `alpha_per_usd` FALLS (coin pumps).
    #[pallet::type_value]
    pub fn DefaultEmaUpPermille() -> u32 {
        50
    }

    /// Master switch for the stake-eligibility layer. While `false` the
    /// oracle/EMA still track and staking still works, but
    /// [`Pallet::is_stake_sufficient`] always returns `true` — the layer is
    /// inert (mirrors [`LockupEnabled`]).
    #[pallet::storage]
    pub type StakeEnabled<T> = StorageValue<_, bool, ValueQuery>;

    /// Last admin-posted native-coin price, fixed-point `alpha_per_usd ×
    /// 1e6` (how many native units buy 1 USD). No floats on-chain.
    #[pallet::storage]
    pub type AlphaPerUsdSpot<T> = StorageValue<_, u128, ValueQuery>;

    /// Asymmetric EMA of [`AlphaPerUsdSpot`] — the value the stake math
    /// uses. Bootstraps to the first posted spot.
    #[pallet::storage]
    pub type AlphaPerUsdEma<T> = StorageValue<_, u128, ValueQuery>;

    /// EMA per-mille factor applied when the coin dumps (fast).
    #[pallet::storage]
    pub type EmaDownPermille<T> = StorageValue<_, u32, ValueQuery, DefaultEmaDownPermille>;

    /// EMA per-mille factor applied when the coin pumps (slow).
    #[pallet::storage]
    pub type EmaUpPermille<T> = StorageValue<_, u32, ValueQuery, DefaultEmaUpPermille>;

    /// Minimum stake (native) to be eligible at all — the cold-start entry
    /// ticket + anti-Sybil floor. Applied per owner account.
    #[pallet::storage]
    pub type StakeFloor<T: Config> = StorageValue<_, BalanceOf<T>, ValueQuery>;

    /// Active stake reserved per owner account.
    #[pallet::storage]
    pub type StakedAmount<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, BalanceOf<T>, ValueQuery>;

    /// Pending stake unbond per owner: `(amount, earliest claimable
    /// block)`. The amount stays RESERVED (slashable) until claimed.
    #[pallet::storage]
    pub type StakeUnbonding<T: Config> = StorageMap<
        _,
        Blake2_128Concat,
        T::AccountId,
        (BalanceOf<T>, BlockNumberFor<T>),
        OptionQuery,
    >;

    // =====================================================================
    // PR-2 — SLASHING (kill-switch gated)
    //
    // The off-chain validator detects a breach (a miner went dark past the
    // liveness window, or stayed stake-deficient past grace) exactly as it
    // already drives `set_miner_status` / `vali_submit_epoch_close`, and
    // submits `slash_stake(owner, amount, reason)`. The authority is
    // on-chain; the detection is off-chain. Slash is sized off value-at-
    // risk so a whale that "takes all then goes dark" loses proportionally.
    //
    // Global kill-switch `SlashingEnabled` (mirrors `LockupEnabled`):
    // default OFF ⇒ a breach only emits `SlashSkippedDisabled`, NO balance
    // is ever burned. Slashed funds either burn (`slash_reserved`) or are
    // repatriated to `SlashBeneficiary` (a tenant-compensation escrow) when
    // set. See docs/design/marketplace-stake-slashing.md §4.
    // =====================================================================

    /// Why a stake was slashed (carried on the event + for audit).
    #[derive(Clone, Copy, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
    pub enum SlashReason {
        /// Miner went dark — no live attestation within the window.
        Liveness,
        /// Quarantined for cause / SLA breach.
        Sla,
        /// Stayed stake-deficient past the top-up grace.
        StakeDeficiency,
        /// Governance / manual action.
        Manual,
    }

    /// Master switch for slashing. While `false`, a breach is recorded as
    /// an event but NO balance is burned (mirrors [`LockupEnabled`]).
    #[pallet::storage]
    pub type SlashingEnabled<T> = StorageValue<_, bool, ValueQuery>;

    /// Where slashed funds go. `None` ⇒ burn (`slash_reserved`); `Some(b)`
    /// ⇒ repatriate to `b`'s free balance (tenant-compensation escrow).
    #[pallet::storage]
    pub type SlashBeneficiary<T: Config> = StorageValue<_, T::AccountId, OptionQuery>;

    /// Cumulative stake ACTUALLY slashed per owner (audit trail; only
    /// incremented when a burn/repatriation truly happened).
    #[pallet::storage]
    pub type SlashRecord<T: Config> =
        StorageMap<_, Blake2_128Concat, T::AccountId, BalanceOf<T>, ValueQuery>;

    // =====================================================================
    // PR-3 — MARKETPLACE pricing (miner-set, capped, ANNOUNCED ahead)
    //
    // A miner prices its own capacity (USD per resource-unit, fixed-point
    // ×1e6). Price is ONE selection input (vali folds it in), never the
    // only one. A change does NOT take effect immediately: it is ANNOUNCED
    // on-chain `PriceChangeNoticeBlocks` ahead, and is both rate-limited
    // (`MinPriceChangeIntervalBlocks`) and magnitude-bounded
    // (`MaxPriceChangeNumer/Denom`, e.g. 3/2 ⇒ ±50% per step, no doubling)
    // and clamped to `[PriceFloor, PriceCeiling]`. vali watches the
    // announcement and migrates any VM that no longer fits the tenant's
    // budget BEFORE the new price bites. See design doc §3.
    // =====================================================================

    /// A scheduled price change for a node.
    #[derive(Clone, Encode, Decode, PartialEq, Eq, RuntimeDebug, TypeInfo, MaxEncodedLen)]
    pub struct PriceChange<BlockNumber> {
        pub new_price: u128,
        /// First block at which `new_price` becomes effective.
        pub effective_block: BlockNumber,
    }

    /// Default price-change notice window (blocks ahead a change is
    /// announced) — ~2 days at 12s/block.
    #[pallet::type_value]
    pub fn DefaultPriceNoticeBlocks<T: Config>() -> BlockNumberFor<T> {
        14_400u32.into()
    }
    /// Default minimum interval between a node's price changes — ~1 day.
    #[pallet::type_value]
    pub fn DefaultMinPriceChangeInterval<T: Config>() -> BlockNumberFor<T> {
        7_200u32.into()
    }
    /// Default max-change ratio numerator (3/2 ⇒ at most ×1.5 / ÷1.5).
    #[pallet::type_value]
    pub fn DefaultMaxPriceChangeNumer() -> u32 {
        3
    }
    /// Default max-change ratio denominator.
    #[pallet::type_value]
    pub fn DefaultMaxPriceChangeDenom() -> u32 {
        2
    }

    /// Current effective USD price per resource-unit (fixed-point ×1e6)
    /// for a node. Absent ⇒ the miner hasn't priced yet.
    #[pallet::storage]
    pub type MinerPrice<T: Config> = StorageMap<_, Blake2_128Concat, [u8; 32], u128, OptionQuery>;

    /// An announced-but-not-yet-effective price change for a node.
    #[pallet::storage]
    pub type PendingPriceChange<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 32], PriceChange<BlockNumberFor<T>>, OptionQuery>;

    /// Block of a node's last price-change ANNOUNCEMENT (rate limiter).
    #[pallet::storage]
    pub type LastPriceChangeBlock<T: Config> =
        StorageMap<_, Blake2_128Concat, [u8; 32], BlockNumberFor<T>, ValueQuery>;

    /// Anti-dumping floor on any price (`0` ⇒ no floor).
    #[pallet::storage]
    pub type PriceFloor<T> = StorageValue<_, u128, ValueQuery>;

    /// Anti-gouging ceiling on any price (`None` ⇒ unbounded).
    #[pallet::storage]
    pub type PriceCeiling<T> = StorageValue<_, u128, OptionQuery>;

    /// Notice window (blocks) a price change is announced ahead.
    #[pallet::storage]
    pub type PriceChangeNoticeBlocks<T: Config> =
        StorageValue<_, BlockNumberFor<T>, ValueQuery, DefaultPriceNoticeBlocks<T>>;

    /// Minimum blocks between a node's successive price changes.
    #[pallet::storage]
    pub type MinPriceChangeIntervalBlocks<T: Config> =
        StorageValue<_, BlockNumberFor<T>, ValueQuery, DefaultMinPriceChangeInterval<T>>;

    /// Max-change ratio numerator/denominator (e.g. 3/2 ⇒ ±50% per step).
    #[pallet::storage]
    pub type MaxPriceChangeNumer<T> = StorageValue<_, u32, ValueQuery, DefaultMaxPriceChangeNumer>;
    #[pallet::storage]
    pub type MaxPriceChangeDenom<T> = StorageValue<_, u32, ValueQuery, DefaultMaxPriceChangeDenom>;

    // =====================================================================
    // Genesis
    // =====================================================================

    #[pallet::genesis_config]
    #[derive(frame_support::DefaultNoBound)]
    pub struct GenesisConfig<T: Config> {
        pub base_child_deposit: Option<BalanceOf<T>>,
        pub lockup_enabled: bool,
        /// Anti-dumping floor on any miner price. `0` (the default) ⇒ NO
        /// floor.
        pub price_floor: u128,
        /// Anti-gouging ceiling on any miner price. `None` (the default)
        /// ⇒ UNBOUNDED.
        ///
        /// ## Why these are here
        ///
        /// [`Pallet::set_price_bounds`] has always existed, and
        /// [`Pallet::announce_price_change`] has always enforced the
        /// bounds — but a freshly deployed chain starts with
        /// `PriceFloor = 0` / `PriceCeiling = None`, so prices are
        /// unbounded until somebody remembers to make that admin call.
        ///
        /// Nobody did. On the live testnet miners carried self-set prices
        /// of 5e6 / 1.5e6 / 0.5e6 µUSD per resource-unit-hour with no
        /// bound of any kind, and because `scoring.resource_units` scales
        /// by ×1000 that priced a single `small` VM at ~7,950 USD/hour.
        /// The bound was not missing from the code; it was missing from
        /// the chain, and nothing surfaced that.
        ///
        /// Putting them in genesis makes the choice EXPLICIT in the chain
        /// spec: a deployment that wants unbounded prices now has to say
        /// so, rather than getting it by forgetting. The defaults stay
        /// permissive so an existing chain spec keeps its exact current
        /// behaviour.
        pub price_ceiling: Option<u128>,
    }

    #[pallet::genesis_build]
    impl<T: Config> BuildGenesisConfig for GenesisConfig<T> {
        fn build(&self) {
            if let Some(deposit) = &self.base_child_deposit {
                BaseChildDepositValue::<T>::put(deposit);
            }
            LockupEnabled::<T>::put(self.lockup_enabled);
            // Same invariant `set_price_bounds` enforces at runtime. A
            // chain spec that inverts them is a spec BUG, and genesis is
            // the one place where panicking is the correct response:
            // better a chain that refuses to start than one that starts
            // with a price policy nobody can satisfy.
            if let Some(ceiling) = self.price_ceiling {
                assert!(
                    self.price_floor <= ceiling,
                    "compute-scoring genesis: price_floor exceeds price_ceiling"
                );
                PriceCeiling::<T>::put(ceiling);
            }
            PriceFloor::<T>::put(self.price_floor);
        }
    }

    // =====================================================================
    // Events + Errors
    // =====================================================================

    #[pallet::event]
    #[pallet::generate_deposit(pub(super) fn deposit_event)]
    pub enum Event<T: Config> {
        // --- §13 anti-Sybil registration ---
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
        /// A child's deposit was unbonded and released.
        ChildUnbonded {
            family: T::AccountId,
            child: T::AccountId,
            node_id: [u8; 32],
            amount: BalanceOf<T>,
        },

        // --- Admin ---
        /// Registration lockup was enabled/disabled by admin.
        LockupEnabledSet { enabled: bool },
        /// Base child deposit floor was set by admin.
        BaseChildDepositSet { deposit: BalanceOf<T> },
        /// Number of fee-free child registrations per family was
        /// set by admin.
        FreeChildSlotsPerFamilySet { slots: u32 },

        // --- PR-I3 audit-stats ---
        /// An audit-VM-signed aggregate was accepted. `served_units`
        /// is the §23 quantity carried on the wire (NOT credited
        /// to per-node storage — see CHANGES.md PR-I4: the
        /// validator aggregates served-units off-chain and writes
        /// them via `vali_submit_epoch_close`). `body_hash` is the
        /// SHA-256 of the canonical CBOR body, stored as the new
        /// `prev_aggregate_hash` for chain continuity.
        AuditStatsSubmitted {
            node_id: [u8; 32],
            epoch: u64,
            served_units: u128,
            body_hash: [u8; 32],
        },
        /// Admin set the audit-VM Ed25519 pubkey for a node.
        AuditVmPubkeySet { node_id: [u8; 32], pubkey: [u8; 32] },

        // --- PR-I4 vali_submit_epoch_close ---
        /// A node's `MinerStatus` changed. Emitted **only on
        /// transitions** (the validator may submit a status update
        /// every epoch for liveness, but identical-status
        /// heartbeats stay silent — see CHANGES.md PR-I4 spec).
        MinerStatusChanged {
            node_id: [u8; 32],
            old_status: MinerStatus,
            new_status: MinerStatus,
            epoch: u64,
        },
        /// An epoch was closed: `EpochWeights[epoch][*]` is now
        /// authoritative and read by the off-chain ranking pallet.
        EpochClosed { epoch: u64, updates: u32 },

        // --- #322 live attestation ---
        /// A KBS L0 verifying key was added to the
        /// [`KbsAttestationPubkeys`] allowlist by admin.
        KbsAttestationPubkeyAdded { pubkey: [u8; 32] },
        /// A KBS L0 verifying key was removed from the
        /// [`KbsAttestationPubkeys`] allowlist by admin.
        KbsAttestationPubkeyRemoved { pubkey: [u8; 32] },
        /// A KBS-signed live attestation was accepted for a tenant
        /// CVM. `body_hash` is the SHA-256 of the canonical CBOR
        /// body, stored as the new `prev_attestation_hash` for the
        /// per-VM replay chain. `vm_id_hash` is the Blake2_128Concat
        /// hash of the `vm_id` bytes — same key the
        /// [`LastLiveAttestation`] storage uses.
        LiveAttestationSubmitted {
            vm_id_hash: [u8; 32],
            node_id: [u8; 32],
            epoch: u64,
            attestation_seq: u64,
            body_hash: [u8; 32],
        },

        // --- PR-1 stake ---
        /// The stake-eligibility layer was enabled/disabled by admin.
        StakeEnabledSet { enabled: bool },
        /// The native-coin USD price was posted; `ema` is the new
        /// asymmetric-EMA value the stake math now uses.
        AlphaPerUsdUpdated { spot: u128, ema: u128 },
        /// The minimum stake floor was set by admin.
        StakeFloorSet { floor: BalanceOf<T> },
        /// The asymmetric-EMA smoothing factors (per-mille) were set.
        EmaPermilleSet { down: u32, up: u32 },
        /// An owner reserved additional stake collateral.
        StakeToppedUp {
            who: T::AccountId,
            amount: BalanceOf<T>,
            total: BalanceOf<T>,
        },
        /// An owner began unbonding stake; it stays reserved (slashable)
        /// until `unbonding_end`.
        StakeUnbondRequested {
            who: T::AccountId,
            amount: BalanceOf<T>,
            unbonding_end: BlockNumberFor<T>,
        },
        /// Gap #2 — an owner signalled exit via `request_unstake` (unbonded
        /// everything, or dropped below the eligibility floor while staking is
        /// live) and `nodes` of their §13 compute nodes were transitioned to
        /// `Quarantined` so the off-chain validator drains them during the
        /// unbonding period. `nodes` counts only nodes that ACTUALLY
        /// transitioned (already-Quarantined nodes are not re-counted).
        OwnerQuarantinedOnUnstake { who: T::AccountId, nodes: u32 },
        /// Fully-unbonded stake was unreserved and released.
        StakeReleased {
            who: T::AccountId,
            amount: BalanceOf<T>,
        },

        // --- PR-2 slashing ---
        /// Slashing was enabled/disabled by admin.
        SlashingEnabledSet { enabled: bool },
        /// The slash beneficiary was set (`None` ⇒ burn).
        SlashBeneficiarySet { beneficiary: Option<T::AccountId> },
        /// Stake was slashed: `amount` burned or repatriated.
        Slashed {
            owner: T::AccountId,
            amount: BalanceOf<T>,
            reason: SlashReason,
        },
        /// A breach was reported but slashing is disabled — `amount` is the
        /// stake that WOULD have been slashed. No balance moved.
        SlashSkippedDisabled {
            owner: T::AccountId,
            amount: BalanceOf<T>,
            reason: SlashReason,
        },

        // --- PR-3 marketplace ---
        /// A node announced a price change effective at `effective_block`.
        /// vali migrates VMs that no longer fit before then.
        PriceChangeAnnounced {
            node_id: [u8; 32],
            old_price: Option<u128>,
            new_price: u128,
            effective_block: BlockNumberFor<T>,
        },
        /// A previously-announced price change became effective.
        PriceChangeApplied { node_id: [u8; 32], new_price: u128 },
        /// Admin set the price floor/ceiling.
        PriceBoundsSet { floor: u128, ceiling: Option<u128> },
        /// Admin set the price-change policy (rate + magnitude + notice).
        PriceChangePolicySet {
            min_interval: BlockNumberFor<T>,
            notice: BlockNumberFor<T>,
            max_numer: u32,
            max_denom: u32,
        },
    }

    #[pallet::error]
    pub enum Error<T> {
        // --- §13 anti-Sybil registration ---
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
        /// Reserved balance was lower than expected; full deposit
        /// could not be unreserved.
        PartialUnreserve,
        /// `free_child_slots_per_family` exceeds
        /// [`Config::MaxChildrenPerFamily`].
        FreeChildSlotsPerFamilyTooLarge,
        /// Raised by `submit_audit_stats` when the aggregate's
        /// `node_id` isn't registered in the `T::Registration`
        /// provider, AND by `force_deregister_child` when the
        /// caller's account has no registration record at all
        /// (vs the distinct `NotAValidator` variant for accounts
        /// that ARE registered but as a non-Validator node type).
        /// Test runtimes binding the trait to `()` will trip
        /// `NodeNotRegistered` on every `force_deregister_child`
        /// call — the fail-closed default.
        NodeNotRegistered,
        /// `force_deregister_child`: the caller IS a registered
        /// owner but their primary node is NOT a
        /// `pallet_registration::NodeType::Validator`. Distinct
        /// from `NodeNotRegistered` so block explorers /
        /// indexers can tell "unknown account" apart from
        /// "registered miner trying to force-deregister".
        NotAValidator,

        // --- PR-I3 audit-stats ---
        /// `AggregateView.chain_genesis` doesn't match
        /// `T::ComputeChainGenesis::get()`.
        AggregateChainGenesisMismatch,
        /// `AggregateView.pallet_instance` doesn't match
        /// `T::ComputePalletInstance::get()`.
        AggregatePalletInstanceMismatch,
        /// `AggregateView.epoch` doesn't match `CurrentEpoch::get()`.
        AggregateEpochMismatch,
        /// `AggregateView.prev_aggregate_hash` doesn't match the
        /// stored last-aggregate-hash for this node.
        AggregatePrevHashMismatch,
        /// `AggregateView.expiry` is in the past relative to the
        /// chain's Unix-time source.
        AggregateExpired,
        /// `AggregateView.interval_end < interval_start`.
        AggregateIntervalInverted,
        /// `AggregateView.node_id` has no registered audit-VM
        /// pubkey in `AuditVmPubkeyByNode`.
        AuditVmPubkeyNotRegistered,
        /// Ed25519 verification of `signed.sig` over `signed.body`
        /// failed.
        InvalidAggregateSignature,
        /// `signed.body` is empty.
        EmptyAggregateBody,
        /// `signed.body` is not the canonical-CBOR encoding of the
        /// submitted `view` — the signature therefore covers
        /// different facts than the ones this call would act on.
        /// Without this gate one honestly-signed body could be
        /// replayed under arbitrary `view` fields (found in review of
        /// thebrain#49).
        AggregateBodyViewMismatch,

        // --- PR-I4 vali_submit_epoch_close ---
        /// `epoch` is not strictly increasing relative to the
        /// stored `CurrentEpoch`.
        EpochRegression,
        /// Duplicate `node_id` inside a single
        /// `vali_submit_epoch_close` batch — the validator MUST
        /// dedupe before submission so a single transaction can't
        /// double-write `EpochWeights[epoch][node]`.
        DuplicateNodeInBatch,

        // --- #322 live attestation ---
        /// `signed.body` is empty.
        EmptyLiveAttestationBody,
        /// `view.chain_genesis` doesn't match
        /// `T::ComputeChainGenesis::get()`.
        LiveAttestationChainGenesisMismatch,
        /// `view.pallet_instance` doesn't match
        /// `T::ComputePalletInstance::get()`.
        LiveAttestationPalletInstanceMismatch,
        /// `view.epoch` doesn't match `CurrentEpoch::get()`.
        LiveAttestationEpochMismatch,
        /// `view.expiry_unix` is at-or-before the chain's Unix
        /// timestamp.
        LiveAttestationExpired,
        /// `view.verified_at_unix < view.observed_at_unix`.
        LiveAttestationWindowInverted,
        /// `view.attestation_seq` is not exactly
        /// `LastLiveAttestation.attestation_seq + 1` (or `1` for the
        /// very first attestation of a VM).
        LiveAttestationSeqMismatch,
        /// `view.prev_attestation_hash` doesn't match the stored
        /// `LastLiveAttestation.body_hash` (or the all-zero
        /// sentinel for the genesis attestation).
        LiveAttestationPrevHashMismatch,
        /// `view.signer_pubkey` is not in the
        /// [`KbsAttestationPubkeys`] allowlist.
        LiveAttestationKbsNotAllowed,
        /// Ed25519 verification of `signed.sig` over `signed.body`
        /// (against `view.signer_pubkey`) failed.
        InvalidLiveAttestationSignature,
        /// `signed.body` did not decode as a canonical-CBOR
        /// `LiveAttestation` (hippius-types schema).
        MalformedLiveAttestationBody,
        /// The decoded `signed.body` disagrees with the submitted
        /// `view` on at least one field this call acts on. Without
        /// this gate one honestly-KBS-signed body could be credited
        /// to any VM / node / epoch with a forged `view` (found in
        /// review of thebrain#49).
        LiveAttestationBodyViewMismatch,
        /// The KBS L0 pubkey is already in the allowlist (admin
        /// `set_kbs_attestation_pubkey`).
        KbsAttestationPubkeyAlreadyAllowed,
        /// The KBS L0 pubkey is not in the allowlist (admin
        /// `remove_kbs_attestation_pubkey`).
        KbsAttestationPubkeyNotAllowed,
        /// The allowlist is full at the configured
        /// `T::MaxKbsAttestationPubkeys` bound.
        KbsAttestationPubkeysFull,

        // --- PR-1 stake ---
        /// `set_alpha_per_usd` was called with a zero price.
        InvalidPrice,
        /// An EMA per-mille factor was 0 or > 1000.
        InvalidEmaFactor,
        /// A stake amount was zero.
        ZeroStake,
        /// The caller's free balance can't cover the requested stake.
        InsufficientBalanceForStake,
        /// The caller's active stake is less than the requested unstake.
        InsufficientStake,
        /// No pending stake unbond exists for the caller.
        NoStakeUnbonding,

        // --- PR-3 marketplace ---
        /// The caller is not the registered operator of the node.
        NotNodeOperator,
        /// The proposed price is outside `[PriceFloor, PriceCeiling]`.
        PriceOutOfBounds,
        /// The price change exceeds the allowed per-step magnitude.
        PriceChangeTooLarge,
        /// A price change was announced too soon after the previous one.
        PriceChangeTooSoon,
        /// An invalid price-change policy (bad ratio / zero denom).
        InvalidPricePolicy,
        /// No pending price change is due for application.
        NoPriceChangeDue,
    }

    // =====================================================================
    // Internal helpers
    // =====================================================================

    impl<T: Config> Pallet<T> {
        fn now() -> BlockNumberFor<T> {
            <frame_system::Pallet<T>>::block_number()
        }

        /// Transition a node's §13 status: materialise only non-default
        /// (non-`Active`) rows, emit [`Event::MinerStatusChanged`] on a real
        /// transition, and return `true` iff the status actually changed.
        ///
        /// The effective previous status when no row exists is
        /// `MinerStatus::Active` — a node that just registered via the §13
        /// anti-Sybil flow is implicitly Active until the validator says
        /// otherwise. Recovery to `Active` REMOVES the row so it doesn't leave
        /// an explicit `Active` row contradicting the "default-Active implicit
        /// / only non-default rows materialise" invariant; identical-status
        /// re-asserts (heartbeats) are silent and write nothing.
        ///
        /// Shared by [`Pallet::vali_submit_epoch_close`] (the per-epoch status
        /// submitter) and the graceful-exit quarantine in
        /// [`Pallet::request_unstake`].
        fn transition_node_status(
            node_id: [u8; 32],
            new_status: MinerStatus,
            now: BlockNumberFor<T>,
            epoch: u64,
        ) -> bool {
            let prev = MinerStatuses::<T>::get(node_id);
            let prev_status = prev
                .as_ref()
                .map(|e| e.status)
                .unwrap_or(MinerStatus::Active);
            if prev_status == new_status {
                // Heartbeat — no storage write, no event.
                return false;
            }
            // Recovery to default-Active REMOVES the row so a node that
            // recovers from Quarantined back to Active doesn't leave an
            // explicit Active row contradicting the "default-Active implicit /
            // only non-default rows materialise" invariant. The event still
            // fires — recovery IS a transition.
            if new_status == MinerStatus::Active {
                MinerStatuses::<T>::remove(node_id);
            } else {
                MinerStatuses::<T>::insert(
                    node_id,
                    MinerStatusEntry {
                        status: new_status,
                        last_transition_block: now,
                        last_transition_epoch: epoch,
                    },
                );
            }
            Self::deposit_event(Event::MinerStatusChanged {
                node_id,
                old_status: prev_status,
                new_status,
                epoch,
            });
            true
        }

        // --- PR-1 stake helpers ---

        /// One asymmetric-EMA step over `alpha_per_usd` (fixed-point ×1e6).
        /// `spot > ema` means the coin DUMPED (more alpha buys $1) → react
        /// fast with `EmaDownPermille`; `spot < ema` means it PUMPED →
        /// react slow with `EmaUpPermille`. Bootstraps to `spot` when no
        /// EMA is set yet.
        fn ema_step(spot: u128, ema: u128) -> u128 {
            if ema == 0 {
                return spot;
            }
            if spot >= ema {
                let perm = EmaDownPermille::<T>::get() as u128;
                ema.saturating_add(spot.saturating_sub(ema).saturating_mul(perm) / 1000)
            } else {
                let perm = EmaUpPermille::<T>::get() as u128;
                ema.saturating_sub(ema.saturating_sub(spot).saturating_mul(perm) / 1000)
            }
        }

        /// Required native stake for a USD value-at-risk, using the
        /// EMA-smoothed price (`alpha_per_usd × 1e6`). Public so the
        /// off-chain validator / a later on-chain guard can size a miner's
        /// obligation. `0` while no price has been posted.
        pub fn required_alpha(value_at_risk_usd: u128) -> BalanceOf<T> {
            let ema = AlphaPerUsdEma::<T>::get();
            let req = value_at_risk_usd.saturating_mul(ema) / 1_000_000u128;
            req.saturated_into::<BalanceOf<T>>()
        }

        /// `true` iff `owner`'s active stake covers `required` (after the
        /// floor), OR the stake layer is disabled. The single eligibility
        /// predicate the selection path / a later guard gates on.
        pub fn is_stake_sufficient(owner: &T::AccountId, required: BalanceOf<T>) -> bool {
            if !StakeEnabled::<T>::get() {
                return true;
            }
            StakedAmount::<T>::get(owner) >= required.max(StakeFloor::<T>::get())
        }

        /// Slash up to `requested` of `owner`'s stake for `reason`,
        /// returning the amount actually slashed. Draws from active stake
        /// first, then from the (still-reserved) unbonding chunk — so stake
        /// in unbonding stays slashable, per the design invariant.
        ///
        /// Kill-switch: while `SlashingEnabled` is `false` this only emits
        /// [`Event::SlashSkippedDisabled`] and moves no balance. Slashed
        /// funds burn (`slash_reserved`) unless a [`SlashBeneficiary`] is
        /// set (then they are repatriated to it).
        fn do_slash(
            owner: &T::AccountId,
            requested: BalanceOf<T>,
            reason: SlashReason,
        ) -> BalanceOf<T> {
            if !SlashingEnabled::<T>::get() {
                Self::deposit_event(Event::SlashSkippedDisabled {
                    owner: owner.clone(),
                    amount: requested,
                    reason,
                });
                return Zero::zero();
            }

            let active = StakedAmount::<T>::get(owner);
            let unbonding = StakeUnbonding::<T>::get(owner)
                .map(|(a, _)| a)
                .unwrap_or_else(Zero::zero);
            let amount = requested.min(active.saturating_add(unbonding));
            if amount.is_zero() {
                return Zero::zero();
            }

            // Physically remove from the reserved balance FIRST and measure
            // what was ACTUALLY moved — so the logical decrement, the
            // `SlashRecord`, and the event reflect reality even if the
            // reserved balance is somehow short of the logical stake. A
            // repatriation that can't land (e.g. a dead beneficiary) falls
            // back to a burn; both report the un-moved remainder.
            let slashed = match SlashBeneficiary::<T>::get() {
                Some(beneficiary) => {
                    let not_repatriated = T::DepositCurrency::repatriate_reserved(
                        owner,
                        &beneficiary,
                        amount,
                        frame_support::traits::BalanceStatus::Free,
                    )
                    .unwrap_or(amount);
                    let repatriated = amount.saturating_sub(not_repatriated);
                    // Burn whatever couldn't be repatriated.
                    let (_imbalance, burn_remainder) =
                        T::DepositCurrency::slash_reserved(owner, not_repatriated);
                    repatriated.saturating_add(not_repatriated.saturating_sub(burn_remainder))
                }
                None => {
                    let (_imbalance, remainder) = T::DepositCurrency::slash_reserved(owner, amount);
                    amount.saturating_sub(remainder)
                }
            };
            if slashed.is_zero() {
                return Zero::zero();
            }

            // Decrement the logical stake by what was ACTUALLY slashed —
            // active stake first, then the still-reserved unbonding chunk.
            let from_active = slashed.min(active);
            StakedAmount::<T>::insert(owner, active.saturating_sub(from_active));
            let from_unbond = slashed.saturating_sub(from_active);
            if !from_unbond.is_zero() {
                StakeUnbonding::<T>::mutate(owner, |slot| {
                    if let Some((amt, end)) = *slot {
                        let left = amt.saturating_sub(from_unbond);
                        *slot = if left.is_zero() {
                            None
                        } else {
                            Some((left, end))
                        };
                    }
                });
            }

            SlashRecord::<T>::mutate(owner, |r| *r = r.saturating_add(slashed));
            Self::deposit_event(Event::Slashed {
                owner: owner.clone(),
                amount: slashed,
                reason,
            });
            slashed
        }

        // --- PR-3 marketplace helpers ---

        /// The price a consumer should use NOW: the pending price if its
        /// notice window has elapsed, else the current effective price.
        /// Lazy — vali always reads the correct value even before
        /// [`apply_price_change`] has materialised it on-chain.
        pub fn effective_price(node_id: &[u8; 32]) -> Option<u128> {
            if let Some(p) = PendingPriceChange::<T>::get(node_id) {
                if Self::now() >= p.effective_block {
                    return Some(p.new_price);
                }
            }
            MinerPrice::<T>::get(node_id)
        }

        /// `true` iff `price` is within the configured `[floor, ceiling]`.
        fn within_price_bounds(price: u128) -> bool {
            if price < PriceFloor::<T>::get() {
                return false;
            }
            match PriceCeiling::<T>::get() {
                Some(c) => price <= c,
                None => true,
            }
        }

        /// `true` iff moving `current` → `new_price` is within the allowed
        /// per-step magnitude `[current·denom/numer, current·numer/denom]`.
        fn within_price_magnitude(current: u128, new_price: u128) -> bool {
            let numer = MaxPriceChangeNumer::<T>::get() as u128;
            let denom = MaxPriceChangeDenom::<T>::get() as u128;
            let upper = current.saturating_mul(numer) / denom;
            let lower = current.saturating_mul(denom) / numer;
            new_price <= upper && new_price >= lower
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

        /// Fee-free registrations allowed per family when lockup
        /// is enabled (`None` → 1).
        pub fn free_child_slots_per_family() -> u32 {
            FreeChildSlotsPerFamily::<T>::get().unwrap_or(1)
        }

        /// Lifetime count of fee-free registrations consumed by
        /// this family (legacy-aware).
        pub fn family_free_slots_used(family: &T::AccountId) -> u32 {
            if let Some(n) = FamilyFreeSlotsUsed::<T>::get(family) {
                n
            } else if FamilyUsedFreeSlot::<T>::get(family) {
                1
            } else {
                0
            }
        }

        fn global_next_deposit_floor_init() -> BalanceOf<T> {
            let base = Self::base_child_deposit();
            let cur = GlobalNextDeposit::<T>::get();
            if cur.is_zero() || cur < base {
                GlobalNextDeposit::<T>::put(base);
                base
            } else {
                cur
            }
        }

        /// Lazily halves the global next deposit based on time
        /// since last paid registration. Never goes below
        /// `BaseChildDeposit`.
        fn apply_lazy_global_halving() -> BalanceOf<T> {
            let base = Self::base_child_deposit();
            let mut next = Self::global_next_deposit_floor_init();
            let period = T::GlobalDepositHalvingPeriodBlocks::get();
            if period.is_zero() {
                return next;
            }

            let last = GlobalLastPaidRegistrationBlock::<T>::get();
            let elapsed = Self::now().saturating_sub(last);

            let periods: u32 = {
                let e: u128 = TryInto::<u128>::try_into(elapsed).ok().unwrap_or(0);
                let p: u128 = TryInto::<u128>::try_into(period).ok().unwrap_or(0);
                if p == 0 {
                    0
                } else {
                    (e / p) as u32
                }
            };

            // Bound the loop to at most `MAX_HALVING_STEPS` iterations.
            // The `next < base` break already caps it at ~log2(next/base)
            // when `base > 0`, BUT if `base == 0` the break never fires and
            // a long inactivity gap (large `elapsed` / small `period`)
            // would spin `periods` times — a weight/DoS hazard. Capping is
            // mathematically identical: a u128 balance is 0 after ≤128
            // halvings, so any extra step is a no-op.
            const MAX_HALVING_STEPS: u32 = 128;
            for _ in 0..periods.min(MAX_HALVING_STEPS) {
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
            // Domain-separated payload:
            // SCALE("HIPPIUS_COMPUTE_NODE_REG_V1", family, child, node_id, nonce)
            //
            // The miner-agent reproduces this byte-for-byte off-chain to
            // produce the `node_sig` (`binaries/miner-agent/src/identity
            // .rs::registration_message`, the `sign-registration`
            // subcommand). Every component is fixed-size, so the SCALE
            // tuple encoding is a plain concatenation — keep BOTH sides
            // in sync if this payload ever changes (e.g. a v2 domain tag
            // or a variable-length field).
            (
                b"HIPPIUS_COMPUTE_NODE_REG_V1",
                family,
                child,
                node_id,
                nonce,
            )
                .encode()
        }

        fn verify_node_sig(node_id: [u8; 32], message: &[u8], sig: [u8; 64]) -> bool {
            let pk = ed25519::Public::from_raw(node_id);
            let sig = ed25519::Signature::from_raw(sig);
            sig.verify(message, &pk)
        }

        /// Reset family-level registration state when the family
        /// has no active children. Preserves `FamilyUsedFreeSlot`
        /// (anti-yoyo: a family that already claimed its first
        /// free child must not get another).
        fn cleanup_family_when_no_active_children(family: &T::AccountId) {
            if FamilyActiveChildren::<T>::get(family) != 0 {
                return;
            }
            FamilyChildren::<T>::remove(family);
            FamilyActiveChildren::<T>::remove(family);
            FamilyCount::<T>::put(FamilyCount::<T>::get().saturating_sub(1));
        }

        /// PR-I6: read the complete `(node_id, weight)` snapshot
        /// for `epoch` — used by [`Pallet::vali_submit_epoch_close`]
        /// to feed `T::RankingsSink`, and by off-chain validators
        /// (via a runtime-API or direct storage iter) to build the
        /// `pallet_rankings::update_rankings(weights, …)` payload.
        ///
        /// Iteration order is the storage iterator's order
        /// (`Blake2_128Concat`-hashed `node_id`), NOT sorted. A
        /// future PR-I6.x runtime adapter that posts
        /// `update_rankings` may want to sort by `node_id` for
        /// deterministic Vec ordering on the wire; for the
        /// `RankingsSink` push we keep the cheap storage-order
        /// snapshot (the sink can re-sort if it cares).
        pub fn epoch_weights_for(epoch: u64) -> Vec<EpochWeightEntry> {
            EpochWeights::<T>::iter_prefix(epoch)
                .map(|(node_id, weight)| EpochWeightEntry { node_id, weight })
                .collect()
        }
    }

    // =====================================================================
    // Extrinsics
    // =====================================================================

    #[pallet::call]
    impl<T: Config> Pallet<T> {
        // --- §13 anti-Sybil registration ----------------------------------

        /// Register a child node under a family.
        ///
        /// - **Fee-free registrations per family** (default **1**;
        ///   configurable via [`FreeChildSlotsPerFamily`] /
        ///   [`Pallet::set_free_child_slots_per_family`]).
        /// - After those are exhausted, the required deposit is
        ///   **global** (network-wide) and **doubles** after each
        ///   paid registration.
        /// - Global deposit **halves** after each
        ///   `GlobalDepositHalvingPeriodBlocks` of inactivity
        ///   (lazy, computed on registration).
        /// - Requires `node_id` (ed25519 pubkey) to sign a
        ///   domain-separated payload including a per-node nonce.
        ///
        /// Signature payload (domain-separated, SCALE-encoded):
        /// `("HIPPIUS_COMPUTE_NODE_REG_V1", family, child, node_id, nonce)`
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
            ensure!(
                total < T::MaxChildrenTotal::get(),
                Error::<T>::TooManyChildrenTotal
            );
            let fam_count = FamilyActiveChildren::<T>::get(&family);
            ensure!(
                fam_count < T::MaxChildrenPerFamily::get(),
                Error::<T>::TooManyChildrenInFamily
            );

            // Enforce MaxFamilies the moment a family claims their
            // first fee-free registration.
            let slots_used = Self::family_free_slots_used(&family);
            let free_slots_limit = Self::free_child_slots_per_family();
            if slots_used == 0 {
                let families = FamilyCount::<T>::get();
                ensure!(
                    families < T::MaxFamilies::get(),
                    Error::<T>::TooManyFamilies
                );
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
            let deposit = if !lockup_enabled || slots_used < free_slots_limit {
                BalanceOf::<T>::zero()
            } else {
                Self::apply_lazy_global_halving()
            };

            if lockup_enabled && !deposit.is_zero() {
                T::DepositCurrency::reserve(&family, deposit)
                    .map_err(|_| Error::<T>::InsufficientDeposit)?;
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
            FamilyChildren::<T>::try_mutate(&family, |v| v.try_push(child.clone()))
                .map_err(|_| Error::<T>::TooManyChildrenInFamily)?;

            // Consume a fee-free slot OR advance the paid global
            // fee curve.
            if slots_used < free_slots_limit {
                let new_used = slots_used.saturating_add(1);
                FamilyFreeSlotsUsed::<T>::insert(&family, new_used);
                FamilyUsedFreeSlot::<T>::insert(&family, true);
                if slots_used == 0 {
                    FamilyCount::<T>::put(FamilyCount::<T>::get().saturating_add(1));
                }
            } else if lockup_enabled {
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
        /// - Node id is released from the active registry, but
        ///   put in cooldown
        /// - Deposit remains reserved until `claim_unbonded`
        #[pallet::call_index(11)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::deregister_child(), Pays::No))]
        pub fn deregister_child(origin: OriginFor<T>, child: T::AccountId) -> DispatchResult {
            let who = ensure_signed(origin)?;
            Self::do_deregister_child(&who, child, /* force = */ false)
        }

        /// Force deregister a child node — callable directly by a
        /// registered validator.
        ///
        /// **PR-I5 design (vs upstream arion)**: this extrinsic
        /// REQUIRES the caller to be the primary account; proxy
        /// resolution is not supported. Upstream arion's
        /// `get_primary_account` scans every registered validator
        /// owner's proxy list (O(N validators)) to map a proxy
        /// back to its primary — a costly weight that doesn't
        /// fit the v1 single-operator trust posture. For v1 the
        /// rare force-deregister is invoked by the operator's
        /// validator AccountId directly; a future PR can add
        /// proxy support if needed (probably as a separate
        /// extrinsic that takes the `real` validator account
        /// explicitly so it doesn't need the reverse lookup).
        ///
        /// Routes through the [`FamilyRegistry::is_registered_family`]
        /// + [`FamilyRegistry::owner_has_validator_node`] trait
        /// abstractions so the scoring pallet's `Config` doesn't
        /// need to inherit `pallet_registration::Config` /
        /// `pallet_proxy::Config` as supertraits. The two checks
        /// trip distinct errors:
        ///
        /// - `NodeNotRegistered` — caller has no registration row
        ///   at all (`()` fail-closed impl always lands here).
        /// - `NotAValidator` — caller IS registered but with a
        ///   non-Validator node type (block explorers /
        ///   indexers can distinguish from `NodeNotRegistered`).
        #[pallet::call_index(12)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::deregister_child(), Pays::No))]
        pub fn force_deregister_child(origin: OriginFor<T>, child: T::AccountId) -> DispatchResult {
            let who = ensure_signed(origin)?;

            // Two-step check so the resulting error variant
            // distinguishes "unknown account" from "registered
            // miner trying to force-deregister".
            ensure!(
                T::FamilyRegistry::is_registered_family(&who),
                Error::<T>::NodeNotRegistered
            );
            ensure!(
                T::FamilyRegistry::owner_has_validator_node(&who),
                Error::<T>::NotAValidator
            );

            Self::do_deregister_child(&who, child, /* force = */ true)
        }

        /// Claim (unbond) the deposit for a deregistered child
        /// after the unbonding period.
        #[pallet::call_index(13)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::claim_unbonded(), Pays::No))]
        pub fn claim_unbonded(origin: OriginFor<T>, child: T::AccountId) -> DispatchResult {
            let who = ensure_signed(origin)?;

            let reg = ChildRegistrations::<T>::get(&child).ok_or(Error::<T>::ChildNotRegistered)?;
            ensure!(who == reg.family, DispatchError::BadOrigin);
            ensure!(
                reg.status == ChildStatus::Unbonding,
                Error::<T>::NotUnbonding
            );
            ensure!(
                Self::now() >= reg.unbonding_end,
                Error::<T>::UnbondingNotReady
            );

            let amount = reg.deposit;
            if LockupEnabled::<T>::get() && !amount.is_zero() {
                let unreleased = T::DepositCurrency::unreserve(&reg.family, amount);
                ensure!(unreleased.is_zero(), Error::<T>::PartialUnreserve);
            }

            // Remove record; cooldown tombstones remain in separate
            // storage.
            ChildRegistrations::<T>::remove(&child);

            Self::deposit_event(Event::ChildUnbonded {
                family: reg.family,
                child,
                node_id: reg.node_id,
                amount,
            });
            Ok(())
        }

        // --- PR-I3 audit-stats --------------------------------------------

        /// Submit an audit-VM-signed compute-served aggregate.
        ///
        /// Verifies the Ed25519 signature over `signed.body` using
        /// the registered audit-VM pubkey for `view.node_id`,
        /// enforces the §23 replay-domain checks (chain_genesis,
        /// pallet_instance, epoch, prev_aggregate_hash, expiry,
        /// interval_start ≤ interval_end), then chains the new
        /// `prev_aggregate_hash` (`sp_io::hashing::sha2_256` of
        /// the body bytes — matches off-chain audit-VM).
        ///
        /// **PR-I4 refactor**: this extrinsic no longer credits
        /// `view.served_units` to any per-node storage. The
        /// audit-VM signature chain is on-chain (binding,
        /// cryptographic) but the served-units accounting moves
        /// to the validator's off-chain aggregator, which writes
        /// the result via [`Pallet::vali_submit_epoch_close`] at
        /// epoch close. See `CHANGES.md` PR-I4 for the rationale
        /// (anti-pollution discipline).
        #[pallet::call_index(40)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::submit_audit_stats(), Pays::No))]
        pub fn submit_audit_stats(
            origin: OriginFor<T>,
            view: AggregateView<T::MaxValidatorIdLen, T::MaxFamilyIdLen, T::MaxAuditVmKeyIdLen>,
            signed: SignedAggregateWire<T::MaxAggregateBody>,
        ) -> DispatchResult {
            T::AuditAuthorityOrigin::ensure_origin(origin)?;

            // (1) Wire-shape sanity.
            ensure!(!signed.body.is_empty(), Error::<T>::EmptyAggregateBody);
            ensure!(
                view.interval_end >= view.interval_start,
                Error::<T>::AggregateIntervalInverted
            );

            // (1b) PR-I5: gate on the registration layer. An
            //      unregistered `node_id` cannot push aggregates
            //      onto the on-chain chain — caught BEFORE the
            //      expensive Ed25519 verify so a flood of bogus
            //      node_ids can't burn weight on signature work.
            ensure!(
                T::Registration::is_node_registered(&view.node_id),
                Error::<T>::NodeNotRegistered
            );

            // (2) §23 replay-domain checks against on-chain state.
            ensure!(
                view.chain_genesis == T::ComputeChainGenesis::get(),
                Error::<T>::AggregateChainGenesisMismatch
            );
            ensure!(
                view.pallet_instance == T::ComputePalletInstance::get(),
                Error::<T>::AggregatePalletInstanceMismatch
            );
            ensure!(
                view.epoch == CurrentEpoch::<T>::get(),
                Error::<T>::AggregateEpochMismatch
            );
            let now_unix = <T::NowUnix as frame_support::traits::UnixTime>::now().as_secs();
            ensure!(view.expiry > now_unix, Error::<T>::AggregateExpired);
            let prev = LastAggregateHashByNode::<T>::get(view.node_id).unwrap_or([0u8; 32]);
            ensure!(
                view.prev_aggregate_hash == prev,
                Error::<T>::AggregatePrevHashMismatch
            );

            // (3) BIND the view to the body. The signature below is
            // over `signed.body`; every gate above and every value
            // committed below reads `view`. Nothing else ties the two
            // together, so without this step one honestly-signed body
            // could be submitted under arbitrary `view` fields
            // (review finding, thebrain#49). The audit-VM signs the
            // canonical-CBOR `ServedDeliveryAggregate`, which this
            // pallet can RECOMPUTE from the view — so the bind is a
            // single byte-equality, not a field-by-field walk.
            //
            // `served_units` is deliberately NOT part of the signed
            // aggregate (it has no field in `ServedDeliveryAggregate`)
            // — it stays a submitter claim carried on the event only,
            // never committed to storage. See the `AggregateView`
            // field doc.
            let expected_body = hippius_types::audit_vm::ServedDeliveryAggregate {
                chain_genesis: &view.chain_genesis,
                pallet_instance: &view.pallet_instance,
                validator_id: view.validator_id.as_slice(),
                family_id: view.family_id.as_slice(),
                node_id: &view.node_id,
                audit_vm_key_id: view.audit_vm_key_id.as_slice(),
                epoch: view.epoch,
                challenge_nonce: &view.challenge_nonce,
                interval_start: view.interval_start,
                interval_end: view.interval_end,
                map_root: &view.map_root,
                totals_root: &view.totals_root,
                prev_aggregate_hash: &view.prev_aggregate_hash,
                expiry: view.expiry,
            }
            .canonical()
            .map_err(|_| Error::<T>::AggregateBodyViewMismatch)?;
            ensure!(
                expected_body.as_slice() == signed.body.as_slice(),
                Error::<T>::AggregateBodyViewMismatch
            );

            // (4) Ed25519 verification — now provably over the same
            // facts the view carries.
            let pubkey_bytes = AuditVmPubkeyByNode::<T>::get(view.node_id)
                .ok_or(Error::<T>::AuditVmPubkeyNotRegistered)?;
            let public = ed25519::Public::from_raw(pubkey_bytes);
            let sig = ed25519::Signature::from_raw(signed.sig);
            ensure!(
                sig.verify(signed.body.as_slice(), &public),
                Error::<T>::InvalidAggregateSignature
            );

            // (4) SHA-256-chain the body. §23 / hippius-types use
            //     SHA-256 throughout; using `T::Hashing`
            //     (typically BlakeTwo256) would diverge from the
            //     off-chain audit-VM and break chain continuity.
            let body_hash_bytes = sp_io::hashing::sha2_256(signed.body.as_slice());
            LastAggregateHashByNode::<T>::insert(view.node_id, body_hash_bytes);

            Self::deposit_event(Event::AuditStatsSubmitted {
                node_id: view.node_id,
                epoch: view.epoch,
                served_units: view.served_units,
                body_hash: body_hash_bytes,
            });
            Ok(())
        }

        /// Admin placeholder: bind an Ed25519 audit-VM public key
        /// to a `node_id`. PR-I6 replaces this with the KBS-issued
        /// certificate flow.
        #[pallet::call_index(41)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::set_audit_vm_pubkey(), Pays::No))]
        pub fn set_audit_vm_pubkey(
            origin: OriginFor<T>,
            node_id: [u8; 32],
            pubkey: [u8; 32],
        ) -> DispatchResult {
            T::ComputeScoringAdminOrigin::ensure_origin(origin)?;
            AuditVmPubkeyByNode::<T>::insert(node_id, pubkey);
            Self::deposit_event(Event::AuditVmPubkeySet { node_id, pubkey });
            Ok(())
        }

        // --- #322 live attestation -----------------------------------------

        /// Add an Ed25519 KBS L0 verifying key to the
        /// [`KbsAttestationPubkeys`] allowlist. Subsequent calls to
        /// [`Pallet::submit_live_attestation`] whose `view.
        /// signer_pubkey` equals `pubkey` will be accepted (modulo
        /// the rest of the verification chain). Admin-only.
        #[pallet::call_index(42)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::set_kbs_attestation_pubkey(), Pays::No))]
        pub fn set_kbs_attestation_pubkey(
            origin: OriginFor<T>,
            pubkey: [u8; 32],
        ) -> DispatchResult {
            T::ComputeScoringAdminOrigin::ensure_origin(origin)?;
            KbsAttestationPubkeys::<T>::try_mutate(|allowlist| -> DispatchResult {
                if allowlist.iter().any(|p| p == &pubkey) {
                    return Err(Error::<T>::KbsAttestationPubkeyAlreadyAllowed.into());
                }
                allowlist
                    .try_push(pubkey)
                    .map_err(|_| Error::<T>::KbsAttestationPubkeysFull)?;
                Ok(())
            })?;
            Self::deposit_event(Event::KbsAttestationPubkeyAdded { pubkey });
            Ok(())
        }

        /// Remove an Ed25519 KBS L0 verifying key from the
        /// allowlist — e.g. for key rotation or a compromised L0
        /// key. Admin-only. In-flight live attestations signed by
        /// the removed key will be rejected on submission.
        #[pallet::call_index(43)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::remove_kbs_attestation_pubkey(), Pays::No))]
        pub fn remove_kbs_attestation_pubkey(
            origin: OriginFor<T>,
            pubkey: [u8; 32],
        ) -> DispatchResult {
            T::ComputeScoringAdminOrigin::ensure_origin(origin)?;
            KbsAttestationPubkeys::<T>::try_mutate(|allowlist| -> DispatchResult {
                let pos = allowlist
                    .iter()
                    .position(|p| p == &pubkey)
                    .ok_or(Error::<T>::KbsAttestationPubkeyNotAllowed)?;
                allowlist.swap_remove(pos);
                Ok(())
            })?;
            Self::deposit_event(Event::KbsAttestationPubkeyRemoved { pubkey });
            Ok(())
        }

        /// Accept one KBS-signed live attestation for a running
        /// tenant CVM (#322 Phase B). The in-VM guest agent (the
        /// "blackbox") periodically asks the kernel for a fresh
        /// `SNP_GET_REPORT`, POSTs it to KBS, and the KBS — after
        /// re-verifying the report against AMD's silicon root + the
        /// §22 allowlist — signs a
        /// `hippius_types::live_attestation::LiveAttestation` body.
        /// A validator relays the resulting
        /// [`SignedLiveAttestationWire`] here.
        ///
        /// Verification gates (every one is required, fail-closed):
        /// 1. wire size — `signed.body` non-empty;
        /// 2. window — `view.verified_at_unix >=
        ///    view.observed_at_unix`;
        /// 3. `node_id` registered in [`Config::Registration`];
        /// 4. replay domain — `chain_genesis`, `pallet_instance`,
        ///    `epoch == CurrentEpoch::get()`;
        /// 5. `view.expiry_unix > now_unix`;
        /// 6. `view.attestation_seq` and `view.prev_attestation_hash`
        ///    chain off [`LastLiveAttestation`] (or off the all-zero
        ///    seed for the first attestation of a VM);
        /// 7. `view.signer_pubkey` in [`KbsAttestationPubkeys`];
        /// 8. Ed25519 verify `signed.sig` against `signed.body`.
        ///
        /// On success: store the SHA-256 of `signed.body` as the
        /// new `prev_attestation_hash` for this `vm_id_hash`,
        /// increment `LiveAttestationCount(epoch, vm_id_hash)`, and
        /// emit [`Event::LiveAttestationSubmitted`].
        ///
        /// Single-attestation, not a batch: keeps the weight model
        /// clean. The validator submits one tx per attestation
        /// today; a batched form (`Vec<SignedLiveAttestationWire>`)
        /// is a follow-up once we benchmark the per-call dispatch
        /// overhead.
        #[pallet::call_index(44)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::submit_live_attestation(), Pays::No))]
        pub fn submit_live_attestation(
            origin: OriginFor<T>,
            view: LiveAttestationView<T::MaxVmIdLen>,
            signed: SignedLiveAttestationWire<T::MaxLiveAttestationBody>,
        ) -> DispatchResult {
            T::AuditAuthorityOrigin::ensure_origin(origin)?;

            // (1) Wire-shape sanity.
            ensure!(
                !signed.body.is_empty(),
                Error::<T>::EmptyLiveAttestationBody
            );
            ensure!(
                view.verified_at_unix >= view.observed_at_unix,
                Error::<T>::LiveAttestationWindowInverted
            );

            // (2) Registration gate — caught BEFORE the expensive
            // Ed25519 verify so a flood of bogus node_ids can't burn
            // weight on signature work. Mirrors `submit_audit_stats`
            // PR-I5 discipline.
            ensure!(
                T::Registration::is_node_registered(&view.node_id),
                Error::<T>::NodeNotRegistered
            );

            // (3) Replay-domain checks against on-chain state.
            ensure!(
                view.chain_genesis == T::ComputeChainGenesis::get(),
                Error::<T>::LiveAttestationChainGenesisMismatch
            );
            ensure!(
                view.pallet_instance == T::ComputePalletInstance::get(),
                Error::<T>::LiveAttestationPalletInstanceMismatch
            );
            ensure!(
                view.epoch == CurrentEpoch::<T>::get(),
                Error::<T>::LiveAttestationEpochMismatch
            );
            let now_unix = <T::NowUnix as frame_support::traits::UnixTime>::now().as_secs();
            ensure!(
                view.expiry_unix > now_unix,
                Error::<T>::LiveAttestationExpired
            );

            // (4) Per-VM replay chain. The vm_id is hashed to a
            // fixed-size key — Blake2_128Concat matches the
            // storage key derivation so we don't depend on Substrate
            // internals for the hash.
            let vm_id_hash = sp_io::hashing::blake2_256(view.vm_id.as_slice());
            let (expected_seq, expected_prev) = match LastLiveAttestation::<T>::get(vm_id_hash) {
                Some(rec) => (rec.attestation_seq.saturating_add(1), rec.body_hash),
                None => (1u64, [0u8; 32]),
            };
            ensure!(
                view.attestation_seq == expected_seq,
                Error::<T>::LiveAttestationSeqMismatch
            );
            ensure!(
                view.prev_attestation_hash == expected_prev,
                Error::<T>::LiveAttestationPrevHashMismatch
            );

            // (5) KBS L0 signer allowlist.
            ensure!(
                KbsAttestationPubkeys::<T>::get()
                    .iter()
                    .any(|p| p == &view.signer_pubkey),
                Error::<T>::LiveAttestationKbsNotAllowed
            );

            // (6) BIND the view to the body. The signature in (7) is
            // over `signed.body`; every gate above and every value
            // committed in (9) reads `view`. Nothing else ties the
            // two together, so without this step ONE honestly-KBS-
            // signed body could be replayed indefinitely — bump
            // `view.attestation_seq`, set `view.prev_attestation_hash`
            // to the last stored hash, point `view.vm_id` /
            // `view.node_id` anywhere — and `LiveAttestationCount`
            // (the number uptime reward scales off) inflates freely
            // (review finding, thebrain#49).
            //
            // Unlike the audit aggregate, the body carries MORE
            // fields than the view (measurement, report digests), so
            // it cannot be recomputed here — decode it (canonical-
            // CBOR, `decode` also runs `validate()`) and compare
            // every field this call acts on.
            let body =
                hippius_types::live_attestation::LiveAttestation::decode(signed.body.as_slice())
                    .map_err(|_| Error::<T>::MalformedLiveAttestationBody)?;
            ensure!(
                body.chain_genesis == view.chain_genesis
                    && body.pallet_instance == view.pallet_instance
                    && body.vm_id.as_bytes() == view.vm_id.as_slice()
                    && body.node_id == view.node_id
                    && body.attestation_seq == view.attestation_seq
                    && body.epoch == view.epoch
                    && body.observed_at_unix == view.observed_at_unix
                    && body.verified_at_unix == view.verified_at_unix
                    && body.prev_attestation_hash == view.prev_attestation_hash
                    && body.expiry_unix == view.expiry_unix
                    && body.signer_pubkey == view.signer_pubkey,
                Error::<T>::LiveAttestationBodyViewMismatch
            );

            // (7) Ed25519 verify the body bytes against the
            // signer's pubkey carried in the view — which (5) proved
            // is allowlisted and (6) proved is the SAME key the body
            // itself names.
            let public = ed25519::Public::from_raw(view.signer_pubkey);
            let sig = ed25519::Signature::from_raw(signed.sig);
            ensure!(
                sig.verify(signed.body.as_slice(), &public),
                Error::<T>::InvalidLiveAttestationSignature
            );

            // (8) SHA-256-chain the body. Same hash function as
            // `submit_audit_stats` so the off-chain prev-hash
            // accounting on the KBS side stays byte-identical.
            let body_hash_bytes = sp_io::hashing::sha2_256(signed.body.as_slice());

            // (9) Commit state.
            LastLiveAttestation::<T>::insert(
                vm_id_hash,
                LiveAttestationRecord {
                    node_id: view.node_id,
                    attestation_seq: view.attestation_seq,
                    body_hash: body_hash_bytes,
                    epoch: view.epoch,
                    observed_at_unix: view.observed_at_unix,
                },
            );
            LiveAttestationCount::<T>::mutate(view.epoch, vm_id_hash, |c| {
                *c = c.saturating_add(1);
            });

            Self::deposit_event(Event::LiveAttestationSubmitted {
                vm_id_hash,
                node_id: view.node_id,
                epoch: view.epoch,
                attestation_seq: view.attestation_seq,
                body_hash: body_hash_bytes,
            });
            Ok(())
        }

        // --- PR-I4 vali_submit_epoch_close --------------------------------

        /// Close the current epoch.
        ///
        /// Writes `EpochWeights[epoch][node_id] = weight` for every
        /// `MinerStatusUpdate` in the batch + transitions
        /// [`MinerStatuses`] rows. **Events are emitted ONLY on
        /// status transitions** — a heartbeat that re-asserts the
        /// same status as the stored row stays silent so the on-
        /// chain event log doesn't pollute with no-op signals.
        ///
        /// **Root-only** (locked Q13 single-operator v1 per
        /// `#40` 2026-05-20 plan révisé). A future PR may relax
        /// this to a multi-vali origin once the trust posture
        /// changes; today the §G "single authoritative submitter"
        /// invariant is structurally pinned by `ensure_root`.
        ///
        /// `epoch` MUST be strictly greater than the stored
        /// `CurrentEpoch` — replaying an old close is an
        /// `EpochRegression` error. Duplicate `node_id`s within a
        /// single batch are rejected with `DuplicateNodeInBatch`.
        #[pallet::call_index(50)]
        #[pallet::weight((
            <T as pallet::Config>::WeightInfo::vali_submit_epoch_close(status_updates.len() as u32),
            Pays::No
        ))]
        pub fn vali_submit_epoch_close(
            origin: OriginFor<T>,
            epoch: u64,
            status_updates: BoundedVec<MinerStatusUpdate, T::MaxMinerStatusUpdatesPerCall>,
        ) -> DispatchResult {
            ensure_root(origin)?;

            let cur = CurrentEpoch::<T>::get();
            ensure!(epoch > cur, Error::<T>::EpochRegression);

            // Reject duplicate node_ids in the batch via an O(n log n)
            // `BTreeSet` insert — the previous O(n²) scan was honest
            // but priced wrong in the weight table (codex/gemini
            // CONVERGENT HIGH). `n` is still bounded by
            // `T::MaxMinerStatusUpdatesPerCall`.
            let mut seen: BTreeSet<[u8; 32]> = BTreeSet::new();
            for u in status_updates.iter() {
                ensure!(seen.insert(u.node_id), Error::<T>::DuplicateNodeInBatch);
            }

            let now = Self::now();
            let updates_len = status_updates.len() as u32;
            let mut snapshot = Vec::with_capacity(updates_len as usize);

            for u in status_updates.into_iter() {
                // (a) Always write the epoch weight — this is the
                //     authoritative §23 reward input that the
                //     off-chain ranking pallet reads.
                EpochWeights::<T>::insert(epoch, u.node_id, u.weight);

                // Collect the snapshot for T::RankingsSink in-memory
                // to avoid an O(n) storage re-read after the loop.
                snapshot.push(EpochWeightEntry {
                    node_id: u.node_id,
                    weight: u.weight,
                });

                // (b) Only write + emit `MinerStatusChanged` if the
                //     **effective** previous status differs. Default state
                //     when no row exists is `MinerStatus::Active` (a node that
                //     just registered via the §13 anti-Sybil flow is implicitly
                //     Active until the validator says otherwise). So:
                //
                //     - prev=None, new=Active → no transition
                //       (heartbeat re-asserting the default;
                //       silent, no storage row written).
                //     - prev=None, new=Quarantined →
                //       transition (Active → Quarantined; row
                //       written + event emitted).
                //     - prev=Some(X), new=X → silent (heartbeat).
                //     - prev=Some(X), new=Y where X != Y →
                //       transition.
                //
                //     This keeps the on-chain state minimal ("1 row par miner,
                //     pas d'historique") — only non-default statuses
                //     materialise as rows. The logic lives in the shared
                //     [`Self::transition_node_status`] helper so the
                //     graceful-exit quarantine in `request_unstake` is
                //     byte-identical.
                Self::transition_node_status(u.node_id, u.new_status, now, epoch);
            }

            CurrentEpoch::<T>::put(epoch);

            // PR-I6: push the closed epoch's full weight snapshot
            // to the runtime-bound `RankingsSink`. Production
            // typically binds `()` (no-op) because off-chain
            // validators read [`Pallet::epoch_weights_for`]
            // directly and post `pallet_rankings::update_rankings`.
            // The bridge test binds a recording sink to pin the
            // payload shape — see `mock::MockRankingsSink`.
            //
            // Snapshot collection is O(updates_len) in-memory.
            // Acceptable: vali_submit_epoch_close fires ONCE per
            // epoch, not per node.
            T::RankingsSink::push_rankings(epoch, &snapshot);

            Self::deposit_event(Event::EpochClosed {
                epoch,
                updates: updates_len,
            });
            Ok(())
        }

        // --- Admin -------------------------------------------------------

        /// Admin: enable/disable registration lockup
        /// (reserve/unbond).
        #[pallet::call_index(30)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::set_lockup_enabled(), Pays::No))]
        pub fn set_lockup_enabled(origin: OriginFor<T>, enabled: bool) -> DispatchResult {
            T::ComputeScoringAdminOrigin::ensure_origin(origin)?;
            LockupEnabled::<T>::put(enabled);
            Self::deposit_event(Event::LockupEnabledSet { enabled });
            Ok(())
        }

        /// Admin: set the base deposit price (floor for the
        /// global fee curve).
        #[pallet::call_index(31)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::set_base_child_deposit(), Pays::No))]
        pub fn set_base_child_deposit(
            origin: OriginFor<T>,
            deposit: BalanceOf<T>,
        ) -> DispatchResult {
            T::ComputeScoringAdminOrigin::ensure_origin(origin)?;
            BaseChildDepositValue::<T>::put(deposit);
            Self::deposit_event(Event::BaseChildDepositSet { deposit });
            Ok(())
        }

        /// Admin: set the number of fee-free registrations per
        /// family.
        #[pallet::call_index(38)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::set_free_child_slots_per_family(), Pays::No))]
        pub fn set_free_child_slots_per_family(origin: OriginFor<T>, slots: u32) -> DispatchResult {
            T::ComputeScoringAdminOrigin::ensure_origin(origin)?;
            ensure!(
                slots <= T::MaxChildrenPerFamily::get(),
                Error::<T>::FreeChildSlotsPerFamilyTooLarge
            );
            FreeChildSlotsPerFamily::<T>::put(slots);
            Self::deposit_event(Event::FreeChildSlotsPerFamilySet { slots });
            Ok(())
        }

        // --- PR-1 stake -------------------------------------------------

        /// Admin: enable/disable the stake-eligibility layer.
        #[pallet::call_index(32)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::set_lockup_enabled(), Pays::No))]
        pub fn set_stake_enabled(origin: OriginFor<T>, enabled: bool) -> DispatchResult {
            T::ComputeScoringAdminOrigin::ensure_origin(origin)?;
            StakeEnabled::<T>::put(enabled);
            Self::deposit_event(Event::StakeEnabledSet { enabled });
            Ok(())
        }

        /// Admin/oracle: post the native-coin USD price (fixed-point
        /// `alpha_per_usd × 1e6`) and advance the asymmetric EMA.
        #[pallet::call_index(33)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::set_base_child_deposit(), Pays::No))]
        pub fn set_alpha_per_usd(origin: OriginFor<T>, spot: u128) -> DispatchResult {
            T::ComputeScoringAdminOrigin::ensure_origin(origin)?;
            ensure!(spot > 0, Error::<T>::InvalidPrice);
            AlphaPerUsdSpot::<T>::put(spot);
            let new_ema = Self::ema_step(spot, AlphaPerUsdEma::<T>::get());
            AlphaPerUsdEma::<T>::put(new_ema);
            Self::deposit_event(Event::AlphaPerUsdUpdated { spot, ema: new_ema });
            Ok(())
        }

        /// Admin: set the minimum stake floor (cold-start entry ticket).
        #[pallet::call_index(34)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::set_base_child_deposit(), Pays::No))]
        pub fn set_stake_floor(origin: OriginFor<T>, floor: BalanceOf<T>) -> DispatchResult {
            T::ComputeScoringAdminOrigin::ensure_origin(origin)?;
            StakeFloor::<T>::put(floor);
            Self::deposit_event(Event::StakeFloorSet { floor });
            Ok(())
        }

        /// Admin: set the asymmetric-EMA smoothing factors (per-mille,
        /// `1..=1000`). `down` reacts to dumps (fast), `up` to pumps
        /// (slow).
        #[pallet::call_index(35)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::set_lockup_enabled(), Pays::No))]
        pub fn set_ema_permille(origin: OriginFor<T>, down: u32, up: u32) -> DispatchResult {
            T::ComputeScoringAdminOrigin::ensure_origin(origin)?;
            ensure!(
                down > 0 && down <= 1000 && up > 0 && up <= 1000,
                Error::<T>::InvalidEmaFactor
            );
            EmaDownPermille::<T>::put(down);
            EmaUpPermille::<T>::put(up);
            Self::deposit_event(Event::EmaPermilleSet { down, up });
            Ok(())
        }

        /// Staker: reserve additional native collateral.
        #[pallet::call_index(36)]
        #[pallet::weight(<T as pallet::Config>::WeightInfo::register_child())]
        pub fn top_up_stake(origin: OriginFor<T>, amount: BalanceOf<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(!amount.is_zero(), Error::<T>::ZeroStake);
            T::DepositCurrency::reserve(&who, amount)
                .map_err(|_| Error::<T>::InsufficientBalanceForStake)?;
            StakedAmount::<T>::mutate(&who, |s| *s = s.saturating_add(amount));
            Self::deposit_event(Event::StakeToppedUp {
                who: who.clone(),
                amount,
                total: StakedAmount::<T>::get(&who),
            });
            Ok(())
        }

        /// Staker: begin unbonding `amount` of active stake. It stays
        /// reserved (slashable) until [`claim_unstaked`] after the
        /// unbonding period.
        ///
        /// **Gap #2 (graceful exit):** if this unstake is an *exit signal* —
        /// the owner unbonds EVERYTHING (`remaining == 0`, unambiguous and
        /// works even with the stake layer disabled), OR, when staking is
        /// live, drops below the eligibility floor — all of the owner's §13
        /// compute nodes are transitioned to `Quarantined`. This lets the
        /// off-chain validator auto-drain + warm-migrate their VMs DURING the
        /// unbonding period rather than after the stake is gone. A partial
        /// unstake that stays sufficiently collateralised is NOT an exit and
        /// does not quarantine.
        #[pallet::call_index(37)]
        #[pallet::weight(<T as pallet::Config>::WeightInfo::request_unstake())]
        pub fn request_unstake(origin: OriginFor<T>, amount: BalanceOf<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(!amount.is_zero(), Error::<T>::ZeroStake);
            let active = StakedAmount::<T>::get(&who);
            ensure!(active >= amount, Error::<T>::InsufficientStake);
            StakedAmount::<T>::insert(&who, active.saturating_sub(amount));
            let end = Self::now().saturating_add(T::UnbondingPeriodBlocks::get());
            StakeUnbonding::<T>::mutate(&who, |slot| {
                let pending = slot.map(|(p, _)| p).unwrap_or_else(Zero::zero);
                // Extend the whole pending chunk to the latest end —
                // conservative: a fresh unbond resets the clock.
                *slot = Some((pending.saturating_add(amount), end));
            });
            Self::deposit_event(Event::StakeUnbondRequested {
                who: who.clone(),
                amount,
                unbonding_end: end,
            });

            // Gap #2 — graceful-exit quarantine. Run AFTER the unbond
            // bookkeeping + event so the stake-layer state is final before we
            // read `remaining`, and so the `StakeUnbondRequested` event
            // precedes the quarantine event in the log.
            let remaining = StakedAmount::<T>::get(&who);
            // Exit signal: unstaking EVERYTHING is unambiguous (works even
            // when staking is disabled, so it's testable); OR, when the stake
            // layer is live, dropping below the eligibility floor. A partial
            // unstake that stays sufficiently collateralised is NOT an exit.
            let exiting = remaining.is_zero()
                || (StakeEnabled::<T>::get() && remaining < StakeFloor::<T>::get());
            if exiting {
                let now = Self::now();
                let epoch = CurrentEpoch::<T>::get();
                let children = FamilyChildren::<T>::get(&who);
                let mut quarantined: u32 = 0;
                for child in children.iter() {
                    if let Some(reg) = ChildRegistrations::<T>::get(child) {
                        if Self::transition_node_status(
                            reg.node_id,
                            MinerStatus::Quarantined,
                            now,
                            epoch,
                        ) {
                            quarantined = quarantined.saturating_add(1);
                        }
                    }
                }
                if quarantined > 0 {
                    Self::deposit_event(Event::OwnerQuarantinedOnUnstake {
                        who: who.clone(),
                        nodes: quarantined,
                    });
                }
            }
            Ok(())
        }

        /// Staker: release fully-unbonded stake.
        #[pallet::call_index(45)]
        #[pallet::weight(<T as pallet::Config>::WeightInfo::claim_unbonded())]
        pub fn claim_unstaked(origin: OriginFor<T>) -> DispatchResult {
            let who = ensure_signed(origin)?;
            let (amount, end) =
                StakeUnbonding::<T>::get(&who).ok_or(Error::<T>::NoStakeUnbonding)?;
            ensure!(Self::now() >= end, Error::<T>::UnbondingNotReady);
            let unreleased = T::DepositCurrency::unreserve(&who, amount);
            ensure!(unreleased.is_zero(), Error::<T>::PartialUnreserve);
            StakeUnbonding::<T>::remove(&who);
            Self::deposit_event(Event::StakeReleased { who, amount });
            Ok(())
        }

        // --- PR-2 slashing ----------------------------------------------

        /// Admin: enable/disable slashing (global kill-switch).
        #[pallet::call_index(46)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::set_lockup_enabled(), Pays::No))]
        pub fn set_slashing_enabled(origin: OriginFor<T>, enabled: bool) -> DispatchResult {
            T::ComputeScoringAdminOrigin::ensure_origin(origin)?;
            SlashingEnabled::<T>::put(enabled);
            Self::deposit_event(Event::SlashingEnabledSet { enabled });
            Ok(())
        }

        /// Admin: set the slash beneficiary (`None` ⇒ burn slashed funds).
        #[pallet::call_index(47)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::set_base_child_deposit(), Pays::No))]
        pub fn set_slash_beneficiary(
            origin: OriginFor<T>,
            beneficiary: Option<T::AccountId>,
        ) -> DispatchResult {
            T::ComputeScoringAdminOrigin::ensure_origin(origin)?;
            match &beneficiary {
                Some(b) => SlashBeneficiary::<T>::put(b),
                None => SlashBeneficiary::<T>::kill(),
            }
            Self::deposit_event(Event::SlashBeneficiarySet { beneficiary });
            Ok(())
        }

        /// Authority: slash `owner`'s stake for a proven breach. The
        /// off-chain validator detects the breach (liveness / deficiency)
        /// and submits this, the same way it drives `set_miner_status`.
        /// No-op on balance while the kill-switch is off (event only).
        #[pallet::call_index(48)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::deregister_child(), Pays::No))]
        pub fn slash_stake(
            origin: OriginFor<T>,
            owner: T::AccountId,
            amount: BalanceOf<T>,
            reason: SlashReason,
        ) -> DispatchResult {
            T::ComputeScoringAdminOrigin::ensure_origin(origin)?;
            ensure!(!amount.is_zero(), Error::<T>::ZeroStake);
            Self::do_slash(&owner, amount, reason);
            Ok(())
        }

        // --- PR-3 marketplace -------------------------------------------

        /// Node operator: announce a price change. It is rate-limited,
        /// magnitude-bounded, clamped to `[floor, ceiling]`, and takes
        /// effect only after the notice window — giving vali time to
        /// migrate any VM that no longer fits the tenant's budget.
        #[pallet::call_index(49)]
        #[pallet::weight(<T as pallet::Config>::WeightInfo::register_child())]
        pub fn announce_price_change(
            origin: OriginFor<T>,
            node_id: [u8; 32],
            new_price: u128,
        ) -> DispatchResult {
            let who = ensure_signed(origin)?;
            ensure!(
                NodeIdToChild::<T>::get(node_id).as_ref() == Some(&who),
                Error::<T>::NotNodeOperator
            );
            // A price MUST be positive: `0` is a nonsensical free price and
            // would trap the node — once `MinerPrice == 0` the magnitude
            // bounds collapse to `[0, 0]`, so it could never be raised.
            ensure!(new_price > 0, Error::<T>::PriceOutOfBounds);
            ensure!(
                Self::within_price_bounds(new_price),
                Error::<T>::PriceOutOfBounds
            );

            let now = Self::now();
            let last = LastPriceChangeBlock::<T>::get(node_id);
            // `last == 0` (never changed) skips the interval gate.
            if !last.is_zero() {
                ensure!(
                    now >= last.saturating_add(MinPriceChangeIntervalBlocks::<T>::get()),
                    Error::<T>::PriceChangeTooSoon
                );
            }
            // Magnitude is bounded only against an existing price; the very
            // first price a miner sets is unconstrained (within bounds).
            if let Some(current) = MinerPrice::<T>::get(node_id) {
                ensure!(
                    Self::within_price_magnitude(current, new_price),
                    Error::<T>::PriceChangeTooLarge
                );
            }

            let effective_block = now.saturating_add(PriceChangeNoticeBlocks::<T>::get());
            PendingPriceChange::<T>::insert(
                node_id,
                PriceChange {
                    new_price,
                    effective_block,
                },
            );
            LastPriceChangeBlock::<T>::insert(node_id, now);
            Self::deposit_event(Event::PriceChangeAnnounced {
                node_id,
                old_price: MinerPrice::<T>::get(node_id),
                new_price,
                effective_block,
            });
            Ok(())
        }

        /// Permissionless: materialise a node's due price change on-chain
        /// (consumers already see it via `effective_price`; this makes the
        /// canonical storage + event).
        #[pallet::call_index(51)]
        #[pallet::weight(<T as pallet::Config>::WeightInfo::set_base_child_deposit())]
        pub fn apply_price_change(origin: OriginFor<T>, node_id: [u8; 32]) -> DispatchResult {
            ensure_signed(origin)?;
            let pending =
                PendingPriceChange::<T>::get(node_id).ok_or(Error::<T>::NoPriceChangeDue)?;
            ensure!(
                Self::now() >= pending.effective_block,
                Error::<T>::NoPriceChangeDue
            );
            MinerPrice::<T>::insert(node_id, pending.new_price);
            PendingPriceChange::<T>::remove(node_id);
            Self::deposit_event(Event::PriceChangeApplied {
                node_id,
                new_price: pending.new_price,
            });
            Ok(())
        }

        /// Admin: set the price floor/ceiling.
        #[pallet::call_index(52)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::set_base_child_deposit(), Pays::No))]
        pub fn set_price_bounds(
            origin: OriginFor<T>,
            floor: u128,
            ceiling: Option<u128>,
        ) -> DispatchResult {
            T::ComputeScoringAdminOrigin::ensure_origin(origin)?;
            if let Some(c) = ceiling {
                ensure!(floor <= c, Error::<T>::InvalidPricePolicy);
            }
            PriceFloor::<T>::put(floor);
            match ceiling {
                Some(c) => PriceCeiling::<T>::put(c),
                None => PriceCeiling::<T>::kill(),
            }
            Self::deposit_event(Event::PriceBoundsSet { floor, ceiling });
            Ok(())
        }

        /// Admin: set the price-change policy — minimum interval, notice
        /// window, and the max per-step ratio (`numer/denom`, `≥ 1`).
        #[pallet::call_index(53)]
        #[pallet::weight((<T as pallet::Config>::WeightInfo::set_lockup_enabled(), Pays::No))]
        pub fn set_price_change_policy(
            origin: OriginFor<T>,
            min_interval: BlockNumberFor<T>,
            notice: BlockNumberFor<T>,
            max_numer: u32,
            max_denom: u32,
        ) -> DispatchResult {
            T::ComputeScoringAdminOrigin::ensure_origin(origin)?;
            ensure!(
                max_denom > 0 && max_numer >= max_denom,
                Error::<T>::InvalidPricePolicy
            );
            MinPriceChangeIntervalBlocks::<T>::put(min_interval);
            PriceChangeNoticeBlocks::<T>::put(notice);
            MaxPriceChangeNumer::<T>::put(max_numer);
            MaxPriceChangeDenom::<T>::put(max_denom);
            Self::deposit_event(Event::PriceChangePolicySet {
                min_interval,
                notice,
                max_numer,
                max_denom,
            });
            Ok(())
        }
    }

    // =====================================================================
    // Internal helpers (extrinsic body shared between deregister + force)
    // =====================================================================

    impl<T: Config> Pallet<T> {
        fn do_deregister_child(
            actor: &T::AccountId,
            child: T::AccountId,
            force: bool,
        ) -> DispatchResult {
            let mut reg =
                ChildRegistrations::<T>::get(&child).ok_or(Error::<T>::ChildNotRegistered)?;
            if !force {
                ensure!(*actor == reg.family, DispatchError::BadOrigin);
            }
            ensure!(
                reg.status == ChildStatus::Active,
                Error::<T>::ChildNotActive
            );

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
            // Clear marketplace price state so a node id re-registered by a
            // different operator never inherits stale current/pending price.
            MinerPrice::<T>::remove(reg.node_id);
            PendingPriceChange::<T>::remove(reg.node_id);
            LastPriceChangeBlock::<T>::remove(reg.node_id);
            TotalActiveChildren::<T>::put(TotalActiveChildren::<T>::get().saturating_sub(1));
            let fam_active = FamilyActiveChildren::<T>::get(&reg.family);
            let new_fam_active = fam_active.saturating_sub(1);
            FamilyActiveChildren::<T>::insert(&reg.family, new_fam_active);
            FamilyChildren::<T>::mutate(&reg.family, |v| {
                if let Some(pos) = v.iter().position(|c| c == &child) {
                    v.swap_remove(pos);
                }
            });
            if new_fam_active.is_zero() {
                Self::cleanup_family_when_no_active_children(&reg.family);
            }

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
    }
}

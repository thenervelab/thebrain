//! Mock runtime for `pallet-compute-scoring` tests (PR-I4 slim).
//!
//! Wires `frame_system` + `pallet_balances` + `pallet_timestamp` +
//! `pallet_compute_scoring` for the new audit-stats + epoch-close
//! surface. After the PR-I4 rip the Config trait shrunk from ~50 to
//! ~17 items, so this mock is now under 100 LOC.

use crate as pallet_compute_scoring;
use crate::pallet::{
    EpochWeightEntry, FamilyRegistry, NodeRegistrationProvider, ProxyVerifier, RankingsSink,
};
use core::cell::RefCell;
use frame_support::traits::{ConstU128, ConstU32, ConstU64};
use frame_support::{derive_impl, parameter_types};
use sp_core::H256;
use sp_runtime::traits::{BlakeTwo256, IdentityLookup};
use sp_runtime::BuildStorage;
use std::collections::BTreeSet;

type Block = frame_system::mocking::MockBlock<TestRuntime>;
pub type AccountId = u64;
pub type Balance = u128;

frame_support::construct_runtime!(
    pub enum TestRuntime {
        System: frame_system,
        Balances: pallet_balances,
        Timestamp: pallet_timestamp,
        ComputeScoring: pallet_compute_scoring,
    }
);

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for TestRuntime {
    type Block = Block;
    type AccountId = AccountId;
    type Lookup = IdentityLookup<Self::AccountId>;
    type AccountData = pallet_balances::AccountData<Balance>;
    type Hash = H256;
    type Hashing = BlakeTwo256;
}

#[derive_impl(pallet_balances::config_preludes::TestDefaultConfig)]
impl pallet_balances::Config for TestRuntime {
    type Balance = Balance;
    type ExistentialDeposit = ConstU128<1>;
    type AccountStore = System;
    // Named reserves (R9): the pallet requires
    // `ReserveIdentifier = [u8; 8]` — the prelude default is `()`.
    type ReserveIdentifier = [u8; 8];
    type MaxReserves = ConstU32<8>;
}

impl pallet_timestamp::Config for TestRuntime {
    type Moment = u64;
    type OnTimestampSet = ();
    type MinimumPeriod = ConstU64<0>;
    type WeightInfo = ();
}

/// Always-true registry / proxy stand-ins for the §13 anti-Sybil
/// registration tests. PR-I5 wires the real
/// `pallet_registration::Pallet` / `pallet_proxy::Pallet` in the
/// production runtime; here we keep lightweight stand-ins so the
/// mock doesn't need to instantiate all 8 of
/// `pallet-registration`'s supertraits (`pallet_babe`,
/// `pallet_balances`, `pallet_credits`, `pallet_staking`,
/// `pallet_proxy`, `pallet_utils`, …).
pub struct DummyFamilyRegistry;
impl FamilyRegistry<AccountId> for DummyFamilyRegistry {
    fn is_registered_family(_family: &AccountId) -> bool {
        true
    }
    fn owner_has_validator_node(_owner: &AccountId) -> bool {
        // PR-I5 tests don't exercise `force_deregister_child`'s
        // validator gate; the production binding via the real
        // `pallet_registration::Pallet<T>` evaluates this against
        // the runtime's registration storage.
        true
    }
}

pub struct DummyProxyVerifier;
impl ProxyVerifier<AccountId> for DummyProxyVerifier {
    fn can_register_child(_family: &AccountId, _child: &AccountId) -> bool {
        true
    }
}

// Thread-local-backed `NodeRegistrationProvider` stand-in for
// PR-I5 tests. Production wires `T::Registration =
// pallet_registration::Pallet<Self>`, which reads on-chain
// storage; tests register/unregister node_ids in this mock
// instead so `submit_audit_stats` can be driven without pulling
// the heavy upstream registration crate into the mock runtime.
thread_local! {
    static REGISTERED_NODES: RefCell<BTreeSet<[u8; 32]>> = const { RefCell::new(BTreeSet::new()) };
}

pub struct MockRegistration;
impl NodeRegistrationProvider for MockRegistration {
    fn is_node_registered(node_id: &[u8; 32]) -> bool {
        REGISTERED_NODES.with(|s| s.borrow().contains(node_id))
    }
}

/// Test helper: mark `node_id` as registered in the mock
/// registration provider. Mirrors what
/// `pallet_registration::register_node` would do on chain.
pub fn mock_register_node(node_id: [u8; 32]) {
    REGISTERED_NODES.with(|s| {
        s.borrow_mut().insert(node_id);
    });
}

/// Test helper: clear all registered node_ids. Called by
/// `new_test_ext` so each test starts with an empty registry.
pub fn mock_clear_registered_nodes() {
    REGISTERED_NODES.with(|s| s.borrow_mut().clear());
}

// PR-I6: thread-local-backed [`RankingsSink`] recording stand-in.
// `vali_submit_epoch_close` pushes `(epoch, Vec<EpochWeightEntry>)`
// into this sink at the end of every close; the bridge test reads
// `mock_drain_pushed_rankings()` to assert the payload matches the
// `EpochWeights` storage. Production runtimes bind `RankingsSink =
// ()` and dispatch `pallet_rankings::update_rankings` from an
// off-chain validator OCW instead.
thread_local! {
    static PUSHED_RANKINGS: RefCell<Vec<(u64, Vec<EpochWeightEntry>)>> =
        const { RefCell::new(Vec::new()) };
}

pub struct MockRankingsSink;
impl RankingsSink for MockRankingsSink {
    fn push_rankings(epoch: u64, entries: &[EpochWeightEntry]) {
        PUSHED_RANKINGS.with(|s| {
            s.borrow_mut().push((epoch, entries.to_vec()));
        });
    }
}

/// Test helper: drain everything the mock `RankingsSink` has
/// recorded since the last drain (or since `new_test_ext`). Each
/// element is one `(epoch, entries)` push.
pub fn mock_drain_pushed_rankings() -> Vec<(u64, Vec<EpochWeightEntry>)> {
    PUSHED_RANKINGS.with(|s| core::mem::take(&mut *s.borrow_mut()))
}

/// Test helper: clear the mock `RankingsSink` recording without
/// returning the contents. Called by `new_test_ext`.
pub fn mock_clear_pushed_rankings() {
    PUSHED_RANKINGS.with(|s| s.borrow_mut().clear());
}

parameter_types! {
    /// Pallet-instance discriminator pinned by the runtime so
    /// `AggregateView::pallet_instance` checks have a stable
    /// target across tests.
    pub const ComputePalletInstance: [u8; 32] = [0xC0; 32];

    /// Chain-genesis discriminator pinned by the runtime — must
    /// match `AggregateView::chain_genesis` exactly. Tests use a
    /// fixed sentinel so the §23 replay-domain check is
    /// deterministic (NOT `frame_system::block_hash(0)`, which
    /// gets pruned after `BlockHashCount` blocks in production).
    pub const ComputeChainGenesis: [u8; 32] = [0x6E; 32];
}

impl pallet_compute_scoring::Config for TestRuntime {
    type RuntimeEvent = RuntimeEvent;
    type ComputeScoringAdminOrigin = frame_system::EnsureRoot<AccountId>;
    type AuditAuthorityOrigin = frame_system::EnsureRoot<AccountId>;
    type DepositCurrency = Balances;
    type FamilyRegistry = DummyFamilyRegistry;
    type ProxyVerifier = DummyProxyVerifier;
    type Registration = MockRegistration;
    type RankingsSink = MockRankingsSink;

    // Registration economics — small caps for test brevity.
    type MaxFamilies = ConstU32<16>;
    type MaxChildrenTotal = ConstU32<128>;
    type MaxChildrenPerFamily = ConstU32<16>;
    type BaseChildDeposit = ConstU128<1>;
    type GlobalDepositHalvingPeriodBlocks = ConstU64<1024>;
    type UnregisterCooldownBlocks = ConstU64<32>;
    type UnbondingPeriodBlocks = ConstU64<64>;
    type WeightInfo = ();

    // PR-I3 audit-stats.
    type MaxAggregateBody = ConstU32<8192>;
    type MaxValidatorIdLen = ConstU32<64>;
    type MaxFamilyIdLen = ConstU32<64>;
    type MaxAuditVmKeyIdLen = ConstU32<64>;
    type ComputePalletInstance = ComputePalletInstance;
    type ComputeChainGenesis = ComputeChainGenesis;
    type NowUnix = Timestamp;

    // PR-I4 epoch close. Mock bound at 128; production runtimes
    // pick from block-weight benchmarks (codex/gemini review LOW:
    // recommend 64–128 starting point, with pagination as a
    // PR-I4.1 follow-up if the network grows past one batch per
    // epoch — see CHANGES.md PR-I4).
    type MaxMinerStatusUpdatesPerCall = ConstU32<128>;
    // Deliberately small so the budget-exhaustion / resume path is
    // reachable in tests without seeding thousands of keys.
    type MaxEpochPruneKeysPerCall = ConstU32<16>;

    // #322 live attestation. Body bound ≥ canonical 360 B + slack;
    // vm_id bound matches the production format (`tnXxxxxx…`);
    // pubkey allowlist sized for one prod KBS + 2 rollover keys.
    type MaxLiveAttestationBody = ConstU32<1024>;
    type MaxVmIdLen = ConstU32<64>;
    type MaxKbsAttestationPubkeys = ConstU32<4>;
}

/// Unix-time value the mock's `Timestamp` pallet exposes via
/// `frame_support::traits::UnixTime` — `set_timestamp` writes
/// milliseconds, so we deliberately pick a value well below any
/// test's `view.expiry` (which is in seconds).
pub const TEST_NOW_MILLIS: u64 = 1_700_005_000_000; // = 1_700_005_000 sec

pub fn new_test_ext() -> sp_io::TestExternalities {
    let storage = frame_system::GenesisConfig::<TestRuntime>::default()
        .build_storage()
        .expect("genesis storage builds");
    let mut ext: sp_io::TestExternalities = storage.into();
    // Advance to block 1 so `frame_system::Pallet::events()` is
    // populated for assertions (events from block 0 are discarded
    // by the system pallet). Also seed Timestamp so `NowUnix`
    // returns a sane value for `AggregateView::expiry` checks.
    ext.execute_with(|| {
        frame_system::Pallet::<TestRuntime>::set_block_number(1);
        pallet_timestamp::Pallet::<TestRuntime>::set_timestamp(TEST_NOW_MILLIS);
        // PR-I5: clear + pre-register the canonical test `NODE_ID`
        // so audit-stats happy/sad paths exercise the post-
        // registration code paths by default. Tests that need to
        // exercise the "unregistered node rejected" gate clear
        // the registry inside their body via
        // `mock_clear_registered_nodes()`.
        mock_clear_registered_nodes();
        mock_register_node(TEST_NODE_ID);
        // PR-I6: also clear the `RankingsSink` recording so every
        // test starts with an empty bridge payload.
        mock_clear_pushed_rankings();
    });
    ext
}

/// Canonical test node id pre-registered by `new_test_ext`. Matches
/// `tests::NODE_ID` (the constant used by every audit-stats test).
pub const TEST_NODE_ID: [u8; 32] = [0xAA; 32];

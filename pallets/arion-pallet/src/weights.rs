//! Weights for `pallet_arion`.
//!
//! The fund-moving and miner-payment paths are MEASURED with frame-benchmarking:
//! `miner_payment_settlement_hook`, `register_child`, `deregister_child`,
//! `claim_unbonded`, `submit_miner_stats` and `set_miner_price`. The remaining
//! entries are still conservative hand-written estimates (single storage ops or
//! paths without a benchmark yet).
//!
//! Measured on: AMD EPYC 9255 24-Core Processor, WASM `Compiled`,
//! chain `benchmark`, STEPS: `50`, REPEAT: `20`, DATE: 2026-10-29.
//!
//! `miner_payment_settlement_hook`'s per-child (`c`) and per-family (`f`) terms
//! are the `bond_extra` + bags-rebag figures measured on `main` (2026-10-27);
//! the departed-payout (`d`) term is the #79 measurement, which is bond-path
//! independent (departed families are paid in free balance, no bond) and so
//! unaffected by the `bond_extra` change. See the note on the function itself.
//!
//! To re-measure (on reference hardware):
//! ```bash
//! cargo build --release --features runtime-benchmarks
//! ./target/release/hippius benchmark pallet \
//!     --chain benchmark \
//!     --pallet pallet_arion \
//!     --extrinsic 'miner_payment_settlement_hook,register_child,deregister_child,claim_unbonded,submit_miner_stats,set_miner_price' \
//!     --steps 50 --repeat 20 \
//!     --output /tmp/arion_weights.rs
//! ```
//! then merge the measured functions into this file's `SubstrateWeight<T>` and
//! `()` impls (do not point `--output` at this file — the generator drops the
//! hand-estimated entries).

#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]
#![allow(missing_docs)]

use frame_support::{traits::Get, weights::{Weight, constants::RocksDbWeight}};
use sp_std::marker::PhantomData;

/// Storage-read budget for [`Pallet::update_user_file_size`] worst-case weight.
///
/// Covers `get_primary_account` when the caller has no local proxy defs (full validator scan +
/// `proxies` per owner) and `get_registered_node_for_owner` (two registration maps). This is a
/// **conservative** upper bound — re-benchmark or raise if on-chain registration counts grow past it.
const UPDATE_USER_FILE_SIZE_READ_BUDGET: u64 = 8_192;

/// Weight functions needed for pallet_arion.
pub trait WeightInfo {
    fn submit_crush_map(n: u32) -> Weight;
    fn submit_miner_stats(n: u32) -> Weight;
    /// `n` = quality rows in the batch. `families` / `children_per_family` bound the
    /// family-weight recompute page, whose cost scales with their product and is not
    /// implied by `n` — an unbounded recompute is what makes actual work outrun the
    /// declared weight as the family set grows.
    fn submit_node_quality(n: u32, families: u32, children_per_family: u32) -> Weight;
    fn submit_attestations(n: u32) -> Weight;
    fn submit_attestation_commitment() -> Weight;
    fn register_child() -> Weight;
    fn deregister_child() -> Weight;
    fn claim_unbonded() -> Weight;
    fn set_lockup_enabled() -> Weight;
    fn set_base_child_deposit() -> Weight;
    fn set_free_child_slots_per_family() -> Weight;
    fn register_warden() -> Weight;
    fn deregister_warden() -> Weight;
    fn prune_attestation_buckets(n: u32) -> Weight;
    fn prune_historical_crush_epochs(n: u32) -> Weight;
    /// `scan` = entries walked, `prune` = entries actually removed. Split because the
    /// walk is bounded by `max_scan` while removals are bounded by the far smaller
    /// `max_children`; keying writes on `scan` over-declares them by the ratio between
    /// the two, reserving most of a block for writes that cannot happen.
    fn prune_stale_node_weights(scan: u32, prune: u32) -> Weight;
    /// See [`UPDATE_USER_FILE_SIZE_READ_BUDGET`].
    fn update_user_file_size() -> Weight;
    /// `on_initialize` miner stats prune: bounded scan + protected-uid pass over registrations.
    fn miner_stats_prune_hook(max_scan: u32, max_children_total: u32) -> Weight;
    fn set_miner_price() -> Weight;
    /// `on_initialize` payment settlement: accrue + aggregate per family + transfer + bond,
    /// then a bounded page of departed-family debt payouts.
    fn miner_payment_settlement_hook(
        max_children_total: u32,
        max_families: u32,
        max_departed: u32,
    ) -> Weight;
}

/// Weights for pallet_arion using the Substrate node and recommended hardware.
pub struct SubstrateWeight<T>(PhantomData<T>);

impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
    /// Storage: Arion CurrentEpoch (r:1 w:1)
    /// Storage: Arion CrushMap (r:0 w:1)
    /// Storage: Arion CrushParams (r:0 w:1)
    /// Storage: Arion MapRootHash (r:0 w:1)
    /// The weight includes `n` miners being processed.
    fn submit_crush_map(n: u32) -> Weight {
        // Base: read epoch + write 4 storage items; up to 4 extra writes when pruning an old epoch
        // Per miner: bounded vec encoding overhead, plus uid binding. Worst case
        // is a full reassignment: reads NodeIdToChild + ChildMinerUid +
        // MinerUidToChild + the accrual, and writes a detach (stats, both index
        // directions, accrual) followed by an install (both directions, accrual).
        Weight::from_parts(25_000_000, 0)
            .saturating_add(Weight::from_parts(500_000, 0).saturating_mul(n.into()))
            .saturating_add(T::DbWeight::get().reads(1))
            .saturating_add(T::DbWeight::get().writes(4))
            .saturating_add(T::DbWeight::get().writes(4))
            .saturating_add(T::DbWeight::get().reads(4u64.saturating_mul(n.into())))
            .saturating_add(T::DbWeight::get().writes(7u64.saturating_mul(n.into())))
    }

    /// **Measured** (`benchmarking.rs`, steps 50, repeat 20, every uid
    /// claimed so each update takes the accrual path).
    ///
    /// Storage: `Arion::CurrentStatsBucket` (r:1 w:1)
    /// Storage: `Arion::MinerUidToChild` (r:n) — the claim check
    /// Storage: `Arion::MinerStatsByUid` (r:n w:n) — read for accrual, then overwritten
    /// Storage: `Arion::MinerAccruals` (r:n w:n)
    /// Storage: `Arion::CurrentNetworkTotals` (w:1)
    fn submit_miner_stats(n: u32) -> Weight {
        Weight::from_parts(11_911_721, 1703)
            .saturating_add(Weight::from_parts(11_042_655, 2650).saturating_mul(n.into()))
            .saturating_add(T::DbWeight::get().reads(1))
            .saturating_add(T::DbWeight::get().reads((3_u64).saturating_mul(n.into())))
            .saturating_add(T::DbWeight::get().writes(2))
            .saturating_add(T::DbWeight::get().writes((2_u64).saturating_mul(n.into())))
    }

    /// Storage: Arion CurrentWeightBucket (r:1 w:1)
    /// Storage: Arion ChildRegistrations (r:n w:0) - verify each child is registered
    /// Storage: Arion NodeQuality (r:0 w:n)
    /// Storage: Arion NodeWeightByChild (r:0 w:n)
    /// Storage: Arion FamilyWeight (r:n w:n) - read and write for each family
    /// Storage: Arion FamilyFirstSeenBucket (r:n w:0) - newcomer check
    fn submit_node_quality(n: u32, families: u32, children_per_family: u32) -> Weight {
        // Base cost + per-node writes + family weight recalculation
        // Family weight calculation involves log2 operations and EMA
        // Reads: 1 base + n child registrations + n family weights + n first seen buckets
        // Writes: 1 base + n qualities + n node weights + n family weights
        let child_slots = (families as u64).saturating_mul(children_per_family as u64);
        Weight::from_parts(20_000_000, 0)
            .saturating_add(Weight::from_parts(5_000_000, 0).saturating_mul(n.into()))
            .saturating_add(T::DbWeight::get().reads(1)) // CurrentWeightBucket
            .saturating_add(T::DbWeight::get().reads(n.into())) // ChildRegistrations
            .saturating_add(T::DbWeight::get().writes(1)) // CurrentWeightBucket
            .saturating_add(T::DbWeight::get().writes(n.into())) // NodeQuality
            .saturating_add(T::DbWeight::get().writes(n.into())) // NodeWeightByChild
            // Family recompute page: per family the FamilyActiveChildren iteration,
            // FamilyChildren, FamilyFirstSeenBucket and FamilyWeight reads; per child slot
            // the ChildRegistrations + NodeWeightLastBucket + NodeQualityByChild reads that
            // `compute_family_weight_from_nodes` performs. Writes are FamilyWeightRaw +
            // FamilyWeight per family, plus the cursor.
            .saturating_add(T::DbWeight::get().reads(1)) // FamilyRecomputeCursor
            .saturating_add(T::DbWeight::get().reads((families as u64).saturating_mul(4)))
            .saturating_add(T::DbWeight::get().reads(child_slots.saturating_mul(3)))
            .saturating_add(T::DbWeight::get().writes(1)) // FamilyRecomputeCursor
            .saturating_add(T::DbWeight::get().writes((families as u64).saturating_mul(2)))
            .saturating_add(Weight::from_parts(2_000_000, 0).saturating_mul(child_slots))
    }

    /// Storage: Arion AuditResults (r:n w:n)
    /// Per attestation:
    /// - Ed25519 signature verification (~50,000 gas equivalent)
    /// - SCALE encoding for signature message
    /// - Storage write
    fn submit_attestations(n: u32) -> Weight {
        // Ed25519 verification is the dominant cost (~50us per signature on reference hardware)
        // SCALE encoding of signing message adds overhead
        // Storage writes for each attestation result
        let base = Weight::from_parts(10_000_000, 0);
        let per_attestation = Weight::from_parts(
            // Ed25519 verify: ~50,000,000 pico-seconds (50us)
            // SCALE encode: ~5,000,000 pico-seconds (5us)
            // Storage: ~10,000,000 pico-seconds (10us)
            65_000_000,
            0
        );
        // Bounded duplicate checks: O(n^2) within batch and O(n * existing) vs stored vec;
        // both lengths are capped by MaxAttestations (~n).
        let n_sq = n.saturating_mul(n);
        let dedup = Weight::from_parts(3_000, 0).saturating_mul(n_sq.into());

        base
            .saturating_add(per_attestation.saturating_mul(n.into()))
            .saturating_add(dedup)
            .saturating_add(T::DbWeight::get().reads(n.into()))
            .saturating_add(T::DbWeight::get().writes(n.into()))
    }

    /// Storage: Arion EpochAttestationCommitments (r:1 w:1)
    fn submit_attestation_commitment() -> Weight {
        Weight::from_parts(15_000_000, 0)
            .saturating_add(T::DbWeight::get().reads(1))
            .saturating_add(T::DbWeight::get().writes(1))
    }

    /// Measured: paid registration path (deposit reserved) with a real ed25519
    /// node signature. See the module header for the benchmark run details.
    fn register_child() -> Weight {
        // Minimum execution time: 91_617_000 picoseconds.
        Weight::from_parts(96_163_000, 0)
            .saturating_add(Weight::from_parts(0, 4834))
            .saturating_add(T::DbWeight::get().reads(23))
            .saturating_add(T::DbWeight::get().writes(11))
    }

    /// Measured: deregistering a family's only active child (the family-cleanup
    /// branch runs). See the module header for the benchmark run details.
    fn deregister_child() -> Weight {
        // Minimum execution time: 51_606_000 picoseconds.
        Weight::from_parts(52_899_000, 0)
            .saturating_add(Weight::from_parts(0, 4063))
            .saturating_add(T::DbWeight::get().reads(11))
            .saturating_add(T::DbWeight::get().writes(19))
    }

    /// Measured: claiming a deregistered child's reserved deposit after the
    /// unbonding period. See the module header for the benchmark run details.
    fn claim_unbonded() -> Weight {
        // Minimum execution time: 36_454_000 picoseconds.
        Weight::from_parts(37_326_000, 0)
            .saturating_add(Weight::from_parts(0, 3941))
            .saturating_add(T::DbWeight::get().reads(5))
            .saturating_add(T::DbWeight::get().writes(9))
    }

    /// Storage: Arion LockupEnabled (r:0 w:1)
    fn set_lockup_enabled() -> Weight {
        Weight::from_parts(5_000_000, 0)
            .saturating_add(T::DbWeight::get().writes(1))
    }

    /// Storage: Arion BaseChildDepositValue (r:0 w:1)
    fn set_base_child_deposit() -> Weight {
        Weight::from_parts(5_000_000, 0)
            .saturating_add(T::DbWeight::get().writes(1))
    }

    /// Storage: Arion FreeChildSlotsPerFamily (r:0 w:1)
    fn set_free_child_slots_per_family() -> Weight {
        Weight::from_parts(5_000_000, 0)
            .saturating_add(T::DbWeight::get().writes(1))
    }

    /// Storage: Arion RegisteredWardens (r:1 w:1)
    fn register_warden() -> Weight {
        Weight::from_parts(20_000_000, 0)
            .saturating_add(T::DbWeight::get().reads(1))
            .saturating_add(T::DbWeight::get().writes(1))
    }

    /// Storage: Arion RegisteredWardens (r:1 w:1)
    fn deregister_warden() -> Weight {
        Weight::from_parts(20_000_000, 0)
            .saturating_add(T::DbWeight::get().reads(1))
            .saturating_add(T::DbWeight::get().writes(1))
    }

    /// Storage: Arion AttestationsByBucket (r:n w:n)
    /// Per bucket: check if exists, then remove all attestations for that bucket
    fn prune_attestation_buckets(n: u32) -> Weight {
        // Each bucket removal requires checking existence and clearing storage
        Weight::from_parts(10_000_000, 0)
            .saturating_add(Weight::from_parts(5_000_000, 0).saturating_mul(n.into()))
            .saturating_add(T::DbWeight::get().reads(n.into())) // contains_key checks
            .saturating_add(T::DbWeight::get().writes(n.into())) // remove_prefix calls
    }

    /// Per epoch: remove EpochParams, EpochMiners, EpochRoot, EpochAttestationCommitments
    fn prune_historical_crush_epochs(n: u32) -> Weight {
        Weight::from_parts(12_000_000, 0)
            .saturating_add(Weight::from_parts(4_000_000, 0).saturating_mul(n.into()))
            .saturating_add(T::DbWeight::get().reads(1))
            .saturating_add(T::DbWeight::get().writes((n as u64).saturating_mul(4)))
    }

    /// Reads scale with `scan`: NodeWeightByChild iteration + NodeWeightLastBucket +
    /// ChildRegistrations per scanned entry, plus a ChildRegistrations re-read and a
    /// family lookup per pruned child, plus the cursor. Writes scale with `prune`
    /// only: weight-trio removals (w:3) plus family sweep (w:3 worst case), plus the
    /// cursor.
    fn prune_stale_node_weights(scan: u32, prune: u32) -> Weight {
        Weight::from_parts(12_000_000, 0)
            .saturating_add(Weight::from_parts(6_000_000, 0).saturating_mul(scan.into()))
            .saturating_add(T::DbWeight::get().reads(
                2_u64
                    .saturating_add((scan as u64).saturating_mul(3))
                    .saturating_add((prune as u64).saturating_mul(2)),
            ))
            .saturating_add(
                T::DbWeight::get().writes(1_u64.saturating_add((prune as u64).saturating_mul(6))),
            )
    }

    /// Storage: Proxy Proxies (r:1+), Registration maps (iteration), UserTotalFilesSize (r:1 w:1),
    /// UserTotalFilesCount (r:1 w:1)
    fn update_user_file_size() -> Weight {
        Weight::from_parts(35_000_000, 0)
            .saturating_add(T::DbWeight::get().reads(UPDATE_USER_FILE_SIZE_READ_BUDGET))
            .saturating_add(T::DbWeight::get().writes(2))
    }

    fn miner_stats_prune_hook(max_scan: u32, max_children_total: u32) -> Weight {
        let scan = max_scan as u64;
        let cap = max_children_total as u64;
        // Protected set: iteration over registrations + uid lookups + one epoch miners read (upper bound).
        let reg_reads = cap.saturating_mul(3).saturating_add(1);
        Weight::from_parts(20_000_000, 0)
            .saturating_add(T::DbWeight::get().reads(scan.saturating_add(reg_reads)))
            .saturating_add(T::DbWeight::get().writes(scan))
    }

    /// **Measured.** Storage: `Arion::MinerPriceUsdPerGbBlock` (w:1)
    fn set_miner_price() -> Weight {
        Weight::from_parts(5_940_000, 0).saturating_add(T::DbWeight::get().writes(1))
    }

    /// **Measured.** Every child payable, every family carrying arrears, the bank
    /// paying in full into fresh accounts and each payout taking a `bond_extra` —
    /// the heavier branch families take from the second settlement on, posed by
    /// pre-bonding and nominating every paid family in the benchmark setup.
    ///
    /// Per child (4 r, 1 w): `ChildRegistrations`, `ChildMinerUid`,
    /// `MinerStatsByUid` reads and a `MinerAccruals` r/w. Per family (8 r, 5 w):
    /// `FamilyArrears` r/w, the bank transfer (`System::Account` r/w), and the
    /// `bond_extra` — `Staking::Bonded`, `VirtualStakers`, `Balances::Freezes`
    /// reads plus `Staking::Ledger`, `Balances::Locks` and the nominating
    /// family's `BagsList::ListNodes` rebag, all r/w. Fixed: the two prices, the
    /// bank's reserve ledgers and requester checks, its payout totals, the
    /// bags-list bag headers, `LastSettlementBlock`.
    ///
    /// The `c` and `f` terms are the values measured on `main` (2026-10-27, steps
    /// 50 / repeat 20, dedicated Hippius benchmark VM, AMD EPYC 7543 32-Core
    /// Processor). At the runtime's caps (1_000 children, 600 families) the
    /// active-settlement part is ~148.4ms of execution plus 8_853 reads and
    /// 4_041 writes, once per settlement interval.
    ///
    /// The `d` term is the bounded departed-family payout page (#79): free-balance
    /// transfers with no bond, so it is independent of the `bond_extra` change —
    /// 2 r/w and a proof-size slice per paid departed family, with no per-unit
    /// ref-time term (within standard error its cost is in the reads/writes).
    /// Range: `c ∈ [0, 1000]`, `f ∈ [0, 600]`, `d ∈ [0, 10]`.
    fn miner_payment_settlement_hook(
        max_children_total: u32,
        max_families: u32,
        max_departed: u32,
    ) -> Weight {
        let (c, f, d) = (max_children_total as u64, max_families as u64, max_departed as u64);
        Weight::from_parts(28_144_331_000, 92522)
            .saturating_add(Weight::from_parts(11_371_952, 2796).saturating_mul(c))
            .saturating_add(Weight::from_parts(181_487_445, 3774).saturating_mul(f))
            // Departed-family payout page (#79): bond-independent, 2 r/w + proof per `d`.
            .saturating_add(Weight::from_parts(0, 2609).saturating_mul(d))
            .saturating_add(T::DbWeight::get().reads(53_u64.saturating_add(c.saturating_mul(4)).saturating_add(f.saturating_mul(8)).saturating_add(d.saturating_mul(2))))
            .saturating_add(T::DbWeight::get().writes(41_u64.saturating_add(c).saturating_add(f.saturating_mul(5)).saturating_add(d.saturating_mul(2))))
    }
}

/// For backwards compatibility and testnets
impl WeightInfo for () {
    fn submit_crush_map(n: u32) -> Weight {
        Weight::from_parts(25_000_000, 0)
            .saturating_add(Weight::from_parts(500_000, 0).saturating_mul(n.into()))
            .saturating_add(RocksDbWeight::get().reads(1))
            .saturating_add(RocksDbWeight::get().writes(4))
    }

    fn submit_miner_stats(n: u32) -> Weight {
        Weight::from_parts(11_911_721, 1703)
            .saturating_add(Weight::from_parts(11_042_655, 2650).saturating_mul(n.into()))
            .saturating_add(RocksDbWeight::get().reads(1))
            .saturating_add(RocksDbWeight::get().reads((3_u64).saturating_mul(n.into())))
            .saturating_add(RocksDbWeight::get().writes(2))
            .saturating_add(RocksDbWeight::get().writes((2_u64).saturating_mul(n.into())))
    }

    fn submit_node_quality(n: u32, families: u32, children_per_family: u32) -> Weight {
        let child_slots = (families as u64).saturating_mul(children_per_family as u64);
        Weight::from_parts(20_000_000, 0)
            .saturating_add(Weight::from_parts(5_000_000, 0).saturating_mul(n.into()))
            .saturating_add(RocksDbWeight::get().reads(1)) // CurrentWeightBucket
            .saturating_add(RocksDbWeight::get().reads(n.into())) // ChildRegistrations
            .saturating_add(RocksDbWeight::get().writes(1)) // CurrentWeightBucket
            .saturating_add(RocksDbWeight::get().writes(n.into())) // NodeQuality
            .saturating_add(RocksDbWeight::get().writes(n.into())) // NodeWeightByChild
            .saturating_add(RocksDbWeight::get().reads(1)) // FamilyRecomputeCursor
            .saturating_add(RocksDbWeight::get().reads((families as u64).saturating_mul(4)))
            .saturating_add(RocksDbWeight::get().reads(child_slots.saturating_mul(3)))
            .saturating_add(RocksDbWeight::get().writes(1)) // FamilyRecomputeCursor
            .saturating_add(RocksDbWeight::get().writes((families as u64).saturating_mul(2)))
            .saturating_add(Weight::from_parts(2_000_000, 0).saturating_mul(child_slots))
    }

    fn submit_attestations(n: u32) -> Weight {
        let n_sq = n.saturating_mul(n);
        Weight::from_parts(10_000_000, 0)
            .saturating_add(Weight::from_parts(65_000_000, 0).saturating_mul(n.into()))
            .saturating_add(Weight::from_parts(3_000, 0).saturating_mul(n_sq.into()))
            .saturating_add(RocksDbWeight::get().reads(n.into()))
            .saturating_add(RocksDbWeight::get().writes(n.into()))
    }

    fn submit_attestation_commitment() -> Weight {
        Weight::from_parts(15_000_000, 0)
            .saturating_add(RocksDbWeight::get().reads(1))
            .saturating_add(RocksDbWeight::get().writes(1))
    }

    fn register_child() -> Weight {
        // Minimum execution time: 91_617_000 picoseconds.
        Weight::from_parts(96_163_000, 0)
            .saturating_add(Weight::from_parts(0, 4834))
            .saturating_add(RocksDbWeight::get().reads(23))
            .saturating_add(RocksDbWeight::get().writes(11))
    }

    fn deregister_child() -> Weight {
        // Minimum execution time: 51_606_000 picoseconds.
        Weight::from_parts(52_899_000, 0)
            .saturating_add(Weight::from_parts(0, 4063))
            .saturating_add(RocksDbWeight::get().reads(11))
            .saturating_add(RocksDbWeight::get().writes(19))
    }

    fn claim_unbonded() -> Weight {
        // Minimum execution time: 36_454_000 picoseconds.
        Weight::from_parts(37_326_000, 0)
            .saturating_add(Weight::from_parts(0, 3941))
            .saturating_add(RocksDbWeight::get().reads(5))
            .saturating_add(RocksDbWeight::get().writes(9))
    }

    fn set_lockup_enabled() -> Weight {
        Weight::from_parts(5_000_000, 0)
            .saturating_add(RocksDbWeight::get().writes(1))
    }

    fn set_base_child_deposit() -> Weight {
        Weight::from_parts(5_000_000, 0)
            .saturating_add(RocksDbWeight::get().writes(1))
    }

    fn set_free_child_slots_per_family() -> Weight {
        Weight::from_parts(5_000_000, 0)
            .saturating_add(RocksDbWeight::get().writes(1))
    }

    fn register_warden() -> Weight {
        Weight::from_parts(20_000_000, 0)
            .saturating_add(RocksDbWeight::get().reads(1))
            .saturating_add(RocksDbWeight::get().writes(1))
    }

    fn deregister_warden() -> Weight {
        Weight::from_parts(20_000_000, 0)
            .saturating_add(RocksDbWeight::get().reads(1))
            .saturating_add(RocksDbWeight::get().writes(1))
    }

    fn prune_attestation_buckets(n: u32) -> Weight {
        Weight::from_parts(10_000_000, 0)
            .saturating_add(Weight::from_parts(5_000_000, 0).saturating_mul(n.into()))
            .saturating_add(RocksDbWeight::get().reads(n.into())) // contains_key checks
            .saturating_add(RocksDbWeight::get().writes(n.into())) // remove_prefix calls
    }

    fn prune_historical_crush_epochs(n: u32) -> Weight {
        Weight::from_parts(12_000_000, 0)
            .saturating_add(Weight::from_parts(4_000_000, 0).saturating_mul(n.into()))
            .saturating_add(RocksDbWeight::get().reads(1))
            .saturating_add(RocksDbWeight::get().writes((n as u64).saturating_mul(4)))
    }

    fn prune_stale_node_weights(scan: u32, prune: u32) -> Weight {
        Weight::from_parts(12_000_000, 0)
            .saturating_add(Weight::from_parts(6_000_000, 0).saturating_mul(scan.into()))
            .saturating_add(RocksDbWeight::get().reads(
                2_u64
                    .saturating_add((scan as u64).saturating_mul(3))
                    .saturating_add((prune as u64).saturating_mul(2)),
            ))
            .saturating_add(
                RocksDbWeight::get().writes(1_u64.saturating_add((prune as u64).saturating_mul(6))),
            )
    }

    fn update_user_file_size() -> Weight {
        Weight::from_parts(35_000_000, 0)
            .saturating_add(RocksDbWeight::get().reads(UPDATE_USER_FILE_SIZE_READ_BUDGET))
            .saturating_add(RocksDbWeight::get().writes(2))
    }

    fn miner_stats_prune_hook(max_scan: u32, max_children_total: u32) -> Weight {
        let scan = max_scan as u64;
        let cap = max_children_total as u64;
        let reg_reads = cap.saturating_mul(3).saturating_add(1);
        Weight::from_parts(20_000_000, 0)
            .saturating_add(RocksDbWeight::get().reads(scan.saturating_add(reg_reads)))
            .saturating_add(RocksDbWeight::get().writes(scan))
    }

    fn set_miner_price() -> Weight {
        Weight::from_parts(5_940_000, 0).saturating_add(RocksDbWeight::get().writes(1))
    }

    // `c`/`f` measured on `main` (`bond_extra` + rebag); `d` is the #79
    // departed-payout page (bond-independent). See the `SubstrateWeight` impl.
    fn miner_payment_settlement_hook(
        max_children_total: u32,
        max_families: u32,
        max_departed: u32,
    ) -> Weight {
        let (c, f, d) = (max_children_total as u64, max_families as u64, max_departed as u64);
        Weight::from_parts(21_048_622_000, 6997)
            .saturating_add(Weight::from_parts(7_626_195, 2797).saturating_mul(c))
            .saturating_add(Weight::from_parts(86_367_685, 3774).saturating_mul(f))
            .saturating_add(Weight::from_parts(0, 2609).saturating_mul(d))
            .saturating_add(RocksDbWeight::get().reads(16_u64.saturating_add(c.saturating_mul(4)).saturating_add(f.saturating_mul(8)).saturating_add(d.saturating_mul(2))))
            .saturating_add(RocksDbWeight::get().writes(4_u64.saturating_add(c).saturating_add(f.saturating_mul(4)).saturating_add(d.saturating_mul(2))))
    }
}
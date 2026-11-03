//! Benchmarks for the arion fund-moving and miner-payment paths.
//!
//! The headline entry is `miner_payment_settlement_hook`. It is the
//! `on_initialize` settlement tick, not an extrinsic, so it is measured with
//! `#[block]` over the internal `settle_miner_payments` rather than being given
//! a benchmark-only dispatch entry. It is posed at its worst case across the
//! three dimensions its declared weight takes (#79):
//!
//! - `c` active children up to `MaxChildrenTotal`, seeded across `FamilyChildren`
//!   with a payable accrual each, so the per-child accrue/convert loop runs in full;
//! - `f` families carrying arrears up to `MaxFamilies`, so the per-family payout +
//!   bond loop runs in full;
//! - `d` departed families up to `MaxDepartedPayoutsPerBlock`, each owed the max the
//!   ledger holds, so the bounded departed-payout page runs in full.
//!
//! Every family that is *paid* (children-families and arrears-families, but not
//! the departed ones, which are paid in free balance) is pre-bonded and
//! nominated first, so settlement takes the heavier `bond_extra` branch *and*
//! the bags-list rebag a nominating family drives — the shape every settlement
//! after the first one meets — rather than the cheaper first-time `bond`. Without
//! this the hook's declared weight, mandatory in `on_initialize`, is measured
//! short.
//!
//! The payout source is primed generously (`ArionBenchmarkHelper::prime_payout_source`)
//! so every payment *succeeds*: a successful payout is the worst case — it writes
//! the staking bond and emits `FamilyPaid`, where a refused one would `continue`
//! and touch less. The token price feed is set the same way so settlement does not
//! early-return on an unpriced token.
//!
//! `register_child` / `deregister_child` / `claim_unbonded` are the child-deposit
//! lifecycle. `register_child` is posed on the *paid* path (lockup on, no free
//! slots) so the deposit reserve is included, and signs the node message with a
//! keystore-generated ed25519 key so the on-chain signature check runs for real.
//! The other two seed an active / unbonding child directly rather than going
//! through `register_child`, so the measured block is only the call under test.
//!
//! `submit_miner_stats` is measured per update with every uid claimed, so each
//! update takes the accrual path as well as the stats write. `set_miner_price`
//! is the single price write.
//!
//! Run (see the module-level runbook the PR links):
//!
//! ```text
//! cargo build --release --features runtime-benchmarks
//! ./target/release/hippius benchmark pallet \
//!     --chain dev \
//!     --pallet pallet_arion \
//!     --extrinsic 'miner_payment_settlement_hook,register_child,deregister_child,claim_unbonded,submit_miner_stats,set_miner_price' \
//!     --steps 50 --repeat 20 \
//!     --output pallets/arion-pallet/src/weights.rs
//! ```

#![cfg(feature = "runtime-benchmarks")]

use super::*;
use frame_benchmarking::v2::*;
use frame_support::traits::{Currency, EnsureOrigin, ReservableCurrency};
use frame_system::RawOrigin;
use sp_core::crypto::KeyTypeId;
use sp_runtime::traits::{SaturatedConversion, Zero};
use sp_std::vec;

/// Keystore key type used only to produce a valid node signature in the
/// `register_child` benchmark. Not used anywhere outside benchmarking.
const NODE_KEY_TYPE: KeyTypeId = KeyTypeId(*b"arnb");

/// Account seed shared by the generated benchmark accounts.
const SEED: u32 = 0;

type BalanceOf<T> = <<T as Config>::DepositCurrency as Currency<
	<T as frame_system::Config>::AccountId,
>>::Balance;

/// A per-child accrued byte-block total that converts to a non-zero,
/// sanely-sized token payout given the prices set below (~1 token/child).
const BYTE_BLOCKS: u128 = 1_000_000_000;

/// 1.0 in the pallet's 18-decimal fixed point.
const E18: u128 = 1_000_000_000_000_000_000;

/// Prices in USD fixed-point (18 decimals): 1 USD/GiB/block, 1 USD/token.
const PRICE_USD_PER_GB_BLOCK: u128 = E18;
const TOKEN_PRICE_USD: u128 = E18;

/// Per-family arrears / departed debt seeded on each row (1 token).
const OWED: u128 = E18;

/// Deposit forced on the paid `register_child` path.
const DEPOSIT: u128 = 1_000_000_000_000;

/// Raw shard bytes each seeded miner holds in the stats benchmark: one GiB.
const SHARD_BYTES: u128 = 1 << 30;

/// Blocks between the seeded accruals and the stats update.
const ELAPSED: u32 = 1_000;

/// Validator-observed stats for the `submit_miner_stats` benchmark.
fn stats(bucket: u32) -> MinerStats {
	MinerStats {
		shard_count: 1_000,
		shard_data_bytes: SHARD_BYTES,
		strikes: 0,
		last_seen_bucket: bucket,
		bandwidth_bytes: 1_000_000,
		integrity_fails: 0,
	}
}

/// Claim `uid` for a fresh child of `family`, with stats and an accrual
/// starting at `since`. Used by the stats benchmark.
fn seed_miner<T: Config>(uid: u32, family: &T::AccountId, since: BlockNumberFor<T>) {
	let child: T::AccountId = account("child", uid, SEED);
	let mut node_id = [0u8; 32];
	node_id[..4].copy_from_slice(&uid.to_le_bytes());
	ChildRegistrations::<T>::insert(
		&child,
		ChildRegistration {
			family: family.clone(),
			node_id,
			status: ChildStatus::Active,
			deposit: 0u32.into(),
			unbonding_end: 0u32.into(),
		},
	);
	ChildMinerUid::<T>::insert(&child, uid);
	MinerUidToChild::<T>::insert(uid, &child);
	MinerStatsByUid::<T>::insert(uid, stats(0));
	MinerAccruals::<T>::insert(uid, MinerAccrual { byte_blocks: 0, last_block: since });
}

/// Seed the full settlement worst case: `c` active children (grouped into
/// families of `MaxChildrenPerFamily`), `f` arrears-carrying families, and `d`
/// departed families owed the ledger max. Every paid family (children-families
/// and arrears-families, not the departed ones) is pre-bonded and nominated so
/// settlement takes the heavier `bond_extra` + bags-rebag branch. Returns the
/// block to settle at.
fn seed_settlement<T: Config>(c: u32, f: u32, d: u32) -> BlockNumberFor<T> {
	let now: BlockNumberFor<T> = 1_000u32.into();
	frame_system::Pallet::<T>::set_block_number(now);

	// Prices so settlement does not early-return and accruals convert to >0 tokens.
	MinerPriceUsdPerGbBlock::<T>::put(PRICE_USD_PER_GB_BLOCK);
	T::BenchmarkHelper::set_token_price(TOKEN_PRICE_USD);

	// Prime the payout source generously so every payout succeeds — the worst
	// case, since a successful payout writes the bond and emits the event.
	let requester = Pallet::<T>::account_id();
	let pool: BalanceOf<T> =
		1_000_000_000_000_000_000_000_000_000_000u128.saturated_into(); // 1e30
	T::BenchmarkHelper::prime_payout_source(&requester, pool);

	// Amount to open each family's staking ledger with, so the payout bonds via
	// the heavier `bond_extra` branch rather than a first-time `bond`.
	let prebond: BalanceOf<T> = OWED.saturated_into();

	// `c` active children, grouped into families at the per-family cap.
	let per_family = T::MaxChildrenPerFamily::get().max(1);
	for i in 0..c {
		let fam_idx = i / per_family;
		let family: T::AccountId = account("settle_fam", fam_idx, SEED);
		let child: T::AccountId = account("settle_child", i, SEED);
		let uid = i.saturating_add(1);
		let mut node_id = [0u8; 32];
		node_id[..4].copy_from_slice(&i.to_le_bytes());

		ChildRegistrations::<T>::insert(
			&child,
			ChildRegistration {
				family: family.clone(),
				node_id,
				status: ChildStatus::Active,
				deposit: Zero::zero(),
				unbonding_end: now,
			},
		);
		ChildMinerUid::<T>::insert(&child, uid);
		MinerUidToChild::<T>::insert(uid, &child);
		// `last_block == now` so accrue adds nothing and leaves the already
		// accumulated byte_blocks snapshot payable.
		MinerAccruals::<T>::insert(uid, MinerAccrual { byte_blocks: BYTE_BLOCKS, last_block: now });
		let _ = FamilyChildren::<T>::try_mutate(&family, |v| v.try_push(child.clone()));
		FamilyActiveChildren::<T>::mutate(&family, |n| *n = n.saturating_add(1));
	}
	TotalActiveChildren::<T>::put(c);

	// Pre-bond every children-family, so its accrual payout takes `bond_extra`.
	let child_families = c.saturating_add(per_family - 1) / per_family; // ceil(c / per_family)
	for fam_idx in 0..child_families {
		let family: T::AccountId = account("settle_fam", fam_idx, SEED);
		T::BenchmarkHelper::prebond_family(&family, prebond);
	}

	// `f` families carrying arrears: each becomes a payout-loop entry, pre-bonded
	// so its arrears payout also takes `bond_extra`.
	for j in 0..f {
		let family: T::AccountId = account("arrears_fam", j, SEED);
		FamilyArrears::<T>::insert(&family, OWED);
		T::BenchmarkHelper::prebond_family(&family, prebond);
	}

	// `d` departed families owed the ledger max (bounded payout page). Departed
	// families are paid in free balance (no bond), so they are not pre-bonded.
	for k in 0..d {
		let family: T::AccountId = account("departed_fam", k, SEED);
		DepartedFamilyArrears::<T>::insert(&family, OWED);
	}
	DepartedArrearsCursor::<T>::kill();

	now
}

#[benchmarks]
mod benchmarks {
	use super::*;

	/// Worst-case settlement tick: accrue every active child, pay + bond every
	/// family (on the heavier `bond_extra` path), and drain a full departed-payout
	/// page. Ranges over all three dimensions the declared weight takes.
	#[benchmark]
	fn miner_payment_settlement_hook(
		c: Linear<0, { T::MaxChildrenTotal::get() }>,
		f: Linear<0, { T::MaxFamilies::get() }>,
		d: Linear<0, { T::MaxDepartedPayoutsPerBlock::get() }>,
	) {
		let now = seed_settlement::<T>(c, f, d);

		// Every arrears-family was pre-bonded, so its payout takes `bond_extra`.
		if f > 0 {
			let family: T::AccountId = account("arrears_fam", 0u32, SEED);
			assert!(
				T::Staking::stake(&family).is_ok(),
				"arrears families are pre-bonded, so settlement takes bond_extra"
			);
		}

		#[block]
		{
			Pallet::<T>::settle_miner_payments(now);
		}

		// The tick ran to completion (did not early-return on price/token).
		assert_eq!(LastSettlementBlock::<T>::get(), Some(now));
	}

	/// Paid registration: lockup on, no free slots, so the deposit is reserved,
	/// with a real node signature so the on-chain ed25519 check runs.
	#[benchmark]
	fn register_child() {
		let family: T::AccountId = account("reg_family", 0, SEED);
		let child: T::AccountId = account("reg_child", 0, SEED);

		frame_system::Pallet::<T>::set_block_number(1_000u32.into());

		// Fund the family for the proxy deposit and the registration deposit.
		let funded: BalanceOf<T> = 1_000_000_000_000_000_000_000u128.saturated_into(); // 1e21
		let _ = T::DepositCurrency::deposit_creating(&family, funded);

		// Force the paid path so the deposit reserve is part of the measurement.
		LockupEnabled::<T>::put(true);
		FreeChildSlotsPerFamily::<T>::put(0u32);
		BaseChildDepositValue::<T>::put(DEPOSIT.saturated_into::<BalanceOf<T>>());
		GlobalNextDeposit::<T>::put(DEPOSIT.saturated_into::<BalanceOf<T>>());
		GlobalLastPaidRegistrationBlock::<T>::put(frame_system::Pallet::<T>::block_number());

		// Preconditions the extrinsic enforces.
		pallet_registration::OwnerToNode::<T>::insert(&family, vec![vec![1u8]]);
		T::BenchmarkHelper::authorize_child(&family, &child);

		// Node keypair: node_id IS the ed25519 public key; sign the exact
		// message the extrinsic reconstructs (nonce 0 for a fresh node id).
		let pk = sp_io::crypto::ed25519_generate(NODE_KEY_TYPE, None);
		let node_id: [u8; 32] = pk.0;
		let nonce: u64 = 0;
		let msg = (b"ARION_NODE_REG_V1", &family, &child, &node_id, nonce).encode();
		let sig = sp_io::crypto::ed25519_sign(NODE_KEY_TYPE, &pk, &msg)
			.expect("benchmark signing key is present in the keystore");
		let node_sig: [u8; 64] = sig.0;

		#[extrinsic_call]
		_(RawOrigin::Signed(family.clone()), family.clone(), child.clone(), node_id, node_sig);

		assert!(ChildRegistrations::<T>::contains_key(&child));
	}

	/// Deregister the family's only active child, so the family-cleanup branch
	/// (the heavier path) also runs.
	#[benchmark]
	fn deregister_child() {
		let family: T::AccountId = account("dereg_family", 0, SEED);
		let child: T::AccountId = account("dereg_child", 0, SEED);
		let now: BlockNumberFor<T> = 1_000u32.into();
		frame_system::Pallet::<T>::set_block_number(now);

		let mut node_id = [0u8; 32];
		node_id[0] = 7;
		ChildRegistrations::<T>::insert(
			&child,
			ChildRegistration {
				family: family.clone(),
				node_id,
				status: ChildStatus::Active,
				deposit: Zero::zero(),
				unbonding_end: now,
			},
		);
		NodeIdToChild::<T>::insert(node_id, &child);
		ChildMinerUid::<T>::insert(&child, 1u32);
		MinerUidToChild::<T>::insert(1u32, &child);
		let _ = FamilyChildren::<T>::try_mutate(&family, |v| v.try_push(child.clone()));
		FamilyActiveChildren::<T>::insert(&family, 1u32);
		TotalActiveChildren::<T>::put(1u32);
		FamilyCount::<T>::put(1u32);

		#[extrinsic_call]
		_(RawOrigin::Signed(family.clone()), child.clone());

		assert_eq!(ChildRegistrations::<T>::get(&child).map(|r| r.status), Some(ChildStatus::Unbonding));
	}

	/// Claim a deregistered child's reserved deposit after the unbonding period.
	#[benchmark]
	fn claim_unbonded() {
		let family: T::AccountId = account("claim_family", 0, SEED);
		let child: T::AccountId = account("claim_child", 0, SEED);
		let now: BlockNumberFor<T> = 1_000u32.into();
		frame_system::Pallet::<T>::set_block_number(now);

		let deposit: BalanceOf<T> = DEPOSIT.saturated_into();
		let funded: BalanceOf<T> = 1_000_000_000_000_000_000_000u128.saturated_into(); // 1e21
		let _ = T::DepositCurrency::deposit_creating(&family, funded);
		// Reserve the deposit so the claim's unreserve moves real funds.
		T::DepositCurrency::reserve(&family, deposit)
			.expect("family funded above the reserve");

		let mut node_id = [0u8; 32];
		node_id[0] = 9;
		ChildRegistrations::<T>::insert(
			&child,
			ChildRegistration {
				family: family.clone(),
				node_id,
				status: ChildStatus::Unbonding,
				deposit,
				unbonding_end: now, // now >= unbonding_end, so the claim is ready
			},
		);
		ChildMinerUid::<T>::insert(&child, 1u32);
		MinerUidToChild::<T>::insert(1u32, &child);

		#[extrinsic_call]
		_(RawOrigin::Signed(family.clone()), child.clone());

		assert!(!ChildRegistrations::<T>::contains_key(&child));
	}

	/// Each update claims a uid and takes the accrual path as well as the stats
	/// write, ranged over `n` updates in the batch.
	#[benchmark]
	fn submit_miner_stats(n: Linear<1, { T::MaxStatsUpdates::get() }>) -> Result<(), BenchmarkError> {
		let origin = T::StatsAuthorityOrigin::try_successful_origin()
			.map_err(|_| BenchmarkError::Weightless)?;
		let family: T::AccountId = account("family", 0, SEED);
		for uid in 1..=n {
			seed_miner::<T>(uid, &family, 1u32.into());
		}
		frame_system::Pallet::<T>::set_block_number((1 + ELAPSED).into());

		let bucket = CurrentStatsBucket::<T>::get().saturating_add(1);
		let updates: BoundedVec<MinerStatsUpdate, T::MaxStatsUpdates> = (1..=n)
			.map(|uid| MinerStatsUpdate { uid, stats: stats(bucket) })
			.collect::<Vec<_>>()
			.try_into()
			.map_err(|_| BenchmarkError::Weightless)?;

		#[extrinsic_call]
		_(origin as T::RuntimeOrigin, bucket, updates, Some(NetworkTotals::default()));

		assert_eq!(CurrentStatsBucket::<T>::get(), bucket);
		let accrued = MinerAccruals::<T>::get(n).map(|a| a.byte_blocks).unwrap_or(0);
		assert_eq!(accrued, SHARD_BYTES * u128::from(ELAPSED), "the update took the accrual path");
		Ok(())
	}

	/// Single price write.
	#[benchmark]
	fn set_miner_price() -> Result<(), BenchmarkError> {
		let origin = T::ArionAdminOrigin::try_successful_origin()
			.map_err(|_| BenchmarkError::Weightless)?;

		#[extrinsic_call]
		_(origin as T::RuntimeOrigin, E18);

		assert_eq!(MinerPriceUsdPerGbBlock::<T>::get(), E18);
		Ok(())
	}
}

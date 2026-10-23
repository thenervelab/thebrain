//! Benchmarks for the Arion miner payment paths.
//!
//! `miner_payment_settlement_hook` is the one that matters: it runs inside
//! `on_initialize`, so its declared weight is mandatory block weight, and it
//! moves funds — a bank transfer and a staking bond per family paid. It is not
//! an extrinsic and should not become one to be measurable, so it is measured
//! with `#[block]` around the settlement itself.
//!
//! The settlement is posed at its most expensive shape for each `(c, f)`:
//!
//! - `c` Active children, every one with a bound uid, stored stats and an
//!   accrual old enough to convert into a payable amount, spread round-robin
//!   over `f` families;
//! - every family also carrying arrears, so the arrears walk visits all `f`;
//! - a bank funded to pay everybody in full, so each family takes the full
//!   path: transfer into an account that does not exist yet, then a first-time
//!   `bond` (no ledger yet), which writes more than `bond_extra`.
//!
//! `submit_miner_stats` is measured per update with every uid claimed, so each
//! update takes the accrual path as well as the stats write.
//!
//! The setup writes storage directly rather than registering children through
//! `register_child`: that path verifies a node signature and reserves a
//! deposit, none of which settlement reads, and its cost would otherwise have
//! to be kept out of the measured block.
//!
//! Run:
//!
//! ```text
//! cargo build --release --features runtime-benchmarks
//! ./target/release/hippius benchmark pallet \
//!     --chain benchmark \
//!     --pallet pallet_arion \
//!     --extrinsic '*' \
//!     --steps 50 --repeat 20
//! ```

#![cfg(feature = "runtime-benchmarks")]

use super::*;
use crate::pallet::Pallet as Arion;
use frame_benchmarking::v2::*;
use frame_support::traits::EnsureOrigin;

const SEED: u32 = 0;

/// 1.0 in the pallet's 18-decimal fixed point.
const E18: u128 = 1_000_000_000_000_000_000;

/// Raw shard bytes each seeded miner holds: one GiB.
const SHARD_BYTES: u128 = 1 << 30;

/// Blocks between the seeded accruals and the settlement. At one USD per
/// GiB-block and a one-USD token, each child is owed `ELAPSED` whole tokens.
const ELAPSED: u32 = 1_000;

/// Arrears each family carries into the settlement.
const ARREARS: u128 = 10 * E18;

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
/// starting at `since`.
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

#[benchmarks]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn miner_payment_settlement_hook(
		c: Linear<1, { T::MaxChildrenTotal::get() }>,
		f: Linear<1, { T::MaxFamilies::get() }>,
	) {
		let since: BlockNumberFor<T> = 1u32.into();
		let now: BlockNumberFor<T> = (1 + ELAPSED).into();
		frame_system::Pallet::<T>::set_block_number(now);

		MinerPriceUsdPerGbBlock::<T>::put(E18);
		T::BenchmarkHelper::set_token_price(E18);

		let families: Vec<T::AccountId> = (0..f).map(|j| account("family", j, SEED)).collect();
		for uid in 1..=c {
			seed_miner::<T>(uid, &families[((uid - 1) % f) as usize], since);
		}
		for family in families.iter() {
			FamilyArrears::<T>::insert(family, ARREARS);
		}

		let per_child = u128::from(ELAPSED) * E18;
		let total_due = per_child * u128::from(c) + ARREARS * u128::from(f);
		T::BenchmarkHelper::fund_payout_source(&Arion::<T>::account_id(), total_due * 2);

		#[block]
		{
			Arion::<T>::settle_miner_payments(now);
		}

		assert_eq!(LastSettlementBlock::<T>::get(), Some(now));
		// Paid in full: nothing carried over, so no family took a cheaper path.
		assert!(families.iter().all(|family| FamilyArrears::<T>::get(family) == 0));
	}

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

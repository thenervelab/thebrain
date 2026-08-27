//! Benchmarks for the date-to-date billing hook.
//!
//! Scope is one function: `charge_account_due`. That is deliberate — it is the
//! unit the renewal drain meters itself in and the unit
//! `MaxSubscriptionChargesPerRun` counts, so it is the only number in this
//! pallet whose accuracy decides whether `on_initialize` stays inside its
//! budget. The other three `WeightInfo` entries are single storage operations
//! whose conservative estimates are already within a rounding error of any
//! measurement.
//!
//! It is measured with `#[block]` rather than `#[extrinsic_call]` because
//! `charge_account_due` is not an extrinsic and should not become one to be
//! measurable — a benchmark-only call in the dispatch enum is a permanent piece
//! of surface added for a temporary purpose.
//!
//! The setup writes storage directly rather than going through `purchase_plan`.
//! Two reasons: the purchase path would put its own cost inside the measured
//! block, and seeding lets the account be posed in its *worst* case — the
//! maximum subscriptions it may hold, all due on the same day, split across
//! both the storage and compute charging sides. The meter takes this figure as
//! the cost of the next account before knowing anything about it, so it has to
//! be the worst case rather than the typical one.
//!
//! Run:
//!
//! ```text
//! cargo build --release --features runtime-benchmarks
//! ./target/release/hippius benchmark pallet \
//!     --chain benchmark \
//!     --pallet pallet_marketplace \
//!     --extrinsic 'charge_account_due' \
//!     --steps 50 --repeat 20
//! ```

#![cfg(feature = "runtime-benchmarks")]

use super::*;
use crate::pallet::Pallet as Marketplace;
use frame_benchmarking::v2::*;
use frame_support::pallet_prelude::{Get, PhantomData};
use frame_system::pallet_prelude::BlockNumberFor;
use pallet_credits::Pallet as CreditsPallet;
use sp_runtime::traits::{Hash, Saturating};
use sp_std::vec;
use sp_std::vec::Vec;

const SEED: u32 = 0;

/// Credits generous enough that no charge in the measured block fails for want
/// of funds — a failed charge takes the deactivation path, which is a
/// *different* and cheaper shape than the renewal we are pricing.
const FUNDING: u128 = 1_000_000_000_000;

/// Plan price small relative to `FUNDING`, so the worst case stays the
/// full-charge path however many subscriptions the account holds.
const PLAN_PRICE: u128 = 1_000;

fn plan_of<T: Config>(index: u32, is_storage_plan: bool) -> Plan<T::Hash> {
	let name: Vec<u8> = vec![b'p', index as u8];
	let id = T::Hashing::hash_of(&name);
	let plan = Plan {
		id,
		plan_name: name,
		plan_description: vec![b'{', b'}'],
		plan_technical_description: vec![b'{', b'}'],
		is_suspended: false,
		price: PLAN_PRICE,
		is_storage_plan,
		is_s3_plan: false,
		storage_limit: if is_storage_plan { Some(1_000_000) } else { None },
	};
	Plans::<T>::insert(id, plan.clone());
	plan
}

/// An account holding `count` active subscriptions, every one of them due, and
/// funded well enough to pay for all of them.
///
/// The first is a Drive plan and the rest are compute, so the charge exercises
/// both sides of the storage/compute split rather than one branch twice.
fn due_account<T: Config>(count: u32) -> T::AccountId {
	let who: T::AccountId = account("subscriber", 0, SEED);

	// Fund through a *batch*, not `do_mint` alone. `consume_credits` spends
	// from `UserBatches` and requires the whole amount to come from there —
	// minting `FreeCredits` with no batch behind it leaves the charge failing
	// with `InsufficientFreeCredits`, which sends `charge_account_due` down the
	// deactivation path. That path is cheaper than a renewal, so a benchmark
	// seeded that way would quietly price the wrong thing and under-fund the
	// meter. The assertion at the end of the benchmark is what catches it.
	let batch_id = NextBatchId::<T>::get();
	Batches::<T>::insert(
		batch_id,
		Batch {
			owner: who.clone(),
			credit_amount: FUNDING,
			alpha_amount: 0,
			remaining_credits: FUNDING,
			remaining_alpha: 0,
			pending_alpha: 0,
			is_frozen: false,
			release_time: BlockNumberFor::<T>::from(0u32),
		},
	);
	UserBatches::<T>::append(&who, batch_id);
	NextBatchId::<T>::put(batch_id.saturating_add(1));
	let _ = CreditsPallet::<T>::do_mint(who.clone(), FUNDING, None);

	let subs: Vec<UserPlanSubscription<T>> = (0..count)
		.map(|i| UserPlanSubscription {
			id: i,
			owner: who.clone(),
			package: plan_of::<T>(i, i == 0),
			cdn_location_id: None,
			active: true,
			last_charged_at: BlockNumberFor::<T>::from(0u32),
			selected_image_name: None,
			// Day 0 with the clock at genesis: `today >= due_day` holds, so
			// every one of these is due in the measured block.
			next_charge_unix_day: Some(0),
			paid_per_month: PLAN_PRICE,
			_phantom: PhantomData,
		})
		.collect();

	UserAllSubscriptionPlans::<T>::insert(&who, subs);
	who
}

#[benchmarks]
mod benchmarks {
	use super::*;

	/// One account's renewal, posed at the worst case the drain can meet:
	/// `MaxActiveSubscriptions` subscriptions, all due, all payable.
	///
	/// Payable is the worst case and not a convenience. A charge that succeeds
	/// takes credits, records a transaction, advances the due date and re-files
	/// the index entry; one that fails deactivates and refunds instead, which
	/// touches less. Pricing the cheaper path would under-fund the meter.
	#[benchmark]
	fn charge_account_due() {
		let subs = T::MaxActiveSubscriptions::get().max(1);
		let who = due_account::<T>(subs);
		let block = BlockNumberFor::<T>::from(1u32);

		#[block]
		{
			Marketplace::<T>::charge_account_due(&who, block);
		}

		// The account was actually charged — a benchmark that measured a
		// no-op would report a fraction of the real cost and the meter would
		// let far too many accounts through.
		assert!(CreditsPallet::<T>::get_free_credits(&who) < FUNDING);
	}
}

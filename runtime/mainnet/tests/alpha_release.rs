//! The matured-alpha release sweep, which had no test at all.
//!
//! A deposit taken with `freeze_for_chargeback` holds its alpha back. As the
//! batch's credits are spent, `consume_credits` moves the matching alpha into
//! `pending_alpha` rather than releasing it, and it stays there until the
//! batch's `release_time` passes — a 15-day chargeback window. The sweep in
//! `on_initialize` is what finally lets it go.
//!
//! Two things are covered here. That the release happens at all, which nothing
//! asserted before; and that paging the sweep divides the work rather than
//! repeating part of it. The second matters because `Batches` is only pruned by
//! chargeback — a batch spent down to zero credits stays forever — so the map
//! the sweep walks grows with every deposit the chain has ever taken, and it
//! walks it on every tick.
//!
//! The batches are seeded directly rather than built by depositing and
//! spending. What is under test is the sweep, and driving `pending_alpha` up
//! through real purchases would spend most of the test on the per-block request
//! limit while posing the same state less precisely.

use frame_support::{
	assert_ok,
	traits::{Currency, Hooks},
};
use hippius_mainnet_runtime::{
	AccountId, Balances, Credits, Hippocampus, Marketplace, Runtime, RuntimeOrigin, System,
};
use sp_core::crypto::Ss58Codec;
use sp_runtime::{AccountId32, BuildStorage};

/// 2026-01-01T00:00:00Z.
const JAN1_2026_MS: u64 = 1_767_225_600_000;
const BANK_FUND: u128 = 1_000_000_000;

/// `BlockChargeCheckInterval`.
const TICK: u64 = 8;

/// Alpha held pending on each seeded batch.
const PENDING: u128 = 1_000;

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

fn authority() -> AccountId {
	account(1)
}

fn admin() -> AccountId {
	AccountId32::from_ss58check("5CVXqxb7mhFTtZVw5BJ8M2ujND9PFymSDxF8bkod6Sm4XJTW").unwrap()
}

fn owner_of(n: u32) -> AccountId {
	let mut raw = [0u8; 32];
	raw[0] = 0xE0;
	raw[1..5].copy_from_slice(&n.to_le_bytes());
	AccountId32::new(raw)
}

fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| {
		System::set_block_number(1);
		pallet_timestamp::Now::<Runtime>::put(JAN1_2026_MS);
		assert_ok!(Credits::add_authority(RuntimeOrigin::root(), authority()));
		let _ = Balances::deposit_creating(&Hippocampus::account_id(), BANK_FUND);
		assert_ok!(Hippocampus::add_requester(
			RuntimeOrigin::signed(admin()),
			Marketplace::account_id(),
		));
	});
    ext
}

/// A frozen batch holding `PENDING` alpha, matured at `release_time`.
fn seed_batch(n: u32, release_time: u64) -> u64 {
	let owner = owner_of(n);
	let batch_id = pallet_marketplace::NextBatchId::<Runtime>::get();
	pallet_marketplace::Batches::<Runtime>::insert(
		batch_id,
		pallet_marketplace::Batch {
			owner: owner.clone(),
			credit_amount: 0,
			alpha_amount: PENDING,
			remaining_credits: 0,
			remaining_alpha: 0,
			pending_alpha: PENDING,
			is_frozen: true,
			release_time,
		},
	);
	pallet_marketplace::NextBatchId::<Runtime>::put(batch_id + 1);
	pallet_credits::AlphaBalances::<Runtime>::insert(&owner, PENDING);
	batch_id
}

fn next_tick_block() -> u64 {
	(System::block_number() / TICK + 1) * TICK
}

fn tick() {
	let next = next_tick_block();
	System::set_block_number(next);
	Marketplace::on_initialize(next);
}

/// The base case nothing asserted before: a matured frozen batch is released,
/// and an unmatured one is left alone.
#[test]
fn a_matured_batch_is_released_and_an_unmatured_one_is_not() {
	new_test_ext().execute_with(|| {
		let matured = seed_batch(0, 1);
		let pending = seed_batch(1, 1_000_000);

		tick();

		let released = pallet_marketplace::Batches::<Runtime>::get(matured).unwrap();
		assert!(!released.is_frozen, "a matured batch is unfrozen");
		assert_eq!(released.pending_alpha, 0, "and its pending alpha is let go");
		assert_eq!(
			pallet_credits::AlphaBalances::<Runtime>::get(owner_of(0)),
			0,
			"the owner's held alpha is drawn down by exactly what was released",
		);

		let held = pallet_marketplace::Batches::<Runtime>::get(pending).unwrap();
		assert!(held.is_frozen, "the chargeback window has not passed for this one");
		assert_eq!(held.pending_alpha, PENDING, "so nothing is released early");
		assert_eq!(pallet_credits::AlphaBalances::<Runtime>::get(owner_of(1)), PENDING);
	});
}

/// The sweep is bounded per tick and round-robins, so it cannot grow into a
/// heavy block as the deposit history does.
///
/// `Batches` is never pruned on spend, so this map only ever grows — and the
/// sweep ran over all of it on every tick while declaring a single read.
#[test]
fn the_release_sweep_is_bounded_and_reaches_every_batch() {
	new_test_ext().execute_with(|| {
		// More batches than one metered tick can afford.
		let total = 1_500u32;
		for n in 0..total {
			seed_batch(n, 1);
		}

		let budget = <Runtime as pallet_marketplace::Config>::AlphaReleaseWeightBudget::get()
			* <Runtime as frame_system::Config>::BlockWeights::get().max_block;

		let block = next_tick_block();
		System::set_block_number(block);
		let consumed = Marketplace::on_initialize(block);
		assert!(
			consumed.ref_time() <= budget.ref_time() * 4,
			"one tick spent {} against a release budget of {}",
			consumed.ref_time(),
			budget.ref_time(),
		);
		assert!(
			pallet_marketplace::AlphaReleaseCursor::<Runtime>::get().is_some(),
			"1500 batches cannot fit in one metered tick — the sweep must park a cursor",
		);

		// The cursor comes back around: every batch is released, none skipped
		// and none released twice.
		for _ in 0..60 {
			tick();
		}
		for n in 0..total {
			let batch = pallet_marketplace::Batches::<Runtime>::get(u64::from(n)).unwrap();
			assert!(!batch.is_frozen, "batch {n} was reached by the paged sweep");
			assert_eq!(batch.pending_alpha, 0, "batch {n} released exactly once");
			assert_eq!(
				pallet_credits::AlphaBalances::<Runtime>::get(owner_of(n)),
				0,
				"owner {n} had exactly their pending alpha drawn down",
			);
		}
	});
}

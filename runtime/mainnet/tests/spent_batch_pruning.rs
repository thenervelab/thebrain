//! Spent deposit batches leave `UserBatches`.
//!
//! `consume_credits` walks an account's batches oldest first, and nothing used
//! to take a used-up batch out of that list — so every debit paid a read, a
//! write and a zero burn for each one, and what a debit cost grew with the
//! account's whole deposit history. A debit now drops the ids it finds fully
//! spent, and a paged `on_idle` pass does the same for lists written before.
//!
//! The `Batches` rows stay throughout: chargeback looks batches up by id.

use frame_support::{
	assert_ok,
	traits::Hooks,
	weights::Weight,
};
use hippius_mainnet_runtime::{
	AccountId, Credits, Marketplace, Runtime, RuntimeEvent, RuntimeOrigin, System,
};
use pallet_marketplace::{
	Batch, Batches, NextBatchId, SpentBatchPruneCursor, SpentBatchPruneDone, UserBatches,
};
use sp_runtime::{AccountId32, BuildStorage};

const JAN1_2026_MS: u64 = 1_767_225_600_000;

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

fn owner_of(n: u32) -> AccountId {
	let mut raw = [0u8; 32];
	raw[0] = 0xE1;
	raw[1..5].copy_from_slice(&n.to_le_bytes());
	AccountId32::new(raw)
}

fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| {
		System::set_block_number(1);
		pallet_timestamp::Now::<Runtime>::put(JAN1_2026_MS);
		assert_ok!(Credits::add_authority(RuntimeOrigin::root(), account(1)));
	});
	ext
}

fn batch(owner: &AccountId, credits: u128) -> Batch<AccountId, u64> {
	Batch {
		owner: owner.clone(),
		credit_amount: credits,
		alpha_amount: 0,
		remaining_credits: credits,
		remaining_alpha: 0,
		pending_alpha: 0,
		is_frozen: false,
		release_time: 0,
	}
}

fn spent(owner: &AccountId) -> Batch<AccountId, u64> {
	Batch { credit_amount: 100, ..batch(owner, 0) }
}

/// Seed a batch the way a deposit would file it, minting only what it can
/// still pay so `FreeCredits` matches the batches behind it.
fn push(owner: &AccountId, b: Batch<AccountId, u64>) -> u64 {
	let id = NextBatchId::<Runtime>::get();
	if b.remaining_credits > 0 {
		assert_ok!(Credits::do_mint(owner.clone(), b.remaining_credits, None));
	}
	Batches::<Runtime>::insert(id, b);
	UserBatches::<Runtime>::append(owner, id);
	NextBatchId::<Runtime>::put(id + 1);
	id
}

fn list(owner: &AccountId) -> Vec<u64> {
	UserBatches::<Runtime>::get(owner).unwrap_or_default()
}

fn zero_burns() -> usize {
	System::events()
		.iter()
		.filter(|r| {
			matches!(
				r.event,
				RuntimeEvent::Credits(pallet_credits::Event::BurnedAccountCredits { amount: 0, .. })
			)
		})
		.count()
}

#[test]
fn a_debit_that_spends_a_batch_drops_its_id_and_keeps_the_row() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		let first = push(&user, batch(&user, 100));
		let second = push(&user, batch(&user, 100));

		assert_ok!(Marketplace::consume_credits(user.clone(), 150));

		assert_eq!(list(&user), vec![second], "the spent batch leaves the list");
		assert_eq!(Batches::<Runtime>::get(second).unwrap().remaining_credits, 50);
		let row = Batches::<Runtime>::get(first).expect("the row stays for chargeback");
		assert_eq!(row.remaining_credits, 0);
		assert!(Marketplace::get_batch_by_id(first).is_some());

		assert_ok!(Marketplace::consume_credits(user.clone(), 50));
		assert_eq!(UserBatches::<Runtime>::get(&user), None, "an empty list is removed");
		assert!(Marketplace::get_batches_for_user(user).is_empty());
	});
}

#[test]
fn spent_batches_ahead_of_the_payer_are_skipped_and_dropped() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		for _ in 0..12 {
			push(&user, spent(&user));
		}
		let payer = push(&user, batch(&user, 1_000));
		System::reset_events();

		assert_ok!(Marketplace::consume_credits(user.clone(), 10));

		assert_eq!(zero_burns(), 0, "a spent batch is not burned from at zero");
		assert_eq!(list(&user), vec![payer]);
		assert_eq!(Batches::<Runtime>::get(payer).unwrap().remaining_credits, 990);
	});
}

#[test]
fn a_frozen_batch_with_pending_alpha_stays_until_it_releases() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		pallet_credits::AlphaBalances::<Runtime>::insert(&user, 500);
		let frozen = push(
			&user,
			Batch {
				pending_alpha: 500,
				is_frozen: true,
				release_time: 100,
				..spent(&user)
			},
		);
		let payer = push(&user, batch(&user, 1_000));

		assert_ok!(Marketplace::consume_credits(user.clone(), 10));
		assert_eq!(list(&user), vec![frozen, payer], "still frozen: it stays listed");

		// Matured: the debit passing over it is what releases the alpha, and
		// once released it is spent.
		System::set_block_number(100);
		assert_ok!(Marketplace::consume_credits(user.clone(), 10));
		let released = Batches::<Runtime>::get(frozen).unwrap();
		assert!(!released.is_frozen);
		assert_eq!(released.pending_alpha, 0);
		assert_eq!(pallet_credits::AlphaBalances::<Runtime>::get(&user), 0);
		assert_eq!(list(&user), vec![payer]);
	});
}

#[test]
fn a_failed_debit_leaves_the_list_untouched() {
	new_test_ext().execute_with(|| {
		let user = account(10);
		let s = push(&user, spent(&user));
		let live = push(&user, batch(&user, 100));

		assert!(Marketplace::consume_credits(user.clone(), 500).is_err());

		assert_eq!(list(&user), vec![s, live], "rolled back with the rest of the debit");
		assert_eq!(Batches::<Runtime>::get(live).unwrap().remaining_credits, 100);
	});
}

#[test]
fn the_idle_cleanup_prunes_old_lists_in_pages_and_finishes() {
	new_test_ext().execute_with(|| {
		// More accounts than one page takes.
		let total = 150u32;
		let mut live = Vec::new();
		for n in 0..total {
			let who = owner_of(n);
			push(&who, spent(&who));
			if n % 3 == 0 {
				// Nothing live: the whole entry goes.
				continue;
			}
			let keep = push(&who, batch(&who, 10));
			push(&who, spent(&who));
			live.push((who, keep));
		}
		// Frozen with alpha still pending: not spent, so it stays.
		let held_owner = owner_of(total);
		let held = push(
			&held_owner,
			Batch { pending_alpha: 1, is_frozen: true, release_time: 1_000, ..spent(&held_owner) },
		);

		Marketplace::on_idle(System::block_number(), Weight::MAX);
		assert!(SpentBatchPruneCursor::<Runtime>::get().is_some(), "one call is one page");
		assert!(!SpentBatchPruneDone::<Runtime>::get());

		for _ in 0..10 {
			Marketplace::on_idle(System::block_number(), Weight::MAX);
		}
		assert!(SpentBatchPruneDone::<Runtime>::get());
		assert_eq!(SpentBatchPruneCursor::<Runtime>::get(), None);

		for n in (0..total).step_by(3) {
			assert_eq!(UserBatches::<Runtime>::get(owner_of(n)), None, "owner {n} had only spent");
		}
		for (who, keep) in &live {
			assert_eq!(list(who), vec![*keep]);
		}
		assert_eq!(list(&held_owner), vec![held]);
		assert_eq!(Batches::<Runtime>::iter().count() as u64, NextBatchId::<Runtime>::get());

		// Finished: one read from here on.
		let db = <Runtime as frame_system::Config>::DbWeight::get();
		let used = Marketplace::on_idle(System::block_number(), Weight::MAX);
		assert!(used.ref_time() <= db.reads(4).ref_time() + db.writes(1).ref_time());
	});
}

#[test]
fn the_idle_cleanup_stays_within_its_budget() {
	new_test_ext().execute_with(|| {
		for n in 0..20 {
			let who = owner_of(n);
			push(&who, spent(&who));
			push(&who, spent(&who));
		}

		assert_eq!(Marketplace::on_idle(System::block_number(), Weight::zero()), Weight::zero());

		let db = <Runtime as frame_system::Config>::DbWeight::get();
		let limit = Weight::from_parts(db.reads_writes(20, 10).ref_time(), u64::MAX);
		let used = Marketplace::on_idle(System::block_number(), limit);
		assert!(used.all_lte(limit));
		let pruned = (0..20).filter(|n| UserBatches::<Runtime>::get(owner_of(*n)).is_none()).count();
		assert!(pruned > 0 && pruned < 20, "partial progress, got {pruned}");
		assert!(SpentBatchPruneCursor::<Runtime>::get().is_some());
	});
}

//! Compute settlement (calls 10-18): contract vectors, the happy paths and a
//! must-refuse test for every check of `compute-miner-pay-chain.md` §1.5 and
//! the interface contract §4.

use crate::{
	mock::*,
	settlement::{self as st, ApprovalInput, ChangeKind, Entry, PayoutParams, RootChange},
	ApprovedUnpaidAlpha, ComputeApproval, ComputeDailyAlpha, ComputeOwed, ComputeOwedTotal,
	ComputePayoutParams, ComputeProgress, ComputeSettlementEnabled, ComputeWindowPaid,
	DistributionEnabled, Error, Event, FirstComputeWindow, NextComputeWindow, PendingRootChange,
	WindowParams, ComputeLastRevisionDay, ComputeRevisionSeq, InFlightApproval, NextRevisionSeq,
};
use frame_support::{
	assert_noop, assert_ok,
	dispatch::{Pays, PostDispatchInfo},
	traits::Currency,
	weights::Weight,
	BoundedVec,
};
use sp_core::{sr25519, Pair, H256};
use sp_runtime::{AccountId32, DispatchError};

const W: u32 = 100;
const PRICE: u128 = 1_000_000_000_000_000_000; // 1 credit per alpha: alpha == usd
const DELAY: u64 = 28_800;
const CHARGED: u128 = 1_000_000;
const FUNDED: u128 = 500_000;

fn h(s: &str) -> Vec<u8> {
	hex::decode(s.trim_start_matches("0x")).expect("hex")
}

fn h256(s: &str) -> H256 {
	H256::from_slice(&h(s))
}

fn approver() -> sr25519::Pair {
	sr25519::Pair::from_seed(&[7u8; 32])
}

fn other_signer() -> sr25519::Pair {
	sr25519::Pair::from_seed(&[8u8; 32])
}

fn approver_account() -> AccountId {
	AccountId::from(approver().public())
}

fn submitter() -> AccountId {
	charlie()
}

fn family(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

fn params() -> PayoutParams {
	PayoutParams { share_bps: 5000, tol_bps: 10, vcpu_hour_uusd: 24657 }
}

fn keys_change(submitters: Vec<AccountId>, approvers: Vec<sr25519::Public>) -> RootChange<AccountId> {
	RootChange::Keys {
		submitters: BoundedVec::try_from(submitters).unwrap(),
		approvers: BoundedVec::try_from(approvers).unwrap(),
	}
}

/// Schedule, wait out the delay, apply.
fn root_apply(change: RootChange<AccountId>) {
	let kind = change.kind();
	assert_ok!(Hippocampus::schedule_compute_root_change(RuntimeOrigin::root(), change));
	System::set_block_number(System::block_number() + DELAY);
	assert_ok!(Hippocampus::apply_compute_root_change(RuntimeOrigin::signed(alice()), kind));
}

fn fresh_price(price: u128) {
	set_alpha_price(price, Some(System::block_number()));
}

/// Keys, params v1, first window `W`, settlement funded (which seeds the
/// owed account with the ED), switch on, fresh price, `W` final with
/// `CHARGED` revenue.
fn setup() {
	root_apply(keys_change(vec![submitter()], vec![approver().public()]));
	root_apply(RootChange::Params(params()));
	root_apply(RootChange::MaxDailyAlpha(1_000_000));
	root_apply(RootChange::FirstWindow(W));
	assert_ok!(Hippocampus::fund_compute_settlement(RuntimeOrigin::signed(alice()), FUNDED));
	assert_eq!(Balances::free_balance(Hippocampus::owed_account()), EXISTENTIAL_DEPOSIT);
	root_apply(RootChange::Enable);
	fresh_price(PRICE);
	set_current_window(W + 8);
	set_final_charged(W, CHARGED);
}

fn entry(seed: u8, usd: u128) -> Entry<AccountId> {
	Entry { family: family(seed), usd, alpha: st::alpha_for(usd, PRICE).unwrap() }
}

fn input_for(window: u32, revision: u8, parts: &[Vec<Entry<AccountId>>], usd: u128) -> ApprovalInput {
	let genesis = Hippocampus::genesis_hash();
	let hashes: Vec<H256> = parts
		.iter()
		.enumerate()
		.map(|(i, e)| st::part_hash(&genesis, window, revision, i as u8, e))
		.collect();
	ApprovalInput {
		parts: parts.len() as u8,
		part_hashes: BoundedVec::try_from(hashes).unwrap(),
		usd,
		price: PRICE,
		payable_root: H256::repeat_byte(0xaa),
		evidence_root: H256::repeat_byte(0xbb),
		upto_hour: u64::from(window) * 24 + 23,
		params_version: ComputePayoutParams::<Test>::get().unwrap().0,
		formula_version: 1,
		params_hash: st::params_hash(&params()),
	}
}

fn sign_input(window: u32, revision: u8, input: &ApprovalInput) -> sr25519::Signature {
	approver().sign(st::approval_digest(&Hippocampus::genesis_hash(), window, revision, input).as_bytes())
}

fn total(parts: &[Vec<Entry<AccountId>>]) -> u128 {
	parts.iter().flatten().map(|e| e.usd).sum()
}

fn submit(window: u32, revision: u8, input: ApprovalInput) -> frame_support::dispatch::DispatchResultWithPostInfo {
	let sig = sign_input(window, revision, &input);
	Hippocampus::submit_compute_approval(RuntimeOrigin::signed(submitter()), window, revision, input, sig)
}

fn approve(window: u32, revision: u8, parts: &[Vec<Entry<AccountId>>]) {
	assert_ok!(submit(window, revision, input_for(window, revision, parts, total(parts))));
}

fn pay(
	window: u32,
	revision: u8,
	part: u8,
	entries: Vec<Entry<AccountId>>,
) -> frame_support::dispatch::DispatchResultWithPostInfo {
	Hippocampus::pay_compute_part(
		RuntimeOrigin::signed(submitter()),
		window,
		revision,
		part,
		BoundedVec::try_from(entries).unwrap(),
	)
}

fn err(e: Error<Test>) -> DispatchError {
	e.into()
}

fn assert_refused(result: frame_support::dispatch::DispatchResultWithPostInfo, e: Error<Test>) {
	assert_eq!(result.map_err(|e| e.error), Err(err(e)));
}

// ---------------------------------------------------------------------------
// Contract vectors (interface contract §7): see `settlement_vectors.rs`.
// ---------------------------------------------------------------------------

use crate::settlement_vectors::*;

fn vector_input_rev0() -> ApprovalInput {
	ApprovalInput {
		parts: 1,
		part_hashes: BoundedVec::try_from(vec![h256(V0_PART0_HASH)]).unwrap(),
		usd: V0_USD,
		price: V_PRICE,
		payable_root: h256(V0_PAYABLE_ROOT),
		evidence_root: h256(V_EVIDENCE_ROOT),
		upto_hour: V0_UPTO_HOUR,
		params_version: 1,
		formula_version: 1,
		params_hash: h256(V_PARAMS_HASH),
	}
}

fn vector_input_rev1() -> ApprovalInput {
	ApprovalInput {
		parts: 1,
		part_hashes: BoundedVec::try_from(vec![h256(V1_PART0_HASH)]).unwrap(),
		usd: V1_USD,
		price: V_PRICE,
		payable_root: h256(V1_PAYABLE_ROOT),
		evidence_root: h256(V_EVIDENCE_ROOT),
		upto_hour: V1_UPTO_HOUR,
		params_version: 1,
		formula_version: 1,
		params_hash: h256(V_PARAMS_HASH),
	}
}

#[test]
fn contract_vectors_reproduce_byte_for_byte() {
	let genesis = H256::repeat_byte(0x11);
	let p = params();
	assert_eq!(st::params_message(&p), h(V_PARAMS_MSG));
	assert_eq!(st::params_hash(&p), h256(V_PARAMS_HASH));

	let rev0 = vector_entries_rev0();
	for e in &rev0 {
		assert_eq!(st::alpha_for(e.usd, V_PRICE), Some(e.alpha));
	}
	assert_eq!(st::part_message(&genesis, V_WINDOW, 0, 0, &rev0), h(V0_PART0_MSG));
	assert_eq!(st::part_hash(&genesis, V_WINDOW, 0, 0, &rev0), h256(V0_PART0_HASH));

	let input = vector_input_rev0();
	assert_eq!(input.usd, rev0.iter().map(|e| e.usd).sum::<u128>());
	assert_eq!(st::approval_message(&genesis, V_WINDOW, 0, &input), h(V0_PAYLOAD));
	let digest = st::approval_digest(&genesis, V_WINDOW, 0, &input);
	assert_eq!(digest, h256(V0_DIGEST));
	let mut revoke_msg = st::REVOKE_DOMAIN.to_vec();
	revoke_msg.extend(codec::Encode::encode(&(genesis, V_WINDOW, 0u8, digest)));
	assert_eq!(revoke_msg, h(V0_REVOKE_MSG));
	assert_eq!(st::revoke_digest(&genesis, V_WINDOW, 0, &digest), h256(V0_REVOKE_DIGEST));

	let rev1 = vector_entries_rev1();
	assert_eq!(st::alpha_for(rev1[0].usd, V_PRICE), Some(rev1[0].alpha));
	assert_eq!(st::part_hash(&genesis, V_WINDOW, 1, 0, &rev1), h256(V1_PART0_HASH));
	let rev1_input = vector_input_rev1();
	assert_eq!(st::approval_message(&genesis, V_WINDOW, 1, &rev1_input), h(V1_PAYLOAD));
	assert_eq!(st::approval_digest(&genesis, V_WINDOW, 1, &rev1_input), h256(V1_DIGEST));

	// The SCALE input decodes back from the payload tail.
	let tail = &h(V0_PAYLOAD)[st::SETTLE_DOMAIN.len() + 32 + 4 + 1..];
	assert_eq!(<ApprovalInput as codec::Decode>::decode(&mut &tail[..]).unwrap(), input);

	// sr25519 is randomised: sign the vector digest and verify it the way
	// the chain does.
	let sig = approver().sign(digest.as_bytes());
	assert!(sp_io::crypto::sr25519_verify(&sig, digest.as_bytes(), &approver().public()));
	assert!(!sp_io::crypto::sr25519_verify(&sig, digest.as_bytes(), &other_signer().public()));
}

/// The least revenue for which `paid` passes the cap at 50 % + 0.1 %.
fn min_charged(paid: u128) -> u128 {
	(paid * 10_000).div_ceil(5_010)
}

#[test]
fn contract_vector_settles_end_to_end_with_its_revision() {
	new_test_ext().execute_with(|| {
		frame_system::BlockHash::<Test>::insert(0, H256::repeat_byte(0x11));
		assert_eq!(Hippocampus::genesis_hash(), H256::repeat_byte(0x11));
		root_apply(keys_change(vec![submitter()], vec![approver().public()]));
		root_apply(RootChange::Params(params()));
		root_apply(RootChange::MaxDailyAlpha(u128::MAX));
		root_apply(RootChange::FirstWindow(V_WINDOW));
		let _ = Balances::deposit_creating(&Hippocampus::settlement_account(), 1_000_000_000_000_000_000);
		let _ = Balances::deposit_creating(&Hippocampus::owed_account(), EXISTENTIAL_DEPOSIT);
		root_apply(RootChange::Enable);
		fresh_price(V_PRICE);
		set_current_window(V_WINDOW + 8);
		for s in 1..=3u8 {
			let _ = Balances::deposit_creating(&family(s), 1_000);
		}

		// One credit less revenue and the vector approval is above the cap.
		set_final_charged(V_WINDOW, min_charged(V0_USD) - 1);
		let sig = approver().sign(h256(V0_DIGEST).as_bytes());
		assert_refused(
			Hippocampus::submit_compute_approval(
				RuntimeOrigin::signed(submitter()),
				V_WINDOW,
				0,
				vector_input_rev0(),
				sig.clone(),
			),
			Error::<Test>::RevenueCapExceeded,
		);
		set_final_charged(V_WINDOW, min_charged(V0_USD));
		assert_eq!(
			Hippocampus::submit_compute_approval(
				RuntimeOrigin::signed(submitter()),
				V_WINDOW,
				0,
				vector_input_rev0(),
				sig
			),
			Ok(PostDispatchInfo { actual_weight: None, pays_fee: Pays::No })
		);
		assert_eq!(
			ComputeApproval::<Test>::get((V_WINDOW, 0)).unwrap().approval_digest,
			h256(V0_DIGEST)
		);
		assert_eq!(WindowParams::<Test>::get(V_WINDOW), Some((1, params(), 1)));
		assert_eq!(InFlightApproval::<Test>::get(), Some((V_WINDOW, 0)));
		let rev0 = vector_entries_rev0();
		assert_ok!(pay(V_WINDOW, 0, 0, rev0.clone()));
		for e in &rev0 {
			assert_eq!(Balances::free_balance(&e.family), 1_000 + e.alpha);
		}
		let rev0_alpha: u128 = rev0.iter().map(|e| e.alpha).sum();
		assert_eq!(ComputeWindowPaid::<Test>::get(V_WINDOW), (V0_USD, rev0_alpha));
		assert_eq!(NextComputeWindow::<Test>::get(), V_WINDOW + 1);
		assert_eq!(ComputeRevisionSeq::<Test>::get((V_WINDOW, 0)), Some(0));
		assert_eq!(NextRevisionSeq::<Test>::get(), 1);
		assert_eq!(ApprovedUnpaidAlpha::<Test>::get(), 0);

		// The escrowed lines matured: their revenue moved into the window and
		// revision 1 pays them on a later day, inside the window's headroom.
		set_current_window(V_WINDOW + 120);
		fresh_price(V_PRICE);
		set_final_charged(V_WINDOW, min_charged(V0_USD + V1_USD) - 1);
		let sig = approver().sign(h256(V1_DIGEST).as_bytes());
		assert_refused(
			Hippocampus::submit_compute_approval(
				RuntimeOrigin::signed(submitter()),
				V_WINDOW,
				1,
				vector_input_rev1(),
				sig.clone(),
			),
			Error::<Test>::RevenueCapExceeded,
		);
		set_final_charged(V_WINDOW, min_charged(V0_USD + V1_USD));
		assert_ok!(Hippocampus::submit_compute_approval(
			RuntimeOrigin::signed(submitter()),
			V_WINDOW,
			1,
			vector_input_rev1(),
			sig
		));
		let rev1 = vector_entries_rev1();
		let before: Vec<u128> = rev1.iter().map(|e| Balances::free_balance(&e.family)).collect();
		assert_ok!(pay(V_WINDOW, 1, 0, rev1.clone()));
		for (e, b) in rev1.iter().zip(before) {
			assert_eq!(Balances::free_balance(&e.family), b + e.alpha);
		}
		assert_eq!(ComputeWindowPaid::<Test>::get(V_WINDOW).0, V0_USD + V1_USD);
		assert_eq!(ComputeRevisionSeq::<Test>::get((V_WINDOW, 1)), Some(1));
		assert_eq!(ComputeLastRevisionDay::<Test>::get(V_WINDOW), Some(V_WINDOW + 120));
	});
}

// ---------------------------------------------------------------------------
// Happy paths
// ---------------------------------------------------------------------------

#[test]
fn a_window_settles_in_ordered_parts_and_advances() {
	new_test_ext().execute_with(|| {
		setup();
		let p0: Vec<_> = (1..=64u8).map(|s| entry(s, 100)).collect();
		let p1 = vec![entry(200, 300)];
		let parts = vec![p0.clone(), p1.clone()];
		for s in 1..=64u8 {
			let _ = Balances::deposit_creating(&family(s), 100);
		}
		let _ = Balances::deposit_creating(&family(200), 100);
		approve(W, 0, &parts);
		assert_eq!(ApprovedUnpaidAlpha::<Test>::get(), 6_700);
		assert_ok!(pay(W, 0, 0, p0));
		assert_eq!(NextComputeWindow::<Test>::get(), W);
		assert_eq!(ComputeWindowPaid::<Test>::get(W), (0, 0));
		assert_eq!(
			pay(W, 0, 1, p1),
			Ok(PostDispatchInfo { actual_weight: None, pays_fee: Pays::No })
		);
		assert_eq!(ComputeWindowPaid::<Test>::get(W), (6_700, 6_700));
		assert_eq!(ApprovedUnpaidAlpha::<Test>::get(), 0);
		assert_eq!(NextComputeWindow::<Test>::get(), W + 1);
		assert_eq!(Balances::free_balance(family(200)), 400);
		assert_eq!(ComputeRevisionSeq::<Test>::get((W, 0)), Some(0));
		assert_eq!(ComputeLastRevisionDay::<Test>::get(W), Some(W + 8));
		assert!(InFlightApproval::<Test>::get().is_none());
		assert_eq!(Balances::free_balance(Hippocampus::settlement_account()), FUNDED - 6_700);
	});
}

#[test]
fn zero_parts_completes_at_approval() {
	new_test_ext().execute_with(|| {
		setup();
		approve(W, 0, &[]);
		assert_eq!(NextComputeWindow::<Test>::get(), W + 1);
		assert!(ComputeRevisionSeq::<Test>::contains_key((W, 0)));
		assert!(InFlightApproval::<Test>::get().is_none());
		assert_noop!(pay(W, 0, 0, vec![entry(1, 1)]).map_err(|e| e.error), err(Error::<Test>::PartOutOfOrder));
	});
}

#[test]
fn zero_parts_with_usd_or_hash_count_mismatch_is_malformed() {
	new_test_ext().execute_with(|| {
		setup();
		let mut input = input_for(W, 0, &[], 0);
		input.usd = 1;
		assert_refused(submit(W, 0, input), Error::<Test>::MalformedApproval);
		let mut input = input_for(W, 0, &[vec![entry(1, 10)]], 10);
		input.parts = 2;
		assert_refused(submit(W, 0, input), Error::<Test>::MalformedApproval);
	});
}

#[test]
fn failed_transfer_is_owed_and_claimable_once() {
	new_test_ext().execute_with(|| {
		setup();
		// family 9 does not exist and 5 is below the ED (10): the transfer fails.
		let parts = vec![vec![entry(9, 5)]];
		approve(W, 0, &parts);
		assert_ok!(pay(W, 0, 0, parts[0].clone()));
		assert_eq!(ComputeOwed::<Test>::get(family(9)), 5);
		assert_eq!(ComputeOwedTotal::<Test>::get(), 5);
		assert_eq!(Balances::free_balance(Hippocampus::owed_account()), EXISTENTIAL_DEPOSIT + 5);
		System::assert_has_event(Event::ComputeOwedRecorded { family: family(9), alpha: 5 }.into());
		// Counted as paid: the window completes.
		assert_eq!(NextComputeWindow::<Test>::get(), W + 1);

		let _ = Balances::deposit_creating(&family(9), 50);
		assert_ok!(Hippocampus::claim_compute_owed(RuntimeOrigin::signed(family(9))));
		assert_eq!(Balances::free_balance(family(9)), 55);
		assert_eq!(ComputeOwedTotal::<Test>::get(), 0);
		assert_noop!(
			Hippocampus::claim_compute_owed(RuntimeOrigin::signed(family(9))),
			Error::<Test>::NothingOwed
		);
		assert_noop!(
			Hippocampus::claim_compute_owed(RuntimeOrigin::signed(dave())),
			Error::<Test>::NothingOwed
		);
	});
}

#[test]
fn full_part_with_every_transfer_failing_goes_to_owed() {
	new_test_ext().execute_with(|| {
		setup();
		let p0: Vec<_> = (1..=64u8).map(|s| entry(s, 5)).collect();
		approve(W, 0, &[p0.clone()]);
		assert_ok!(pay(W, 0, 0, p0));
		assert_eq!(ComputeOwedTotal::<Test>::get(), 320);
		for s in 1..=64u8 {
			assert_eq!(ComputeOwed::<Test>::get(family(s)), 5);
		}
	});
}

#[test]
fn unseeded_owed_account_fails_the_whole_part() {
	new_test_ext().execute_with(|| {
		setup();
		// Drain the seed so a 5-planck move to the owed account cannot land.
		frame_system::Account::<Test>::remove(Hippocampus::owed_account());
		let parts = vec![vec![entry(1, 1_000), entry(9, 5)]];
		let _ = Balances::deposit_creating(&family(1), 100);
		approve(W, 0, &parts);
		let settlement_before = Balances::free_balance(Hippocampus::settlement_account());
		let reserved_before = ApprovedUnpaidAlpha::<Test>::get();
		let daily_before = ComputeDailyAlpha::<Test>::get();
		assert_refused(pay(W, 0, 0, parts[0].clone()), Error::<Test>::OwedTransferFailed);
		// Nothing moved, the part is still next: the earlier entry's payment
		// is undone with the rest, so a retry cannot pay it twice.
		assert_eq!(Balances::free_balance(family(1)), 100);
		assert_eq!(Balances::free_balance(Hippocampus::settlement_account()), settlement_before);
		assert_eq!(ComputeProgress::<Test>::get((W, 0)).unwrap().next_part, 0);
		assert_eq!(ApprovedUnpaidAlpha::<Test>::get(), reserved_before);
		assert_eq!(ComputeDailyAlpha::<Test>::get(), daily_before);
		assert_eq!(ComputeOwedTotal::<Test>::get(), 0);
		// Seeded again, the same part pays once.
		let _ = Balances::deposit_creating(&Hippocampus::owed_account(), EXISTENTIAL_DEPOSIT);
		assert_ok!(pay(W, 0, 0, parts[0].clone()));
		assert_eq!(Balances::free_balance(family(1)), 1_100);
		assert_eq!(ComputeOwed::<Test>::get(family(9)), 5);
	});
}

#[test]
fn enable_needs_both_accounts_seeded() {
	new_test_ext().execute_with(|| {
		assert_ok!(Hippocampus::schedule_compute_root_change(RuntimeOrigin::root(), RootChange::Enable));
		System::set_block_number(System::block_number() + DELAY);
		assert_noop!(
			Hippocampus::apply_compute_root_change(RuntimeOrigin::signed(alice()), ChangeKind::Enable),
			Error::<Test>::SettlementNotSeeded
		);
		// The refused apply keeps the change pending.
		assert!(PendingRootChange::<Test>::get(ChangeKind::Enable).is_some());
		let _ = Balances::deposit_creating(&Hippocampus::settlement_account(), 1_000);
		assert_noop!(
			Hippocampus::apply_compute_root_change(RuntimeOrigin::signed(alice()), ChangeKind::Enable),
			Error::<Test>::SettlementNotSeeded
		);
		// Funding seeds the owed account too.
		assert_ok!(Hippocampus::fund_compute_settlement(RuntimeOrigin::signed(alice()), 1_000));
		assert_eq!(Balances::free_balance(Hippocampus::owed_account()), EXISTENTIAL_DEPOSIT);
		assert_ok!(Hippocampus::apply_compute_root_change(RuntimeOrigin::signed(alice()), ChangeKind::Enable));
		assert!(ComputeSettlementEnabled::<Test>::get());
		// An owed claim leaves the seed behind, so it can be claimed in full.
	});
}

#[test]
fn revision_pays_escrowed_hours_on_a_later_day() {
	new_test_ext().execute_with(|| {
		setup();
		let _ = Balances::deposit_creating(&family(1), 100);
		approve(W, 0, &[vec![entry(1, 100_000)]]);
		assert_ok!(pay(W, 0, 0, vec![entry(1, 100_000)]));
		// Same day: refused.
		assert_refused(
			submit(W, 1, input_for(W, 1, &[vec![entry(1, 10)]], 10)),
			Error::<Test>::RevisionSameDay,
		);
		set_current_window(W + 9);
		approve(W, 1, &[vec![entry(1, 10)]]);
		assert_ok!(pay(W, 1, 0, vec![entry(1, 10)]));
		assert_eq!(ComputeWindowPaid::<Test>::get(W), (100_010, 100_010));
		// Revision 0 of the next window is unaffected by revisions.
		assert_eq!(NextComputeWindow::<Test>::get(), W + 1);
	});
}

#[test]
fn revoke_frees_the_slot_before_any_part_is_paid() {
	new_test_ext().execute_with(|| {
		setup();
		let parts = vec![vec![entry(1, 100)]];
		approve(W, 0, &parts);
		let digest = ComputeApproval::<Test>::get((W, 0)).unwrap().approval_digest;
		let genesis = Hippocampus::genesis_hash();

		// Wrong signer, or a signature over another revision, is refused.
		let bad = other_signer().sign(st::revoke_digest(&genesis, W, 0, &digest).as_bytes());
		assert_refused(
			Hippocampus::revoke_compute_approval(RuntimeOrigin::signed(submitter()), W, 0, bad),
			Error::<Test>::BadApproverSignature,
		);
		let other_rev = approver().sign(st::revoke_digest(&genesis, W, 1, &digest).as_bytes());
		assert_refused(
			Hippocampus::revoke_compute_approval(RuntimeOrigin::signed(submitter()), W, 0, other_rev),
			Error::<Test>::BadApproverSignature,
		);
		let good = approver().sign(st::revoke_digest(&genesis, W, 0, &digest).as_bytes());
		assert_refused(
			Hippocampus::revoke_compute_approval(RuntimeOrigin::signed(dave()), W, 0, good.clone()),
			Error::<Test>::NotComputeSubmitter,
		);
		assert_ok!(Hippocampus::revoke_compute_approval(
			RuntimeOrigin::signed(submitter()),
			W,
			0,
			good
		));
		assert!(ComputeApproval::<Test>::get((W, 0)).is_none());
		assert!(WindowParams::<Test>::get(W).is_none());
		assert_eq!(ApprovedUnpaidAlpha::<Test>::get(), 0);
		// The slot can be approved again.
		approve(W, 0, &parts);
	});
}

#[test]
fn revoke_is_refused_once_a_part_is_paid() {
	new_test_ext().execute_with(|| {
		setup();
		let p0: Vec<_> = (1..=64u8).map(|s| entry(s, 100)).collect();
		for s in 1..=64u8 {
			let _ = Balances::deposit_creating(&family(s), 100);
		}
		approve(W, 0, &[p0.clone(), vec![entry(200, 1)]]);
		assert_ok!(pay(W, 0, 0, p0));
		let digest = ComputeApproval::<Test>::get((W, 0)).unwrap().approval_digest;
		let sig = approver().sign(st::revoke_digest(&Hippocampus::genesis_hash(), W, 0, &digest).as_bytes());
		assert_refused(
			Hippocampus::revoke_compute_approval(RuntimeOrigin::signed(submitter()), W, 0, sig),
			Error::<Test>::ApprovalAlreadyStarted,
		);
	});
}

// ---------------------------------------------------------------------------
// Must refuse: switches, origins, keys
// ---------------------------------------------------------------------------

#[test]
fn either_switch_off_refuses_submit_and_pay_only_distribution_stops_claims() {
	new_test_ext().execute_with(|| {
		setup();
		let parts = vec![vec![entry(1, 100)]];
		let input = input_for(W, 0, &parts, 100);

		ComputeSettlementEnabled::<Test>::put(false);
		assert_refused(submit(W, 0, input.clone()), Error::<Test>::ComputeSettlementDisabled);
		assert_refused(pay(W, 0, 0, parts[0].clone()), Error::<Test>::ComputeSettlementDisabled);
		// A settlement pause does not stop claims: alice is refused only for
		// having nothing owed.
		assert_noop!(
			Hippocampus::claim_compute_owed(RuntimeOrigin::signed(alice())),
			Error::<Test>::NothingOwed
		);

		ComputeSettlementEnabled::<Test>::put(true);
		DistributionEnabled::<Test>::put(false);
		assert_refused(submit(W, 0, input.clone()), Error::<Test>::DistributionDisabled);
		assert_refused(pay(W, 0, 0, parts[0].clone()), Error::<Test>::DistributionDisabled);
		assert_noop!(
			Hippocampus::claim_compute_owed(RuntimeOrigin::signed(alice())),
			Error::<Test>::DistributionDisabled
		);

		// A part of an accepted approval is refused once a switch goes off.
		DistributionEnabled::<Test>::put(true);
		let _ = Balances::deposit_creating(&family(1), 100);
		assert_ok!(submit(W, 0, input));
		assert_ok!(Hippocampus::disable_compute_settlement(RuntimeOrigin::root()));
		assert_refused(pay(W, 0, 0, parts[0].clone()), Error::<Test>::ComputeSettlementDisabled);
	});
}

#[test]
fn admin_can_only_disable() {
	new_test_ext().execute_with(|| {
		setup();
		// The mock's admin origin is root; a signed account is neither admin
		// nor root.
		assert_noop!(
			Hippocampus::disable_compute_settlement(RuntimeOrigin::signed(alice())),
			DispatchError::BadOrigin
		);
		for change in [
			RootChange::Enable,
			RootChange::Params(params()),
			keys_change(vec![dave()], vec![approver().public()]),
			RootChange::MaxDailyAlpha(1),
			RootChange::Defund(1),
		] {
			assert_noop!(
				Hippocampus::schedule_compute_root_change(RuntimeOrigin::signed(alice()), change),
				DispatchError::BadOrigin
			);
		}
		// Disabling drops a pending enable.
		ComputeSettlementEnabled::<Test>::put(false);
		assert_ok!(Hippocampus::schedule_compute_root_change(RuntimeOrigin::root(), RootChange::Enable));
		assert_ok!(Hippocampus::disable_compute_settlement(RuntimeOrigin::root()));
		assert!(PendingRootChange::<Test>::get(ChangeKind::Enable).is_none());
		assert!(!ComputeSettlementEnabled::<Test>::get());
	});
}

#[test]
fn root_change_waits_for_its_delay() {
	new_test_ext().execute_with(|| {
		assert_ok!(Hippocampus::fund_compute_settlement(RuntimeOrigin::signed(alice()), 1_000));
		assert_ok!(Hippocampus::schedule_compute_root_change(RuntimeOrigin::root(), RootChange::Enable));
		System::set_block_number(System::block_number() + DELAY - 1);
		assert_noop!(
			Hippocampus::apply_compute_root_change(RuntimeOrigin::signed(alice()), ChangeKind::Enable),
			Error::<Test>::ChangeNotDue
		);
		System::set_block_number(System::block_number() + 1);
		assert_ok!(Hippocampus::apply_compute_root_change(RuntimeOrigin::signed(alice()), ChangeKind::Enable));
		assert!(ComputeSettlementEnabled::<Test>::get());
		assert_noop!(
			Hippocampus::apply_compute_root_change(RuntimeOrigin::signed(alice()), ChangeKind::Enable),
			Error::<Test>::NoPendingChange
		);
		// Cancel.
		assert_ok!(Hippocampus::schedule_compute_root_change(
			RuntimeOrigin::root(),
			RootChange::MaxDailyAlpha(5)
		));
		assert_ok!(Hippocampus::cancel_compute_root_change(RuntimeOrigin::root(), ChangeKind::MaxDailyAlpha));
		System::set_block_number(System::block_number() + DELAY);
		assert_noop!(
			Hippocampus::apply_compute_root_change(RuntimeOrigin::signed(alice()), ChangeKind::MaxDailyAlpha),
			Error::<Test>::NoPendingChange
		);
	});
}

#[test]
fn first_window_is_set_once() {
	new_test_ext().execute_with(|| {
		root_apply(RootChange::FirstWindow(W));
		assert_eq!(FirstComputeWindow::<Test>::get(), Some(W));
		assert_noop!(
			Hippocampus::schedule_compute_root_change(RuntimeOrigin::root(), RootChange::FirstWindow(W + 1)),
			Error::<Test>::FirstWindowAlreadySet
		);
	});
}

#[test]
fn invalid_params_are_refused() {
	new_test_ext().execute_with(|| {
		for p in [
			PayoutParams { share_bps: 9_995, tol_bps: 5, vcpu_hour_uusd: 1 },
			PayoutParams { share_bps: 0, tol_bps: 5, vcpu_hour_uusd: 1 },
			PayoutParams { share_bps: 5_000, tol_bps: 5, vcpu_hour_uusd: 0 },
		] {
			assert_noop!(
				Hippocampus::schedule_compute_root_change(RuntimeOrigin::root(), RootChange::Params(p)),
				Error::<Test>::InvalidParams
			);
		}
	});
}

#[test]
fn overlapping_empty_or_repeated_keys_are_refused() {
	new_test_ext().execute_with(|| {
		let a = approver().public();
		for change in [
			keys_change(vec![], vec![a]),
			keys_change(vec![submitter()], vec![]),
			keys_change(vec![submitter(), submitter()], vec![a]),
			keys_change(vec![submitter()], vec![a, a]),
			// The approver key's account doubles as a submitter.
			keys_change(vec![approver_account()], vec![a]),
		] {
			assert_noop!(
				Hippocampus::schedule_compute_root_change(RuntimeOrigin::root(), change),
				Error::<Test>::InvalidSettlementKeys
			);
		}
	});
}

#[test]
fn a_key_with_another_role_is_refused_when_applied_and_at_approval() {
	new_test_ext().execute_with(|| {
		give_other_role(dave());
		assert_noop!(
			Hippocampus::schedule_compute_root_change(
				RuntimeOrigin::root(),
				keys_change(vec![dave()], vec![approver().public()])
			),
			Error::<Test>::SettlementKeyHasOtherRole
		);
		setup();
		// The approver key's account becomes, say, a credits authority.
		give_other_role(approver_account());
		assert_refused(
			submit(W, 0, input_for(W, 0, &[vec![entry(1, 1)]], 1)),
			Error::<Test>::SettlementKeyHasOtherRole,
		);
	});
}

#[test]
fn non_submitter_is_refused() {
	new_test_ext().execute_with(|| {
		setup();
		let parts = vec![vec![entry(1, 100)]];
		let input = input_for(W, 0, &parts, 100);
		let sig = sign_input(W, 0, &input);
		assert_refused(
			Hippocampus::submit_compute_approval(RuntimeOrigin::signed(dave()), W, 0, input.clone(), sig),
			Error::<Test>::NotComputeSubmitter,
		);
		assert_ok!(submit(W, 0, input));
		assert_refused(
			Hippocampus::pay_compute_part(
				RuntimeOrigin::signed(dave()),
				W,
				0,
				0,
				BoundedVec::try_from(parts[0].clone()).unwrap(),
			),
			Error::<Test>::NotComputeSubmitter,
		);
	});
}

#[test]
fn signature_by_non_approver_or_over_other_context_is_refused() {
	new_test_ext().execute_with(|| {
		setup();
		let input = input_for(W, 0, &[vec![entry(1, 100)]], 100);
		let genesis = Hippocampus::genesis_hash();
		let by_other = other_signer().sign(st::approval_digest(&genesis, W, 0, &input).as_bytes());
		let other_genesis =
			approver().sign(st::approval_digest(&H256::repeat_byte(0x99), W, 0, &input).as_bytes());
		let other_window = approver().sign(st::approval_digest(&genesis, W + 1, 0, &input).as_bytes());
		let other_revision = approver().sign(st::approval_digest(&genesis, W, 1, &input).as_bytes());
		for sig in [by_other, other_genesis, other_window, other_revision] {
			assert_refused(
				Hippocampus::submit_compute_approval(
					RuntimeOrigin::signed(submitter()),
					W,
					0,
					input.clone(),
					sig,
				),
				Error::<Test>::BadApproverSignature,
			);
		}
		// A changed field invalidates the signature too.
		let sig = sign_input(W, 0, &input);
		let mut tampered = input.clone();
		tampered.usd += 1;
		assert_refused(
			Hippocampus::submit_compute_approval(RuntimeOrigin::signed(submitter()), W, 0, tampered, sig),
			Error::<Test>::BadApproverSignature,
		);
	});
}

// ---------------------------------------------------------------------------
// Must refuse: order, revisions, params, revenue, price
// ---------------------------------------------------------------------------

#[test]
fn one_approval_in_flight_across_all_windows() {
	new_test_ext().execute_with(|| {
		setup();
		let _ = Balances::deposit_creating(&family(1), 100);
		approve(W, 0, &[vec![entry(1, 10)]]);
		assert_ok!(pay(W, 0, 0, vec![entry(1, 10)]));
		// Revision 0 of W + 1 in flight blocks a revision of W.
		set_final_charged(W + 1, CHARGED);
		approve(W + 1, 0, &[vec![entry(1, 10)]]);
		set_current_window(W + 9);
		assert_refused(
			submit(W, 1, input_for(W, 1, &[vec![entry(1, 1)]], 1)),
			Error::<Test>::ApprovalInFlight,
		);
		assert_ok!(pay(W + 1, 0, 0, vec![entry(1, 10)]));
		approve(W, 1, &[vec![entry(1, 1)]]);
		assert_ok!(pay(W, 1, 0, vec![entry(1, 1)]));
		// Completion order is global.
		assert_eq!(ComputeRevisionSeq::<Test>::get((W, 0)), Some(0));
		assert_eq!(ComputeRevisionSeq::<Test>::get((W + 1, 0)), Some(1));
		assert_eq!(ComputeRevisionSeq::<Test>::get((W, 1)), Some(2));
		assert_eq!(NextRevisionSeq::<Test>::get(), 3);
		assert!(InFlightApproval::<Test>::get().is_none());
	});
}

#[test]
fn parts_without_usd_is_malformed() {
	new_test_ext().execute_with(|| {
		setup();
		let mut input = input_for(W, 0, &[vec![entry(1, 10)]], 10);
		input.usd = 0;
		assert_refused(submit(W, 0, input), Error::<Test>::MalformedApproval);
	});
}

#[test]
fn defund_does_not_hold_back_owed_funds() {
	new_test_ext().execute_with(|| {
		setup();
		approve(W, 0, &[vec![entry(9, 5)]]);
		assert_ok!(pay(W, 0, 0, vec![entry(9, 5)]));
		assert_eq!(ComputeOwedTotal::<Test>::get(), 5);
		root_apply(RootChange::RevenueAccount(dave()));
		root_apply(RootChange::Defund(u128::MAX));
		// The owed 5 already sit in the owed account; the settlement account
		// keeps only its seed.
		assert_eq!(Balances::free_balance(Hippocampus::settlement_account()), EXISTENTIAL_DEPOSIT);
		assert_eq!(Balances::free_balance(Hippocampus::owed_account()), EXISTENTIAL_DEPOSIT + 5);
	});
}

#[test]
fn owed_funds_stay_claimable_while_settlement_is_paused() {
	new_test_ext().execute_with(|| {
		setup();
		approve(W, 0, &[vec![entry(9, 5)]]);
		assert_ok!(pay(W, 0, 0, vec![entry(9, 5)]));
		let _ = Balances::deposit_creating(&family(9), 50);
		assert_ok!(Hippocampus::disable_compute_settlement(RuntimeOrigin::root()));
		assert_ok!(Hippocampus::claim_compute_owed(RuntimeOrigin::signed(family(9))));
		assert_eq!(Balances::free_balance(family(9)), 55);
	});
}

#[test]
fn records_past_the_revision_horizon_are_pruned() {
	use frame_support::traits::Hooks;
	new_test_ext().execute_with(|| {
		setup();
		let _ = Balances::deposit_creating(&family(1), 100);
		// W: revisions 0 and 1; W + 1: revision 0.
		approve(W, 0, &[vec![entry(1, 10)]]);
		assert_ok!(pay(W, 0, 0, vec![entry(1, 10)]));
		set_current_window(W + 9);
		approve(W, 1, &[vec![entry(1, 1)]]);
		assert_ok!(pay(W, 1, 0, vec![entry(1, 1)]));
		set_final_charged(W + 1, CHARGED);
		approve(W + 1, 0, &[vec![entry(1, 10)]]);
		assert_ok!(pay(W + 1, 0, 0, vec![entry(1, 10)]));

		let max = Weight::from_parts(u64::MAX, u64::MAX);
		// No proof-size room, no work, even with all the time in the world.
		set_current_window(W + 161);
		assert_eq!(
			Hippocampus::on_idle(System::block_number(), Weight::from_parts(u64::MAX, 0)),
			Weight::zero()
		);
		assert!(ComputeApproval::<Test>::contains_key((W, 0)));
		// Inside the horizon nothing goes.
		set_current_window(W + 160);
		Hippocampus::on_idle(System::block_number(), max);
		assert!(ComputeApproval::<Test>::contains_key((W, 0)));

		// Past it for W only.
		set_current_window(W + 161);
		Hippocampus::on_idle(System::block_number(), max);
		for k in 0..=1u8 {
			assert!(!ComputeApproval::<Test>::contains_key((W, k)));
			assert!(!ComputeProgress::<Test>::contains_key((W, k)));
			assert!(!ComputeRevisionSeq::<Test>::contains_key((W, k)));
		}
		assert!(WindowParams::<Test>::get(W).is_none());
		assert_eq!(ComputeWindowPaid::<Test>::get(W), (0, 0));
		assert!(ComputeLastRevisionDay::<Test>::get(W).is_none());
		assert_eq!(crate::ComputePrunedBefore::<Test>::get(), W + 1);
		assert!(ComputeApproval::<Test>::contains_key((W + 1, 0)));

		// The guards still hold for the pruned window.
		fresh_price(PRICE);
		set_final_charged(W, CHARGED);
		assert_refused(
			submit(W, 2, input_for(W, 2, &[vec![entry(1, 1)]], 1)),
			Error::<Test>::WindowPruned,
		);
		assert_refused(submit(W, 0, input_for(W, 0, &[], 0)), Error::<Test>::WindowPruned);
	});
}

#[test]
fn pruning_waits_for_the_window_in_flight_and_is_bounded() {
	use frame_support::traits::Hooks;
	new_test_ext().execute_with(|| {
		setup();
		let _ = Balances::deposit_creating(&family(1), 100);
		approve(W, 0, &[vec![entry(1, 10)]]);
		assert_ok!(pay(W, 0, 0, vec![entry(1, 10)]));
		// A revision of W submitted on its last allowed day, still in flight.
		set_current_window(W + 160);
		approve(W, 1, &[vec![entry(1, 1)]]);
		set_current_window(W + 200);
		let max = Weight::from_parts(u64::MAX, u64::MAX);
		Hippocampus::on_idle(System::block_number(), max);
		assert!(ComputeApproval::<Test>::contains_key((W, 0)));
		assert!(ComputeApproval::<Test>::contains_key((W, 1)));
		// No weight, no work.
		assert_eq!(Hippocampus::on_idle(System::block_number(), Weight::zero()), Weight::zero());
	});
}

#[test]
fn window_before_first_or_out_of_order_is_refused() {
	new_test_ext().execute_with(|| {
		setup();
		set_final_charged(W - 1, CHARGED);
		set_final_charged(W + 1, CHARGED);
		assert_refused(submit(W - 1, 0, input_for(W - 1, 0, &[], 0)), Error::<Test>::WindowBeforeFirst);
		assert_refused(submit(W + 1, 0, input_for(W + 1, 0, &[], 0)), Error::<Test>::WindowOutOfOrder);
		approve(W, 0, &[]);
		assert_refused(submit(W, 0, input_for(W, 0, &[], 0)), Error::<Test>::ApprovalExists);
		approve(W + 1, 0, &[]);
	});
}

#[test]
fn revision_needs_the_previous_completed_and_stays_within_horizon() {
	new_test_ext().execute_with(|| {
		setup();
		let _ = Balances::deposit_creating(&family(1), 100);
		// Revision 1 before revision 0.
		assert_refused(
			submit(W, 1, input_for(W, 1, &[vec![entry(1, 1)]], 1)),
			Error::<Test>::RevisionOutOfOrder,
		);
		// Revision 0 approved but not completed: it is in flight.
		approve(W, 0, &[vec![entry(1, 10)]]);
		set_current_window(W + 9);
		assert_refused(
			submit(W, 1, input_for(W, 1, &[vec![entry(1, 1)]], 1)),
			Error::<Test>::ApprovalInFlight,
		);
		assert_ok!(pay(W, 0, 0, vec![entry(1, 10)]));
		// Revision 2 before revision 1.
		set_current_window(W + 10);
		assert_refused(
			submit(W, 2, input_for(W, 2, &[vec![entry(1, 1)]], 1)),
			Error::<Test>::RevisionOutOfOrder,
		);
		// After W + 160.
		set_current_window(W + 161);
		assert_refused(
			submit(W, 1, input_for(W, 1, &[vec![entry(1, 1)]], 1)),
			Error::<Test>::RevisionTooLate,
		);
		set_current_window(W + 160);
		approve(W, 1, &[vec![entry(1, 1)]]);
	});
}

#[test]
fn params_mismatch_is_refused() {
	new_test_ext().execute_with(|| {
		setup();
		let parts = vec![vec![entry(1, 100)]];
		let mut input = input_for(W, 0, &parts, 100);
		input.params_version = 2;
		assert_refused(submit(W, 0, input), Error::<Test>::ParamsMismatch);
		let mut input = input_for(W, 0, &parts, 100);
		input.params_hash = H256::repeat_byte(1);
		assert_refused(submit(W, 0, input), Error::<Test>::ParamsMismatch);
		let mut input = input_for(W, 0, &parts, 100);
		input.formula_version = 2;
		assert_refused(submit(W, 0, input), Error::<Test>::ParamsMismatch);
	});
}

#[test]
fn revisions_stay_on_the_params_pinned_at_revision_zero() {
	new_test_ext().execute_with(|| {
		setup();
		let _ = Balances::deposit_creating(&family(1), 100);
		approve(W, 0, &[vec![entry(1, 10)]]);
		assert_ok!(pay(W, 0, 0, vec![entry(1, 10)]));
		// Params v2 lands; revision 1 of W must still sign v1.
		let v2 = PayoutParams { share_bps: 4000, tol_bps: 10, vcpu_hour_uusd: 24657 };
		root_apply(RootChange::Params(v2));
		fresh_price(PRICE);
		set_current_window(W + 9);
		let mut input = input_for(W, 1, &[vec![entry(1, 1)]], 1);
		input.params_version = 2;
		input.params_hash = st::params_hash(&v2);
		assert_refused(submit(W, 1, input), Error::<Test>::ParamsMismatch);
		let mut input = input_for(W, 1, &[vec![entry(1, 1)]], 1);
		input.params_version = 1;
		assert_ok!(submit(W, 1, input));
	});
}

#[test]
fn revenue_not_final_is_refused() {
	new_test_ext().execute_with(|| {
		setup();
		clear_final_charged(W);
		assert_refused(submit(W, 0, input_for(W, 0, &[], 0)), Error::<Test>::RevenueNotFinal);
	});
}

#[test]
fn approval_above_the_revenue_cap_is_refused() {
	new_test_ext().execute_with(|| {
		setup();
		// cap = 1_000_000 × 5010 / 10_000 = 501_000 exactly.
		let over = vec![vec![entry(1, 501_001)]];
		assert_refused(submit(W, 0, input_for(W, 0, &over, 501_001)), Error::<Test>::RevenueCapExceeded);
		let at = vec![vec![entry(1, 501_000)]];
		assert_ok!(submit(W, 0, input_for(W, 0, &at, 501_000)));
	});
}

#[test]
fn revision_cap_counts_what_earlier_revisions_paid() {
	new_test_ext().execute_with(|| {
		setup();
		let _ = Balances::deposit_creating(&family(1), 100);
		approve(W, 0, &[vec![entry(1, 400_000)]]);
		assert_ok!(pay(W, 0, 0, vec![entry(1, 400_000)]));
		set_current_window(W + 9);
		assert_refused(
			submit(W, 1, input_for(W, 1, &[vec![entry(1, 101_001)]], 101_001)),
			Error::<Test>::RevenueCapExceeded,
		);
		// Unfrozen escrow raised charged(W): the headroom follows.
		set_final_charged(W, CHARGED + 2_000);
		assert_ok!(submit(W, 1, input_for(W, 1, &[vec![entry(1, 101_001)]], 101_001)));
	});
}

#[test]
fn stale_unset_or_mismatched_price_is_refused() {
	new_test_ext().execute_with(|| {
		setup();
		let input = input_for(W, 0, &[], 0);
		set_alpha_price(PRICE, Some(System::block_number().saturating_sub(MaxAlphaPriceAge::get() + 1)));
		assert_refused(submit(W, 0, input.clone()), Error::<Test>::AlphaPriceStale);
		set_alpha_price(PRICE, None);
		assert_refused(submit(W, 0, input.clone()), Error::<Test>::AlphaPriceStale);
		set_alpha_price(0, Some(System::block_number()));
		assert_refused(submit(W, 0, input.clone()), Error::<Test>::AlphaPriceStale);
		fresh_price(PRICE + 1);
		assert_refused(submit(W, 0, input.clone()), Error::<Test>::AlphaPriceMismatch);
		set_alpha_price(PRICE, Some(System::block_number() - MaxAlphaPriceAge::get()));
		assert_ok!(submit(W, 0, input));
	});
}

#[test]
fn a_price_change_after_approval_does_not_change_the_parts() {
	new_test_ext().execute_with(|| {
		setup();
		let _ = Balances::deposit_creating(&family(1), 100);
		let parts = vec![vec![entry(1, 1_000)]];
		approve(W, 0, &parts);
		fresh_price(PRICE * 2);
		assert_ok!(pay(W, 0, 0, parts[0].clone()));
		assert_eq!(Balances::free_balance(family(1)), 1_100);
	});
}

// ---------------------------------------------------------------------------
// Must refuse: parts
// ---------------------------------------------------------------------------

#[test]
fn part_out_of_order_twice_or_with_a_hash_mismatch_is_refused() {
	new_test_ext().execute_with(|| {
		setup();
		let p0: Vec<_> = (1..=64u8).map(|s| entry(s, 100)).collect();
		let p1 = vec![entry(200, 50)];
		for s in 1..=64u8 {
			let _ = Balances::deposit_creating(&family(s), 100);
		}
		let _ = Balances::deposit_creating(&family(200), 100);
		approve(W, 0, &[p0.clone(), p1.clone()]);
		assert_refused(pay(W, 0, 1, p1.clone()), Error::<Test>::PartOutOfOrder);
		let mut tampered = p0.clone();
		tampered[0].usd += 1;
		tampered[0].alpha += 1;
		assert_refused(pay(W, 0, 0, tampered), Error::<Test>::PartHashMismatch);
		assert_ok!(pay(W, 0, 0, p0.clone()));
		assert_refused(pay(W, 0, 0, p0), Error::<Test>::PartOutOfOrder);
		assert_refused(pay(W, 0, 2, p1.clone()), Error::<Test>::PartOutOfOrder);
		assert_ok!(pay(W, 0, 1, p1.clone()));
		assert_refused(pay(W, 0, 1, p1), Error::<Test>::PartOutOfOrder);
		assert_refused(pay(W, 1, 0, vec![entry(1, 1)]), Error::<Test>::NoApproval);
	});
}

#[test]
fn non_full_part_before_the_last_is_refused() {
	new_test_ext().execute_with(|| {
		setup();
		let p0 = vec![entry(1, 100)];
		let p1 = vec![entry(2, 100)];
		approve(W, 0, &[p0.clone(), p1]);
		assert_refused(pay(W, 0, 0, p0), Error::<Test>::NonCanonicalPart);
	});
}

#[test]
fn families_not_ascending_within_or_across_parts_are_refused() {
	new_test_ext().execute_with(|| {
		setup();
		let within = vec![vec![entry(2, 100), entry(1, 100)]];
		approve(W, 0, &within);
		assert_refused(pay(W, 0, 0, within[0].clone()), Error::<Test>::FamiliesNotAscending);

		let dup = vec![vec![entry(1, 100), entry(1, 100)]];
		set_final_charged(W + 1, CHARGED);
		let digest = ComputeApproval::<Test>::get((W, 0)).unwrap().approval_digest;
		let sig = approver().sign(st::revoke_digest(&Hippocampus::genesis_hash(), W, 0, &digest).as_bytes());
		assert_ok!(Hippocampus::revoke_compute_approval(RuntimeOrigin::signed(submitter()), W, 0, sig));
		approve(W, 0, &dup);
		assert_refused(pay(W, 0, 0, dup[0].clone()), Error::<Test>::FamiliesNotAscending);
	});
}

#[test]
fn families_not_ascending_across_parts_are_refused() {
	new_test_ext().execute_with(|| {
		setup();
		let p0: Vec<_> = (1..=64u8).map(|s| entry(s, 100)).collect();
		for s in 1..=64u8 {
			let _ = Balances::deposit_creating(&family(s), 100);
		}
		// Part 1 starts at a family below the last one of part 0.
		let p1 = vec![entry(64, 50)];
		approve(W, 0, &[p0.clone(), p1.clone()]);
		assert_ok!(pay(W, 0, 0, p0));
		assert_refused(pay(W, 0, 1, p1), Error::<Test>::FamiliesNotAscending);
	});
}

#[test]
fn zero_usd_entry_is_refused() {
	new_test_ext().execute_with(|| {
		setup();
		let parts = vec![vec![entry(1, 0), entry(2, 100)]];
		approve(W, 0, &parts);
		assert_refused(pay(W, 0, 0, parts[0].clone()), Error::<Test>::ZeroUsd);
	});
}

#[test]
fn alpha_not_matching_the_stored_price_is_refused() {
	new_test_ext().execute_with(|| {
		setup();
		let mut e = entry(1, 100);
		e.alpha = 101;
		let parts = vec![vec![e]];
		approve(W, 0, &parts);
		assert_refused(pay(W, 0, 0, parts[0].clone()), Error::<Test>::AlphaMismatch);
	});
}

#[test]
fn sums_above_the_approval_or_unequal_on_the_last_part_are_refused() {
	new_test_ext().execute_with(|| {
		setup();
		// The backend signed a total below the entries.
		let parts = vec![vec![entry(1, 100)]];
		assert_ok!(submit(W, 0, input_for(W, 0, &parts, 99)));
		assert_refused(pay(W, 0, 0, parts[0].clone()), Error::<Test>::TotalsExceeded);
	});
	new_test_ext().execute_with(|| {
		setup();
		// ... or above them: the last part leaves the window short.
		let parts = vec![vec![entry(1, 100)]];
		assert_ok!(submit(W, 0, input_for(W, 0, &parts, 101)));
		assert_refused(pay(W, 0, 0, parts[0].clone()), Error::<Test>::TotalsMismatch);
	});
}

#[test]
fn funds_below_the_seed_are_refused() {
	new_test_ext().execute_with(|| {
		setup();
		let _ = Balances::deposit_creating(&family(1), 100);
		// FUNDED − ED is spendable; one more planck is not.
		set_final_charged(W, CHARGED * 10);
		let parts = vec![vec![entry(1, FUNDED - EXISTENTIAL_DEPOSIT + 1)]];
		approve(W, 0, &parts);
		assert_refused(pay(W, 0, 0, parts[0].clone()), Error::<Test>::InsufficientSettlementFunds);
		assert_ok!(Hippocampus::fund_compute_settlement(RuntimeOrigin::signed(alice()), 1));
		assert_ok!(pay(W, 0, 0, parts[0].clone()));
		assert_eq!(Balances::free_balance(Hippocampus::settlement_account()), EXISTENTIAL_DEPOSIT);
	});
}

#[test]
fn daily_alpha_cap_is_enforced_per_utc_day() {
	new_test_ext().execute_with(|| {
		setup();
		root_apply(RootChange::MaxDailyAlpha(150));
		fresh_price(PRICE);
		let _ = Balances::deposit_creating(&family(1), 100);
		approve(W, 0, &[vec![entry(1, 100)]]);
		assert_ok!(pay(W, 0, 0, vec![entry(1, 100)]));
		set_final_charged(W + 1, CHARGED);
		approve(W + 1, 0, &[vec![entry(1, 100)]]);
		assert_refused(pay(W + 1, 0, 0, vec![entry(1, 100)]), Error::<Test>::DailyAlphaCapExceeded);
		set_current_window(W + 9);
		assert_ok!(pay(W + 1, 0, 0, vec![entry(1, 100)]));
		assert_eq!(ComputeDailyAlpha::<Test>::get(), (u64::from(W + 9), 100));
	});
}

#[test]
fn defund_is_capped_by_open_approvals() {
	new_test_ext().execute_with(|| {
		setup();
		assert_noop!(
			Hippocampus::schedule_compute_root_change(RuntimeOrigin::root(), RootChange::Defund(0)),
			Error::<Test>::ZeroAmount
		);
		approve(W, 0, &[vec![entry(1, 200_000)]]);
		let kind = ChangeKind::Defund;
		assert_ok!(Hippocampus::schedule_compute_root_change(
			RuntimeOrigin::root(),
			RootChange::Defund(u128::MAX)
		));
		System::set_block_number(System::block_number() + DELAY);
		assert_noop!(
			Hippocampus::apply_compute_root_change(RuntimeOrigin::signed(alice()), kind),
			Error::<Test>::NoRevenueAccount
		);
		root_apply(RootChange::RevenueAccount(dave()));
		root_apply(RootChange::Defund(u128::MAX));
		// free − ED − reserved stays behind for the open approval.
		assert_eq!(Balances::free_balance(dave()), FUNDED - EXISTENTIAL_DEPOSIT - 200_000);
		assert_eq!(
			Balances::free_balance(Hippocampus::settlement_account()),
			EXISTENTIAL_DEPOSIT + 200_000
		);
		assert_ok!(Hippocampus::schedule_compute_root_change(RuntimeOrigin::root(), RootChange::Defund(1)));
		System::set_block_number(System::block_number() + DELAY);
		assert_noop!(
			Hippocampus::apply_compute_root_change(RuntimeOrigin::signed(alice()), kind),
			Error::<Test>::NothingToDefund
		);
	});
}

// ---------------------------------------------------------------------------
// The legacy bank paths are untouched by settlement
// ---------------------------------------------------------------------------

#[test]
fn bank_paths_cannot_reach_settlement_funds_and_vice_versa() {
	use crate::DepositType;
	new_test_ext().execute_with(|| {
		setup();
		// Bank: storage + compute emission and some revenue.
		assert_ok!(Hippocampus::deposit(RuntimeOrigin::signed(bob()), 2_000, DepositType::Emission));
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(bob()),
			2_000,
			DepositType::ComputeEmission
		));
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(bob()),
			1_000,
			DepositType::MarketplaceRevenue
		));
		let settlement_before = Balances::free_balance(Hippocampus::settlement_account());

		whitelist_miner_payment_caller(alice());
		let _ = Balances::deposit_creating(&family(1), 100);
		set_storage_miners(vec![(family(1), 1)]);
		set_compute_miners(vec![(family(1), 1)]);
		assert_ok!(Hippocampus::pay_storage_miners(RuntimeOrigin::signed(alice()), 2_000));
		assert_ok!(Hippocampus::pay_compute_miners(RuntimeOrigin::signed(alice()), 2_000));
		assert_ok!(Hippocampus::add_requester(RuntimeOrigin::root(), dave()));
		assert_eq!(Hippocampus::request_payment(&dave(), &family(1), 10_000), Ok(1_000 - EXISTENTIAL_DEPOSIT));
		assert_eq!(Balances::free_balance(Hippocampus::settlement_account()), settlement_before);

		// Settlement pays from its own account; the bank does not move.
		let bank_before = Balances::free_balance(hippocampus_account());
		approve(W, 0, &[vec![entry(1, 1_000)]]);
		assert_ok!(pay(W, 0, 0, vec![entry(1, 1_000)]));
		assert_eq!(Balances::free_balance(hippocampus_account()), bank_before);
		assert_eq!(Balances::free_balance(Hippocampus::settlement_account()), settlement_before - 1_000);
	});
}

#[test]
fn settlement_accounts_are_distinct_from_the_bank() {
	new_test_ext().execute_with(|| {
		let bank = hippocampus_account();
		assert_ne!(Hippocampus::settlement_account(), bank);
		assert_ne!(Hippocampus::owed_account(), bank);
		assert_ne!(Hippocampus::settlement_account(), Hippocampus::owed_account());
	});
}

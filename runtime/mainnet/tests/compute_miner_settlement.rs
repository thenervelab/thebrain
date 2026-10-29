//! Compute miner pay, end to end in the runtime: the marketplace's
//! value-backed compute revenue per source window feeds hippocampus
//! settlement, whose approvals the backend signs and vali submits.
//!
//! Spec: hippius-backend `docs/design/compute-miner-pay-chain.md` §1-§2 and
//! the interface contract `compute-miner-pay-interfaces.md` §3-§4.

use frame_support::{
	assert_noop, assert_ok,
	dispatch::{DispatchClass, GetDispatchInfo},
	traits::{Currency, GetStorageVersion, OnIdle, OnRuntimeUpgrade, StorageVersion},
	BoundedVec,
};
use hippius_mainnet_runtime::{
	migrations::HippocampusComputeSettlementV2, AccountId, Balances, Credits, Hippocampus,
	Marketplace, Runtime, RuntimeCall, RuntimeOrigin, System,
};
use pallet_hippocampus::{
	settlement::{self as st, ApprovalInput, ChangeKind, Entry, PayoutParams, RootChange},
	ComputeRevenueSource, DepositType,
};
use pallet_marketplace::{
	BatchBackedCredits, ComputeChargedByWindow, FrozenComputeCharge, UsageKind, UsageRowV2,
	COMPUTE_BILLING_PERIOD_MS,
};
use sp_core::{crypto::Ss58Codec, sr25519, Pair, H256};
use sp_runtime::{AccountId32, BuildStorage, DispatchError};

/// 2026-01-01T00:00:00Z.
const JAN1_2026_MS: u64 = 1_767_225_600_000;
const NOW: u64 = JAN1_2026_MS / COMPUTE_BILLING_PERIOD_MS;
/// The latest closed hour, and its source window (UTC day).
const PERIOD: u64 = NOW - 1;
const W: u32 = (PERIOD / 24) as u32;
const LAG: u64 = 72;
/// 1 credit per alpha.
const PRICE: u128 = 1_000_000_000_000_000_000;
const ED: u128 = 500;
const DELAY: u64 = 2 * 14_400;

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

/// Credits authority: deposits and sets the price.
fn depositor() -> AccountId {
	account(1)
}

fn biller() -> AccountId {
	account(2)
}

fn sudo() -> AccountId {
	account(3)
}

fn vali() -> AccountId {
	account(4)
}

fn approver() -> sr25519::Pair {
	sr25519::Pair::from_seed(&[7u8; 32])
}

fn admin() -> AccountId {
	AccountId32::from_ss58check("5CVXqxb7mhFTtZVw5BJ8M2ujND9PFymSDxF8bkod6Sm4XJTW").unwrap()
}

fn new_test_ext() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| {
		System::set_block_number(1);
		pallet_timestamp::Now::<Runtime>::put(JAN1_2026_MS);
		assert_ok!(Credits::add_authority(RuntimeOrigin::root(), depositor()));
		assert_ok!(Credits::set_alpha_price(RuntimeOrigin::signed(depositor()), PRICE));
		assert_ok!(Marketplace::set_compute_billing_authorities(
			RuntimeOrigin::root(),
			vec![biller()]
		));
		assert_ok!(Marketplace::set_compute_charge_caps(
			RuntimeOrigin::root(),
			1_000_000_000,
			10_000_000_000
		));
		assert_ok!(Marketplace::set_sudo_key(RuntimeOrigin::root(), sudo()));
		let _ = Balances::deposit_creating(&sudo(), 1_000_000_000_000);
		let _ = Balances::deposit_creating(&vali(), 1_000_000);
		let _ = Balances::deposit_creating(&Hippocampus::account_id(), 1_000_000);
		assert_ok!(Hippocampus::add_requester(
			RuntimeOrigin::signed(admin()),
			Marketplace::account_id()
		));
	});
	ext
}

fn deposit(who: &AccountId, credits: u128, alpha: u128, freeze: bool) -> u64 {
	let id = pallet_marketplace::NextBatchId::<Runtime>::get();
	assert_ok!(Marketplace::deposit(
		RuntimeOrigin::signed(depositor()),
		who.clone(),
		credits,
		alpha,
		freeze,
		None
	));
	id
}

fn row(who: &AccountId, lines: &[(UsageKind, u128)], h: u8) -> UsageRowV2<AccountId> {
	UsageRowV2 {
		who: who.clone(),
		total: lines.iter().map(|(_, a)| a).sum(),
		usage_hash: H256::repeat_byte(h),
		breakdown: lines.to_vec().try_into().unwrap(),
	}
}

fn submit_v2(period: u64, rows: Vec<UsageRowV2<AccountId>>) {
	assert_ok!(Marketplace::submit_usage_v2(
		RuntimeOrigin::signed(biller()),
		period,
		rows.try_into().unwrap()
	));
}

fn charged(window: u32) -> u128 {
	ComputeChargedByWindow::<Runtime>::get(window)
}

fn make_final(window: u32) {
	let period = (u64::from(window) + 1) * 24 + LAG;
	pallet_timestamp::Now::<Runtime>::put(period * COMPUTE_BILLING_PERIOD_MS);
}

// ---------------------------------------------------------------------------
// Revenue accumulator (marketplace)
// ---------------------------------------------------------------------------

#[test]
fn only_the_value_backed_compute_share_counts() {
	new_test_ext().execute_with(|| {
		let alice = account(10);
		let bob = account(11);
		let b = deposit(&alice, 1_000_000, 1_000_000, false);
		assert_eq!(BatchBackedCredits::<Runtime>::get(b), 1_000_000);
		deposit(&bob, 1_000_000, 1_000_000, false);

		// v2: only the Compute line counts.
		submit_v2(PERIOD, vec![row(&alice, &[(UsageKind::Compute, 600), (UsageKind::Bandwidth, 400)], 1)]);
		assert_eq!(charged(W), 600);
		// A v1 row has no breakdown: it cannot be attributed to Compute, so
		// it counts nothing (fail closed), though the account is charged.
		let bob_before = Credits::get_free_credits(&bob);
		assert_ok!(Marketplace::submit_compute_usage(
			RuntimeOrigin::signed(biller()),
			PERIOD,
			vec![(bob.clone(), 500, H256::repeat_byte(2))]
		));
		assert_eq!(Credits::get_free_credits(&bob), bob_before - 500);
		assert_eq!(charged(W), 600);
		// A duplicate adds nothing.
		submit_v2(PERIOD, vec![row(&alice, &[(UsageKind::Compute, 600), (UsageKind::Bandwidth, 400)], 1)]);
		assert_eq!(charged(W), 600);
		// A conflict adds nothing.
		submit_v2(PERIOD, vec![row(&alice, &[(UsageKind::Compute, 700)], 9)]);
		assert_eq!(charged(W), 600);
		// Nor does a refused row (above the per-account cap).
		let carol = account(12);
		deposit(&carol, 10_000_000_000, 10_000_000_000, false);
		submit_v2(PERIOD, vec![row(&carol, &[(UsageKind::Compute, 1_000_000_001)], 3)]);
		assert_eq!(Credits::get_free_credits(&carol), 10_000_000_000);
		assert_eq!(charged(W), 600);
	});
}

#[test]
fn unbacked_or_half_backed_credits_count_what_backs_them() {
	new_test_ext().execute_with(|| {
		// No alpha: minted credits without backing never raise the cap.
		let a = account(10);
		let b = deposit(&a, 1_000_000, 0, false);
		assert_eq!(BatchBackedCredits::<Runtime>::get(b), 0);
		submit_v2(PERIOD, vec![row(&a, &[(UsageKind::Compute, 1_000)], 1)]);
		assert_eq!(charged(W), 0);

		// Half the credits backed: half of each compute debit counts.
		let h = account(11);
		let b = deposit(&h, 1_000_000, 500_000, false);
		assert_eq!(BatchBackedCredits::<Runtime>::get(b), 500_000);
		submit_v2(PERIOD, vec![row(&h, &[(UsageKind::Compute, 1_000)], 2)]);
		assert_eq!(charged(W), 500);

		// The sudo account could not pay the backing: none of it landed.
		let s = account(12);
		let drained = Balances::free_balance(sudo());
		let _ = Balances::slash(&sudo(), drained);
		let b = deposit(&s, 1_000_000, 1_000_000, false);
		assert_eq!(pallet_marketplace::UnbackedBatchAlpha::<Runtime>::get(b), 1_000_000);
		assert_eq!(BatchBackedCredits::<Runtime>::get(b), 0);
		submit_v2(PERIOD, vec![row(&s, &[(UsageKind::Compute, 1_000)], 3)]);
		assert_eq!(charged(W), 500);
	});
}

#[test]
fn a_deposit_without_a_fresh_price_backs_nothing() {
	new_test_ext().execute_with(|| {
		System::set_block_number(1 + hippius_mainnet_runtime::MaxAlphaPriceAge::get() + 1);
		let a = account(10);
		let b = deposit(&a, 1_000_000, 1_000_000, false);
		assert_eq!(BatchBackedCredits::<Runtime>::get(b), 0);
	});
}

#[test]
fn arrears_and_their_later_collection_add_nothing() {
	new_test_ext().execute_with(|| {
		let a = account(10);
		// Nothing to pay with: the row goes to arrears.
		submit_v2(PERIOD, vec![row(&a, &[(UsageKind::Compute, 1_000)], 1)]);
		assert_eq!(pallet_marketplace::ComputeArrears::<Runtime>::get(&a), 1_000);
		assert_eq!(charged(W), 0);
		deposit(&a, 1_000_000, 1_000_000, false);
		assert_ok!(Marketplace::settle_compute_arrears(RuntimeOrigin::signed(biller()), vec![a.clone()]));
		assert_eq!(pallet_marketplace::ComputeArrears::<Runtime>::get(&a), 0);
		assert_eq!(charged(W), 0);
		// The next charged hour counts its own compute only, not the arrears.
		let arrears_row = row(&a, &[(UsageKind::Compute, 300)], 2);
		submit_v2(PERIOD - 1, vec![arrears_row.clone()]);
		assert_eq!(charged(W), 300);
	});
}

#[test]
fn charged_is_final_only_after_the_billing_lag() {
	new_test_ext().execute_with(|| {
		let a = account(10);
		deposit(&a, 1_000_000, 1_000_000, false);
		submit_v2(PERIOD, vec![row(&a, &[(UsageKind::Compute, 1_000)], 1)]);
		assert_eq!(Marketplace::final_compute_charged(W), None);
		let final_period = (u64::from(W) + 1) * 24 + LAG;
		pallet_timestamp::Now::<Runtime>::put((final_period - 1) * COMPUTE_BILLING_PERIOD_MS);
		assert_eq!(Marketplace::final_compute_charged(W), None);
		pallet_timestamp::Now::<Runtime>::put(final_period * COMPUTE_BILLING_PERIOD_MS);
		assert_eq!(Marketplace::final_compute_charged(W), Some(1_000));
		assert_eq!(
			<hippius_mainnet_runtime::MarketplaceComputeRevenue as ComputeRevenueSource>::final_charged(W),
			Some(1_000)
		);
	});
}

#[test]
fn frozen_batch_revenue_counts_only_after_unfreeze() {
	new_test_ext().execute_with(|| {
		let a = account(10);
		let b = deposit(&a, 1_000_000, 1_000_000, true);
		submit_v2(PERIOD, vec![row(&a, &[(UsageKind::Compute, 1_000)], 1)]);
		assert_eq!(charged(W), 0);
		assert_eq!(FrozenComputeCharge::<Runtime>::get(b, W), 1_000);

		assert_noop!(
			Marketplace::release_frozen_charge(RuntimeOrigin::signed(account(20)), b),
			pallet_marketplace::Error::<Runtime>::BatchStillFrozen
		);
		let batch = pallet_marketplace::Batches::<Runtime>::get(b).unwrap();
		System::set_block_number(batch.release_time);
		assert_ok!(Marketplace::release_frozen_charge(RuntimeOrigin::signed(account(20)), b));
		assert_eq!(charged(W), 1_000);
		assert_eq!(FrozenComputeCharge::<Runtime>::get(b, W), 0);
	});
}

#[test]
fn idle_blocks_release_unfrozen_escrow() {
	new_test_ext().execute_with(|| {
		let a = account(10);
		let b = deposit(&a, 1_000_000, 1_000_000, true);
		submit_v2(PERIOD, vec![row(&a, &[(UsageKind::Compute, 1_000)], 1)]);
		let max = <Runtime as frame_system::Config>::BlockWeights::get().max_block;
		// Still frozen: the idle sweep leaves it.
		Marketplace::on_idle(System::block_number(), max);
		assert_eq!(charged(W), 0);
		let batch = pallet_marketplace::Batches::<Runtime>::get(b).unwrap();
		System::set_block_number(batch.release_time);
		Marketplace::on_idle(System::block_number(), max);
		Marketplace::on_idle(System::block_number(), max);
		assert_eq!(charged(W), 1_000);
		assert_eq!(FrozenComputeCharge::<Runtime>::get(b, W), 0);
	});
}

#[test]
fn a_chargeback_deletes_the_escrow() {
	new_test_ext().execute_with(|| {
		let a = account(10);
		let b = deposit(&a, 1_000_000, 1_000_000, true);
		submit_v2(PERIOD, vec![row(&a, &[(UsageKind::Compute, 1_000)], 1)]);
		assert_eq!(FrozenComputeCharge::<Runtime>::get(b, W), 1_000);
		assert_ok!(Marketplace::chargeback(RuntimeOrigin::root(), b));
		assert_eq!(FrozenComputeCharge::<Runtime>::get(b, W), 0);
		assert_eq!(BatchBackedCredits::<Runtime>::get(b), 0);
		let batch_time = System::block_number() + 15 * 28_800;
		System::set_block_number(batch_time);
		let max = <Runtime as frame_system::Config>::BlockWeights::get().max_block;
		Marketplace::on_idle(System::block_number(), max);
		assert_eq!(charged(W), 0);
	});
}

#[test]
fn bought_credits_are_backed_at_the_purchase_price() {
	new_test_ext().execute_with(|| {
		assert_ok!(Marketplace::sudo_set_buy_credits_enabled(RuntimeOrigin::root(), true));
		let buyer = account(10);
		let _ = Balances::deposit_creating(&buyer, 1_000_000_000_000_000_000);
		let id = pallet_marketplace::NextBatchId::<Runtime>::get();
		let alpha = 100_000_000_000_000_000; // 0.1 alpha
		assert_ok!(Marketplace::buy_credits(RuntimeOrigin::signed(buyer.clone()), alpha, 0, None));
		let batch = pallet_marketplace::Batches::<Runtime>::get(id).unwrap();
		assert_eq!(BatchBackedCredits::<Runtime>::get(id), batch.credit_amount);
		submit_v2(PERIOD, vec![row(&buyer, &[(UsageKind::Compute, 1_000)], 1)]);
		assert_eq!(charged(W), 1_000);
	});
}

#[test]
fn a_top_up_adds_its_own_backing() {
	new_test_ext().execute_with(|| {
		assert_ok!(Marketplace::sudo_set_buy_credits_enabled(RuntimeOrigin::root(), true));
		let buyer = account(10);
		let _ = Balances::deposit_creating(&buyer, 1_000_000_000_000_000_000_000);
		let alpha = 100_000_000_000_000_000;
		let max = hippius_mainnet_runtime::MaxBatchesPerBuyer::get();
		// `MaxRequestsPerBlock` is per account per block window.
		let buy = || {
			pallet_marketplace::UserRequestsCount::<Runtime>::remove(&buyer);
			assert_ok!(Marketplace::buy_credits(RuntimeOrigin::signed(buyer.clone()), alpha, 0, None));
		};
		for _ in 0..max {
			buy();
		}
		let last = pallet_marketplace::LastPurchaseBatch::<Runtime>::get(&buyer).unwrap();
		let before = BatchBackedCredits::<Runtime>::get(last);
		buy();
		assert_eq!(pallet_marketplace::LastPurchaseBatch::<Runtime>::get(&buyer), Some(last));
		let batch = pallet_marketplace::Batches::<Runtime>::get(last).unwrap();
		assert_eq!(BatchBackedCredits::<Runtime>::get(last), before * 2);
		assert_eq!(BatchBackedCredits::<Runtime>::get(last), batch.credit_amount);
	});
}

#[test]
fn a_full_usage_call_still_fits_one_extrinsic() {
	new_test_ext().execute_with(|| {
		let rows: Vec<_> = (0..250u32)
			.map(|n| {
				let mut raw = [0u8; 32];
				raw[..4].copy_from_slice(&n.to_le_bytes());
				let lines: Vec<(UsageKind, u128)> =
					(0..16u8).map(|k| (UsageKind::Compute, u128::from(k) + 1)).collect();
				UsageRowV2 {
					who: AccountId32::new(raw),
					total: lines.iter().map(|(_, a)| a).sum(),
					usage_hash: H256::zero(),
					breakdown: lines.try_into().unwrap(),
				}
			})
			.collect();
		let info = RuntimeCall::Marketplace(pallet_marketplace::Call::submit_usage_v2 {
			period: PERIOD,
			rows: rows.try_into().unwrap(),
		})
		.get_dispatch_info();
		let max_extrinsic = <Runtime as frame_system::Config>::BlockWeights::get()
			.get(DispatchClass::Normal)
			.max_extrinsic
			.unwrap();
		assert!(info.weight.all_lte(max_extrinsic), "{:?} > {:?}", info.weight, max_extrinsic);
	});
}

// ---------------------------------------------------------------------------
// Settlement (hippocampus) against real revenue
// ---------------------------------------------------------------------------

fn params() -> PayoutParams {
	PayoutParams { share_bps: 5000, tol_bps: 10, vcpu_hour_uusd: 24657 }
}

fn root_apply(change: RootChange<AccountId>) {
	let kind = change.kind();
	assert_ok!(Hippocampus::schedule_compute_root_change(RuntimeOrigin::root(), change));
	System::set_block_number(System::block_number() + DELAY);
	assert_ok!(Hippocampus::apply_compute_root_change(RuntimeOrigin::signed(vali()), kind));
}

fn keys() -> RootChange<AccountId> {
	RootChange::Keys {
		submitters: BoundedVec::try_from(vec![vali()]).unwrap(),
		approvers: BoundedVec::try_from(vec![approver().public()]).unwrap(),
	}
}

fn configure_settlement(first: u32) {
	root_apply(keys());
	root_apply(RootChange::Params(params()));
	root_apply(RootChange::MaxDailyAlpha(1_000_000_000_000));
	root_apply(RootChange::FirstWindow(first));
	// Seeds the owed account too; enabling refuses unseeded accounts.
	assert_ok!(Hippocampus::fund_compute_settlement(RuntimeOrigin::signed(sudo()), 1_000_000));
	assert_eq!(Balances::free_balance(Hippocampus::owed_account()), ED);
	root_apply(RootChange::Enable);
	assert_ok!(Credits::set_alpha_price(RuntimeOrigin::signed(depositor()), PRICE));
}

fn input(window: u32, entries: &[Entry<AccountId>]) -> ApprovalInput {
	let genesis = Hippocampus::genesis_hash();
	let parts: Vec<H256> = if entries.is_empty() {
		vec![]
	} else {
		vec![st::part_hash(&genesis, window, 0, 0, entries)]
	};
	ApprovalInput {
		parts: parts.len() as u8,
		part_hashes: BoundedVec::try_from(parts).unwrap(),
		usd: entries.iter().map(|e| e.usd).sum(),
		price: PRICE,
		payable_root: H256::repeat_byte(1),
		evidence_root: H256::repeat_byte(2),
		upto_hour: u64::from(window) * 24 + 23,
		params_version: 1,
		formula_version: 1,
		params_hash: st::params_hash(&params()),
	}
}

fn sign(window: u32, input: &ApprovalInput) -> sr25519::Signature {
	approver().sign(st::approval_digest(&Hippocampus::genesis_hash(), window, 0, input).as_bytes())
}

#[test]
fn billing_revenue_bounds_and_funds_a_settlement() {
	new_test_ext().execute_with(|| {
		// A customer pays 1_000_000 credits of compute in window W, fully backed.
		let customer = account(10);
		deposit(&customer, 10_000_000, 10_000_000, false);
		submit_v2(PERIOD, vec![row(&customer, &[(UsageKind::Compute, 1_000_000)], 1)]);
		assert_eq!(charged(W), 1_000_000);

		configure_settlement(W);
		make_final(W);
		let bank_before = Balances::free_balance(Hippocampus::account_id());

		// r + ε = 50.1 %: 501_000 credits at most; one more is refused.
		let family = account(30);
		let _ = Balances::deposit_creating(&family, ED);
		let over = vec![Entry { family: family.clone(), usd: 501_001, alpha: 501_001 }];
		let i = input(W, &over);
		let s = sign(W, &i);
		assert_noop!(
			Hippocampus::submit_compute_approval(RuntimeOrigin::signed(vali()), W, 0, i, s),
			pallet_hippocampus::Error::<Runtime>::RevenueCapExceeded
		);
		let ok = vec![Entry { family: family.clone(), usd: 501_000, alpha: 501_000 }];
		let i = input(W, &ok);
		let s = sign(W, &i);
		assert_ok!(Hippocampus::submit_compute_approval(RuntimeOrigin::signed(vali()), W, 0, i, s));
		assert_ok!(Hippocampus::pay_compute_part(
			RuntimeOrigin::signed(vali()),
			W,
			0,
			0,
			BoundedVec::try_from(ok).unwrap()
		));
		assert_eq!(Balances::free_balance(&family), ED + 501_000);
		assert_eq!(Balances::free_balance(Hippocampus::settlement_account()), 1_000_000 - 501_000);
		// The bank is untouched.
		assert_eq!(Balances::free_balance(Hippocampus::account_id()), bank_before);
		assert_eq!(pallet_hippocampus::NextComputeWindow::<Runtime>::get(), W + 1);
	});
}

#[test]
fn a_payment_below_ed_is_owed_and_claimed_in_full() {
	new_test_ext().execute_with(|| {
		let customer = account(10);
		deposit(&customer, 10_000_000, 10_000_000, false);
		submit_v2(PERIOD, vec![row(&customer, &[(UsageKind::Compute, 1_000_000)], 1)]);
		configure_settlement(W);
		make_final(W);
		// A fresh family paid less than the ED: the transfer fails, the
		// amount moves to the (seeded) owed account.
		let family = account(31);
		let owed = vec![Entry { family: family.clone(), usd: ED - 1, alpha: ED - 1 }];
		let i = input(W, &owed);
		let s = sign(W, &i);
		assert_ok!(Hippocampus::submit_compute_approval(RuntimeOrigin::signed(vali()), W, 0, i, s));
		assert_ok!(Hippocampus::pay_compute_part(
			RuntimeOrigin::signed(vali()),
			W,
			0,
			0,
			BoundedVec::try_from(owed).unwrap()
		));
		assert_eq!(pallet_hippocampus::ComputeOwed::<Runtime>::get(&family), ED - 1);
		assert_eq!(Balances::free_balance(Hippocampus::owed_account()), ED + ED - 1);
		// Once the family exists it claims all of it; the seed stays.
		let _ = Balances::deposit_creating(&family, ED);
		assert_ok!(Hippocampus::claim_compute_owed(RuntimeOrigin::signed(family.clone())));
		assert_eq!(Balances::free_balance(&family), ED + ED - 1);
		assert_eq!(Balances::free_balance(Hippocampus::owed_account()), ED);
		assert_noop!(
			Hippocampus::claim_compute_owed(RuntimeOrigin::signed(family)),
			pallet_hippocampus::Error::<Runtime>::NothingOwed
		);
	});
}

#[test]
fn legacy_bank_payouts_work_beside_settlement() {
	new_test_ext().execute_with(|| {
		configure_settlement(W);
		let settlement = Balances::free_balance(Hippocampus::settlement_account());
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(sudo()),
			10_000,
			DepositType::ComputeEmission
		));
		assert_ok!(Hippocampus::add_miner_payment_caller(RuntimeOrigin::signed(admin()), sudo()));
		// No compute miner weights: refused exactly as before, nothing moves.
		assert_noop!(
			Hippocampus::pay_compute_miners(RuntimeOrigin::signed(sudo()), 10_000),
			pallet_hippocampus::Error::<Runtime>::NoEligibleComputeMiners
		);
		let paid = Hippocampus::request_payment(&Marketplace::account_id(), &account(40), 1_000);
		assert_eq!(paid, Ok(1_000));
		assert_eq!(Balances::free_balance(Hippocampus::settlement_account()), settlement);
	});
}

#[test]
fn a_settlement_key_with_another_role_is_refused() {
	new_test_ext().execute_with(|| {
		for other in [depositor(), biller()] {
			let change = RootChange::Keys {
				submitters: BoundedVec::try_from(vec![other]).unwrap(),
				approvers: BoundedVec::try_from(vec![approver().public()]).unwrap(),
			};
			assert_noop!(
				Hippocampus::schedule_compute_root_change(RuntimeOrigin::root(), change),
				pallet_hippocampus::Error::<Runtime>::SettlementKeyHasOtherRole
			);
		}
		assert_ok!(Marketplace::set_charge_cap_admins(RuntimeOrigin::root(), vec![account(50)]));
		let change = RootChange::Keys {
			submitters: BoundedVec::try_from(vec![account(50)]).unwrap(),
			approvers: BoundedVec::try_from(vec![approver().public()]).unwrap(),
		};
		assert_noop!(
			Hippocampus::schedule_compute_root_change(RuntimeOrigin::root(), change),
			pallet_hippocampus::Error::<Runtime>::SettlementKeyHasOtherRole
		);
		// A key that later becomes a credits authority blocks approvals.
		configure_settlement(W);
		assert_ok!(Credits::add_authority(RuntimeOrigin::root(), vali()));
		make_final(W);
		assert_ok!(Credits::set_alpha_price(RuntimeOrigin::signed(depositor()), PRICE));
		let i = input(W, &[]);
		let s = sign(W, &i);
		assert_noop!(
			Hippocampus::submit_compute_approval(RuntimeOrigin::signed(vali()), W, 0, i, s),
			pallet_hippocampus::Error::<Runtime>::SettlementKeyHasOtherRole
		);
	});
}

#[test]
fn the_admin_can_disable_but_not_enable() {
	new_test_ext().execute_with(|| {
		assert_noop!(
			Hippocampus::schedule_compute_root_change(RuntimeOrigin::signed(admin()), RootChange::Enable),
			DispatchError::BadOrigin
		);
		configure_settlement(W);
		assert_ok!(Hippocampus::disable_compute_settlement(RuntimeOrigin::signed(admin())));
		assert!(!pallet_hippocampus::ComputeSettlementEnabled::<Runtime>::get());
		assert_noop!(
			Hippocampus::apply_compute_root_change(RuntimeOrigin::signed(admin()), ChangeKind::Enable),
			pallet_hippocampus::Error::<Runtime>::NoPendingChange
		);
	});
}

#[test]
fn settlement_calls_fit_one_extrinsic() {
	new_test_ext().execute_with(|| {
		let max_extrinsic = <Runtime as frame_system::Config>::BlockWeights::get()
			.get(DispatchClass::Normal)
			.max_extrinsic
			.unwrap();
		let entries: Vec<Entry<AccountId>> =
			(0..64u8).map(|n| Entry { family: account(n), usd: 1, alpha: 1 }).collect();
		let pay = RuntimeCall::Hippocampus(pallet_hippocampus::Call::pay_compute_part {
			window: W,
			revision: 0,
			part: 0,
			entries: BoundedVec::try_from(entries).unwrap(),
		})
		.get_dispatch_info();
		assert!(pay.weight.all_lte(max_extrinsic), "{:?}", pay.weight);
		let i = input(W, &[]);
		let s = sign(W, &i);
		let submit = RuntimeCall::Hippocampus(pallet_hippocampus::Call::submit_compute_approval {
			window: W,
			revision: 0,
			input: i,
			backend_sig: s,
		})
		.get_dispatch_info();
		assert!(submit.weight.all_lte(max_extrinsic));
	});
}

// ---------------------------------------------------------------------------
// Migration
// ---------------------------------------------------------------------------

#[test]
fn hippocampus_v2_migration_only_stamps_the_version() {
	new_test_ext().execute_with(|| {
		StorageVersion::new(1).put::<Hippocampus>();
		assert_ok!(Hippocampus::deposit(RuntimeOrigin::signed(sudo()), 5_000, DepositType::Emission));
		assert_ok!(Hippocampus::deposit(
			RuntimeOrigin::signed(sudo()),
			7_000,
			DepositType::MarketplaceRevenue
		));
		let before = HippocampusComputeSettlementV2::<Runtime>::legacy_bank_state();

		HippocampusComputeSettlementV2::<Runtime>::on_runtime_upgrade();
		assert_eq!(Hippocampus::on_chain_storage_version(), 2);
		assert_eq!(HippocampusComputeSettlementV2::<Runtime>::legacy_bank_state(), before);
		assert!(HippocampusComputeSettlementV2::<Runtime>::settlement_inert());

		// Idempotent: a second run does nothing.
		HippocampusComputeSettlementV2::<Runtime>::on_runtime_upgrade();
		assert_eq!(Hippocampus::on_chain_storage_version(), 2);
		assert_eq!(HippocampusComputeSettlementV2::<Runtime>::legacy_bank_state(), before);
	});
}

#[test]
fn marketplace_v3_migration_only_stamps_the_version() {
	new_test_ext().execute_with(|| {
		StorageVersion::new(2).put::<Marketplace>();
		pallet_marketplace::migrations::Migrate::<Runtime>::on_runtime_upgrade();
		assert_eq!(Marketplace::on_chain_storage_version(), 3);
		assert!(ComputeChargedByWindow::<Runtime>::iter().next().is_none());
	});
}

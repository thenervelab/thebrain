//! Benchmarks for the compute settlement calls (10-18).
//!
//! Each call is posed at its worst case:
//! - `submit_compute_approval`: four approver keys with the signing key
//!   last (four sr25519 verifications), four submitters, sixteen part hashes;
//! - `pay_compute_part(n)`: `n` entries up to `MaxEntriesPerPart`, the last
//!   part of its approval (completion runs), every family absent and paid
//!   below the existential deposit, so every transfer fails and goes to the
//!   owed account;
//! - `revoke_compute_approval`: four keys, signer last;
//! - `apply_compute_root_change`: `Defund`, the only change that transfers.
//!
//! Signing uses the benchmark keystore (`sr25519_generate` / `sr25519_sign`).
//!
//! Run:
//!
//! ```text
//! cargo build --release --features runtime-benchmarks
//! ./target/release/hippius benchmark pallet --chain benchmark \
//!     --pallet pallet_hippocampus --extrinsic '*' --steps 50 --repeat 20
//! ```

#![cfg(feature = "runtime-benchmarks")]

use super::*;
use crate::settlement::{self as st, AlphaPriceSource, ComputeRevenueSource};

use frame_benchmarking::v2::*;
use frame_support::{
	traits::{Currency, EnsureOrigin},
	BoundedVec,
};
use frame_system::RawOrigin;
use sp_core::{crypto::KeyTypeId, sr25519, H256};
use sp_runtime::traits::{Get, SaturatedConversion, Zero};
use sp_std::vec::Vec;

const SEED: u32 = 0;
const KEY: KeyTypeId = KeyTypeId(*b"cmps");
const WINDOW: u32 = 100;

fn params() -> st::PayoutParams {
	st::PayoutParams { share_bps: 5000, tol_bps: 10, vcpu_hour_uusd: 24657 }
}

fn fund<T: Config>(who: &T::AccountId, amount: u128) {
	T::Currency::make_free_balance_be(who, amount.saturated_into());
}

/// Four submitters (the caller first) and four approver keys, the signing
/// key last; params v1; first window `WINDOW`, final with ample revenue;
/// switch on; fresh price of one credit per alpha (alpha == usd).
fn setup<T: Config>() -> (T::AccountId, sr25519::Public) {
	let caller: T::AccountId = whitelisted_caller();
	let submitters: Vec<T::AccountId> = (0..4u32)
		.map(|i| if i == 0 { caller.clone() } else { account("submitter", i, SEED) })
		.collect();
	let approvers: Vec<sr25519::Public> =
		(0..4).map(|_| sp_io::crypto::sr25519_generate(KEY, None)).collect();
	let signer = approvers[3];
	ComputeSettlementSubmitters::<T>::put(BoundedVec::try_from(submitters).unwrap());
	ComputeSettlementApproverKeys::<T>::put(BoundedVec::try_from(approvers).unwrap());
	ComputePayoutParams::<T>::put((1u32, params()));
	ComputeMaxDailyAlpha::<T>::put(u128::MAX);
	FirstComputeWindow::<T>::put(WINDOW);
	NextComputeWindow::<T>::put(WINDOW);
	ComputeSettlementEnabled::<T>::put(true);
	DistributionEnabled::<T>::put(true);
	T::ComputeRevenue::bench_set_final_charged(WINDOW, u128::MAX / 2);
	T::AlphaPrice::bench_set_price(st::ONE);
	let ed: u128 = T::Currency::minimum_balance().saturated_into();
	fund::<T>(&Pallet::<T>::settlement_account(), 1_000_000_000_000_000_000_000);
	fund::<T>(&Pallet::<T>::owed_account(), ed.saturating_mul(10));
	fund::<T>(&caller, 1_000_000_000_000_000_000_000);
	(caller, signer)
}

fn sign(signer: &sr25519::Public, digest: &H256) -> sr25519::Signature {
	sp_io::crypto::sr25519_sign(KEY, signer, digest.as_bytes()).expect("benchmark keystore")
}

/// `n` absent families in ascending order, each owed less than the ED.
fn failing_entries<T: Config>(n: u32) -> Vec<st::Entry<T::AccountId>> {
	let ed: u128 = T::Currency::minimum_balance().saturated_into();
	let usd = ed.saturating_sub(1).max(1);
	let mut families: Vec<T::AccountId> = (0..n).map(|i| account("family", i, SEED)).collect();
	families.sort();
	families
		.into_iter()
		.map(|family| st::Entry { family, usd, alpha: st::alpha_for(usd, st::ONE).unwrap() })
		.collect()
}

fn input_for<T: Config>(
	window: u32,
	revision: u8,
	parts: &[Vec<st::Entry<T::AccountId>>],
	hashes: usize,
) -> st::ApprovalInput {
	let genesis = Pallet::<T>::genesis_hash();
	let mut part_hashes: Vec<H256> = parts
		.iter()
		.enumerate()
		.map(|(i, e)| st::part_hash(&genesis, window, revision, i as u8, e))
		.collect();
	while part_hashes.len() < hashes {
		part_hashes.push(H256::repeat_byte(part_hashes.len() as u8));
	}
	let usd: u128 = parts.iter().flatten().map(|e| e.usd).sum::<u128>().max(1);
	st::ApprovalInput {
		parts: part_hashes.len() as u8,
		part_hashes: BoundedVec::try_from(part_hashes).unwrap(),
		usd,
		price: st::ONE,
		payable_root: H256::repeat_byte(0xaa),
		evidence_root: H256::repeat_byte(0xbb),
		upto_hour: u64::from(window) * 24 + 23,
		params_version: 1,
		formula_version: st::FORMULA_VERSION,
		params_hash: st::params_hash(&params()),
	}
}

fn submit<T: Config>(
	caller: &T::AccountId,
	signer: &sr25519::Public,
	input: st::ApprovalInput,
) -> H256 {
	let digest = st::approval_digest(&Pallet::<T>::genesis_hash(), WINDOW, 0, &input);
	let sig = sign(signer, &digest);
	Pallet::<T>::submit_compute_approval(
		RawOrigin::Signed(caller.clone()).into(),
		WINDOW,
		0,
		input,
		sig,
	)
	.expect("approval accepted");
	digest
}

#[benchmarks]
mod benchmarks {
	use super::*;

	#[benchmark]
	fn submit_compute_approval() {
		let (caller, signer) = setup::<T>();
		let input = input_for::<T>(WINDOW, 0, &[], st::MAX_PARTS as usize);
		let digest = st::approval_digest(&Pallet::<T>::genesis_hash(), WINDOW, 0, &input);
		let sig = sign(&signer, &digest);

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), WINDOW, 0, input, sig);

		assert!(ComputeApproval::<T>::contains_key((WINDOW, 0)));
	}

	#[benchmark]
	fn pay_compute_part(n: Linear<1, { T::MaxEntriesPerPart::get() }>) {
		let (caller, signer) = setup::<T>();
		let entries = failing_entries::<T>(n);
		let input = input_for::<T>(WINDOW, 0, &[entries.clone()], 1);
		submit::<T>(&caller, &signer, input);
		let bounded: BoundedVec<_, T::MaxEntriesPerPart> = BoundedVec::try_from(entries).unwrap();

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), WINDOW, 0, 0, bounded);

		assert_eq!(NextComputeWindow::<T>::get(), WINDOW + 1);
		assert!(!ComputeOwedTotal::<T>::get().is_zero());
	}

	#[benchmark]
	fn claim_compute_owed() {
		let (_, _) = setup::<T>();
		let family: T::AccountId = account("family", 0, SEED);
		let ed: u128 = T::Currency::minimum_balance().saturated_into();
		fund::<T>(&family, ed);
		let owed: BalanceOf<T> = ed.saturated_into();
		ComputeOwed::<T>::insert(&family, owed);
		ComputeOwedTotal::<T>::put(owed);
		fund::<T>(&Pallet::<T>::owed_account(), ed.saturating_mul(10));

		#[extrinsic_call]
		_(RawOrigin::Signed(family.clone()));

		assert!(ComputeOwed::<T>::get(&family).is_zero());
	}

	#[benchmark]
	fn fund_compute_settlement() {
		let (caller, _) = setup::<T>();
		let amount: BalanceOf<T> = 1_000_000u128.saturated_into();

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), amount);
	}

	#[benchmark]
	fn revoke_compute_approval() {
		let (caller, signer) = setup::<T>();
		let input = input_for::<T>(WINDOW, 0, &[], st::MAX_PARTS as usize);
		let digest = submit::<T>(&caller, &signer, input);
		let sig = sign(&signer, &st::revoke_digest(&Pallet::<T>::genesis_hash(), WINDOW, 0, &digest));

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), WINDOW, 0, sig);

		assert!(!ComputeApproval::<T>::contains_key((WINDOW, 0)));
	}

	#[benchmark]
	fn schedule_compute_root_change() -> Result<(), BenchmarkError> {
		let origin =
			T::SettlementRootOrigin::try_successful_origin().map_err(|_| BenchmarkError::Weightless)?;
		let submitters: Vec<T::AccountId> = (0..4u32).map(|i| account("s", i, SEED)).collect();
		let approvers: Vec<sr25519::Public> =
			(0..4).map(|_| sp_io::crypto::sr25519_generate(KEY, None)).collect();
		let change = st::RootChange::Keys {
			submitters: BoundedVec::try_from(submitters).unwrap(),
			approvers: BoundedVec::try_from(approvers).unwrap(),
		};

		#[extrinsic_call]
		_(origin as T::RuntimeOrigin, change);

		assert!(PendingRootChange::<T>::contains_key(st::ChangeKind::Keys));
		Ok(())
	}

	#[benchmark]
	fn apply_compute_root_change() {
		let (caller, _) = setup::<T>();
		let to: T::AccountId = account("revenue", 0, SEED);
		fund::<T>(&to, T::Currency::minimum_balance().saturated_into());
		ComputeRevenueAccount::<T>::put(&to);
		let now = frame_system::Pallet::<T>::block_number();
		PendingRootChange::<T>::insert(
			st::ChangeKind::Defund,
			(st::RootChange::<T::AccountId>::Defund(u128::MAX), now),
		);

		#[extrinsic_call]
		_(RawOrigin::Signed(caller), st::ChangeKind::Defund);

		assert!(!PendingRootChange::<T>::contains_key(st::ChangeKind::Defund));
	}

	#[benchmark]
	fn cancel_compute_root_change() -> Result<(), BenchmarkError> {
		let origin =
			T::SettlementRootOrigin::try_successful_origin().map_err(|_| BenchmarkError::Weightless)?;
		let now = frame_system::Pallet::<T>::block_number();
		PendingRootChange::<T>::insert(
			st::ChangeKind::Enable,
			(st::RootChange::<T::AccountId>::Enable, now),
		);

		#[extrinsic_call]
		_(origin as T::RuntimeOrigin, st::ChangeKind::Enable);

		assert!(!PendingRootChange::<T>::contains_key(st::ChangeKind::Enable));
		Ok(())
	}

	#[benchmark]
	fn disable_compute_settlement() -> Result<(), BenchmarkError> {
		let origin =
			T::SettlementRootOrigin::try_successful_origin().map_err(|_| BenchmarkError::Weightless)?;
		ComputeSettlementEnabled::<T>::put(true);
		let now = frame_system::Pallet::<T>::block_number();
		PendingRootChange::<T>::insert(
			st::ChangeKind::Enable,
			(st::RootChange::<T::AccountId>::Enable, now),
		);

		#[extrinsic_call]
		_(origin as T::RuntimeOrigin);

		assert!(!ComputeSettlementEnabled::<T>::get());
		Ok(())
	}

	impl_benchmark_test_suite!(Pallet, crate::mock::new_test_ext(), crate::mock::Test);
}


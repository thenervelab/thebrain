//! The `Plan::is_s3_plan` storage migration, driven against storage written in
//! the exact byte layout production is running today.
//!
//! `Plan` sits in the *middle* of `UserPlanSubscription`, so the trailing-length
//! probe that made `next_charge_unix_day` and `paid_per_month` additive cannot
//! absorb a new `Plan` field — everything after `package` would be misparsed.
//! These tests therefore write raw v1 bytes, run the upgrade, and read back
//! through the live types: if the re-encode were skipped or got the field order
//! wrong, the decode would fail or come back with garbage rather than quietly
//! passing.

use frame_support::{
	assert_ok,
	traits::{OnRuntimeUpgrade, StorageVersion},
	Blake2_128Concat, StorageHasher, Twox128,
};
use hippius_mainnet_runtime::{AccountId, Marketplace, Runtime, RuntimeOrigin, System};
use parity_scale_codec::Encode;
use sp_runtime::{traits::Hash, AccountId32, BuildStorage};

type Hashed = <Runtime as frame_system::Config>::Hash;

/// `Plan` as stored before `is_s3_plan` was introduced.
#[derive(Encode)]
struct PlanV1 {
	id: Hashed,
	plan_name: Vec<u8>,
	plan_description: Vec<u8>,
	plan_technical_description: Vec<u8>,
	is_suspended: bool,
	price: u128,
	is_storage_plan: bool,
	storage_limit: Option<u128>,
}

/// `UserPlanSubscription` as stored before `is_s3_plan`, current encoding.
#[derive(Encode)]
struct SubV1 {
	id: u32,
	owner: AccountId,
	package: PlanV1,
	cdn_location_id: Option<u32>,
	active: bool,
	last_charged_at: u64,
	selected_image_name: Option<Vec<u8>>,
	next_charge_unix_day: Option<u32>,
	paid_per_month: u128,
}

/// The older encoding still sitting in storage for rows the `paid_per_month`
/// backfill never rewrote: it simply ends after `selected_image_name`.
#[derive(Encode)]
struct LegacySubV1 {
	id: u32,
	owner: AccountId,
	package: PlanV1,
	cdn_location_id: Option<u32>,
	active: bool,
	last_charged_at: u64,
	selected_image_name: Option<Vec<u8>>,
}

fn account(seed: u8) -> AccountId {
	AccountId32::new([seed; 32])
}

fn plan_id(name: &[u8]) -> Hashed {
	<Runtime as frame_system::Config>::Hashing::hash_of(&name.to_vec())
}

fn plan_v1(name: &[u8], price: u128, is_storage_plan: bool) -> PlanV1 {
	PlanV1 {
		id: plan_id(name),
		plan_name: name.to_vec(),
		plan_description: b"{}".to_vec(),
		plan_technical_description: b"{}".to_vec(),
		is_suspended: false,
		price,
		is_storage_plan,
		storage_limit: Some(1_000_000),
	}
}

fn storage_key(item: &[u8], key: &[u8]) -> Vec<u8> {
	let mut full = Twox128::hash(b"Marketplace").to_vec();
	full.extend_from_slice(&Twox128::hash(item));
	full.extend_from_slice(&Blake2_128Concat::hash(key));
	full
}

fn put_plan(plan: PlanV1) {
	let key = storage_key(b"Plans", &plan.id.encode());
	frame_support::storage::unhashed::put_raw(&key, &plan.encode());
}

fn put_subs(who: &AccountId, encoded_subs: Vec<u8>) {
	let key = storage_key(b"UserAllSubscriptionPlans", &who.encode());
	frame_support::storage::unhashed::put_raw(&key, &encoded_subs);
}

/// Genesis with the pallet pinned to the pre-migration storage version, so the
/// upgrade actually runs rather than short-circuiting as already-migrated.
fn ext_at_v1() -> sp_io::TestExternalities {
	let t = frame_system::GenesisConfig::<Runtime>::default().build_storage().unwrap();
	let mut ext = sp_io::TestExternalities::new(t);
	ext.execute_with(|| {
		System::set_block_number(1);
		StorageVersion::new(1).put::<pallet_marketplace::Pallet<Runtime>>();
	});
	ext
}

fn run_upgrade() {
	pallet_marketplace::migrations::Migrate::<Runtime>::on_runtime_upgrade();
}

#[test]
fn existing_plans_survive_and_become_non_s3() {
	ext_at_v1().execute_with(|| {
		put_plan(plan_v1(b"solo", 1_000, true));
		put_plan(plan_v1(b"compute", 2_000, false));

		run_upgrade();

		let solo = Marketplace::plans(plan_id(b"solo")).expect("solo plan survives");
		assert_eq!(solo.plan_name, b"solo".to_vec(), "fields before the new one are intact");
		assert_eq!(solo.price, 1_000);
		assert!(solo.is_storage_plan);
		assert!(!solo.is_s3_plan, "no pre-existing plan is retroactively S3");
		assert!(solo.is_drive_plan());
		assert_eq!(solo.storage_limit, Some(1_000_000), "the field after the new one is intact");

		let compute = Marketplace::plans(plan_id(b"compute")).expect("compute plan survives");
		assert!(!compute.is_storage_plan);
		assert!(!compute.is_s3_plan);

		assert_eq!(StorageVersion::get::<pallet_marketplace::Pallet<Runtime>>(), 2);
	});
}

#[test]
fn subscriptions_survive_with_every_field_intact() {
	ext_at_v1().execute_with(|| {
		let user = account(11);
		let subs = vec![
			SubV1 {
				id: 7,
				owner: user.clone(),
				package: plan_v1(b"solo", 1_000, true),
				cdn_location_id: None,
				active: true,
				last_charged_at: 42,
				selected_image_name: None,
				next_charge_unix_day: Some(20_485),
				paid_per_month: 950,
			},
			SubV1 {
				id: 8,
				owner: user.clone(),
				package: plan_v1(b"compute", 2_000, false),
				cdn_location_id: Some(3),
				active: false,
				last_charged_at: 43,
				selected_image_name: Some(b"ubuntu".to_vec()),
				next_charge_unix_day: None,
				paid_per_month: 2_000,
			},
		];
		put_subs(&user, subs.encode());

		run_upgrade();

		let got = Marketplace::user_all_subscription_plans(&user);
		assert_eq!(got.len(), 2, "no row was dropped");

		// The fields that sit *after* `package` are the ones a bad re-encode
		// would shift, so they are what this pins.
		assert_eq!(got[0].id, 7);
		assert_eq!(got[0].cdn_location_id, None);
		assert!(got[0].active);
		assert_eq!(got[0].last_charged_at, 42);
		assert_eq!(got[0].selected_image_name, None);
		assert_eq!(got[0].next_charge_unix_day, Some(20_485));
		assert_eq!(got[0].paid_per_month, 950, "the referral-discounted price is preserved");
		assert!(got[0].package.is_drive_plan());

		assert_eq!(got[1].id, 8);
		assert_eq!(got[1].cdn_location_id, Some(3));
		assert!(!got[1].active);
		assert_eq!(got[1].last_charged_at, 43);
		assert_eq!(got[1].selected_image_name, Some(b"ubuntu".to_vec()));
		assert_eq!(got[1].next_charge_unix_day, None);
		assert_eq!(got[1].paid_per_month, 2_000);
		assert!(!got[1].package.is_storage_plan);
		assert!(!got[1].package.is_s3_plan);
	});
}

#[test]
fn rows_in_the_oldest_encoding_are_read_and_backfilled() {
	ext_at_v1().execute_with(|| {
		let user = account(12);
		// One row, in the encoding that predates both `next_charge_unix_day`
		// and `paid_per_month` — the trailing-length probe only resolves for
		// the last element of the vec, which a single row always is.
		let subs = vec![LegacySubV1 {
			id: 1,
			owner: user.clone(),
			package: plan_v1(b"solo", 1_000, true),
			cdn_location_id: None,
			active: true,
			last_charged_at: 5,
			selected_image_name: None,
		}];
		put_subs(&user, subs.encode());

		run_upgrade();

		let got = Marketplace::user_all_subscription_plans(&user);
		assert_eq!(got.len(), 1);
		assert_eq!(got[0].next_charge_unix_day, None, "legacy rows charge on the next 1st");
		assert_eq!(got[0].paid_per_month, 1_000, "backfilled from the plan price");
		assert!(!got[0].package.is_s3_plan);
	});
}

#[test]
fn the_migration_does_not_run_twice() {
	ext_at_v1().execute_with(|| {
		put_plan(plan_v1(b"solo", 1_000, true));
		run_upgrade();

		// Mark an existing plan as S3 the way `add_new_plan` now can, then run
		// the upgrade again: a second re-encode would reset the flag.
		assert_ok!(Marketplace::add_new_plan(
			RuntimeOrigin::root(),
			b"s3".to_vec(),
			b"{}".to_vec(),
			b"{}".to_vec(),
			1_000,
			false,
			true,
			None,
		));

		run_upgrade();

		assert!(Marketplace::plans(plan_id(b"s3")).expect("s3 plan exists").is_s3_plan);
		assert_eq!(StorageVersion::get::<pallet_marketplace::Pallet<Runtime>>(), 2);
	});
}

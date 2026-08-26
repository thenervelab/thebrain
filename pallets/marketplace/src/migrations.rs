use super::*;
use core::marker::PhantomData;
use frame_support::{
	pallet_prelude::*,
	traits::{OnRuntimeUpgrade, StorageVersion},
};
use sp_std::prelude::*;

/// Storage shapes as they exist on chain *before* `Plan::is_s3_plan` was added.
///
/// `Plan` is not the last field of `UserPlanSubscription`, so a new field on it
/// cannot be picked up by a trailing-length probe the way `next_charge_unix_day`
/// and `paid_per_month` were — everything after `package` would be misparsed.
/// Hence a real re-encode of both maps rather than a lenient `Decode`.
mod v1 {
	use super::*;
	use frame_system::pallet_prelude::BlockNumberFor;
	use pallet_utils::SubscriptionId;

	#[derive(Decode)]
	pub struct Plan<Hash> {
		pub id: Hash,
		pub plan_name: Vec<u8>,
		pub plan_description: Vec<u8>,
		pub plan_technical_description: Vec<u8>,
		pub is_suspended: bool,
		pub price: u128,
		pub is_storage_plan: bool,
		pub storage_limit: Option<u128>,
	}

	impl<Hash> Plan<Hash> {
		/// Every pre-existing plan is a Drive plan or a compute plan; the S3
		/// flavour is introduced by this migration, so nothing can already be it.
		pub fn upgrade(self) -> crate::Plan<Hash> {
			crate::Plan {
				id: self.id,
				plan_name: self.plan_name,
				plan_description: self.plan_description,
				plan_technical_description: self.plan_technical_description,
				is_suspended: self.is_suspended,
				price: self.price,
				is_storage_plan: self.is_storage_plan,
				is_s3_plan: false,
				storage_limit: self.storage_limit,
			}
		}
	}

	pub struct UserPlanSubscription<T: frame_system::Config> {
		pub id: SubscriptionId,
		pub owner: T::AccountId,
		pub package: Plan<T::Hash>,
		pub cdn_location_id: Option<u32>,
		pub active: bool,
		pub last_charged_at: BlockNumberFor<T>,
		pub selected_image_name: Option<Vec<u8>>,
		pub next_charge_unix_day: Option<u32>,
		pub paid_per_month: u128,
	}

	/// Byte-for-byte the decode the pallet shipped with, so rows still stored in
	/// the older encodings (no `next_charge_unix_day`, no `paid_per_month`) are
	/// read exactly as the running runtime reads them today.
	impl<T: frame_system::Config> Decode for UserPlanSubscription<T> {
		fn decode<I: codec::Input>(input: &mut I) -> Result<Self, codec::Error> {
			let id = SubscriptionId::decode(input)?;
			let owner = T::AccountId::decode(input)?;
			let package = Plan::<T::Hash>::decode(input)?;
			let cdn_location_id = Option::<u32>::decode(input)?;
			let active = bool::decode(input)?;
			let last_charged_at = BlockNumberFor::<T>::decode(input)?;
			let selected_image_name = Option::<Vec<u8>>::decode(input)?;

			let next_charge_unix_day = match input.remaining_len()? {
				Some(0) => None,
				_ => Option::<u32>::decode(input)?,
			};

			let paid_per_month = match input.remaining_len()? {
				Some(0) => package.price,
				_ => u128::decode(input)?,
			};

			Ok(Self {
				id,
				owner,
				package,
				cdn_location_id,
				active,
				last_charged_at,
				selected_image_name,
				next_charge_unix_day,
				paid_per_month,
			})
		}
	}

	impl<T: frame_system::Config> UserPlanSubscription<T> {
		pub fn upgrade(self) -> crate::UserPlanSubscription<T> {
			// Best-effort: the historical referral discount cannot be
			// reconstructed. Rows predating the field decode as `package.price`
			// above; this only guards a zero left by older/uninitialized state,
			// which would otherwise refund unused prepaid months at nothing.
			let paid_per_month =
				if self.paid_per_month == 0 { self.package.price } else { self.paid_per_month };

			crate::UserPlanSubscription {
				id: self.id,
				owner: self.owner,
				package: self.package.upgrade(),
				cdn_location_id: self.cdn_location_id,
				active: self.active,
				last_charged_at: self.last_charged_at,
				selected_image_name: self.selected_image_name,
				next_charge_unix_day: self.next_charge_unix_day,
				paid_per_month,
				_phantom: PhantomData,
			}
		}
	}
}

pub struct Migrate<T>(PhantomData<T>);

impl<T: Config> OnRuntimeUpgrade for Migrate<T> {
	fn on_runtime_upgrade() -> Weight {
		let current_version = StorageVersion::get::<Pallet<T>>();

		// v0 and v1 share one step: the v2 re-encode rewrites every row anyway,
		// and it folds in the `paid_per_month` backfill v0 owed. A chain still on
		// v0 must not run the old backfill first — it iterates with the *new*
		// types and would drop every row it cannot decode.
		if current_version < 2 {
			let weight = Self::migrate_to_v2();
			StorageVersion::new(2).put::<Pallet<T>>();
			weight
		} else {
			log::info!("Skipping marketplace migration, already migrated.");
			Weight::zero()
		}
	}

	#[cfg(feature = "try-runtime")]
	fn post_upgrade(_state: Vec<u8>) -> Result<(), sp_runtime::TryRuntimeError> {
		ensure!(
			StorageVersion::get::<Pallet<T>>() == StorageVersion::new(2),
			"marketplace storage version was not advanced to 2"
		);

		// Both maps must decode under the new shapes, and no plan may claim both
		// storage flavours — that would occupy two per-account slots at once.
		for (_id, plan) in Plans::<T>::iter() {
			ensure!(!(plan.is_storage_plan && plan.is_s3_plan), "plan claims both Drive and S3");
		}
		for (_who, subs) in UserAllSubscriptionPlans::<T>::iter() {
			for sub in subs {
				ensure!(
					!(sub.package.is_storage_plan && sub.package.is_s3_plan),
					"subscribed plan claims both Drive and S3"
				);
			}
		}

		Ok(())
	}
}

impl<T: Config> Migrate<T> {
	/// Re-encode `Plans` and `UserAllSubscriptionPlans` with `Plan::is_s3_plan`.
	///
	/// Every existing plan becomes a non-S3 plan, which preserves today's
	/// behaviour exactly: Drive plans keep exempting Drive bytes from hourly
	/// billing and no account is retroactively treated as holding S3 cover.
	fn migrate_to_v2() -> Weight {
		let mut plans: u64 = 0;
		Plans::<T>::translate::<v1::Plan<T::Hash>, _>(|_id, plan| {
			plans = plans.saturating_add(1);
			Some(plan.upgrade())
		});

		let mut accounts: u64 = 0;
		UserAllSubscriptionPlans::<T>::translate::<Vec<v1::UserPlanSubscription<T>>, _>(
			|_who, subs| {
				accounts = accounts.saturating_add(1);
				Some(subs.into_iter().map(|sub| sub.upgrade()).collect())
			},
		);

		log::info!(
			target: "runtime::marketplace",
			"marketplace v2: re-encoded {} plans and {} subscription lists",
			plans,
			accounts,
		);

		// `translate` reads and writes every entry of both maps.
		let touched = plans.saturating_add(accounts);
		T::DbWeight::get().reads_writes(touched.saturating_add(1), touched)
	}
}

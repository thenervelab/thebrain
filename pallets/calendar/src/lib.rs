#![cfg_attr(not(feature = "std"), no_std)]

pub use pallet::*;

mod calendar;

#[cfg(test)]
mod mock;
#[cfg(test)]
mod tests;

/// Trait other pallets (e.g. a subscription billing pallet) should depend on,
/// rather than the concrete `Pallet<T>`.
pub trait MonthCalendar {
	fn days_in_current_month() -> u8;
	fn days_remaining_in_current_month() -> u8;
	/// Unix day (UTC) of the 1st of the calendar month `n` months from now.
	fn unix_day_of_first_of_month_in(n: u32) -> u32;
}

#[frame_support::pallet]
pub mod pallet {
	use super::*;
	use sp_runtime::SaturatedConversion;

	#[pallet::pallet]
	#[pallet::without_storage_info]
	pub struct Pallet<T>(_);

	#[pallet::config]
	pub trait Config: frame_system::Config + pallet_timestamp::Config {}

	impl<T: Config> Pallet<T> {
		/// UNIX millis of the current block, as `u64`.
		fn now_ms() -> u64 {
			pallet_timestamp::Pallet::<T>::get().saturated_into::<u64>()
		}

		pub fn days_in_current_month() -> u8 {
			calendar::days_in_month(Self::now_ms())
		}

		pub fn days_remaining_in_current_month() -> u8 {
			calendar::days_remaining_in_month(Self::now_ms())
		}

		pub fn unix_day_of_first_of_month_in(n: u32) -> u32 {
			calendar::unix_day_of_first_of_month_in(Self::now_ms(), n)
		}
	}

	impl<T: Config> MonthCalendar for Pallet<T> {
		fn days_in_current_month() -> u8 {
			Self::days_in_current_month()
		}
		fn days_remaining_in_current_month() -> u8 {
			Self::days_remaining_in_current_month()
		}
		fn unix_day_of_first_of_month_in(n: u32) -> u32 {
			Self::unix_day_of_first_of_month_in(n)
		}
	}
}

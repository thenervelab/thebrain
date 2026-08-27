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
	/// Unix day (UTC) of the 1st of the calendar month immediately after `unix_day`.
	fn unix_day_of_first_of_month_after(unix_day: u32) -> u32;
	/// Day of the month (1..=31) of `unix_day`.
	fn day_of_month(unix_day: u32) -> u8;
	/// Number of days in the calendar month containing `unix_day`.
	fn days_in_month_of_day(unix_day: u32) -> u8;
	/// The billing anniversary `n` calendar months from `unix_day`, on `anchor`
	/// day-of-month, clamped to the target month's last day.
	fn add_months_clamped(unix_day: u32, anchor: u8, n: i32) -> u32;
	/// Today as a unix day (UTC).
	fn current_unix_day() -> u32;
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

		pub fn unix_day_of_first_of_month_after(unix_day: u32) -> u32 {
			calendar::unix_day_of_first_of_month_after(unix_day)
		}

		pub fn day_of_month(unix_day: u32) -> u8 {
			calendar::day_of_month(unix_day)
		}

		pub fn days_in_month_of_day(unix_day: u32) -> u8 {
			calendar::days_in_month_of_day(unix_day)
		}

		pub fn add_months_clamped(unix_day: u32, anchor: u8, n: i32) -> u32 {
			calendar::add_months_clamped(unix_day, anchor, n)
		}

		/// Today as a unix day (UTC), from the chain timestamp.
		///
		/// The billing pallet derived this itself; sharing the one definition
		/// keeps "what day is it" and "what day does this land on" from drifting
		/// apart by a rounding rule.
		pub fn current_unix_day() -> u32 {
			(Self::now_ms() / 86_400_000u64) as u32
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
		fn unix_day_of_first_of_month_after(unix_day: u32) -> u32 {
			Self::unix_day_of_first_of_month_after(unix_day)
		}
		fn day_of_month(unix_day: u32) -> u8 {
			Self::day_of_month(unix_day)
		}
		fn days_in_month_of_day(unix_day: u32) -> u8 {
			Self::days_in_month_of_day(unix_day)
		}
		fn add_months_clamped(unix_day: u32, anchor: u8, n: i32) -> u32 {
			Self::add_months_clamped(unix_day, anchor, n)
		}
		fn current_unix_day() -> u32 {
			Self::current_unix_day()
		}
	}
}

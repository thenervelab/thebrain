//! Pure date math. No FRAME, no runtime — just `u64 ms` in, `u8` out.
//! Kept separate so it can be exhaustively unit-tested without a mock runtime.

use time::{Month, OffsetDateTime};

/// Number of days in the calendar month containing `unix_ms`.
/// Returns `0` if the timestamp is outside `OffsetDateTime`'s representable range
/// (cannot happen for any plausible block timestamp).
pub fn days_in_month(unix_ms: u64) -> u8 {
	let Some(date) = date_from_unix_ms(unix_ms) else { return 0 };
	month_length(date.year(), date.month())
}

/// Days remaining in the current month, **inclusive** of today.
/// On the 1st of a 30-day month → 30. On the last day → 1.
pub fn days_remaining_in_month(unix_ms: u64) -> u8 {
	let Some(date) = date_from_unix_ms(unix_ms) else { return 0 };
	month_length(date.year(), date.month())
		.saturating_sub(date.day())
		.saturating_add(1)
}

fn date_from_unix_ms(unix_ms: u64) -> Option<time::Date> {
	let secs = (unix_ms / 1_000) as i64;
	OffsetDateTime::from_unix_timestamp(secs).ok().map(|dt| dt.date())
}

/// Unix day (UTC) of the 1st of the calendar month that is `n` months after the
/// month containing `unix_ms`.
///
/// - `n = 1` means "the first day of next month".
/// - `n = 12` means "the first day of the month twelve months from now".
///
/// Returns `0` if the timestamp is outside `OffsetDateTime`'s representable range.
pub fn unix_day_of_first_of_month_in(unix_ms: u64, n: u32) -> u32 {
	let Some(date) = date_from_unix_ms(unix_ms) else { return 0 };

	let mut year = date.year();
	let mut month = (date.month() as u8 as u32).saturating_add(n); // 1..=12 + n
	year += ((month.saturating_sub(1)) / 12) as i32;
	month = ((month.saturating_sub(1)) % 12) + 1;

	let Ok(first) = time::Date::from_calendar_date(
		year,
		time::Month::try_from(month as u8).unwrap_or(time::Month::January),
		1,
	) else {
		return 0;
	};

	let Ok(dt) = first.with_hms(0, 0, 0).map(|t| t.assume_utc()) else { return 0 };
	(dt.unix_timestamp() as u64 / 86_400u64) as u32
}

fn month_length(year: i32, month: Month) -> u8 {
	use Month::*;
	match month {
		January | March | May | July | August | October | December => 31,
		April | June | September | November => 30,
		February => if is_leap_year(year) { 29 } else { 28 },
	}
}

fn is_leap_year(year: i32) -> bool {
	(year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

#[cfg(test)]
mod tests {
	use super::*;

	// Helper: build a UTC unix-millis timestamp from Y/M/D.
	fn ts(year: i32, month: u8, day: u8) -> u64 {
		let date = time::Date::from_calendar_date(year, time::Month::try_from(month).unwrap(), day)
			.unwrap();
		let dt = date.with_hms(0, 0, 0).unwrap().assume_utc();
		(dt.unix_timestamp() as u64) * 1_000
	}

	#[test]
	fn feb_non_leap() {
		assert_eq!(days_in_month(ts(2026, 2, 15)), 28);
		assert_eq!(days_remaining_in_month(ts(2026, 2, 15)), 14);
	}

	#[test]
	fn feb_leap() {
		assert_eq!(days_in_month(ts(2024, 2, 15)), 29);
		assert_eq!(days_remaining_in_month(ts(2024, 2, 15)), 15);
		// Centennial non-leap.
		assert_eq!(days_in_month(ts(2100, 2, 1)), 28);
		// Quadricentennial leap.
		assert_eq!(days_in_month(ts(2000, 2, 1)), 29);
	}

	#[test]
	fn first_of_month() {
		assert_eq!(days_remaining_in_month(ts(2026, 4, 1)), 30); // April has 30
		assert_eq!(days_remaining_in_month(ts(2026, 1, 1)), 31);
	}

	#[test]
	fn last_of_month() {
		assert_eq!(days_remaining_in_month(ts(2026, 1, 31)), 1);
		assert_eq!(days_remaining_in_month(ts(2026, 12, 31)), 1);
		assert_eq!(days_remaining_in_month(ts(2024, 2, 29)), 1);
	}

	#[test]
	fn month_lengths() {
		assert_eq!(days_in_month(ts(2026, 1, 10)), 31);
		assert_eq!(days_in_month(ts(2026, 4, 10)), 30);
		assert_eq!(days_in_month(ts(2026, 7, 10)), 31);
		assert_eq!(days_in_month(ts(2026, 11, 10)), 30);
	}
}

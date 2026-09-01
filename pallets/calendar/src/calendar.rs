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

fn date_from_unix_day(unix_day: u32) -> Option<time::Date> {
	date_from_unix_ms((unix_day as u64).saturating_mul(86_400_000u64))
}

fn unix_day_of(date: time::Date) -> u32 {
	let Ok(dt) = date.with_hms(0, 0, 0).map(|t| t.assume_utc()) else { return 0 };
	let secs = dt.unix_timestamp();
	if secs < 0 {
		return 0;
	}
	(secs as u64 / 86_400u64) as u32
}

/// Day of the month (1..=31) of `unix_day`.
/// Returns `0` if `unix_day` is outside the representable range.
pub fn day_of_month(unix_day: u32) -> u8 {
	date_from_unix_day(unix_day).map_or(0, |date| date.day())
}

/// Number of days in the calendar month containing `unix_day`.
/// Returns `0` if `unix_day` is outside the representable range.
pub fn days_in_month_of_day(unix_day: u32) -> u8 {
	date_from_unix_day(unix_day).map_or(0, |date| month_length(date.year(), date.month()))
}

/// The billing anniversary `n` calendar months from `unix_day`, landing on
/// `anchor` day-of-month, clamped to the target month's last day.
///
/// This is the whole cycle rule. Adding a calendar month and clamping the day
/// is correct by construction for every anchor, where `unix_day + days_in_month`
/// is not: from Jan 30 the latter lands on Mar 2 and never recovers, while this
/// gives Feb 28 and then — because the anchor is remembered rather than
/// re-derived from the clamped date — Mar 30.
///
/// A 31st subscriber therefore returns to the 31st in every month that has one,
/// rather than sticking on the 28th after a single February.
///
/// `n` is signed so the refund and carry-credit paths can walk *backwards*
/// through the same anniversaries the charge path walks forwards; the two must
/// agree or money is over- or under-credited. Returns `0` if the result is
/// outside the representable range.
pub fn add_months_clamped(unix_day: u32, anchor: u8, n: i32) -> u32 {
	let Some(date) = date_from_unix_day(unix_day) else { return 0 };

	// Months since year 0, shifted by `n`, then split back out. Going through a
	// single absolute month number keeps the year rollover correct in both
	// directions without a sign-dependent branch.
	let absolute = (date.year() as i64)
		.saturating_mul(12)
		.saturating_add(date.month() as u8 as i64)
		.saturating_sub(1)
		.saturating_add(n as i64);
	if absolute < 0 {
		return 0;
	}

	let year = (absolute / 12) as i32;
	let month_num = ((absolute % 12) + 1) as u8;
	let Ok(month) = Month::try_from(month_num) else { return 0 };

	// The clamp: an anchor that does not exist in the target month becomes that
	// month's last day. Only 28, 29 and 30 can ever be produced this way, which
	// is what makes a derived day of 31 unambiguous.
	let day = anchor.min(month_length(year, month)).max(1);

	let Ok(target) = time::Date::from_calendar_date(year, month, day) else { return 0 };
	unix_day_of(target)
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

/// Unix day (UTC) of the 1st of the calendar month immediately after `unix_day`.
///
/// `unix_day` is a day-count since Unix epoch (UTC). If `unix_day` is already the
/// 1st of a month, this returns the 1st of the next month.
pub fn unix_day_of_first_of_month_after(unix_day: u32) -> u32 {
	let unix_ms = (unix_day as u64).saturating_mul(86_400u64).saturating_mul(1_000u64);
	unix_day_of_first_of_month_in(unix_ms, 1)
}

fn month_length(year: i32, month: Month) -> u8 {
	use Month::*;
	match month {
		January | March | May | July | August | October | December => 31,
		April | June | September | November => 30,
		February => {
			if is_leap_year(year) {
				29
			} else {
				28
			}
		},
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

	// ---- anniversary math ----

	/// Unix day of Y/M/D.
	fn day(year: i32, month: u8, d: u8) -> u32 {
		(ts(year, month, d) / 86_400_000) as u32
	}

	#[test]
	fn day_of_month_and_month_length_from_day() {
		assert_eq!(day_of_month(day(2026, 1, 31)), 31);
		assert_eq!(day_of_month(day(2026, 2, 1)), 1);
		assert_eq!(days_in_month_of_day(day(2026, 2, 10)), 28);
		assert_eq!(days_in_month_of_day(day(2024, 2, 10)), 29);
		assert_eq!(days_in_month_of_day(day(2026, 4, 10)), 30);
	}

	/// Anchors 1..=28 exist in every month, so the anniversary is the plain
	/// same-day-next-month with no clamping anywhere in the year.
	#[test]
	fn anchors_up_to_28_never_clamp() {
		for anchor in 1u8..=28 {
			let mut d = day(2026, 1, anchor);
			for month in 2u8..=12 {
				d = add_months_clamped(d, anchor, 1);
				assert_eq!(d, day(2026, month, anchor), "anchor {anchor} month {month}");
				assert_eq!(day_of_month(d), anchor);
			}
		}
	}

	/// The case the whole clamp exists for: a 31st subscriber must come *back*
	/// to the 31st, not stick on whatever short month clipped them.
	#[test]
	fn anchor_31_returns_after_february() {
		let mut d = day(2026, 1, 31);
		let expected = [
			(2, 28), // clamped — Feb 2026 is not a leap year
			(3, 31), // back to the anchor
			(4, 30), // clamped
			(5, 31),
			(6, 30),
			(7, 31),
			(8, 31),
			(9, 30),
			(10, 31),
			(11, 30),
			(12, 31),
		];
		for (month, expected_day) in expected {
			d = add_months_clamped(d, 31, 1);
			assert_eq!(d, day(2026, month, expected_day), "month {month}");
		}
	}

	#[test]
	fn anchors_29_and_30_clamp_only_in_february() {
		// Feb 2026 has 28 days: both 29 and 30 clamp to the 28th, then recover.
		assert_eq!(add_months_clamped(day(2026, 1, 29), 29, 1), day(2026, 2, 28));
		assert_eq!(add_months_clamped(day(2026, 2, 28), 29, 1), day(2026, 3, 29));
		assert_eq!(add_months_clamped(day(2026, 1, 30), 30, 1), day(2026, 2, 28));
		assert_eq!(add_months_clamped(day(2026, 2, 28), 30, 1), day(2026, 3, 30));
		// April has 30, so anchor 30 fits and anchor 31 clamps.
		assert_eq!(add_months_clamped(day(2026, 3, 30), 30, 1), day(2026, 4, 30));
		assert_eq!(add_months_clamped(day(2026, 3, 31), 31, 1), day(2026, 4, 30));
	}

	#[test]
	fn leap_and_century_february() {
		// 2024 is a leap year: anchor 29 fits exactly, 30 and 31 clamp to 29.
		assert_eq!(add_months_clamped(day(2024, 1, 29), 29, 1), day(2024, 2, 29));
		assert_eq!(add_months_clamped(day(2024, 1, 30), 30, 1), day(2024, 2, 29));
		assert_eq!(add_months_clamped(day(2024, 1, 31), 31, 1), day(2024, 2, 29));
		// 2100 is a century non-leap year.
		assert_eq!(add_months_clamped(day(2100, 1, 31), 31, 1), day(2100, 2, 28));
		// 2000 is a quadricentennial leap year.
		assert_eq!(add_months_clamped(day(2000, 1, 31), 31, 1), day(2000, 2, 29));
	}

	#[test]
	fn year_rollover_both_directions() {
		assert_eq!(add_months_clamped(day(2026, 12, 15), 15, 1), day(2027, 1, 15));
		assert_eq!(add_months_clamped(day(2027, 1, 15), 15, -1), day(2026, 12, 15));
		assert_eq!(add_months_clamped(day(2026, 6, 15), 15, 12), day(2027, 6, 15));
		assert_eq!(add_months_clamped(day(2026, 6, 15), 15, -12), day(2025, 6, 15));
	}

	/// Stepping forward then back must land where it started whenever the
	/// anchor exists in both months — this is what lets the refund path walk
	/// the same anniversaries the charge path walks.
	#[test]
	fn forward_and_back_round_trips() {
		for anchor in 1u8..=28 {
			for month in 1u8..=12 {
				let start = day(2026, month, anchor);
				let forward = add_months_clamped(start, anchor, 1);
				assert_eq!(add_months_clamped(forward, anchor, -1), start);
			}
		}
	}

	/// Anchor 1 is what every pre-existing subscription derives, so this is the
	/// legacy-parity guarantee: the new math must reproduce
	/// `unix_day_of_first_of_month_after` exactly, for a full year.
	#[test]
	fn anchor_1_matches_first_of_month_after() {
		let mut d = day(2026, 1, 1);
		for _ in 0..24 {
			assert_eq!(add_months_clamped(d, 1, 1), unix_day_of_first_of_month_after(d));
			d = unix_day_of_first_of_month_after(d);
		}
	}

	/// A cycle is never shorter than 28 days, which is what keeps a re-filed
	/// index entry ahead of the day cursor.
	#[test]
	fn cycle_is_never_shorter_than_28_days() {
		for anchor in 1u8..=31 {
			let mut d = day(2024, 1, anchor.min(31));
			for _ in 0..36 {
				let next = add_months_clamped(d, anchor, 1);
				assert!(next > d, "anchor {anchor}: not monotonic");
				assert!(next - d >= 28, "anchor {anchor}: cycle of {} days", next - d);
				d = next;
			}
		}
	}
}

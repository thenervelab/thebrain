use crate::{mock::*, MonthCalendar, Pallet};

#[test]
fn reads_from_chain_timestamp() {
	new_test_ext().execute_with(|| {
		set_date(2024, 2, 15);
		assert_eq!(Pallet::<Test>::days_in_current_month(), 29);
		assert_eq!(Pallet::<Test>::days_remaining_in_current_month(), 15);

		set_date(2026, 4, 1);
		assert_eq!(Pallet::<Test>::days_in_current_month(), 30);
		assert_eq!(Pallet::<Test>::days_remaining_in_current_month(), 30);

		set_date(2026, 12, 31);
		assert_eq!(Pallet::<Test>::days_in_current_month(), 31);
		assert_eq!(Pallet::<Test>::days_remaining_in_current_month(), 1);
	});
}

#[test]
fn trait_impl_matches_inherent() {
	new_test_ext().execute_with(|| {
		set_date(2026, 7, 10);
		assert_eq!(
			<Pallet<Test> as MonthCalendar>::days_in_current_month(),
			Pallet::<Test>::days_in_current_month(),
		);
		assert_eq!(
			<Pallet<Test> as MonthCalendar>::days_remaining_in_current_month(),
			Pallet::<Test>::days_remaining_in_current_month(),
		);

		let today = Pallet::<Test>::current_unix_day();
		assert_eq!(<Pallet<Test> as MonthCalendar>::current_unix_day(), today);
		assert_eq!(
			<Pallet<Test> as MonthCalendar>::day_of_month(today),
			Pallet::<Test>::day_of_month(today),
		);
		assert_eq!(
			<Pallet<Test> as MonthCalendar>::days_in_month_of_day(today),
			Pallet::<Test>::days_in_month_of_day(today),
		);
		assert_eq!(
			<Pallet<Test> as MonthCalendar>::add_months_clamped(today, 10, 1),
			Pallet::<Test>::add_months_clamped(today, 10, 1),
		);
	});
}

/// `current_unix_day` must agree with the timestamp the rest of the pallet
/// reads — the billing pallet used to derive this itself, and the two
/// definitions drifting apart would shift every due date by a day.
#[test]
fn current_unix_day_tracks_the_chain_clock() {
	new_test_ext().execute_with(|| {
		set_date(2026, 7, 10);
		let today = Pallet::<Test>::current_unix_day();
		assert_eq!(Pallet::<Test>::day_of_month(today), 10);
		assert_eq!(Pallet::<Test>::days_in_month_of_day(today), 31);

		set_date(2026, 2, 28);
		let feb = Pallet::<Test>::current_unix_day();
		assert_eq!(Pallet::<Test>::day_of_month(feb), 28);
		assert_eq!(Pallet::<Test>::days_in_month_of_day(feb), 28);
	});
}

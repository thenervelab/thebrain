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
	});
}

use crate as pallet_calendar;
use frame_support::{derive_impl, traits::ConstU64};
use sp_runtime::BuildStorage;

type Block = frame_system::mocking::MockBlock<Test>;

frame_support::construct_runtime!(
	pub enum Test {
		System: frame_system,
		Timestamp: pallet_timestamp,
		Calendar: pallet_calendar,
	}
);

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Block = Block;
}

impl pallet_timestamp::Config for Test {
	type Moment = u64;
	type OnTimestampSet = ();
	type MinimumPeriod = ConstU64<1>;
	type WeightInfo = ();
}

impl pallet_calendar::Config for Test {}

pub fn new_test_ext() -> sp_io::TestExternalities {
	frame_system::GenesisConfig::<Test>::default().build_storage().unwrap().into()
}

/// Set the chain timestamp to the start of the given UTC date (00:00:00).
pub fn set_date(year: i32, month: u8, day: u8) {
	let date =
		time::Date::from_calendar_date(year, time::Month::try_from(month).unwrap(), day).unwrap();
	let dt = date.with_hms(0, 0, 0).unwrap().assume_utc();
	let ms = (dt.unix_timestamp() as u64) * 1_000;
	pallet_timestamp::Pallet::<Test>::set_timestamp(ms);
}

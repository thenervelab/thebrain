//! Test runtime for `pallet-compute-scoring`.
//!
//! Deliberately light: the merit source is decoupled behind
//! [`super::MeritSource`], so the mock wires a `thread_local` fake instead
//! of pulling in pallet-arion + its registration/balances dependencies.

use crate as pallet_compute_scoring;
use core::cell::RefCell;
use frame_support::{derive_impl, traits::EnsureOrigin};
use sp_runtime::{traits::IdentityLookup, BuildStorage};

type Block = frame_system::mocking::MockBlock<Test>;

frame_support::construct_runtime!(
	pub enum Test {
		System: frame_system,
		ComputeScoring: pallet_compute_scoring,
	}
);

#[derive_impl(frame_system::config_preludes::TestDefaultConfig)]
impl frame_system::Config for Test {
	type Block = Block;
	type AccountId = u64;
	type Lookup = IdentityLookup<Self::AccountId>;
}

thread_local! {
	static MINERS: RefCell<sp_std::vec::Vec<([u8; 32], u64, u128)>> = const { RefCell::new(sp_std::vec::Vec::new()) };
}

/// Test merit source — `set_miners` controls what `close_epoch` snapshots.
pub struct MockMerit;
impl pallet_compute_scoring::MeritSource<u64> for MockMerit {
	fn registered_miners() -> sp_std::vec::Vec<([u8; 32], u64, u128)> {
		MINERS.with(|m| m.borrow().clone())
	}
}

/// Set the fleet the next `close_epoch` will snapshot.
pub fn set_miners(v: sp_std::vec::Vec<([u8; 32], u64, u128)>) {
	MINERS.with(|m| *m.borrow_mut() = v);
}

/// An origin that always passes the authority check (the mock's "council").
pub struct AlwaysAuthority;
impl EnsureOrigin<RuntimeOrigin> for AlwaysAuthority {
	type Success = ();
	fn try_origin(o: RuntimeOrigin) -> Result<Self::Success, RuntimeOrigin> {
		// Accept Root; reject everything else (so we can test the gate).
		match o.clone().into() {
			Ok(frame_system::RawOrigin::Root) => Ok(()),
			_ => Err(o),
		}
	}
	#[cfg(feature = "runtime-benchmarks")]
	fn try_successful_origin() -> Result<RuntimeOrigin, ()> {
		Ok(RuntimeOrigin::root())
	}
}

frame_support::parameter_types! {
	pub const MaxMinersPerEpochClose: u32 = 1_000;
}

impl pallet_compute_scoring::Config for Test {
	type RuntimeEvent = RuntimeEvent;
	type AuthorityOrigin = AlwaysAuthority;
	type Merit = MockMerit;
	type MaxMinersPerEpochClose = MaxMinersPerEpochClose;
	type WeightInfo = ();
}

pub fn new_test_ext() -> sp_io::TestExternalities {
	set_miners(sp_std::vec::Vec::new());
	let t = frame_system::GenesisConfig::<Test>::default().build_storage().unwrap();
	let mut ext: sp_io::TestExternalities = t.into();
	ext.execute_with(|| System::set_block_number(1));
	ext
}

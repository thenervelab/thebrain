#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]

use frame_support::{traits::Get, weights::{Weight, constants::RocksDbWeight}};
use sp_std::marker::PhantomData;

/// Weight functions needed for pallet_execution_unit.
pub trait WeightInfo {
    fn register_node() -> Weight;
    fn trigger_benchmark() -> Weight;
    fn store_benchmark_result() -> Weight;
}

/// Default weights for pallet_execution_unit
pub struct SubstrateWeight<T>(PhantomData<T>);
impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
    fn register_node() -> Weight {
        Weight::from_parts(10_000, 0).saturating_add(T::DbWeight::get().writes(1))
    }

    fn trigger_benchmark() -> Weight {
        Weight::from_parts(15_000, 0)
            .saturating_add(T::DbWeight::get().reads(2))
            .saturating_add(T::DbWeight::get().writes(1))
    }

    fn store_benchmark_result() -> Weight {
        Weight::from_parts(10_000, 0)
            .saturating_add(T::DbWeight::get().reads(1))
            .saturating_add(T::DbWeight::get().writes(1))
    }
}

impl WeightInfo for () {
    fn register_node() -> Weight {
        Weight::from_parts(10_000, 0)
    }

    fn trigger_benchmark() -> Weight {
        Weight::from_parts(15_000, 0)
    }

    fn store_benchmark_result() -> Weight {
        Weight::from_parts(10_000, 0)
    }
}
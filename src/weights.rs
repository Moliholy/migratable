#![cfg_attr(rustfmt, rustfmt_skip)]
#![allow(unused_parens)]
#![allow(unused_imports)]
#![allow(missing_docs)]

use frame_support::{traits::Get, weights::{Weight, constants::RocksDbWeight}};
use core::marker::PhantomData;

pub trait WeightInfo {
	fn migration_noop() -> Weight;
	fn migrate() -> Weight;
	fn on_runtime_upgrade_noop() -> Weight;
	fn on_runtime_upgrade_in_progress() -> Weight;
	fn on_runtime_upgrade() -> Weight;
}

pub struct SubstrateWeight<T>(PhantomData<T>);
impl<T: frame_system::Config> WeightInfo for SubstrateWeight<T> {
	fn migration_noop() -> Weight {
		Weight::from_parts(3_489_000, 1627)
			.saturating_add(T::DbWeight::get().reads(1_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
	fn migrate() -> Weight {
		Weight::from_parts(13_100_000, 3631)
			.saturating_add(T::DbWeight::get().reads(2_u64))
			.saturating_add(T::DbWeight::get().writes(2_u64))
	}
	fn on_runtime_upgrade_noop() -> Weight {
		Weight::from_parts(4_994_000, 3607)
			.saturating_add(T::DbWeight::get().reads(1_u64))
	}
	fn on_runtime_upgrade_in_progress() -> Weight {
		Weight::from_parts(6_945_000, 3632)
			.saturating_add(T::DbWeight::get().reads(2_u64))
	}
	fn on_runtime_upgrade() -> Weight {
		Weight::from_parts(7_372_000, 3607)
			.saturating_add(T::DbWeight::get().reads(2_u64))
			.saturating_add(T::DbWeight::get().writes(1_u64))
	}
}

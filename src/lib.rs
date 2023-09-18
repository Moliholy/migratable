#![cfg_attr(not(feature = "std"), no_std)]

//! Multi-block Migration framework.
//!
//! This module allows us to define a migratable as a sequence of [`MigrationStep`]s that can be
//! executed across multiple blocks.
//!
//! # Usage
//!
//! Each pallet must define a set of [`MigrationStep`] where the version number is specified.
//! For example, `vX.rs` defines a migratable from version `X - 1` to version `X`.
//!
//! ## Example:
//!
//! To configure a migratable to `v3` for a runtime using `v2` of a given pallet on the chain,
//! you would set the `Migrations` type as follows:
//!
//! ```
//! use my_pallet::migrations::{v2, v3};
//! # pub enum Runtime {};
//! type Migrations = (v2::Migration<Runtime>, v3::Migration<Runtime>);
//! ```
//!
//! ## Notes:
//!
//! - Migrations should always be tested with `try-runtime` before being deployed.
//! - By testing with `try-runtime` against a live network, you ensure that all migratable steps work
//!   and that you have included the required steps.
//!
//! ## Low Level / Implementation Details
//!
//! When a migratable starts and [`OnRuntimeUpgrade::on_runtime_upgrade`] is called, instead of
//! performing the actual migratable, we set a custom storage item [`MigrationInProgress`].
//! This storage item defines a [`Cursor`] for the current migratable.
//!
//! If the [`MigrationInProgress`] storage item exists, it means a migratable is in progress, and its
//! value holds a cursor for the current migratable step. These migratable steps are executed during
//! [`Hooks<BlockNumber>::on_idle`] or when the [`Pallet::migrate`] dispatchable is
//! called.
//!
//! While the migratable is in progress, all dispatchables except `migrate`, are blocked, and returns
//! a `MigrationInProgress` error.

pub use migratable_procedural::{config, hooks, pallet};
pub mod weights;

extern crate alloc;

use frame_support::{
    pallet_prelude::{BoundedVec, Encode, MaxEncodedLen, StorageVersion, Weight},
    traits::ConstU32,
};
pub use log;
use parity_scale_codec::{Codec, Decode};
use sp_runtime::Saturating;
#[cfg(feature = "try-runtime")]
use sp_runtime::TryRuntimeError;
#[cfg(feature = "try-runtime")]
use sp_std::prelude::*;

const PROOF_ENCODE: &str = "Tuple::max_encoded_len() < Cursor::max_encoded_len()` is verified in `Self::integrity_test()`; qed";
const PROOF_DECODE: &str =
    "We encode to the same type in this trait only. No other code touches this item; qed";

fn invalid_version(version: StorageVersion) -> ! {
    panic!("Required migratable {version:?} not supported by this runtime. This is a bug.");
}

/// The cursor used to encode the position (usually the last iterated key) of the current migratable
/// step.
pub type Cursor = BoundedVec<u8, ConstU32<1024>>;

/// IsFinished describes whether a migratable is finished or not.
pub enum IsFinished {
    Yes,
    No,
}

/// A trait that allows to migrate storage from one version to another.
///
/// The migratable is done in steps. The migratable is finished when
/// `step()` returns `IsFinished::Yes`.
pub trait MigrationStep: Codec + MaxEncodedLen + Default {
    /// Returns the version of the migratable.
    const VERSION: u16;

    /// Returns the maximum weight that can be consumed in a single step.
    fn max_step_weight() -> Weight;

    /// Process one step of the migratable.
    ///
    /// Returns whether the migratable is finished and the weight consumed.
    fn step(&mut self) -> (IsFinished, Weight);

    /// Verify that the migratable step fits into `Cursor`, and that `max_step_weight` is not greater
    /// than `max_block_weight`.
    fn integrity_test(max_block_weight: Weight) {
        if Self::max_step_weight().any_gt(max_block_weight) {
            panic!(
                "Invalid max_step_weight for Migration {}. Value should be lower than {}",
                Self::VERSION,
                max_block_weight
            );
        }

        let len = <Self as MaxEncodedLen>::max_encoded_len();
        let max = Cursor::bound();
        if len > max {
            panic!(
                "Migration {} has size {} which is bigger than the maximum of {}",
                Self::VERSION,
                len,
                max,
            );
        }
    }

    /// Execute some pre-checks prior to running the first step of this migratable.
    #[cfg(feature = "try-runtime")]
    fn pre_upgrade_step() -> Result<Vec<u8>, TryRuntimeError> {
        Ok(Vec::new())
    }

    /// Execute some post-checks after running the last step of this migratable.
    #[cfg(feature = "try-runtime")]
    fn post_upgrade_step(_state: Vec<u8>) -> Result<(), TryRuntimeError> {
        Ok(())
    }
}

/// A noop migratable that can be used when there is no migratable to be done for a given version.
#[doc(hidden)]
#[derive(frame_support::DefaultNoBound, Encode, Decode, MaxEncodedLen)]
pub struct NoopMigration<const N: u16>;

impl<const N: u16> MigrationStep for NoopMigration<N> {
    const VERSION: u16 = N;
    fn max_step_weight() -> Weight {
        Weight::zero()
    }
    fn step(&mut self) -> (IsFinished, Weight) {
        (IsFinished::Yes, Weight::zero())
    }
}

mod private {
    use crate::MigrationStep;

    pub trait Sealed {}

    #[impl_trait_for_tuples::impl_for_tuples(10)]
    #[tuple_types_custom_trait_bound(MigrationStep)]
    impl Sealed for Tuple {}
}

/// Defines a sequence of migrations.
///
/// The sequence must be defined by a tuple of migrations, each of which must implement the
/// `MigrationStep` trait. Migrations must be ordered by their versions with no gaps.
pub trait MigrateSequence: private::Sealed {
    /// Returns the range of versions that this migrations sequence can handle.
    /// Migrations must be ordered by their versions with no gaps.
    const VERSION_RANGE: (u16, u16);

    /// Returns the default cursor for the given version.
    fn new(version: StorageVersion) -> Cursor;

    #[cfg(feature = "try-runtime")]
    fn pre_upgrade_step(_version: StorageVersion) -> Result<Vec<u8>, TryRuntimeError> {
        Ok(Vec::new())
    }

    #[cfg(feature = "try-runtime")]
    fn post_upgrade_step(_version: StorageVersion, _state: Vec<u8>) -> Result<(), TryRuntimeError> {
        Ok(())
    }

    /// Execute the migratable step until the weight limit is reached.
    fn steps(version: StorageVersion, cursor: &[u8], weight_left: &mut Weight) -> StepResult;

    /// Verify that the migratable step fits into `Cursor`, and that `max_step_weight` is not greater
    /// than `max_block_weight`.
    fn integrity_test(max_block_weight: Weight);

    /// Returns whether migrating from `in_storage` to `target` is supported.
    ///
    /// A migratable is supported if `VERSION_RANGE` is (in_storage + 1, target).
    fn is_upgrade_supported(in_storage: StorageVersion, target: StorageVersion) -> bool {
        let (low, high) = Self::VERSION_RANGE;
        target == high && in_storage + 1 == low
    }
}

/// The result of running the migratable.
#[derive(Debug, PartialEq)]
pub enum MigrateResult {
    /// No migratable was performed
    NoMigrationPerformed,
    /// No migratable currently in progress
    NoMigrationInProgress,
    /// A migratable is in progress
    InProgress { steps_done: u32 },
    /// All migrations are completed
    Completed,
}

/// The result of running a migratable step.
#[derive(Debug, PartialEq)]
pub enum StepResult {
    InProgress { cursor: Cursor, steps_done: u32 },
    Completed { steps_done: u32 },
}

#[impl_trait_for_tuples::impl_for_tuples(10)]
#[tuple_types_custom_trait_bound(MigrationStep)]
impl MigrateSequence for Tuple {
    const VERSION_RANGE: (u16, u16) = {
        let mut versions: (u16, u16) = (0, 0);
        for_tuples!(
            #(
                match versions {
                    (0, 0) => {
                        versions = (Tuple::VERSION, Tuple::VERSION);
                    },
                    (min_version, last_version) if Tuple::VERSION == last_version + 1 => {
                        versions = (min_version, Tuple::VERSION);
                    },
                    _ => panic!("Migrations must be ordered by their versions with no gaps.")
                }
            )*
        );
        versions
    };

    fn new(version: StorageVersion) -> Cursor {
        for_tuples!(
            #(
                if version == Tuple::VERSION {
                    return Tuple::default().encode().try_into().expect(PROOF_ENCODE)
                }
            )*
        );
        invalid_version(version)
    }

    #[cfg(feature = "try-runtime")]
    /// Execute the pre-checks of the step associated with this version.
    fn pre_upgrade_step(version: StorageVersion) -> Result<Vec<u8>, TryRuntimeError> {
        for_tuples!(
            #(
                if version == Tuple::VERSION {
                    return Tuple::pre_upgrade_step()
                }
            )*
        );
        invalid_version(version)
    }

    #[cfg(feature = "try-runtime")]
    /// Execute the post-checks of the step associated with this version.
    fn post_upgrade_step(version: StorageVersion, state: Vec<u8>) -> Result<(), TryRuntimeError> {
        for_tuples!(
            #(
                if version == Tuple::VERSION {
                    return Tuple::post_upgrade_step(state)
                }
            )*
        );
        invalid_version(version)
    }

    fn steps(version: StorageVersion, mut cursor: &[u8], weight_left: &mut Weight) -> StepResult {
        for_tuples!(
            #(
                if version == Tuple::VERSION {
                    let mut migration = <Tuple as Decode>::decode(&mut cursor)
                        .expect(PROOF_DECODE);
                    let max_weight = Tuple::max_step_weight();
                    let mut steps_done = 0;
                    while weight_left.all_gt(max_weight) {
                        let (finished, weight) = migration.step();
                        steps_done.saturating_accrue(1);
                        weight_left.saturating_reduce(weight);
                        if matches!(finished, IsFinished::Yes) {
                            return StepResult::Completed{ steps_done }
                        }
                    }
                    return StepResult::InProgress{cursor: migration.encode().try_into().expect(PROOF_ENCODE), steps_done }
                }
            )*
        );
        invalid_version(version)
    }

    fn integrity_test(max_block_weight: Weight) {
        for_tuples!(
            #(
                Tuple::integrity_test(max_block_weight);
            )*
        );
    }
}

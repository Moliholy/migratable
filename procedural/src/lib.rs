use quote::{quote, ToTokens};
use std::collections::BTreeMap;
use syn::{parse_macro_input, parse_quote};

const ON_IDLE_HOOK: &str = "on_idle";
const INTEGRITY_TEST_HOOK: &str = "integrity_test";

#[proc_macro_attribute]
pub fn hooks(
    _attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let mut input = parse_macro_input!(item as syn::ItemImpl);
    let custom_code_set: sp_std::collections::btree_set::BTreeSet<&str> =
        [ON_IDLE_HOOK, INTEGRITY_TEST_HOOK]
            .iter()
            .cloned()
            .collect();
    // check the required functions are present
    let inputs = input.items.clone();
    let implemented_fn = inputs
        .iter()
        .filter_map(|item| match item {
            syn::ImplItem::Fn(method) => Some((method.sig.ident.to_string(), method)),
            _ => None,
        })
        .collect::<BTreeMap<_, _>>();
    // add default on_idle implementation if needed
    if !implemented_fn.contains_key(ON_IDLE_HOOK) {
        let default_on_idle_hook = quote!(
            fn on_idle(
                _block: frame_system::pallet_prelude::BlockNumberFor<T>,
                mut remaining_weight: frame_support::weights::Weight,
            ) -> frame_support::weights::Weight {
                frame_support::weights::Weight::zero()
            }
        );
        input.items.push(parse_quote! { #default_on_idle_hook });
    }
    // add default integrity_test implementation if needed
    if !implemented_fn.contains_key(INTEGRITY_TEST_HOOK) {
        let default_integrity_test_hook = quote!(
            fn integrity_test() {}
        );
        input
            .items
            .push(parse_quote! { #default_integrity_test_hook });
    }
    // modify the actual hooks
    for item in &mut input.items {
        if let syn::ImplItem::Fn(method) = item {
            let fn_name = method.sig.ident.to_string();
            if custom_code_set.contains(&fn_name.as_str()) {
                let curr_impl = method.block.to_token_stream();
                match fn_name.as_str() {
                    ON_IDLE_HOOK => {
                        // get the second attribute's name
                        if let Some(syn::FnArg::Typed(remaining_weight_attr)) =
                            method.sig.inputs.last()
                        {
                            if let syn::Pat::Ident(remaining_weight_ident) =
                                *remaining_weight_attr.pat.clone()
                            {
                                let remaining_weight_name = remaining_weight_ident.ident;
                                let new_code = quote!(
                                    {
                                        let mut weight_sum = #remaining_weight_name.clone();
                                        loop {
                                            let (result, weight) = Migration::<T>::migrate(#remaining_weight_name);
                                            #remaining_weight_name.saturating_reduce(weight);

                                            match result {
                                                // There is not enough weight to perform a migration, or make any progress, we
                                                // just return the remaining weight.
                                                migratable::MigrateResult::NoMigrationPerformed | migratable::MigrateResult::InProgress { steps_done: 0 } => return #remaining_weight_name,
                                                // Migration is still in progress, we can start the next step.
                                                migratable::MigrateResult::InProgress { .. } => continue,
                                                // Either no migration is in progress, or we are done with all migrations, we
                                                // can do some more other work with the remaining weight.
                                                migratable::MigrateResult::Completed | migratable::MigrateResult::NoMigrationInProgress => break,
                                            }
                                        };
                                        weight_sum.saturating_reduce(#remaining_weight_name);
                                        weight_sum
                                    }
                                );
                                // mutate the block to include the new code
                                method.block = parse_quote! {
                                    {
                                        let migration_weight = #new_code;
                                        let mut weight = #curr_impl;
                                        weight.saturating_add(migration_weight);
                                        weight
                                    }
                                };
                            } else {
                                panic!("on_idle hook does not have the second parameter correctly defined");
                            }
                        } else {
                            panic!("on_idle hook is not properly defined");
                        }
                    }
                    INTEGRITY_TEST_HOOK => {
                        let new_code = quote!(
                            Migration::<T>::integrity_test();
                        );
                        method.block = parse_quote! {
                            {
                                #new_code
                                #curr_impl
                            }
                        };
                    }
                    _ => {}
                }
            } else {
                panic!("\"{}\" not found in pallet hooks", fn_name);
            }
        }
    }
    let modified_impl = quote! {
        #input
    };
    modified_impl.into()
}

/// Adds the `Migrations` type to `Config`
#[proc_macro_attribute]
pub fn config(
    _attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let mut input = parse_macro_input!(item as syn::ItemTrait);
    let migrations = quote!(
        /// The sequence of migration steps that will be applied during a migration.
        type Migrations: migratable::MigrateSequence;
    );
    input.items.push(parse_quote! { #migrations });
    let output = quote! {
        #input
    };
    output.into()
}

/// Adds the following to the pallet module:
/// - `MigrationInProgress` storage item.
/// -
#[proc_macro_attribute]
pub fn pallet(
    _attr: proc_macro::TokenStream,
    item: proc_macro::TokenStream,
) -> proc_macro::TokenStream {
    let mut input = parse_macro_input!(item as syn::ItemMod);
    let content = &mut input.content.as_mut().unwrap().1;

    // add storage
    let storage = quote!(
        /// A migration can span across multiple blocks. This storage defines a cursor to track the
        /// progress of the migration, enabling us to resume from the last completed position.
        #[pallet::storage]
        pub type MigrationInProgress<T: Config> =
            StorageValue<_, migratable::Cursor, frame_support::storage::types::OptionQuery>;
    );
    content.push(parse_quote! { #storage });

    // add migration struct
    let migration = quote!(
        /// Performs all necessary migrations based on `StorageVersion`.
        ///
        /// If `TEST_ALL_STEPS == true` and `try-runtime` is enabled, this will run all the migrations
        /// inside `on_runtime_upgrade`. This should be set to false in tests that want to ensure the step
        /// by step migratable works.
        pub struct Migration<T: Config, const TEST_ALL_STEPS: bool = true>(
            core::marker::PhantomData<T>,
        );
    );
    content.push(parse_quote! { #migration });

    // add migration logic
    let expand = generate_mod_expand();
    content.push(parse_quote! { #expand });

    let output = quote! {
        #input
    };
    output.into()
}

/// Implements the required traits for the `Migration` struct.
fn generate_mod_expand() -> proc_macro2::TokenStream {
    quote!(
        const _: () = {
            use migratable::MigrateSequence;
            const LOG_TARGET: &str = "migratable";

            const _: () = {
                #[cfg(feature = "try-runtime")]
                impl<T: Config, const TEST_ALL_STEPS: bool> Migration<T, TEST_ALL_STEPS> {
                    fn run_all_steps() -> Result<(), sp_runtime::TryRuntimeError> {
                        let mut weight = frame_support::weights::Weight::zero();
                        let name = <Pallet<T>>::name();
                        loop {
                            let in_progress_version = <Pallet<T>>::on_chain_storage_version() + 1;
                            let state = T::Migrations::pre_upgrade_step(in_progress_version)?;
                            let (status, w) = Self::migrate(frame_support::weights::Weight::MAX);
                            weight.saturating_accrue(w);
                            migratable::log::info!(
                                target: LOG_TARGET,
                                "{name}: Migration step {:?} weight = {}",
                                in_progress_version,
                                weight
                            );
                            T::Migrations::post_upgrade_step(in_progress_version, state)?;
                            if matches!(status, MigrateResult::Completed) {
                                break;
                            }
                        }

                        let name = <Pallet<T>>::name();
                        log::info!(target: LOG_TARGET, "{name}: Migration steps weight = {}", weight);
                        Ok(())
                    }
                }
            };

            const _: () = {
                use migratable::weights::WeightInfo;
                impl<T: Config, const TEST_ALL_STEPS: bool> frame_support::traits::OnRuntimeUpgrade
                    for Migration<T, TEST_ALL_STEPS>
                {
                    fn on_runtime_upgrade() -> frame_support::weights::Weight {
                        let name = <Pallet<T>>::name();
                        let latest_version = <Pallet<T>>::current_storage_version();
                        let storage_version = <Pallet<T>>::on_chain_storage_version();

                        if storage_version == latest_version {
                            migratable::log::warn!(
                                target: LOG_TARGET,
                                "{name}: No Migration performed storage_version = latest_version = {:?}",
                                &storage_version
                            );
                            return migratable::weights::SubstrateWeight::<T>::on_runtime_upgrade_noop();
                        }

                        // In case a migratable is already in progress we create the next migratable
                        // (if any) right when the current one finishes.
                        if Self::in_progress() {
                            migratable::log::warn!(
                                target: LOG_TARGET,
                                "{name}: Migration already in progress {:?}",
                                &storage_version
                            );

                            return migratable::weights::SubstrateWeight::<T>::on_runtime_upgrade_in_progress();
                        }

                        migratable::log::info!(
                            target: LOG_TARGET,
                            "{name}: Upgrading storage from {storage_version:?} to {latest_version:?}.",
                        );

                        let cursor = T::Migrations::new(storage_version + 1);
                        MigrationInProgress::<T>::set(Some(cursor));

                        #[cfg(feature = "try-runtime")]
                        if TEST_ALL_STEPS {
                            Self::run_all_steps().unwrap()
                        }

                        migratable::weights::SubstrateWeight::<T>::on_runtime_upgrade()
                    }

                    #[cfg(feature = "try-runtime")]
                    fn pre_upgrade(
                    ) -> Result<frame_support::sp_std::vec::Vec<u8>, sp_runtime::TryRuntimeError>
                    {
                        // We can't really do much here as our migrations do not happen during the runtime upgrade.
                        // Instead, we call the migrations `pre_upgrade` and `post_upgrade` hooks when we iterate
                        // over our migrations.
                        let storage_version = <Pallet<T>>::on_chain_storage_version();
                        let target_version = <Pallet<T>>::current_storage_version();

                        ensure!(
								storage_version != target_version,
								"No upgrade: Please remove this migratable from your runtime upgrade configuration."
							);

                        migratable::log::debug!(
                            target: LOG_TARGET,
                            "Requested migratable of {} from {:?}(on-chain storage version) to {:?}(current storage version)",
                            <Pallet<T>>::name(), storage_version, target_version
                        );

                        ensure!(
								T::Migrations::is_upgrade_supported(storage_version, target_version),
								"Unsupported upgrade: VERSION_RANGE should be (on-chain storage version + 1, current storage version)"
							);
                        Ok(Default::default())
                    }
                }
            };

            const _: () = {
                use migratable::weights::WeightInfo;
                impl<T: Config, const TEST_ALL_STEPS: bool> Migration<T, TEST_ALL_STEPS> {
                    /// Verify that each migratable's step of the [`Config::Migrations`] sequence fits into
                    /// `Cursor`.
                    pub(crate) fn integrity_test() {
                        let max_weight = <T as frame_system::Config>::BlockWeights::get().max_block;
                        T::Migrations::integrity_test(max_weight)
                    }

                    /// Migrate
                    /// Return the weight used and whether or not a migratable is in progress
                    pub(crate) fn migrate(
                        weight_limit: frame_support::weights::Weight,
                    ) -> (migratable::MigrateResult, frame_support::weights::Weight)
                    {
                        let name = <Pallet<T>>::name();
                        let mut weight_left = weight_limit;

                        if weight_left
                            .checked_reduce(migratable::weights::SubstrateWeight::<T>::migrate())
                            .is_none()
                        {
                            return (
                                migratable::MigrateResult::NoMigrationPerformed,
                                Weight::zero(),
                            );
                        }

                        MigrationInProgress::<T>::mutate_exists(|progress| {
                            let Some(cursor_before) = progress.as_mut() else {
                                return (
                                    migratable::MigrateResult::NoMigrationInProgress,
                                    migratable::weights::SubstrateWeight::<T>::migration_noop(),
                                );
                            };

                            // if a migratable is running it is always upgrading to the next version
                            let storage_version = <Pallet<T>>::on_chain_storage_version();
                            let in_progress_version = storage_version + 1;

                            migratable::log::info!(
                                target: LOG_TARGET,
                                "{name}: Migrating from {:?} to {:?},",
                                storage_version,
                                in_progress_version,
                            );

                            let result = match T::Migrations::steps(
                                in_progress_version,
                                cursor_before.as_ref(),
                                &mut weight_left,
                            ) {
                                migratable::StepResult::InProgress { cursor, steps_done } => {
                                    *progress = Some(cursor);
                                    migratable::MigrateResult::InProgress { steps_done }
                                }
                                migratable::StepResult::Completed { steps_done } => {
                                    in_progress_version.put::<Pallet<T>>();
                                    if <Pallet<T>>::current_storage_version() != in_progress_version
                                    {
                                        migratable::log::info!(
                                            target: LOG_TARGET,
                                            "{name}: Next migratable is {:?},",
                                            in_progress_version + 1
                                        );
                                        *progress =
                                            Some(T::Migrations::new(in_progress_version + 1));
                                        migratable::MigrateResult::InProgress { steps_done }
                                    } else {
                                        migratable::log::info!(
                                            target: LOG_TARGET,
                                            "{name}: All migrations done. At version {:?},",
                                            in_progress_version
                                        );
                                        *progress = None;
                                        migratable::MigrateResult::Completed
                                    }
                                }
                            };

                            (result, weight_limit.saturating_sub(weight_left))
                        })
                    }

                    pub(crate) fn ensure_migrated() -> frame_support::dispatch::DispatchResult {
                        if Self::in_progress() {
                            Err(frame_support::sp_runtime::DispatchError::Other(
                                "There is a migration in progress",
                            ))
                        } else {
                            Ok(())
                        }
                    }

                    pub(crate) fn in_progress() -> bool {
                        MigrationInProgress::<T>::exists()
                    }
                }
            };
        };
    )
}

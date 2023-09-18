# Migratable

This project is a library used to have `pallet-contracts`-like migration schema for Substrate pallets.


## Setup

1) Add the following statements:
   - `#[migratable::pallet]` in the pallet's module.
   - `#[migratable::config]` in the pallet's configuration.
   - `#[migratable::hooks]` in the pallet's hooks.

2) Declare the pallet's hooks impl block. There's no need to implement any function.

3) Make sure you add the corresponding storage version with the `#[pallet::storage_version()]` statement.

```rust
#[migratable::pallet]
#[frame_support::pallet]
pub mod pallet {
    // -- snip --

    #[migratable::config]
    #[pallet::config]
    pub trait Config: frame_system::Config {
        // -- snip --
    }

    const STORAGE_VERSION: StorageVersion = StorageVersion::new(1);

    #[pallet::pallet]
    #[pallet::storage_version(STORAGE_VERSION)]
    pub struct Pallet<T>(_);

    #[migratable::hooks]
    #[pallet::hooks]
    impl<T: Config> Hooks<BlockNumberFor<T>> for Pallet<T> {}
}
```

4) When instantiating the pallet, make sure to add the migrations that apply. Some examples [here](https://github.com/paritytech/polkadot-sdk/tree/master/substrate/frame/contracts/src/migration).

```rust
impl my_pallet::Config for Runtime {
   // -- snip --
    
    type Migrations = (
        my_pallet::migration::v2::Migration<Self>,
        my_pallet::migration::v3::Migration<Self>,
        my_pallet::migration::v4::Migration<Self>,
        // add as many as needed
    );
}
```

5) Add the pallet's `Migration` struct to `Executive`:

```rust
pub type Executive = frame_executive::Executive<
   Runtime,
   Block,
   frame_system::ChainContext<Runtime>,
   Runtime,
   AllPalletsWithSystem,
   // make sure to add here the migration
   (
      my_pallet::pallet::Migration<Runtime>,
   ),
>;
```
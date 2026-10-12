#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

extern crate alloc;

use alloc::string::String;
use pvm_contract_sdk::{Address, Lazy, Mapping, U256};

/// Composable sub-storage struct. After `#[storage]` expansion, this type
/// implements `StorageComponent` *and* `StorageLayoutEmit`, so the outer
/// contract renders it as a single struct-typed entry in the storage-layout
/// JSON, with its members described in the `types` table.
#[pvm_contract_sdk::storage]
pub struct Erc20State {
    pub total_supply: Lazy<U256>,
    pub balances: Mapping<Address, U256>,
    pub allowances: Mapping<Address, Mapping<Address, U256>>,
}

#[pvm_contract_sdk::storage]
pub struct MetadataState {
    pub name: Lazy<String>,
    pub symbol: Lazy<String>,
}

/// Contract that embeds two `#[storage]` sub-structs plus a flat field.
/// Verifies that the storageLayout JSON renders each sub-struct solc-style
/// (one `t_struct(..)N_storage`-typed entry + a `types` table entry) through
/// `StorageLayoutEmit::emit_entries` / `StorageTypeName::emit_members`.
#[pvm_contract_sdk::contract]
mod composed_contract {
    use super::*;

    pub struct ComposedContract {
        pub erc20: Erc20State,
        pub metadata: MetadataState,
        pub paused: Lazy<bool>,
    }

    impl ComposedContract {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

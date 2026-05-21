#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

extern crate alloc;

use alloc::string::String;
use pvm_contract_sdk::{Address, Bytes, Lazy, Mapping, U256};

/// Contract that mixes static and dynamic storage value types. Verifies that
/// the storage-layout JSON emitted under `--features abi-gen` resolves type
/// names through `SolEncode::SOL_NAME` for both `Lazy<T>` and `Mapping<K, V>`
/// — including dynamic V (`String`, `Bytes`).
#[pvm_contract_sdk::contract]
mod storage_mix {
    use super::*;

    pub struct StorageMix {
        pub total_supply: Lazy<U256>,
        pub name: Lazy<String>,
        pub blob: Lazy<Bytes>,
        pub balances: Mapping<Address, U256>,
        pub bios: Mapping<Address, String>,
        pub attachments: Mapping<U256, Bytes>,
        pub nested: Mapping<Address, Mapping<Address, U256>>,
    }

    impl StorageMix {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

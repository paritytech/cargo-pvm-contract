#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::{Address, Lazy};

/// Contract with auto-numbered storage only — no `#[slot]` attribute anywhere.
/// Verifies that the storage-layout JSON is emitted for the default
/// auto-numbering mode, with solc-style sub-word packing: `count` and `owner`
/// share slot 0 (`uint32` at offset 0, `address` at offset 4).
#[pvm_contract_sdk::contract]
mod counter {
    use super::*;

    pub struct Counter {
        pub count: Lazy<u32>,
        pub owner: Lazy<Address>,
    }

    impl Counter {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

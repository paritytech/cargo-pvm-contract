#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::{Address, Lazy};

/// The `#[slot(0)]`-anchored twin of `storage_layout_auto.rs`: the same two
/// sub-word fields, wrapped in a `#[storage]` sub-struct pinned at slot 0.
/// Apart from the dotted `state.` label prefix, its layout must be identical
/// to the auto-numbered form — asserted by
/// `auto_and_anchored_layouts_agree` in `abi_output.rs`.
#[pvm_contract_sdk::storage]
pub struct CounterState {
    pub count: Lazy<u32>,
    pub owner: Lazy<Address>,
}

#[pvm_contract_sdk::contract]
mod anchored_counter {
    use super::*;

    pub struct AnchoredCounter {
        #[slot(0)]
        pub state: CounterState,
    }

    impl AnchoredCounter {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

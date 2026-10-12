#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

extern crate alloc;

use pvm_contract_sdk::{Address, Lazy, Mapping, StorageVec, U256};

/// Sub-storage struct with packed sub-word members. `flag` (1 byte),
/// `small` (4 bytes) and `who` (20 bytes) share struct-relative slot 0 at
/// solc offsets 0, 1 and 5; `balances` takes struct-relative slot 1.
#[pvm_contract_sdk::storage]
pub struct Counters {
    pub flag: Lazy<bool>,
    pub small: Lazy<u32>,
    pub who: Lazy<Address>,
    pub balances: Mapping<Address, U256>,
}

/// Sub-storage struct nesting `Counters`, followed by a sub-word member and
/// a dynamic array. A struct member always ends its slot, so `tag` starts a
/// fresh struct-relative slot 2 (offset 0) rather than packing after `who`.
#[pvm_contract_sdk::storage]
pub struct Shell {
    pub counters: Counters,
    pub tag: Lazy<u16>,
    pub items: StorageVec<U256>,
}

/// Contract whose storageLayout has non-zero `offset`s inside a sub-struct's
/// `members`. Verifies that packed sub-word members render with solc's
/// least-significant-byte offsets, that a nested sub-struct is registered
/// once in `types` and referenced from both the outer struct's `members` and
/// the top-level entry, and that the auto-numbered top-level slots chain
/// through `Shell::SLOTS` (4) and `Counters::SLOTS` (2).
#[pvm_contract_sdk::contract]
mod packed_substruct {
    use super::*;

    pub struct PackedSubstruct {
        pub shell: Shell,
        pub again: Counters,
        pub sentinel: Lazy<U256>,
    }

    impl PackedSubstruct {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]
#![allow(dead_code)]

use pvm_contract_sdk::{Address, Lazy};

/// A contract combining auto-numbered sub-word fields with a raw external
/// (`#[slot(raw = KEY)]`) field. Verifies two things at once:
///   1. The raw field is exempt from the numeric-vs-auto mixing rule, so the
///      auto fields keep solc-style sub-word packing (`flag` + `count` share
///      slot 0 at different offsets) instead of being forced full-slot.
///   2. The raw external slot is OMITTED from `storageLayout` (like solc).
const IMPLEMENTATION_SLOT: [u8; 32] = [
    0x36, 0x08, 0x94, 0xa1, 0x3b, 0xa1, 0xa3, 0x21, 0x06, 0x67, 0xc8, 0x28, 0x49, 0x2d, 0xb9, 0x8d,
    0xca, 0x3e, 0x20, 0x76, 0xcc, 0x37, 0x35, 0xa9, 0x20, 0xa3, 0xca, 0x50, 0x5d, 0x38, 0x2b, 0xbc,
];

#[pvm_contract_sdk::contract]
mod proxy_raw_slot {
    use super::*;

    pub struct ProxyRawSlot {
        pub flag: Lazy<bool>,
        pub count: Lazy<u32>,
        #[slot(raw = IMPLEMENTATION_SLOT)]
        pub impl_addr: Lazy<Address>,
    }

    impl ProxyRawSlot {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_sdk::method]
        pub fn count(&self) -> u32 {
            self.count.get()
        }

        #[pvm_contract_sdk::method]
        pub fn implementation(&self) -> Address {
            self.impl_addr.get()
        }
    }
}

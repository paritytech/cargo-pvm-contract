#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

//! Minimal EIP-1967 upgradeable proxy — exercises `#[slot(raw = KEY)]` end to
//! end on the polkavm target, so the macro-generated field construction
//! (`StorageComponent::new_at(StorageKey::from_raw(..), 32 - PACKED_BYTES, ..)`)
//! is actually compiled for riscv, not just token-checked on the host.

use pvm_contract_sdk::{Address, Lazy};

// Note: `paused` is an auto-numbered sub-word field; it coexists with the raw
// external `impl_addr` slot below (raw slots are exempt from the mixing rule),
// so this also proves auto-numbering + `#[slot(raw)]` compile together on-chain.

/// `bytes32(uint256(keccak256("eip1967.proxy.implementation")) - 1)`.
const IMPLEMENTATION_SLOT: [u8; 32] = [
    0x36, 0x08, 0x94, 0xa1, 0x3b, 0xa1, 0xa3, 0x21, 0x06, 0x67, 0xc8, 0x28, 0x49, 0x2d, 0xb9, 0x8d,
    0xca, 0x3e, 0x20, 0x76, 0xcc, 0x37, 0x35, 0xa9, 0x20, 0xa3, 0xca, 0x50, 0x5d, 0x38, 0x2b, 0xbc,
];

#[pvm_contract_sdk::contract(allocator = "pico")]
mod proxy {
    use super::*;

    pub struct Proxy {
        pub paused: Lazy<bool>,
        #[slot(raw = IMPLEMENTATION_SLOT)]
        pub impl_addr: Lazy<Address>,
    }

    impl Proxy {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_sdk::method]
        pub fn implementation(&self) -> Address {
            self.impl_addr.get()
        }

        #[pvm_contract_sdk::method]
        pub fn paused(&self) -> bool {
            self.paused.get()
        }

        #[pvm_contract_sdk::method]
        pub fn upgrade_to(&mut self, new_implementation: Address) {
            self.impl_addr.set(&new_implementation);
        }
    }
}

#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_types::{PolkaVmHost as api, StorageFlags};

#[pvm_contract_macros::contract("Flipper.sol", allocator = "pico")]
mod flipper {
    use super::*;

    const STORAGE_KEY: [u8; 32] = [0u8; 32];

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), pvm_contract_types::EmptyError> {
        // Initialize to false (0)
        api::set_storage(StorageFlags::empty(), &STORAGE_KEY, &[0u8; 32]);
        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn flip() {
        let current = read_value();
        let new_val = if current { 0u8 } else { 1u8 };
        let mut buf = [0u8; 32];
        buf[31] = new_val;
        api::set_storage(StorageFlags::empty(), &STORAGE_KEY, &buf);
    }

    #[pvm_contract_macros::method]
    pub fn get() -> bool {
        read_value()
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), pvm_contract_types::EmptyError> {
        Ok(())
    }

    fn read_value() -> bool {
        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        match api::get_storage(StorageFlags::empty(), &STORAGE_KEY, &mut out) {
            Ok(_) => buf[31] != 0,
            Err(_) => false,
        }
    }
}

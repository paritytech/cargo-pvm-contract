#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::{PolkaVmHost, StorageFlags};

#[pvm_contract_sdk::contract("Flipper.sol", allocator = "pico")]
mod flipper {
    use super::*;

    const STORAGE_KEY: [u8; 32] = [0u8; 32];

    #[pvm_contract_sdk::constructor]
    pub fn new() -> Result<(), pvm_contract_sdk::EmptyError> {
        // Initialize to false (0)
        PolkaVmHost::set_storage(StorageFlags::empty(), &STORAGE_KEY, &[0u8; 32]);
        Ok(())
    }

    #[pvm_contract_sdk::method]
    pub fn flip() {
        let current = read_value();
        let new_val = if current { 0u8 } else { 1u8 };
        let mut buf = [0u8; 32];
        buf[31] = new_val;
        PolkaVmHost::set_storage(StorageFlags::empty(), &STORAGE_KEY, &buf);
    }

    #[pvm_contract_sdk::method]
    pub fn get() -> bool {
        read_value()
    }

    #[pvm_contract_sdk::fallback]
    pub fn fallback() -> Result<(), pvm_contract_sdk::EmptyError> {
        Ok(())
    }

    fn read_value() -> bool {
        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        match PolkaVmHost::get_storage(StorageFlags::empty(), &STORAGE_KEY, &mut out) {
            Ok(_) => buf[31] != 0,
            Err(_) => false,
        }
    }
}

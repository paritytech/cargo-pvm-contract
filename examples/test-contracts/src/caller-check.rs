#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_types::{PolkaVmHost as api, StorageFlags};

#[pvm_contract_macros::contract("CallerCheck.sol", allocator = "pico")]
mod caller_check {
    use super::*;
    use pvm_contract_types::Address;

    const LAST_CALLER_KEY: [u8; 32] = [0u8; 32];

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), pvm_contract_types::EmptyError> {
        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn get_caller() -> Address {
        let mut caller = [0u8; 20];
        api::caller(&mut caller);
        caller.into()
    }

    #[pvm_contract_macros::method]
    pub fn record_caller() {
        let mut caller = [0u8; 20];
        api::caller(&mut caller);
        let mut buf = [0u8; 32];
        buf[12..32].copy_from_slice(&caller);
        api::set_storage(StorageFlags::empty(), &LAST_CALLER_KEY, &buf);
    }

    #[pvm_contract_macros::method]
    pub fn get_last_caller() -> Address {
        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        match api::get_storage(StorageFlags::empty(), &LAST_CALLER_KEY, &mut out) {
            Ok(_) => {
                let mut addr = [0u8; 20];
                addr.copy_from_slice(&buf[12..32]);
                addr.into()
            }
            Err(_) => Address::from([0u8; 20]),
        }
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), pvm_contract_types::EmptyError> {
        Ok(())
    }
}

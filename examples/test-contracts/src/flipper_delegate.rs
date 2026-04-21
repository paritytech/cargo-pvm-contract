#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pallet_revive_uapi::StorageFlags;

pvm_contract_macros::abi_import!(alloc = true, {
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface Flipper {
    function flip() external;
    function get() external view returns (bool);
}
});

#[pvm_contract_macros::contract("DelegateFlipper.sol", allocator = "pico")]
mod flipper_delegate {
    use super::*;
    use pvm_contract_core::call::CallError;
    use pvm_contract_types::PolkaVmHost as api;

    const STORAGE_KEY: [u8; 32] = [0u8; 32];
    use flipper::{self, Flipper};

    sol_revert_enum! {
        pub enum Error {
            CallError(CallError)
        }
    }

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), Error> {
        // Initialize to false (0)
        api::set_storage(StorageFlags::empty(), &STORAGE_KEY, &[0u8; 32]);
        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn delegate_flipper(addr: Address) -> Result<(), Error> {
        let flip = Flipper::from_address(addr).flip();
        Ok(flip.delegate_call()?)
    }

    #[pvm_contract_macros::method]
    pub fn get() -> bool {
        read_value()
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), Error> {
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

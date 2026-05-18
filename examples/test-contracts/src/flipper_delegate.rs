#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::StorageFlags;

pvm_contract_sdk::abi_import! {
    #![abi_import(alloc = true)]
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface Flipper {
    function flip() external;
    function get() external view returns (bool);
}
}

#[pvm_contract_sdk::contract("DelegateFlipper.sol", allocator = "pico")]
mod flipper_delegate {
    use super::*;
    use pvm_contract_sdk::CallError;
    use pvm_contract_sdk::HostApi;

    const STORAGE_KEY: [u8; 32] = [0u8; 32];
    use flipper::{self, Flipper};

    #[derive(SolError, Debug)]
    pub enum Error {
        CallError(CallError),
    }

    pub struct FlipperDelegate;

    impl FlipperDelegate {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), Error> {
            self.host()
                .set_storage(StorageFlags::empty(), &STORAGE_KEY, &[0u8; 32]);
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn delegate_flipper(&mut self, addr: Address) -> Result<(), Error> {
            let flip = Flipper::from_address(addr).flip();
            Ok(flip.delegate_call(self)?)
        }

        #[pvm_contract_sdk::method]
        pub fn get(&self) -> bool {
            self.read_value()
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), Error> {
            Ok(())
        }

        fn read_value(&self) -> bool {
            let mut buf = [0u8; 32];
            let mut out = &mut buf[..];
            match self
                .host()
                .get_storage(StorageFlags::empty(), &STORAGE_KEY, &mut out)
            {
                Ok(_) => buf[31] != 0,
                Err(_) => false,
            }
        }
    }
}

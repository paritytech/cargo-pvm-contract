#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use alloy_core::sol;
use pallet_revive_uapi::{HostFnImpl as api, StorageFlags};

sol!(
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface Flipper {
    function flip() external;
    function get() external view returns (bool);
}

);

#[pvm_contract_macros::contract("DelegateFlipper.sol", allocator = "pico")]
mod flipper {
    use crate::Flipper::FlipperCalls;
    use alloy_core::sol_types::SolInterface;
    use pvm_contract_core::call::{CallError, new_payable, new_view};
    use pvm_contract_types::*;

    use super::*;

    #[derive(Debug, Clone, Copy)]
    pub enum Error {
        Unexpected,
    }

    impl From<CallError> for Error {
        fn from(_value: CallError) -> Self {
            Self::Unexpected
        }
    }

    impl AsRef<[u8]> for Error {
        fn as_ref(&self) -> &[u8] {
            match *self {
                Error::Unexpected => b"Unexpected",
            }
        }
    }

    const STORAGE_KEY: [u8; 32] = [0u8; 32];

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), Error> {
        // Initialize to false (0)
        api::set_storage(StorageFlags::empty(), &STORAGE_KEY, &[0u8; 32]);
        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn delegate_flipper(addr: Address) -> Result<(), CallError> {
        let flip = FlipperCalls::flip(Flipper::flipCall).abi_encode();

        new_payable(addr, flip.as_slice()).delegate_call::<()>()
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

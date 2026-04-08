#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use alloy_core::sol;

sol!(
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface Flipper {
    function flip() external;
    function get() external view returns (bool);
}

);

#[pvm_contract_macros::contract("FlipperCallAlloy.sol", allocator = "pico")]
mod flipper_call_alloy {

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

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), Error> {
        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn call_flipper(addr: Address) -> Result<(), CallError> {
        let get = FlipperCalls::get(Flipper::getCall).abi_encode();
        let flip = FlipperCalls::flip(Flipper::flipCall).abi_encode();

        let res = new_view(addr, get.as_slice()).call::<bool>()?;
        assert_eq!(res, false);
        new_payable(addr, flip.as_slice()).call::<()>()?;
        let res = new_view(addr, get.as_slice()).call::<bool>()?;
        assert_eq!(res, true);
        Ok(())
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), Error> {
        Ok(())
    }
}

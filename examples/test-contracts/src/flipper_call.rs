#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

pvm_contract_sdk::abi_import!(alloc = true, {
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface Flipper {
    function flip() external;
    function get() external view returns (bool);
}
});

#[pvm_contract_sdk::contract("FlipperCallAlloy.sol", allocator = "pico")]
mod flipper_call_alloy {
    use pvm_contract_sdk::*;

    use super::*;
    use flipper::{self, Flipper};
    sol_revert_enum! {
        pub enum Error {
            CallError(CallError)
        }
    }
    #[pvm_contract_sdk::constructor]
    pub fn new() -> Result<(), Error> {
        Ok(())
    }

    #[pvm_contract_sdk::method]
    pub fn call_flipper(addr: Address) -> Result<(), Error> {
        let flipper = Flipper::from_address(addr);
        let get = flipper.get();
        let flip = flipper.flip();

        let res = get.call()?;
        assert_eq!(res, false);
        let _ = flip.call()?;
        let res = get.call()?;
        assert_eq!(res, true);
        Ok(())
    }

    #[pvm_contract_sdk::fallback]
    pub fn fallback() -> Result<(), Error> {
        Ok(())
    }
}

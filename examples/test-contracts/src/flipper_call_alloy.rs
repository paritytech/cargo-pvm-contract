#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

pvm_contract_macros::abi_import!(alloc = true, {
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface Flipper {
    function flip() external;
    function get() external view returns (bool);
}
});

#[pvm_contract_macros::contract("FlipperCallAlloy.sol", allocator = "pico")]
mod flipper_call_alloy {

    use pvm_contract_core::call::CallError;
    use pvm_contract_types::*;

    use super::*;
    type Error = pvm_contract_types::EmptyError;

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), Error> {
        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn call_flipper(addr: Address) -> Result<(), CallError> {
        let flipper = Flipper::from_address(addr);
        let get = flipper.get();
        let flip = flipper.flip();
        let mut input = [0u8; 512];
        let mut output = [0u8; 512];

        let res = get.call_raw(&mut input, &mut output)?;
        assert_eq!(res, false);
        let _ = flip.call_raw(&mut input, &mut output)?;
        let res = get.call_raw(&mut input, &mut output)?;
        assert_eq!(res, true);
        Ok(())
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), Error> {
        Ok(())
    }
}

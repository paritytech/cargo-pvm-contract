#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

pvm_contract_sdk::abi_import! {
#![abi_import(alloc = true)]
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface Flipper {
    function flip() external;
    function get() external view returns (bool);
}
}

#[pvm_contract_sdk::contract("FlipperCallAlloy.sol", allocator = "pico")]
mod flipper_call_alloy {
    use pvm_contract_sdk::CallError;
    use pvm_contract_sdk::*;

    use super::*;
    use flipper::{self, Flipper};

    sol_revert_enum! {
        pub enum Error {
            CallError(CallError)
        }
    }

    pub struct FlipperCallAlloy;

    impl FlipperCallAlloy {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), Error> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn call_flipper(&mut self, addr: Address) -> Result<(), Error> {
            let flipper = Flipper::from_address(addr);
            let get = flipper.get();
            let flip = flipper.flip();

            // View callee — `&self` borrow of the contract root suffices;
            // `&mut self` here coerces to `&Self` automatically.
            let res = get.call(self)?;
            assert_eq!(res, false);
            // Nonpayable callee — requires `&mut Self` borrow.
            let _ = flip.call(self)?;
            let res = get.call(self)?;
            assert_eq!(res, true);
            Ok(())
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), Error> {
            Ok(())
        }
    }
}

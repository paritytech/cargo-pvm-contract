#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

// E2E fixture: reentrancy opt-in through the typed `abi_import!` API.

pvm_contract_sdk::abi_import! {
#![abi_import(alloc = true)]
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface ReentrancyCallback {
    function callBack(address lender) external;
}
}

#[pvm_contract_sdk::contract("ReentrancyTyped.sol", allocator = "pico")]
mod reentrancy_typed {
    use pvm_contract_sdk::CallError;
    use pvm_contract_sdk::*;

    use super::*;
    use reentrancy_callback::ReentrancyCallback;

    #[derive(SolError, Debug)]
    pub enum Error {
        CallError(CallError),
        Panic(Panic),
        Revert(RevertString),
    }

    pub struct ReentrancyTyped;

    impl ReentrancyTyped {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), Error> {
            Ok(())
        }

        /// Landing point for the borrower's callback.
        #[pvm_contract_sdk::method]
        pub fn on_callback(&mut self) -> Result<bool, Error> {
            Ok(true)
        }

        /// Outbound typed call WITH the reentrancy opt-in. The borrower's
        /// callback into `on_callback()` must succeed.
        #[pvm_contract_sdk::method]
        pub fn lend_with_reentry(&mut self, borrower: Address) -> Result<(), Error> {
            let mut own = [0u8; 20];
            self.host().address(&mut own);

            let call = ReentrancyCallback::from_address(borrower)
                .call_back(own.into())
                .allow_reentry();
            let _ = call.call(self)?;
            Ok(())
        }

        /// Same chain WITHOUT the opt-in. pallet-revive must deny the
        /// callback with `ReentranceDenied`.
        #[pvm_contract_sdk::method]
        pub fn lend_without_reentry(&mut self, borrower: Address) -> Result<(), Error> {
            let mut own = [0u8; 20];
            self.host().address(&mut own);

            let call = ReentrancyCallback::from_address(borrower).call_back(own.into());
            let _ = call.call(self)?;
            Ok(())
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), Error> {
            Ok(())
        }
    }
}
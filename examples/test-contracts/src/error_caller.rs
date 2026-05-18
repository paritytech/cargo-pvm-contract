#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

pvm_contract_sdk::abi_import! {
#![abi_import(alloc = true)]
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

interface ErrorHandling {
    error AlwaysReverts();
    error ZeroNotAllowed();

    function willRevert() external;
    function willSucceed() external view returns (bool);
    function setGuarded(uint256 val) external;
    function getGuarded() external view returns (uint256);
}
}

#[pvm_contract_sdk::contract("ErrorCaller.sol", allocator = "pico")]
mod error_caller {

    use alloc::string::ToString;
    use pvm_contract_sdk::CallError;
    use pvm_contract_sdk::*;

    use crate::error_handling::{AlwaysReverts, ZeroNotAllowed};

    use super::*;
    use error_handling::{self, ErrorHandling};

    #[derive(SolError, Debug, PartialEq)]
    pub enum ImportedErrors {
        AlwaysReverts(error_handling::AlwaysReverts),
        ZeroNotAllowed(error_handling::ZeroNotAllowed),
    }

    #[derive(SolError, Debug, PartialEq)]
    pub enum Error {
        ImportedErrors(ImportedErrors),
        CallError(CallError),
        Panic(Panic),
        Revert(RevertString),
    }

    pub struct ErrorCaller;

    impl ErrorCaller {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), Error> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn call_error(&mut self, addr: Address) -> Result<(), Error> {
            let error = ErrorHandling::from_address(addr);
            let reverts = error.will_revert();

            let Err(res) = reverts.call(self) else {
                return Err(RevertString("failed call".to_string()).into());
            };
            let res = res.try_decode_error::<Error>(self.host());
            assert_eq!(
                res,
                Ok(Some(Error::ImportedErrors(ImportedErrors::AlwaysReverts(
                    AlwaysReverts {}
                ))))
            );

            let Err(res) = error.set_guarded(U256::ZERO).call(self) else {
                return Err(RevertString("failed call".to_string()).into());
            };
            let res = res.try_decode_error::<Error>(self.host());
            assert_eq!(
                res,
                Ok(Some(Error::ImportedErrors(ImportedErrors::ZeroNotAllowed(
                    ZeroNotAllowed {}
                ))))
            );

            Ok(())
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), Error> {
            Ok(())
        }
    }
}

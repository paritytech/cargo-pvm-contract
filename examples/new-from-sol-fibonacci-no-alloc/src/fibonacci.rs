#![no_main]
#![no_std]

use pallet_revive_uapi::{HostFnImpl as api, StorageFlags};
use ruint::aliases::U256;

#[pvm_contract_macros::contract("Fibonacci.sol", no_alloc, buffer = 256)]
mod contract {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Error {
        // Add your errors here
    }

    impl AsRef<[u8]> for Error {
        fn as_ref(&self) -> &[u8] {
            match *self {
                // Match your errors here
            }
        }
    }

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), Error> {
        Ok(())
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), Error> {
        Ok(())
    }

    // TODO: Implement the following methods from Fibonacci.sol:

    // #[pvm_contract_macros::method]
    // pub fn fibonacci(arg0: u32) -> Result<u32, Error> {
    //     todo!()
    // }

}
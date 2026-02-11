#![no_main]
#![no_std]

use pallet_revive_uapi::{HostFnImpl as api, StorageFlags};
use ruint::aliases::U256;

#[pvm_contract_macros::contract("MyToken.sol", no_alloc, buffer = 256)]
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

     // TODO: Implement the following methods from MyToken.sol:

     #[pvm_contract_macros::method]
     pub fn balance_of(account: [u8; 20]) -> Result<U256, Error> {
         todo!()
     }

     #[pvm_contract_macros::method]
     pub fn mint(to: [u8; 20], amount: U256) -> Result<(), Error> {
         todo!()
     }

     #[pvm_contract_macros::method]
     pub fn total_supply() -> Result<U256, Error> {
         todo!()
     }

     #[pvm_contract_macros::method]
     pub fn transfer(to: [u8; 20], amount: U256) -> Result<(), Error> {
         todo!()
     }

}
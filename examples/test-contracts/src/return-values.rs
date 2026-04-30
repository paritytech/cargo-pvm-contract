#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::U256;

#[pvm_contract_sdk::contract("ReturnValues.sol", allocator = "pico")]
mod return_values {
    use super::*;
    use pvm_contract_sdk::{Address};

    pub struct ReturnValues;

    impl ReturnValues {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn get_pair(&self) -> (U256, bool) {
            (U256::from(42u64), true)
        }

        #[pvm_contract_sdk::method]
        pub fn get_triple(&self) -> (U256, Address, bool) {
            let addr = Address::from([0xABu8; 20]);
            (U256::from(123u64), addr, false)
        }

        #[pvm_contract_sdk::method]
        pub fn identity(&self, val: U256) -> U256 {
            val
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }
    }
}

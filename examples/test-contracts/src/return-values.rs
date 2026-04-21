#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use ruint::aliases::U256;

#[pvm_contract_macros::contract("ReturnValues.sol", allocator = "pico")]
mod return_values {
    use super::*;
    use pvm_contract_types::{Address, HostApi, PolkaVmHost};

    pub struct ReturnValues<H: HostApi = PolkaVmHost> {
        pub host: H,
    }

    impl<H: HostApi> ReturnValues<H> {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_types::EmptyError> {
            Ok(())
        }

        #[pvm_contract_macros::method]
        pub fn get_pair(&self) -> (U256, bool) {
            (U256::from(42u64), true)
        }

        #[pvm_contract_macros::method]
        pub fn get_triple(&self) -> (U256, Address, bool) {
            let addr = Address::from([0xABu8; 20]);
            (U256::from(123u64), addr, false)
        }

        #[pvm_contract_macros::method]
        pub fn identity(&self, val: U256) -> U256 {
            val
        }

        #[pvm_contract_macros::fallback]
        pub fn fallback(&mut self) -> Result<(), pvm_contract_types::EmptyError> {
            Ok(())
        }
    }
}

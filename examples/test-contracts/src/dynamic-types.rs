#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::U256;

#[pvm_contract_sdk::contract("DynamicTypes.sol", allocator = "pico")]
mod dynamic_types {
    use super::*;
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;
    use pvm_contract_sdk::{Bytes};

    pub struct DynamicTypes;

    impl DynamicTypes {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn get_string_length(&self, s: String) -> U256 {
            U256::from(s.len())
        }

        #[pvm_contract_sdk::method]
        pub fn echo_string(&self) -> String {
            String::from("hello world")
        }

        #[pvm_contract_sdk::method]
        pub fn get_bytes_length(&self, b: Bytes) -> U256 {
            U256::from(b.0.len())
        }

        #[pvm_contract_sdk::method]
        pub fn echo_bytes(&self) -> Bytes {
            Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF])
        }

        #[pvm_contract_sdk::method]
        pub fn sum_array(&self, arr: Vec<U256>) -> U256 {
            let mut sum = U256::ZERO;
            for v in arr {
                sum = sum.wrapping_add(v);
            }
            sum
        }

        #[pvm_contract_sdk::method]
        pub fn get_array(&self) -> Vec<U256> {
            vec![U256::from(10), U256::from(20), U256::from(30)]
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }
    }
}

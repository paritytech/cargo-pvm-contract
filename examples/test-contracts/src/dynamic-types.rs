#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use ruint::aliases::U256;

#[pvm_contract_macros::contract("DynamicTypes.sol", allocator = "pico")]
mod dynamic_types {
    use super::*;
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;
    use pvm_contract_types::Bytes;

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), pvm_contract_types::EmptyError> {
        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn get_string_length(s: String) -> U256 {
        U256::from(s.len())
    }

    #[pvm_contract_macros::method]
    pub fn echo_string() -> String {
        String::from("hello world")
    }

    #[pvm_contract_macros::method]
    pub fn get_bytes_length(b: Bytes) -> U256 {
        U256::from(b.0.len())
    }

    #[pvm_contract_macros::method]
    pub fn echo_bytes() -> Bytes {
        Bytes(vec![0xDE, 0xAD, 0xBE, 0xEF])
    }

    #[pvm_contract_macros::method]
    pub fn sum_array(arr: Vec<U256>) -> U256 {
        let mut sum = U256::ZERO;
        for v in arr {
            sum = sum.wrapping_add(v);
        }
        sum
    }

    #[pvm_contract_macros::method]
    pub fn get_array() -> Vec<U256> {
        vec![U256::from(10), U256::from(20), U256::from(30)]
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), pvm_contract_types::EmptyError> {
        Ok(())
    }
}

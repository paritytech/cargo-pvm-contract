#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use ruint::aliases::U256;

#[pvm_contract_macros::contract("CompositeTypes.sol", allocator = "pico")]
mod composite_types {
    use super::*;

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), pvm_contract_types::EmptyError> {
        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn sum_fixed_array(scores: [U256; 3]) -> U256 {
        scores[0].wrapping_add(scores[1]).wrapping_add(scores[2])
    }

    #[pvm_contract_macros::method]
    pub fn get_fixed_array() -> [U256; 3] {
        [U256::from(10), U256::from(20), U256::from(30)]
    }

    #[pvm_contract_macros::method]
    pub fn process_tuple(data: (U256, bool)) -> U256 {
        if data.1 { data.0 } else { U256::ZERO }
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), pvm_contract_types::EmptyError> {
        Ok(())
    }
}

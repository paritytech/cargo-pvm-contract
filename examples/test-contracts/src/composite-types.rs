#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::U256;

#[pvm_contract_sdk::contract("CompositeTypes.sol", allocator = "pico")]
mod composite_types {
    use super::*;
    use pvm_contract_sdk::{HostApi};

    pub struct CompositeTypes;

    impl CompositeTypes {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn sum_fixed_array(&self, scores: [U256; 3]) -> U256 {
            scores[0].wrapping_add(scores[1]).wrapping_add(scores[2])
        }

        #[pvm_contract_sdk::method]
        pub fn get_fixed_array(&self) -> [U256; 3] {
            [U256::from(10), U256::from(20), U256::from(30)]
        }

        #[pvm_contract_sdk::method]
        pub fn process_tuple(&self, data: (U256, bool)) -> U256 {
            if data.1 {
                data.0
            } else {
                U256::ZERO
            }
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }
    }
}

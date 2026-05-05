#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::{SolType, U256};

#[derive(SolType)]
pub struct Point {
    a: U256,
    b: U256,
}

#[pvm_contract_sdk::contract("PointAdder.sol", allocator = "pico")]
mod point_adder {
    use super::*;

    pub struct PointAdder;

    impl PointAdder {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn add(&mut self, a: Point, b: Point) -> Point {
            Point {
                a: a.a + b.a,
                b: a.b + b.b,
            }
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }
    }
}

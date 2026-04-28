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
    #[pvm_contract_sdk::constructor]
    pub fn new() -> Result<(), pvm_contract_sdk::EmptyError> {
        Ok(())
    }

    #[pvm_contract_sdk::method]
    pub fn add(a: Point, b: Point) -> Point {
        Point {
            a: a.a + b.a,
            b: a.b + b.b,
        }
    }

    #[pvm_contract_sdk::fallback]
    pub fn fallback() -> Result<(), pvm_contract_sdk::EmptyError> {
        Ok(())
    }
}

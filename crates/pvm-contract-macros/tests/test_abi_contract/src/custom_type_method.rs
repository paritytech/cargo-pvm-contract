#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

extern crate alloc;

use pvm_contract_sdk::U256;

#[derive(pvm_contract_macros::SolType)]
pub struct MyPoint {
    pub x: U256,
    pub y: U256,
}

#[pvm_contract_macros::contract]
mod my_contract {
    use super::MyPoint;

    #[pvm_contract_macros::constructor]
    pub fn new() {}

    #[pvm_contract_macros::method]
    pub fn touch(value: MyPoint) -> MyPoint {
        value
    }
}

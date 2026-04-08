#![no_main]
#![no_std]

use ruint::aliases::U256;

#[derive(pvm_contract_macros::SolType)]
pub struct MyPoint {
    pub x: U256,
    pub y: U256,
}

#[pvm_contract_macros::contract(allocator = "pico")]
mod my_contract {
    use super::MyPoint;

    #[pvm_contract_macros::constructor]
    pub fn new() {}

    #[pvm_contract_macros::method]
    pub fn touch(value: MyPoint) -> MyPoint {
        value
    }
}

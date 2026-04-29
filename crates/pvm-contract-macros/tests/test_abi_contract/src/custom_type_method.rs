#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

extern crate alloc;

use pvm_contract_sdk::U256;

#[derive(pvm_contract_sdk::SolType)]
pub struct MyPoint {
    pub x: U256,
    pub y: U256,
}

#[pvm_contract_sdk::contract]
mod my_contract {
    use super::MyPoint;
    use pvm_contract_sdk::{HostApi};

    pub struct MyContract;

    impl MyContract {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_sdk::method]
        pub fn touch(&self, value: MyPoint) -> MyPoint {
            value
        }
    }
}

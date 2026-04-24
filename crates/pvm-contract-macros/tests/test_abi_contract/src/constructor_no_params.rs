#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

#[pvm_contract_sdk::contract]
mod my_contract {
    use pvm_contract_sdk::{HostApi};

    pub struct MyContract;

    impl MyContract {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

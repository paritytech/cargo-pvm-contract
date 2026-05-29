#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

#[pvm_contract_sdk::contract]
mod my_contract {
    pub struct MyContract;

    impl MyContract {
        #[pvm_contract_sdk::constructor]
        #[pvm_contract_sdk::payable]
        pub fn new(&mut self) {}
    }
}

#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

#[pvm_contract_sdk::contract]
mod my_contract {
    use pvm_contract_sdk::{Address};
    use ruint::aliases::U256;

    pub struct MyContract;

    impl MyContract {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_sdk::method]
        pub fn set_flag(&mut self, flag: bool) {}

        #[pvm_contract_sdk::method]
        pub fn transfer(&mut self, to: Address, amount: U256, nonce: u32) -> bool {
            true
        }

        #[pvm_contract_sdk::method]
        pub fn get_count(&self) -> u64 {
            0
        }

        #[pvm_contract_sdk::method]
        pub fn add(a: u64, b: u64) -> u64 {
            a + b
        }

        #[pvm_contract_sdk::method]
        #[pvm_contract_sdk::payable]
        pub fn deposit(&mut self) {}
    }
}

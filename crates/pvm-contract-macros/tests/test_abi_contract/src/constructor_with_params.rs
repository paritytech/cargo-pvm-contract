#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

#[pvm_contract_sdk::contract]
mod my_contract {
    use pvm_contract_sdk::{Address};
    use ruint::aliases::U256;

    pub struct MyContract;

    impl MyContract {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self, owner: Address, supply: U256) {}

        #[pvm_contract_sdk::method]
        pub fn balance_of(&self, account: Address) -> U256 {
            U256::ZERO
        }
    }
}

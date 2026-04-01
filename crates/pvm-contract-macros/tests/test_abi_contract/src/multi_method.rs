#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

#[pvm_contract_macros::contract]
mod my_contract {
    use pvm_contract_types::Address;
    use ruint::aliases::U256;

    #[pvm_contract_macros::constructor]
    pub fn new() {}

    /// Method with no return type
    #[pvm_contract_macros::method]
    pub fn set_flag(flag: bool) {}

    /// Method with multiple params of different sizes
    #[pvm_contract_macros::method]
    pub fn transfer(to: Address, amount: U256, nonce: u32) -> bool {
        true
    }

    /// Method with no params but a return
    #[pvm_contract_macros::method]
    pub fn get_count() -> u64 {
        0
    }
}

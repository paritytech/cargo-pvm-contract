#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

#[pvm_contract_macros::contract]
mod my_contract {
    #[pvm_contract_macros::constructor]
    pub fn new() {}
}

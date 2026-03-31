#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

#[pvm_contract_macros::contract]
mod my_contract {
    use pvm_contract_types::Address;
    use ruint::aliases::U256;

    #[pvm_contract_macros::constructor]
    pub fn new(owner: Address, supply: U256) {}

    #[pvm_contract_macros::method]
    pub fn balance_of(account: Address) -> U256 {
        U256::ZERO
    }
}

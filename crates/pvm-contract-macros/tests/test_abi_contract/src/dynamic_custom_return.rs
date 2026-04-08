#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

extern crate alloc;

use alloc::string::String;

#[derive(pvm_contract_macros::SolType)]
pub struct Named {
    pub id: u64,
    pub name: String,
}

#[pvm_contract_macros::contract(allocator = "bump")]
mod my_contract {
    use super::Named;
    use alloc::string::String;

    #[pvm_contract_macros::constructor]
    pub fn new() {}

    #[pvm_contract_macros::method]
    pub fn get_named() -> Named {
        Named {
            id: 42,
            name: String::from("hello"),
        }
    }

    #[pvm_contract_macros::method]
    pub fn process(data: Named, flag: bool) -> u64 {
        if flag {
            data.id
        } else {
            0
        }
    }
}

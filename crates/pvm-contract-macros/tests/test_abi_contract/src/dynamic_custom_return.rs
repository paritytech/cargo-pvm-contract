#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

extern crate alloc;

use alloc::string::String;

#[derive(pvm_contract_sdk::SolType)]
pub struct Named {
    pub id: u64,
    pub name: String,
}

#[pvm_contract_sdk::contract(allocator = "bump")]
mod my_contract {
    use super::Named;
    use alloc::string::String;
    use pvm_contract_sdk::{HostApi};

    pub struct MyContract;

    impl MyContract {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_sdk::method]
        pub fn get_named(&self) -> Named {
            Named {
                id: 42,
                name: String::from("hello"),
            }
        }

        #[pvm_contract_sdk::method]
        pub fn process(&self, data: Named, flag: bool) -> u64 {
            if flag {
                data.id
            } else {
                0
            }
        }
    }
}

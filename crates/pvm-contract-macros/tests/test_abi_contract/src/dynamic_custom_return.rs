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
    use pvm_contract_types::{HostApi, PolkaVmHost};

    pub struct MyContract<H: HostApi = PolkaVmHost> {
        pub host: H,
    }

    impl<H: HostApi> MyContract<H> {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn get_named(&self) -> Named {
            Named {
                id: 42,
                name: String::from("hello"),
            }
        }

        #[pvm_contract_macros::method]
        pub fn process(&self, data: Named, flag: bool) -> u64 {
            if flag {
                data.id
            } else {
                0
            }
        }
    }
}

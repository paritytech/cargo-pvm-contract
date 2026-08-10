#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

#[pvm_contract_sdk::contract]
mod my_contract {
    use pvm_contract_sdk::HostApi;

    pub struct MyContract;

    #[derive(Debug, Clone, Copy, pvm_contract_sdk::SolType)]
    #[repr(u8)]
    enum B {
        First,
        Second,
    }

    #[derive(Debug, Clone, Copy, pvm_contract_sdk::SolType)]
    struct A {
        b: B,
    }

    impl MyContract {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_sdk::method]
        pub fn set_flag(&mut self, flag: B) -> A {
            A { b: flag }
        }
    }
}

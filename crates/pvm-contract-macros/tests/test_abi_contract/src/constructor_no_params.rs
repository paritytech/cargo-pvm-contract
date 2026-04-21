#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

#[pvm_contract_macros::contract]
mod my_contract {
    use pvm_contract_types::{HostApi, PolkaVmHost};

    pub struct MyContract<H: HostApi = PolkaVmHost> {
        pub host: H,
    }

    impl<H: HostApi> MyContract<H> {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }
}

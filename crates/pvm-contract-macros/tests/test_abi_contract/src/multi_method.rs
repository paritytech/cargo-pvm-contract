#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

#[pvm_contract_macros::contract]
mod my_contract {
    use pvm_contract_types::{Address, HostApi, PolkaVmHost};
    use ruint::aliases::U256;

    pub struct MyContract<H: HostApi = PolkaVmHost> {
        pub host: H,
    }

    impl<H: HostApi> MyContract<H> {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn set_flag(&mut self, flag: bool) {}

        #[pvm_contract_macros::method]
        pub fn transfer(&mut self, to: Address, amount: U256, nonce: u32) -> bool {
            true
        }

        #[pvm_contract_macros::method]
        pub fn get_count(&self) -> u64 {
            0
        }
    }
}

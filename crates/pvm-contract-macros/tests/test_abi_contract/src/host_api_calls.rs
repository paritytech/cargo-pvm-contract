#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::Address;
use pvm_contract_sdk::U256;

/// Contract that calls host APIs in method bodies (via the receiver `self.host`).
/// Verifies that abi-gen cfg-gating correctly excludes method bodies which
/// reference `HostApi` methods that are `unimplemented!()` stubs on the host
/// target used for abi-gen compilation.
#[pvm_contract_sdk::contract]
mod my_contract {
    use super::*;
    use pvm_contract_sdk::{StorageFlags};

    pub struct MyContract;

    impl MyContract {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_sdk::method]
        pub fn read_storage(&self, key: U256) -> U256 {
            let key_bytes = key.to_be_bytes::<32>();
            let mut buf = [0u8; 32];
            let mut out = buf.as_mut_slice();
            let _ = self.host().get_storage(StorageFlags::empty(), &key_bytes, &mut out);
            U256::from_be_bytes::<32>(buf)
        }

        #[pvm_contract_sdk::method]
        pub fn write_storage(&mut self, key: U256, value: U256) {
            let key_bytes = key.to_be_bytes::<32>();
            let value_bytes = value.to_be_bytes::<32>();
            self.host().set_storage(StorageFlags::empty(), &key_bytes, &value_bytes);
        }

        #[pvm_contract_sdk::method]
        pub fn get_caller(&self) -> Address {
            let mut caller = [0u8; 20];
            self.host().caller(&mut caller);
            Address::from(caller)
        }
    }
}

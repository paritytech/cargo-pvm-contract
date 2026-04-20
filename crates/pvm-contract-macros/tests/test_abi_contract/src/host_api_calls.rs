#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_types::Address;
use ruint::aliases::U256;

/// Test contract that calls host APIs in method bodies.
/// Verifies that abi-gen cfg-gating correctly excludes function bodies
/// (which reference HostFnImpl methods that don't exist on host targets)
/// while still producing correct ABI output from type signatures.
#[pvm_contract_macros::contract]
mod my_contract {
    use super::*;
    use pallet_revive_uapi::{HostFnImpl as api, StorageFlags};

    #[pvm_contract_macros::constructor]
    pub fn new() {}

    #[pvm_contract_macros::method]
    pub fn read_storage(key: U256) -> U256 {
        let key_bytes = key.to_be_bytes::<32>();
        let mut buf = [0u8; 32];
        let mut out = buf.as_mut_slice();
        let _ = api::get_storage(StorageFlags::empty(), &key_bytes, &mut out);
        U256::from_be_bytes::<32>(buf)
    }

    #[pvm_contract_macros::method]
    pub fn write_storage(key: U256, value: U256) {
        let key_bytes = key.to_be_bytes::<32>();
        let value_bytes = value.to_be_bytes::<32>();
        api::set_storage(StorageFlags::empty(), &key_bytes, &value_bytes);
    }

    #[pvm_contract_macros::method]
    pub fn get_caller() -> Address {
        let mut caller = [0u8; 20];
        api::caller(&mut caller);
        Address::from(caller)
    }
}

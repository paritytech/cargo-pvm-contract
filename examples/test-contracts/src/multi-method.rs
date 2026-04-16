#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pallet_revive_uapi::{HostFnImpl as api, StorageFlags};
use ruint::aliases::U256;

#[pvm_contract_macros::contract("MultiMethod.sol", allocator = "pico")]
mod multi_method {
    use super::*;

    const COUNTER_KEY: [u8; 32] = [0u8; 32];

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), pvm_contract_types::EmptyError> {
        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn add(a: U256, b: U256) -> U256 {
        a + b
    }

    #[pvm_contract_macros::method]
    pub fn mul(a: U256, b: U256) -> U256 {
        a * b
    }

    #[pvm_contract_macros::method]
    pub fn is_zero(val: U256) -> bool {
        val == U256::ZERO
    }

    #[pvm_contract_macros::method]
    pub fn get_counter() -> U256 {
        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        match api::get_storage(StorageFlags::empty(), &COUNTER_KEY, &mut out) {
            Ok(_) => U256::from_be_bytes::<32>(buf),
            Err(_) => U256::ZERO,
        }
    }

    #[pvm_contract_macros::method]
    pub fn increment() {
        let current = get_counter();
        let new_val = current + U256::from(1u64);
        api::set_storage(StorageFlags::empty(), &COUNTER_KEY, &new_val.to_be_bytes::<32>());
    }

    #[pvm_contract_macros::method]
    pub fn reset() {
        api::set_storage(StorageFlags::empty(), &COUNTER_KEY, &[0u8; 32]);
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), pvm_contract_types::EmptyError> {
        Ok(())
    }
}

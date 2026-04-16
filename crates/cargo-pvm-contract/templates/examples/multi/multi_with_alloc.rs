#![no_main]
#![no_std]

use pvm_contract_types::Address;
use ruint::aliases::U256;

#[pvm_contract_macros::contract("Multi.sol", allocator = "bump")]
mod multi {
    use super::*;

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), pvm_contract_types::EmptyError> {
        Ok(())
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), pvm_contract_types::EmptyError> {
        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn add(a: u32, b: u32) -> u32 {
        a.wrapping_add(b)
    }

    #[pvm_contract_macros::method]
    pub fn multiply(a: u64, b: u64) -> u64 {
        a.wrapping_mul(b)
    }

    #[pvm_contract_macros::method]
    pub fn is_even(n: u32) -> bool {
        (n & 1) == 0
    }

    #[pvm_contract_macros::method]
    pub fn negate(value: U256) -> U256 {
        !value + U256::from(1u8)
    }

    #[pvm_contract_macros::method]
    pub fn max(a: U256, b: U256) -> U256 {
        if a > b { a } else { b }
    }

    #[pvm_contract_macros::method]
    pub fn hash(account: Address) -> U256 {
        let mut bytes = [0u8; 32];
        bytes[12..].copy_from_slice(account.as_ref());
        U256::from_be_bytes::<32>(bytes)
    }

    #[pvm_contract_macros::method]
    pub fn sum3(a: u32, b: u32, c: u32) -> u32 {
        a.wrapping_add(b).wrapping_add(c)
    }

    #[pvm_contract_macros::method]
    pub fn bit_and(a: U256, b: U256) -> U256 {
        a & b
    }

    #[pvm_contract_macros::method]
    pub fn is_zero(value: U256) -> bool {
        value == U256::ZERO
    }

    #[pvm_contract_macros::method]
    pub fn increment(n: u32) -> u32 {
        n.wrapping_add(1)
    }
}

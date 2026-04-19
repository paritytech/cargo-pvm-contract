#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pallet_revive_uapi::{HostFnImpl as api, StorageFlags};
use ruint::aliases::U256;

#[pvm_contract_macros::contract("Payable.sol", allocator = "pico")]
mod payable {
    use super::*;
    use pvm_contract_types::Address;

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), pvm_contract_types::EmptyError> {
        Ok(())
    }

    #[pvm_contract_macros::method]
    #[pvm_contract_macros::payable]
    pub fn deposit() {
        let caller = get_caller();
        credit(&caller, msg_value());
    }

    #[pvm_contract_macros::method]
    #[pvm_contract_macros::payable]
    pub fn deposit_to(to: Address) {
        let to: [u8; 20] = to.into();
        credit(&to, msg_value());
    }

    #[pvm_contract_macros::method]
    pub fn transfer(to: Address, amount: U256) -> bool {
        let caller = get_caller();
        let from_balance = balance(&caller);
        if from_balance < amount {
            return false;
        }
        let to: [u8; 20] = to.into();
        set_balance(&caller, from_balance - amount);
        credit(&to, amount);
        true
    }

    #[pvm_contract_macros::method]
    pub fn balance_of(who: Address) -> U256 {
        let who: [u8; 20] = who.into();
        balance(&who)
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), pvm_contract_types::EmptyError> {
        Ok(())
    }

    fn msg_value() -> U256 {
        let mut buf = [0u8; 32];
        api::value_transferred(&mut buf);
        U256::from_le_bytes(buf)
    }

    fn get_caller() -> [u8; 20] {
        let mut caller = [0u8; 20];
        api::caller(&mut caller);
        caller
    }

    fn balance_key(addr: &[u8; 20]) -> [u8; 32] {
        let mut input = [0u8; 64];
        input[12..32].copy_from_slice(addr);
        input[63] = 1;

        let mut key = [0u8; 32];
        api::hash_keccak_256(&input, &mut key);
        key
    }

    fn balance(addr: &[u8; 20]) -> U256 {
        let key = balance_key(addr);
        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        match api::get_storage(StorageFlags::empty(), &key, &mut out) {
            Ok(_) => U256::from_be_bytes::<32>(buf),
            Err(_) => U256::ZERO,
        }
    }

    fn set_balance(addr: &[u8; 20], amount: U256) {
        let key = balance_key(addr);
        api::set_storage(StorageFlags::empty(), &key, &amount.to_be_bytes::<32>());
    }

    fn credit(addr: &[u8; 20], amount: U256) {
        let current = balance(addr);
        set_balance(addr, current.saturating_add(amount));
    }
}

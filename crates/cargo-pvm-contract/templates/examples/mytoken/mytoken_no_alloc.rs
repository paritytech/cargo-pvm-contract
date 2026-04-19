#![no_main]
#![no_std]

use pvm_contract_types::{PolkaVmHost as api, StorageFlags};
use ruint::aliases::U256;

#[pvm_contract_macros::contract("MyToken.sol", buffer = 256)]
mod my_token {
    use super::*;
    use pvm_contract_types::Address;

    #[derive(Debug, pvm_contract_macros::SolError)]
    pub struct InsufficientBalance;

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), pvm_contract_types::EmptyError> {
        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn total_supply() -> U256 {
        let key = total_supply_key();
        let mut supply_bytes = [0u8; 32];
        let mut supply_slice = &mut supply_bytes[..];

        match api::get_storage(StorageFlags::empty(), &key, &mut supply_slice) {
            Ok(_) => U256::from_be_bytes::<32>(supply_bytes),
            Err(_) => U256::ZERO,
        }
    }

    #[pvm_contract_macros::method]
    pub fn balance_of(account: Address) -> U256 {
        let account: [u8; 20] = account.into();
        let key = balance_key(&account);
        let mut balance_bytes = [0u8; 32];
        let mut balance_slice = &mut balance_bytes[..];

        match api::get_storage(StorageFlags::empty(), &key, &mut balance_slice) {
            Ok(_) => U256::from_be_bytes::<32>(balance_bytes),
            Err(_) => U256::ZERO,
        }
    }

    #[pvm_contract_macros::method]
    pub fn transfer(to: Address, amount: U256) -> Result<(), InsufficientBalance> {
        let caller = get_caller();
        let sender_balance = balance_of(caller.into());

        if sender_balance < amount {
            return Err(InsufficientBalance);
        }

        let new_sender_balance = sender_balance - amount;
        let recipient_balance = balance_of(to);
        let new_recipient_balance = recipient_balance + amount;

        let to: [u8; 20] = to.into();
        set_balance(&caller, new_sender_balance);
        set_balance(&to, new_recipient_balance);
        emit_transfer(&caller, &to, amount);

        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn mint(to: Address, amount: U256) -> Result<(), InsufficientBalance> {
        let new_recipient_balance = balance_of(to).saturating_add(amount);

        let to: [u8; 20] = to.into();
        set_balance(&to, new_recipient_balance);

        let new_supply = total_supply().saturating_add(amount);
        set_total_supply(new_supply);

        let zero_address = [0u8; 20];
        emit_transfer(&zero_address, &to, amount);
        Ok(())
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), pvm_contract_types::EmptyError> {
        Ok(())
    }

    fn total_supply_key() -> [u8; 32] {
        [0u8; 32]
    }

    fn balance_key(addr: &[u8; 20]) -> [u8; 32] {
        let mut input = [0u8; 64];
        input[12..32].copy_from_slice(addr);
        input[63] = 1;

        let mut key = [0u8; 32];
        api::hash_keccak_256(&input, &mut key);
        key
    }

    fn set_total_supply(amount: U256) {
        let key = total_supply_key();
        api::set_storage(StorageFlags::empty(), &key, &amount.to_be_bytes::<32>());
    }

    fn set_balance(addr: &[u8; 20], amount: U256) {
        let key = balance_key(addr);
        api::set_storage(StorageFlags::empty(), &key, &amount.to_be_bytes::<32>());
    }

    fn get_caller() -> [u8; 20] {
        let mut caller = [0u8; 20];
        api::caller(&mut caller);
        caller
    }

    const TRANSFER_EVENT_SIGNATURE: [u8; 32] = [
        0xdd, 0xf2, 0x52, 0xad, 0x1b, 0xe2, 0xc8, 0x9b, 0x69, 0xc2, 0xb0, 0x68, 0xfc, 0x37, 0x8d,
        0xaa, 0x95, 0x2b, 0xa7, 0xf1, 0x63, 0xc4, 0xa1, 0x16, 0x28, 0xf5, 0x5a, 0x4d, 0xf5, 0x23,
        0xb3, 0xef,
    ];

    fn emit_transfer(from: &[u8; 20], to: &[u8; 20], value: U256) {
        let mut from_topic = [0u8; 32];
        from_topic[12..32].copy_from_slice(from);

        let mut to_topic = [0u8; 32];
        to_topic[12..32].copy_from_slice(to);

        let topics = [TRANSFER_EVENT_SIGNATURE, from_topic, to_topic];
        let data = value.to_be_bytes::<32>();
        api::deposit_event(&topics, &data);
    }
}

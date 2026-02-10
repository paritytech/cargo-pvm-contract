#![no_main]
#![no_std]

#[cfg(not(any(target_arch = "riscv32", target_arch = "riscv64")))]
use pvm_contract_builder_dsl::pallet_revive_uapi::StorageFlags;
#[cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
use pvm_contract_builder_dsl::pallet_revive_uapi::{HostFn as _, HostFnImpl as api, StorageFlags};
use pvm_contract_builder_dsl::ruint::aliases::U256;

#[cfg(not(any(target_arch = "riscv32", target_arch = "riscv64")))]
mod api {
    use super::StorageFlags;

    pub fn hash_keccak_256(_input: &[u8], output: &mut [u8; 32]) {
        *output = [0u8; 32]
    }

    pub fn get_storage(
        _flags: StorageFlags,
        _key: &[u8; 32],
        _output: &mut &mut [u8],
    ) -> Result<(), ()> {
        Err(())
    }

    pub fn set_storage(_flags: StorageFlags, _key: &[u8; 32], _value: &[u8; 32]) {}

    pub fn caller(output: &mut [u8; 20]) {
        *output = [0u8; 20]
    }

    pub fn deposit_event(_topics: &[[u8; 32]], _data: &[u8; 32]) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InsufficientBalance,
}

impl AsRef<[u8]> for Error {
    fn as_ref(&self) -> &[u8] {
        match *self {
            Error::InsufficientBalance => b"InsufficientBalance",
        }
    }
}

pvm_contract_builder_dsl::pvm_contract! {
    no_alloc(buffer = 256);

    constructor fn new() -> Result<(), Error> {
        Ok(())
    }

    fallback fn fallback() -> Result<(), Error> {
        Ok(())
    }

    #[method("totalSupply()", returns(U256))]
    fn total_supply() -> U256 {
        get_total_supply()
    }

    #[method("balanceOf(address)", returns(U256))]
    fn balance_of(account: [u8; 20]) -> U256 {
        get_balance(&account)
    }

    #[method("transfer(address,uint256)", result)]
    fn transfer(to: [u8; 20], amount: U256) -> Result<(), Error> {
        let caller = get_caller();
        let sender_balance = get_balance(&caller);

        if sender_balance < amount {
            return Err(Error::InsufficientBalance);
        }

        let new_sender_balance = sender_balance - amount;
        let recipient_balance = get_balance(&to);
        let new_recipient_balance = recipient_balance + amount;

        set_balance(&caller, new_sender_balance);
        set_balance(&to, new_recipient_balance);
        emit_transfer(&caller, &to, amount);

        Ok(())
    }

    #[method("mint(address,uint256)", result)]
    fn mint(to: [u8; 20], amount: U256) -> Result<(), Error> {
        let new_recipient_balance = get_balance(&to).saturating_add(amount);
        set_balance(&to, new_recipient_balance);

        let new_supply = get_total_supply().saturating_add(amount);
        set_total_supply(new_supply);

        let zero_address = [0u8; 20];
        emit_transfer(&zero_address, &to, amount);
        Ok(())
    }
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

fn get_total_supply() -> U256 {
    let key = total_supply_key();
    let mut supply_bytes = [0u8; 32];
    let mut supply_slice = &mut supply_bytes[..];

    match api::get_storage(StorageFlags::empty(), &key, &mut supply_slice) {
        Ok(_) => U256::from_be_bytes::<32>(supply_bytes),
        Err(_) => U256::ZERO,
    }
}

fn set_total_supply(amount: U256) {
    let key = total_supply_key();
    api::set_storage(StorageFlags::empty(), &key, &amount.to_be_bytes::<32>());
}

fn get_balance(addr: &[u8; 20]) -> U256 {
    let key = balance_key(addr);
    let mut balance_bytes = [0u8; 32];
    let mut balance_slice = &mut balance_bytes[..];

    match api::get_storage(StorageFlags::empty(), &key, &mut balance_slice) {
        Ok(_) => U256::from_be_bytes::<32>(balance_bytes),
        Err(_) => U256::ZERO,
    }
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
    0xdd, 0xf2, 0x52, 0xad, 0x1b, 0xe2, 0xc8, 0x9b, 0x69, 0xc2, 0xb0, 0x68, 0xfc, 0x37, 0x8d, 0xaa,
    0x95, 0x2b, 0xa7, 0xf1, 0x63, 0xc4, 0xa1, 0x16, 0x28, 0xf5, 0x5a, 0x4d, 0xf5, 0x23, 0xb3, 0xef,
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

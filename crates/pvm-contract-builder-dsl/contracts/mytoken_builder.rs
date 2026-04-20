#![cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
#![no_main]
#![no_std]

use pvm_contract_types::{HostApi as _, PolkaVmHost, ReturnFlags, StorageFlags};
use pvm_contract_builder_dsl::{ContractBuilder, solidity_selector};
use pvm_contract_types::{SolDecode, SolEncode, StaticEncodedLen};
use ruint::aliases::U256;

const TOTAL_SUPPLY_SELECTOR: [u8; 4] = solidity_selector("totalSupply()");
const BALANCE_OF_SELECTOR: [u8; 4] = solidity_selector("balanceOf(address)");
const TRANSFER_SELECTOR: [u8; 4] = solidity_selector("transfer(address,uint256)");
const MINT_SELECTOR: [u8; 4] = solidity_selector("mint(address,uint256)");

const TRANSFER_EVENT_SIGNATURE: [u8; 32] = [
    0xdd, 0xf2, 0x52, 0xad, 0x1b, 0xe2, 0xc8, 0x9b, 0x69, 0xc2, 0xb0, 0x68, 0xfc, 0x37, 0x8d, 0xaa,
    0x95, 0x2b, 0xa7, 0xf1, 0x63, 0xc4, 0xa1, 0x16, 0x28, 0xf5, 0x5a, 0x4d, 0xf5, 0x23, 0xb3, 0xef,
];

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe {
        core::arch::asm!("unimp");
        core::hint::unreachable_unchecked()
    }
}

use pvm_contract_types::PolkaVmHost as api;

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

#[unsafe(no_mangle)]
#[polkavm_derive::polkavm_export]
pub extern "C" fn deploy() {}

#[unsafe(no_mangle)]
#[polkavm_derive::polkavm_export]
pub extern "C" fn call() {
    ContractBuilder::new()
        .method(TOTAL_SUPPLY_SELECTOR, total_supply_handler)
        .method(BALANCE_OF_SELECTOR, balance_of_handler)
        .method(TRANSFER_SELECTOR, transfer_handler)
        .method(MINT_SELECTOR, mint_handler)
        .dispatch::<PolkaVmHost, 256>()
}

fn total_supply_handler(_input: &[u8]) {
    let key = total_supply_key();
    let mut supply_bytes = [0u8; 32];
    let mut supply_slice = &mut supply_bytes[..];

    let result = match api::get_storage(StorageFlags::empty(), &key, &mut supply_slice) {
        Ok(_) => U256::from_be_bytes::<32>(supply_bytes),
        Err(_) => U256::ZERO,
    };
    let mut buf = [0u8; <U256 as StaticEncodedLen>::ENCODED_SIZE];
    result.encode_to(&mut buf);
    PolkaVmHost::return_value(ReturnFlags::empty(), &buf);
}

fn balance_of_handler(input: &[u8]) {
    let account = <[u8; 20]>::decode_at(input, 0);
    let key = balance_key(&account);
    let mut balance_bytes = [0u8; 32];
    let mut balance_slice = &mut balance_bytes[..];

    let result = match api::get_storage(StorageFlags::empty(), &key, &mut balance_slice) {
        Ok(_) => U256::from_be_bytes::<32>(balance_bytes),
        Err(_) => U256::ZERO,
    };
    let mut buf = [0u8; <U256 as StaticEncodedLen>::ENCODED_SIZE];
    result.encode_to(&mut buf);
    PolkaVmHost::return_value(ReturnFlags::empty(), &buf);
}

fn transfer_handler(input: &[u8]) {
    let to = <[u8; 20]>::decode_at(input, 0);
    let amount = U256::decode_at(input, <[u8; 20] as StaticEncodedLen>::ENCODED_SIZE);

    let caller = get_caller();
    let sender_key = balance_key(&caller);
    let mut sender_balance_bytes = [0u8; 32];
    let mut sender_balance_slice = &mut sender_balance_bytes[..];
    let sender_balance = match api::get_storage(
        StorageFlags::empty(),
        &sender_key,
        &mut sender_balance_slice,
    ) {
        Ok(_) => U256::from_be_bytes::<32>(sender_balance_bytes),
        Err(_) => U256::ZERO,
    };

    if sender_balance < amount {
        PolkaVmHost::return_value(ReturnFlags::REVERT, Error::InsufficientBalance.as_ref());
    }

    let new_sender_balance = sender_balance - amount;
    let recipient_key = balance_key(&to);
    let mut recipient_balance_bytes = [0u8; 32];
    let mut recipient_balance_slice = &mut recipient_balance_bytes[..];
    let recipient_balance = match api::get_storage(
        StorageFlags::empty(),
        &recipient_key,
        &mut recipient_balance_slice,
    ) {
        Ok(_) => U256::from_be_bytes::<32>(recipient_balance_bytes),
        Err(_) => U256::ZERO,
    };
    let new_recipient_balance = recipient_balance + amount;

    set_balance(&caller, new_sender_balance);
    set_balance(&to, new_recipient_balance);
    emit_transfer(&caller, &to, amount);
}

fn mint_handler(input: &[u8]) {
    let to = <[u8; 20]>::decode_at(input, 0);
    let amount = U256::decode_at(input, <[u8; 20] as StaticEncodedLen>::ENCODED_SIZE);

    let recipient_key = balance_key(&to);
    let mut recipient_balance_bytes = [0u8; 32];
    let mut recipient_balance_slice = &mut recipient_balance_bytes[..];
    let recipient_balance = match api::get_storage(
        StorageFlags::empty(),
        &recipient_key,
        &mut recipient_balance_slice,
    ) {
        Ok(_) => U256::from_be_bytes::<32>(recipient_balance_bytes),
        Err(_) => U256::ZERO,
    };
    let new_recipient_balance = recipient_balance.saturating_add(amount);
    set_balance(&to, new_recipient_balance);

    let supply_key = total_supply_key();
    let mut supply_bytes = [0u8; 32];
    let mut supply_slice = &mut supply_bytes[..];
    let supply = match api::get_storage(StorageFlags::empty(), &supply_key, &mut supply_slice) {
        Ok(_) => U256::from_be_bytes::<32>(supply_bytes),
        Err(_) => U256::ZERO,
    };
    let new_supply = supply.saturating_add(amount);
    set_total_supply(new_supply);

    let zero_address = [0u8; 20];
    emit_transfer(&zero_address, &to, amount);
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

fn emit_transfer(from: &[u8; 20], to: &[u8; 20], value: U256) {
    let mut from_topic = [0u8; 32];
    from_topic[12..32].copy_from_slice(from);

    let mut to_topic = [0u8; 32];
    to_topic[12..32].copy_from_slice(to);

    let topics = [TRANSFER_EVENT_SIGNATURE, from_topic, to_topic];
    let data = value.to_be_bytes::<32>();
    api::deposit_event(&topics, &data);
}

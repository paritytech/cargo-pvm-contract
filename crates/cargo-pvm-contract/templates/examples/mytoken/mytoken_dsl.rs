#![cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
#![no_main]
#![no_std]

use pvm_contract_builder_dsl::{
    ContractBuilder, HandlerResult, assert_non_payable_deploy, solidity_selector,
};
use pvm_contract_builder_dsl::pvm_contract_types::{
    Address, Host, HostApi, SolEncode, StaticDecode, StaticEncodedLen, StorageFlags,
};
use pvm_contract_builder_dsl::ruint::aliases::U256;

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

#[unsafe(no_mangle)]
#[polkavm_derive::polkavm_export]
pub extern "C" fn deploy() {
    assert_non_payable_deploy(&Host::new());
}

#[unsafe(no_mangle)]
#[polkavm_derive::polkavm_export]
pub extern "C" fn call() {
    let host = Host::new();
    ContractBuilder::new()
        .method(TOTAL_SUPPLY_SELECTOR, total_supply_handler)
        .method(BALANCE_OF_SELECTOR, balance_of_handler)
        .method(TRANSFER_SELECTOR, transfer_handler)
        .method(MINT_SELECTOR, mint_handler)
        .dispatch_impl::<256>(&host);
}

fn total_supply_handler(host: &Host, _input: &[u8], output: &mut [u8]) -> HandlerResult {
    let key = total_supply_key();
    let mut supply_bytes = [0u8; 32];
    let mut supply_slice = &mut supply_bytes[..];

    let result = match host.get_storage(StorageFlags::empty(), &key, &mut supply_slice) {
        Ok(_) => U256::from_be_bytes::<32>(supply_bytes),
        Err(_) => U256::ZERO,
    };
    let len = <U256 as StaticEncodedLen>::ENCODED_SIZE;
    result.encode_to(&mut output[..len]);
    HandlerResult::Ok(len)
}

fn balance_of_handler(host: &Host, input: &[u8], output: &mut [u8]) -> HandlerResult {
    let account = unsafe { <Address>::decode_unchecked(input, 0) };
    let account: [u8; 20] = account.into();
    let key = balance_key(host, &account);
    let mut balance_bytes = [0u8; 32];
    let mut balance_slice = &mut balance_bytes[..];

    let result = match host.get_storage(StorageFlags::empty(), &key, &mut balance_slice) {
        Ok(_) => U256::from_be_bytes::<32>(balance_bytes),
        Err(_) => U256::ZERO,
    };
    let len = <U256 as StaticEncodedLen>::ENCODED_SIZE;
    result.encode_to(&mut output[..len]);
    HandlerResult::Ok(len)
}

fn transfer_handler(host: &Host, input: &[u8], output: &mut [u8]) -> HandlerResult {
    let to = unsafe { <Address>::decode_unchecked(input, 0) };
    let to: [u8; 20] = to.into();
    let amount =
        unsafe { U256::decode_unchecked(input, <Address as StaticEncodedLen>::ENCODED_SIZE) };

    let caller = get_caller(host);
    let sender_key = balance_key(host, &caller);
    let mut sender_balance_bytes = [0u8; 32];
    let mut sender_balance_slice = &mut sender_balance_bytes[..];
    let sender_balance = match host.get_storage(
        StorageFlags::empty(),
        &sender_key,
        &mut sender_balance_slice,
    ) {
        Ok(_) => U256::from_be_bytes::<32>(sender_balance_bytes),
        Err(_) => U256::ZERO,
    };

    if sender_balance < amount {
        let msg = b"InsufficientBalance";
        output[..msg.len()].copy_from_slice(msg);
        return HandlerResult::Revert(msg.len());
    }

    let new_sender_balance = sender_balance - amount;
    let recipient_key = balance_key(host, &to);
    let mut recipient_balance_bytes = [0u8; 32];
    let mut recipient_balance_slice = &mut recipient_balance_bytes[..];
    let recipient_balance = match host.get_storage(
        StorageFlags::empty(),
        &recipient_key,
        &mut recipient_balance_slice,
    ) {
        Ok(_) => U256::from_be_bytes::<32>(recipient_balance_bytes),
        Err(_) => U256::ZERO,
    };
    let new_recipient_balance = recipient_balance + amount;

    set_balance(host, &caller, new_sender_balance);
    set_balance(host, &to, new_recipient_balance);
    emit_transfer(host, &caller, &to, amount);
    HandlerResult::Ok(0)
}

fn mint_handler(host: &Host, input: &[u8], _output: &mut [u8]) -> HandlerResult {
    let to = unsafe { <Address>::decode_unchecked(input, 0) };
    let to: [u8; 20] = to.into();
    let amount =
        unsafe { U256::decode_unchecked(input, <Address as StaticEncodedLen>::ENCODED_SIZE) };

    let recipient_key = balance_key(host, &to);
    let mut recipient_balance_bytes = [0u8; 32];
    let mut recipient_balance_slice = &mut recipient_balance_bytes[..];
    let recipient_balance = match host.get_storage(
        StorageFlags::empty(),
        &recipient_key,
        &mut recipient_balance_slice,
    ) {
        Ok(_) => U256::from_be_bytes::<32>(recipient_balance_bytes),
        Err(_) => U256::ZERO,
    };
    let new_recipient_balance = recipient_balance.saturating_add(amount);
    set_balance(host, &to, new_recipient_balance);

    let supply_key = total_supply_key();
    let mut supply_bytes = [0u8; 32];
    let mut supply_slice = &mut supply_bytes[..];
    let supply = match host.get_storage(StorageFlags::empty(), &supply_key, &mut supply_slice) {
        Ok(_) => U256::from_be_bytes::<32>(supply_bytes),
        Err(_) => U256::ZERO,
    };
    let new_supply = supply.saturating_add(amount);
    set_total_supply(host, new_supply);

    emit_transfer(host, &[0u8; 20], &to, amount);
    HandlerResult::Ok(0)
}

fn total_supply_key() -> [u8; 32] {
    [0u8; 32]
}

fn balance_key(host: &Host, addr: &[u8; 20]) -> [u8; 32] {
    let mut input = [0u8; 64];
    input[12..32].copy_from_slice(addr);
    input[63] = 1;

    let mut key = [0u8; 32];
    host.hash_keccak_256(&input, &mut key);
    key
}

fn set_total_supply(host: &Host, amount: U256) {
    let key = total_supply_key();
    host.set_storage(StorageFlags::empty(), &key, &amount.to_be_bytes::<32>());
}

fn set_balance(host: &Host, addr: &[u8; 20], amount: U256) {
    let key = balance_key(host, addr);
    host.set_storage(StorageFlags::empty(), &key, &amount.to_be_bytes::<32>());
}

fn get_caller(host: &Host) -> [u8; 20] {
    let mut caller = [0u8; 20];
    host.caller(&mut caller);
    caller
}

fn emit_transfer(host: &Host, from: &[u8; 20], to: &[u8; 20], value: U256) {
    let mut from_topic = [0u8; 32];
    from_topic[12..32].copy_from_slice(from);

    let mut to_topic = [0u8; 32];
    to_topic[12..32].copy_from_slice(to);

    let topics = [TRANSFER_EVENT_SIGNATURE, from_topic, to_topic];
    let data = value.to_be_bytes::<32>();
    host.deposit_event(&topics, &data);
}

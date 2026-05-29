#![no_main]
#![no_std]

use pvm_contract_builder_dsl::{
    ContractBuilder, HandlerResult, assert_non_payable_deploy, solidity_selector,
};
use pvm_contract_sdk::{
    Address, Host, HostApi, SolDecode, SolEncode, SolRevert, StaticEncodedLen, StorageFlags, U256,
};

#[global_allocator]
static mut ALLOC: picoalloc::Mutex<picoalloc::Allocator<picoalloc::ArrayPointer<1024>>> = {
    static mut ARRAY: picoalloc::Array<1024> = picoalloc::Array([0u8; 1024]);

    picoalloc::Mutex::new(picoalloc::Allocator::new(unsafe {
        picoalloc::ArrayPointer::new(&raw mut ARRAY)
    }))
};

const TOTAL_SUPPLY_SELECTOR: [u8; 4] = solidity_selector("totalSupply()");
const BALANCE_OF_SELECTOR: [u8; 4] = solidity_selector("balanceOf(address)");
const TRANSFER_SELECTOR: [u8; 4] = solidity_selector("transfer(address,uint256)");
const MINT_SELECTOR: [u8; 4] = solidity_selector("mint(address,uint256)");

// Events use `#[derive(SolEvent)]` even in the DSL (manual-dispatch) path:
// the derive is independent of how methods are routed and computes the topic
// hash at compile time, replacing the hand-pasted keccak constant and manual
// topic packing. Raw `host.deposit_event(...)` is still available (see
// specs/builder-dsl.md) for advanced cases.
#[derive(pvm_contract_sdk::SolEvent)]
struct Transfer {
    #[indexed]
    from: Address,
    #[indexed]
    to: Address,
    value: U256,
}

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe {
        core::arch::asm!("unimp");
        core::hint::unreachable_unchecked()
    }
}

#[derive(Debug, pvm_contract_sdk::SolError)]
pub struct InsufficientBalance;

pvm_contract_sdk::sol_revert_enum! {
    pub enum TokenError {
        InsufficientBalance(InsufficientBalance),
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
    result.encode_to(&mut output[..<U256 as StaticEncodedLen>::ENCODED_SIZE]);
    HandlerResult::Ok(<U256 as StaticEncodedLen>::ENCODED_SIZE)
}

fn balance_of_handler(host: &Host, input: &[u8], output: &mut [u8]) -> HandlerResult {
    let account = <Address>::decode_at(input, 0).unwrap();
    let account: [u8; 20] = account.into();
    let key = balance_key(host, &account);
    let mut balance_bytes = [0u8; 32];
    let mut balance_slice = &mut balance_bytes[..];

    let result = match host.get_storage(StorageFlags::empty(), &key, &mut balance_slice) {
        Ok(_) => U256::from_be_bytes::<32>(balance_bytes),
        Err(_) => U256::ZERO,
    };
    result.encode_to(&mut output[..<U256 as StaticEncodedLen>::ENCODED_SIZE]);
    HandlerResult::Ok(<U256 as StaticEncodedLen>::ENCODED_SIZE)
}

fn transfer_handler(host: &Host, input: &[u8], output: &mut [u8]) -> HandlerResult {
    let to = <Address>::decode_at(input, 0).unwrap();
    let to: [u8; 20] = to.into();
    let amount = U256::decode_at(input, <Address as StaticEncodedLen>::ENCODED_SIZE).unwrap();

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
        let n = SolRevert::revert_data(&InsufficientBalance, output);
        return HandlerResult::Revert(n);
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
    let to = <Address>::decode_at(input, 0).unwrap();
    let to: [u8; 20] = to.into();
    let amount = U256::decode_at(input, <Address as StaticEncodedLen>::ENCODED_SIZE).unwrap();

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

    let zero_address = [0u8; 20];
    emit_transfer(host, &zero_address, &to, amount);
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
    Transfer {
        from: Address(*from),
        to: Address(*to),
        value,
    }
    .emit(host);
}

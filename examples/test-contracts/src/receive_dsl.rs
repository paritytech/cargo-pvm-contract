#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_builder_dsl::pvm_contract_types::{
    Host, HostApi, PolkaVmHost, SolEncode, StaticEncodedLen, StorageFlags,
};
use pvm_contract_builder_dsl::ruint::aliases::U256;
use pvm_contract_builder_dsl::{solidity_selector, ContractBuilder, HandlerResult};

#[global_allocator]
static mut ALLOC: picoalloc::Mutex<picoalloc::Allocator<picoalloc::ArrayPointer<1024>>> = {
    static mut ARRAY: picoalloc::Array<1024> = picoalloc::Array([0u8; 1024]);

    picoalloc::Mutex::new(picoalloc::Allocator::new(unsafe {
        picoalloc::ArrayPointer::new(&raw mut ARRAY)
    }))
};

const TOTAL_RECEIVED_SELECTOR: [u8; 4] = solidity_selector("totalReceived()");
const RECEIVE_COUNT_SELECTOR: [u8; 4] = solidity_selector("receiveCount()");

const TOTAL_KEY: [u8; 32] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
];
const COUNT_KEY: [u8; 32] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1,
];

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe {
        core::arch::asm!("unimp");
        core::hint::unreachable_unchecked()
    }
}

fn read_u256<H: HostApi>(host: &H, key: &[u8; 32]) -> U256 {
    let mut buf = [0u8; 32];
    let mut out = &mut buf[..];
    match host.get_storage(StorageFlags::empty(), key, &mut out) {
        Ok(_) => U256::from_be_bytes::<32>(buf),
        Err(_) => U256::ZERO,
    }
}

fn write_u256<H: HostApi>(host: &H, key: &[u8; 32], value: U256) {
    host.set_storage(StorageFlags::empty(), key, &value.to_be_bytes::<32>());
}

fn receive_handler<H: HostApi>(host: &H, _input: &[u8], _output: &mut [u8]) -> HandlerResult {
    let mut value_buf = [0u8; 32];
    host.value_transferred(&mut value_buf);
    let value = U256::from_le_bytes(value_buf);

    let total = read_u256(host, &TOTAL_KEY);
    write_u256(host, &TOTAL_KEY, total.saturating_add(value));

    let count = read_u256(host, &COUNT_KEY);
    write_u256(host, &COUNT_KEY, count.saturating_add(U256::from(1u8)));

    HandlerResult::Ok(0)
}

fn total_received_handler<H: HostApi>(host: &H, _input: &[u8], output: &mut [u8]) -> HandlerResult {
    let total = read_u256(host, &TOTAL_KEY);
    let len = <U256 as StaticEncodedLen>::ENCODED_SIZE;
    total.encode_to(&mut output[..len]);
    HandlerResult::Ok(len)
}

fn receive_count_handler<H: HostApi>(host: &H, _input: &[u8], output: &mut [u8]) -> HandlerResult {
    let count = read_u256(host, &COUNT_KEY);
    let len = <U256 as StaticEncodedLen>::ENCODED_SIZE;
    count.encode_to(&mut output[..len]);
    HandlerResult::Ok(len)
}

#[unsafe(no_mangle)]
#[polkavm_derive::polkavm_export]
pub extern "C" fn deploy() {}

#[unsafe(no_mangle)]
#[polkavm_derive::polkavm_export]
pub extern "C" fn call() {
    let host = Host::new();
    ContractBuilder::new()
        .method(TOTAL_RECEIVED_SELECTOR, total_received_handler)
        .method(RECEIVE_COUNT_SELECTOR, receive_count_handler)
        .receive(receive_handler)
        .dispatch_impl::<256>(&host);
}

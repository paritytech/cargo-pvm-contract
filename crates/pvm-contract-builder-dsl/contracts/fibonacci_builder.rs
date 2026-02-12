#![cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
#![no_main]
#![no_std]

use pallet_revive_uapi::{HostFn as _, HostFnImpl, ReturnFlags};
use pvm_contract_builder_dsl::ContractBuilder;
use pvm_contract_builder_dsl::solidity_selector;
use pvm_contract_types::{SolDecode, SolEncode, StaticEncodedLen};

const FIBONACCI_SELECTOR: [u8; 4] = solidity_selector("fibonacci(uint32)");

#[panic_handler]
fn panic(_info: &core::panic::PanicInfo) -> ! {
    unsafe {
        core::arch::asm!("unimp");
        core::hint::unreachable_unchecked()
    }
}

#[unsafe(no_mangle)]
#[polkavm_derive::polkavm_export]
pub extern "C" fn deploy() {}

#[unsafe(no_mangle)]
#[polkavm_derive::polkavm_export]
pub extern "C" fn call() {
    ContractBuilder::new()
        .method(FIBONACCI_SELECTOR, fibonacci_handler)
        .dispatch::<HostFnImpl, 256>()
}

fn fibonacci_handler(input: &[u8]) {
    let n = u32::decode_at(input, 0);
    let result = fibonacci(n);
    let mut buf = [0u8; <u32 as StaticEncodedLen>::ENCODED_SIZE];
    result.encode_to(&mut buf);
    HostFnImpl::return_value(ReturnFlags::empty(), &buf);
}

fn fibonacci(n: u32) -> u32 {
    if n <= 1 {
        n
    } else {
        let mut a = 0u32;
        let mut b = 1u32;
        for _ in 2..=n {
            let c = a.wrapping_add(b);
            a = b;
            b = c;
        }
        b
    }
}

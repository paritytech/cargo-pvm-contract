#![cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
#![no_main]
#![no_std]

use pvm_contract_builder_dsl::pvm_contract_types::{
    Host, SolEncode, SolError, StaticDecode, StaticEncodedLen,
};
use pvm_contract_builder_dsl::{ContractBuilder, HandlerResult, solidity_selector};

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
    let host = Host::new();
    ContractBuilder::new()
        .method(FIBONACCI_SELECTOR, fibonacci_handler)
        .dispatch_impl::<256>(&host);
}

fn fibonacci_handler(_host: &Host, input: &[u8], output: &mut [u8]) -> HandlerResult {
    let n = unsafe { u32::decode_unchecked(input, 0) };
    let result = fibonacci(n);
    let len = <u32 as StaticEncodedLen>::ENCODED_SIZE;
    result.encode_to(&mut output[..len]);
    HandlerResult::Ok(len)
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

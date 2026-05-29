#![cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
#![no_main]
#![no_std]

use pvm_contract_builder_dsl::{
    ContractBuilder, HandlerResult, assert_non_payable_deploy, solidity_selector,
};
use pvm_contract_builder_dsl::pvm_contract_types::{
    Host, SolEncode, StaticDecode, StaticEncodedLen,
};
use pvm_contract_builder_dsl::ruint::aliases::U256;

const ADD_SELECTOR: [u8; 4] = solidity_selector("add(uint32,uint32)");
const MULTIPLY_SELECTOR: [u8; 4] = solidity_selector("multiply(uint64,uint64)");
const IS_EVEN_SELECTOR: [u8; 4] = solidity_selector("isEven(uint32)");
const NEGATE_SELECTOR: [u8; 4] = solidity_selector("negate(uint256)");
const MAX_SELECTOR: [u8; 4] = solidity_selector("max(uint256,uint256)");
const HASH_SELECTOR: [u8; 4] = solidity_selector("hash(address)");
const SUM3_SELECTOR: [u8; 4] = solidity_selector("sum3(uint32,uint32,uint32)");
const BIT_AND_SELECTOR: [u8; 4] = solidity_selector("bitAnd(uint256,uint256)");
const IS_ZERO_SELECTOR: [u8; 4] = solidity_selector("isZero(uint256)");
const INCREMENT_SELECTOR: [u8; 4] = solidity_selector("increment(uint32)");

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
        .method(ADD_SELECTOR, add_handler)
        .method(MULTIPLY_SELECTOR, multiply_handler)
        .method(IS_EVEN_SELECTOR, is_even_handler)
        .method(NEGATE_SELECTOR, negate_handler)
        .method(MAX_SELECTOR, max_handler)
        .method(HASH_SELECTOR, hash_handler)
        .method(SUM3_SELECTOR, sum3_handler)
        .method(BIT_AND_SELECTOR, bit_and_handler)
        .method(IS_ZERO_SELECTOR, is_zero_handler)
        .method(INCREMENT_SELECTOR, increment_handler)
        .dispatch_impl::<256>(&host);
}

fn add_handler(_host: &Host, input: &[u8], output: &mut [u8]) -> HandlerResult {
    let a = unsafe { u32::decode_unchecked(input, 0) };
    let b = unsafe { u32::decode_unchecked(input, <u32 as StaticEncodedLen>::ENCODED_SIZE) };
    let result = a.wrapping_add(b);
    let len = <u32 as StaticEncodedLen>::ENCODED_SIZE;
    result.encode_to(&mut output[..len]);
    HandlerResult::Ok(len)
}

fn multiply_handler(_host: &Host, input: &[u8], output: &mut [u8]) -> HandlerResult {
    let a = unsafe { u64::decode_unchecked(input, 0) };
    let b = unsafe { u64::decode_unchecked(input, <u64 as StaticEncodedLen>::ENCODED_SIZE) };
    let result = a.wrapping_mul(b);
    let len = <u64 as StaticEncodedLen>::ENCODED_SIZE;
    result.encode_to(&mut output[..len]);
    HandlerResult::Ok(len)
}

fn is_even_handler(_host: &Host, input: &[u8], output: &mut [u8]) -> HandlerResult {
    let n = unsafe { u32::decode_unchecked(input, 0) };
    let result = (n & 1) == 0;
    let len = <bool as StaticEncodedLen>::ENCODED_SIZE;
    result.encode_to(&mut output[..len]);
    HandlerResult::Ok(len)
}

fn negate_handler(_host: &Host, input: &[u8], output: &mut [u8]) -> HandlerResult {
    let value = unsafe { U256::decode_unchecked(input, 0) };
    let result = !value + U256::from(1u8);
    let len = <U256 as StaticEncodedLen>::ENCODED_SIZE;
    result.encode_to(&mut output[..len]);
    HandlerResult::Ok(len)
}

fn max_handler(_host: &Host, input: &[u8], output: &mut [u8]) -> HandlerResult {
    let a = unsafe { U256::decode_unchecked(input, 0) };
    let b = unsafe { U256::decode_unchecked(input, <U256 as StaticEncodedLen>::ENCODED_SIZE) };
    let result = if a > b { a } else { b };
    let len = <U256 as StaticEncodedLen>::ENCODED_SIZE;
    result.encode_to(&mut output[..len]);
    HandlerResult::Ok(len)
}

fn hash_handler(_host: &Host, input: &[u8], output: &mut [u8]) -> HandlerResult {
    let account = unsafe { <[u8; 20]>::decode_unchecked(input, 0) };
    let mut bytes = [0u8; 32];
    bytes[12..].copy_from_slice(&account);
    let result = U256::from_be_bytes::<32>(bytes);
    let len = <U256 as StaticEncodedLen>::ENCODED_SIZE;
    result.encode_to(&mut output[..len]);
    HandlerResult::Ok(len)
}

fn sum3_handler(_host: &Host, input: &[u8], output: &mut [u8]) -> HandlerResult {
    let a = unsafe { u32::decode_unchecked(input, 0) };
    let b = unsafe { u32::decode_unchecked(input, <u32 as StaticEncodedLen>::ENCODED_SIZE) };
    let c = unsafe { u32::decode_unchecked(input, <u32 as StaticEncodedLen>::ENCODED_SIZE * 2) };
    let result = a.wrapping_add(b).wrapping_add(c);
    let len = <u32 as StaticEncodedLen>::ENCODED_SIZE;
    result.encode_to(&mut output[..len]);
    HandlerResult::Ok(len)
}

fn bit_and_handler(_host: &Host, input: &[u8], output: &mut [u8]) -> HandlerResult {
    let a = unsafe { U256::decode_unchecked(input, 0) };
    let b = unsafe { U256::decode_unchecked(input, <U256 as StaticEncodedLen>::ENCODED_SIZE) };
    let result = a & b;
    let len = <U256 as StaticEncodedLen>::ENCODED_SIZE;
    result.encode_to(&mut output[..len]);
    HandlerResult::Ok(len)
}

fn is_zero_handler(_host: &Host, input: &[u8], output: &mut [u8]) -> HandlerResult {
    let value = unsafe { U256::decode_unchecked(input, 0) };
    let result = value == U256::ZERO;
    let len = <bool as StaticEncodedLen>::ENCODED_SIZE;
    result.encode_to(&mut output[..len]);
    HandlerResult::Ok(len)
}

fn increment_handler(_host: &Host, input: &[u8], output: &mut [u8]) -> HandlerResult {
    let n = unsafe { u32::decode_unchecked(input, 0) };
    let result = n.wrapping_add(1);
    let len = <u32 as StaticEncodedLen>::ENCODED_SIZE;
    result.encode_to(&mut output[..len]);
    HandlerResult::Ok(len)
}

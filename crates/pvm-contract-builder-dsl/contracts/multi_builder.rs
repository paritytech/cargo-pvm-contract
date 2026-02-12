#![cfg(any(target_arch = "riscv32", target_arch = "riscv64"))]
#![no_main]
#![no_std]

use pallet_revive_uapi::{HostFn as _, HostFnImpl, ReturnFlags};
use pvm_contract_builder_dsl::{ContractBuilder, solidity_selector};
use pvm_contract_types::{SolDecode, SolEncode, StaticEncodedLen};
use ruint::aliases::U256;

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
pub extern "C" fn deploy() {}

#[unsafe(no_mangle)]
#[polkavm_derive::polkavm_export]
pub extern "C" fn call() {
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
        .dispatch::<HostFnImpl, 256>()
}

fn add_handler(input: &[u8]) {
    let a = u32::decode_at(input, 0);
    let b = u32::decode_at(input, <u32 as StaticEncodedLen>::ENCODED_SIZE);
    let result = a.wrapping_add(b);
    let mut buf = [0u8; <u32 as StaticEncodedLen>::ENCODED_SIZE];
    result.encode_to(&mut buf);
    HostFnImpl::return_value(ReturnFlags::empty(), &buf);
}

fn multiply_handler(input: &[u8]) {
    let a = u64::decode_at(input, 0);
    let b = u64::decode_at(input, <u64 as StaticEncodedLen>::ENCODED_SIZE);
    let result = a.wrapping_mul(b);
    let mut buf = [0u8; <u64 as StaticEncodedLen>::ENCODED_SIZE];
    result.encode_to(&mut buf);
    HostFnImpl::return_value(ReturnFlags::empty(), &buf);
}

fn is_even_handler(input: &[u8]) {
    let n = u32::decode_at(input, 0);
    let result = (n & 1) == 0;
    let mut buf = [0u8; <bool as StaticEncodedLen>::ENCODED_SIZE];
    result.encode_to(&mut buf);
    HostFnImpl::return_value(ReturnFlags::empty(), &buf);
}

fn negate_handler(input: &[u8]) {
    let value = U256::decode_at(input, 0);
    let result = !value + U256::from(1u8);
    let mut buf = [0u8; <U256 as StaticEncodedLen>::ENCODED_SIZE];
    result.encode_to(&mut buf);
    HostFnImpl::return_value(ReturnFlags::empty(), &buf);
}

fn max_handler(input: &[u8]) {
    let a = U256::decode_at(input, 0);
    let b = U256::decode_at(input, <U256 as StaticEncodedLen>::ENCODED_SIZE);
    let result = if a > b { a } else { b };
    let mut buf = [0u8; <U256 as StaticEncodedLen>::ENCODED_SIZE];
    result.encode_to(&mut buf);
    HostFnImpl::return_value(ReturnFlags::empty(), &buf);
}

fn hash_handler(input: &[u8]) {
    let account = <[u8; 20]>::decode_at(input, 0);
    let mut bytes = [0u8; 32];
    bytes[12..].copy_from_slice(&account);
    let result = U256::from_be_bytes::<32>(bytes);
    let mut buf = [0u8; <U256 as StaticEncodedLen>::ENCODED_SIZE];
    result.encode_to(&mut buf);
    HostFnImpl::return_value(ReturnFlags::empty(), &buf);
}

fn sum3_handler(input: &[u8]) {
    let a = u32::decode_at(input, 0);
    let b = u32::decode_at(input, <u32 as StaticEncodedLen>::ENCODED_SIZE);
    let c = u32::decode_at(input, <u32 as StaticEncodedLen>::ENCODED_SIZE * 2);
    let result = a.wrapping_add(b).wrapping_add(c);
    let mut buf = [0u8; <u32 as StaticEncodedLen>::ENCODED_SIZE];
    result.encode_to(&mut buf);
    HostFnImpl::return_value(ReturnFlags::empty(), &buf);
}

fn bit_and_handler(input: &[u8]) {
    let a = U256::decode_at(input, 0);
    let b = U256::decode_at(input, <U256 as StaticEncodedLen>::ENCODED_SIZE);
    let result = a & b;
    let mut buf = [0u8; <U256 as StaticEncodedLen>::ENCODED_SIZE];
    result.encode_to(&mut buf);
    HostFnImpl::return_value(ReturnFlags::empty(), &buf);
}

fn is_zero_handler(input: &[u8]) {
    let value = U256::decode_at(input, 0);
    let result = value == U256::ZERO;
    let mut buf = [0u8; <bool as StaticEncodedLen>::ENCODED_SIZE];
    result.encode_to(&mut buf);
    HostFnImpl::return_value(ReturnFlags::empty(), &buf);
}

fn increment_handler(input: &[u8]) {
    let n = u32::decode_at(input, 0);
    let result = n.wrapping_add(1);
    let mut buf = [0u8; <u32 as StaticEncodedLen>::ENCODED_SIZE];
    result.encode_to(&mut buf);
    HostFnImpl::return_value(ReturnFlags::empty(), &buf);
}

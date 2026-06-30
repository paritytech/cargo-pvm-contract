#![cfg(not(feature = "abi-gen"))]
//! Native unit tests exercising macro-generated `route()` + `Router` impl
//! against `MockHost`. Proves that contract dispatch is host-agnostic and
//! fully runnable off-target.
//!
//! These tests bypass `call()` / `deploy()` (riscv64-only) and invoke the
//! generated `route()` directly. On host targets, dispatch arms call
//! `host.return_value(...)` which captures into the `MockHost` rather than
//! diverging — the test reads the captured [`ReturnValue`] (flags + data)
//! after `route()` returns to inspect the contract's response.

use pvm_contract_types::{
    Address, MockHost, MockHostBuilder, ReturnFlags, Router, SolDecode, SolEncode, StaticEncodedLen,
};
use ruint::aliases::U256;

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract]
mod my_token {
    use super::*;

    pub struct MyContract;

    impl MyContract {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn double(&self, n: u64) -> u64 {
            n.wrapping_mul(2)
        }

        #[pvm_contract_macros::method]
        pub fn noop(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn balance_of(&self, _account: Address) -> U256 {
            U256::from(42u64)
        }
    }
}

fn selector(sig: &str) -> [u8; 4] {
    pvm_contract_types::const_selector(sig)
}

fn encode_u64(n: u64) -> Vec<u8> {
    let mut buf = vec![0u8; <u64 as StaticEncodedLen>::ENCODED_SIZE];
    n.encode_to(&mut buf);
    buf
}

fn encode_address(addr: Address) -> Vec<u8> {
    let mut buf = vec![0u8; <Address as StaticEncodedLen>::ENCODED_SIZE];
    addr.encode_to(&mut buf);
    buf
}

fn new_contract() -> (my_token::MyContract, MockHost) {
    let mock = MockHostBuilder::new().build();
    let contract = my_token::MyContract::with_host(mock.clone());
    (contract, mock)
}

#[test]
fn route_matches_selector_and_returns_encoded_u64() {
    let (mut contract, mock) = new_contract();
    let sel = selector("double(uint64)");
    let input = encode_u64(21);

    let outcome = my_token::route(&mut contract, sel, &input);
    assert_eq!(outcome, Some(()));

    let rv = mock
        .take_return_value()
        .expect("contract called return_value");
    assert_eq!(rv.flags, ReturnFlags::empty());
    let returned = u64::decode_at(&rv.data, 0).unwrap();
    assert_eq!(returned, 42);
}

#[test]
fn route_void_method_returns_empty_ok() {
    let (mut contract, mock) = new_contract();
    let sel = selector("noop()");

    let outcome = my_token::route(&mut contract, sel, &[]);
    assert_eq!(outcome, Some(()));

    let rv = mock
        .take_return_value()
        .expect("contract called return_value");
    assert_eq!(rv.flags, ReturnFlags::empty());
    assert_eq!(rv.data, &[] as &[u8]);
}

#[test]
fn route_unknown_selector_returns_unhandled() {
    let (mut contract, mock) = new_contract();

    let outcome = my_token::route(&mut contract, [0xDE, 0xAD, 0xBE, 0xEF], &[]);

    assert_eq!(outcome, None);
    assert!(mock.take_return_value().is_none());
}

#[test]
fn route_short_input_reverts_with_invalid_calldata() {
    let (mut contract, mock) = new_contract();
    let sel = selector("double(uint64)");
    let short_input = [0u8; 1]; // need at least 32 bytes for u64

    let outcome = my_token::route(&mut contract, sel, &short_input);
    assert_eq!(outcome, Some(()));

    let rv = mock
        .take_return_value()
        .expect("contract called return_value");
    assert_eq!(rv.flags, ReturnFlags::REVERT);
    assert_eq!(
        rv.data,
        pvm_contract_types::framework_errors::INVALID_CALLDATA.as_slice()
    );
}

#[test]
fn router_trait_impl_delegates_to_module_route() {
    let (mut contract, mock) = new_contract();
    // Rust `balance_of` becomes Solidity `balanceOf` (snake_case → camelCase).
    let sel = selector("balanceOf(address)");
    let input = encode_address(Address::from([0xAA; 20]));

    // Call through the Router trait rather than the free function.
    let outcome = <my_token::MyContract as Router>::route(&mut contract, sel, &input);
    assert_eq!(outcome, Some(()));

    let rv = mock
        .take_return_value()
        .expect("contract called return_value");
    assert_eq!(rv.flags, ReturnFlags::empty());
    let returned = U256::decode_at(&rv.data, 0).unwrap();
    assert_eq!(returned, U256::from(42u64));
}

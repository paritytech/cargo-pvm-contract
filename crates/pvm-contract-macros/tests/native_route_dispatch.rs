//! Native unit tests exercising macro-generated `route()` + `Router` impl
//! against `MockHost`. Proves that contract dispatch is host-agnostic and
//! fully runnable off-target.
//!
//! These tests bypass `call()` / `deploy()` (riscv64-only) and invoke the
//! generated `route()` directly, asserting on the returned `DispatchResult`.

use pvm_contract_types::{
    Address, DispatchResult, Host, MockHostBuilder, Router, SolDecode, SolEncode, StaticEncodedLen,
};
use ruint::aliases::U256;

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract]
mod my_token {
    use super::*;
    use pvm_contract_types::{HostApi};

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

fn new_contract() -> my_token::MyContract {
    my_token::MyContract {
        host: Host::from_dyn(Box::new(MockHostBuilder::new().build())),
    }
}

#[test]
fn route_matches_selector_and_returns_encoded_u64() {
    let mut contract = new_contract();
    let sel = selector("double(uint64)");
    let input = encode_u64(21);
    let mut out = [0u8; 256];

    let result = my_token::route(&mut contract, sel, &input, &mut out);

    match result {
        DispatchResult::Ok(data) => {
            let returned = u64::decode_at(data, 0);
            assert_eq!(returned, 42);
        }
        other => panic!("expected Ok, got {:?}", other),
    }
}

#[test]
fn route_void_method_returns_empty_ok() {
    let mut contract = new_contract();
    let sel = selector("noop()");
    let mut out = [0u8; 256];

    let result = my_token::route(&mut contract, sel, &[], &mut out);

    match result {
        DispatchResult::Ok(data) => assert_eq!(data, &[] as &[u8]),
        other => panic!("expected Ok(&[]), got {:?}", other),
    }
}

#[test]
fn route_unknown_selector_returns_unhandled() {
    let mut contract = new_contract();
    let mut out = [0u8; 256];

    let result = my_token::route(&mut contract, [0xDE, 0xAD, 0xBE, 0xEF], &[], &mut out);

    assert!(matches!(result, DispatchResult::Unhandled));
}

#[test]
fn route_short_input_reverts_with_invalid_calldata() {
    let mut contract = new_contract();
    let sel = selector("double(uint64)");
    let mut out = [0u8; 256];
    let short_input = [0u8; 1]; // need at least 32 bytes for u64

    let result = my_token::route(&mut contract, sel, &short_input, &mut out);

    match result {
        DispatchResult::Revert(data) => {
            assert_eq!(
                data,
                &pvm_contract_types::framework_errors::INVALID_CALLDATA
            );
        }
        other => panic!("expected Revert(INVALID_CALLDATA), got {:?}", other),
    }
}

#[test]
fn router_trait_impl_delegates_to_module_route() {
    let mut contract = new_contract();
    // Rust `balance_of` becomes Solidity `balanceOf` (snake_case → camelCase).
    let sel = selector("balanceOf(address)");
    let input = encode_address(Address::from([0xAA; 20]));
    let mut out = [0u8; 256];

    // Call through the Router trait rather than the free function.
    let result = <my_token::MyContract as Router<Host>>::route(
        &mut contract,
        sel,
        &input,
        &mut out,
    );

    match result {
        DispatchResult::Ok(data) => {
            let returned = U256::decode_at(data, 0);
            assert_eq!(returned, U256::from(42u64));
        }
        other => panic!("expected Ok, got {:?}", other),
    }
}

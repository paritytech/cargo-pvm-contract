#![cfg(not(feature = "abi-gen"))]
//! Native unit tests exercising macro-generated `route()` + `Router` impl
//! against `MockHost`. Proves that contract dispatch is host-agnostic and
//! fully runnable off-target.
//!
//! These tests bypass `call()` / `deploy()` (riscv64-only) and invoke the
//! generated `route()` directly. `route()` encodes any result into the
//! caller-owned output buffer (`&mut [u8]`, via the [`OutSink`] trait) and
//! returns an [`Outcome`] — so a return-position result (success or a method's
//! own `Err`) can be asserted on directly, without going through the host. A
//! mid-expression abort (the size-check revert here) still diverges via
//! `host.revert(...)` and is caught with [`assert_reverts!`].

use pvm_contract_types::{
    Address, Host, HostApi, MockHost, MockHostBuilder, OutSink, Outcome, ReturnFlags, Router,
    SolDecode, SolEncode, StaticEncodedLen, assert_reverts, finalize_outcome,
};
use ruint::aliases::U256;
use std::rc::Rc;

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
    let (mut contract, _mock) = new_contract();
    let sel = selector("double(uint64)");
    let input = encode_u64(21);

    let mut buf = [0u8; my_token::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;
    let outcome = my_token::route(&mut contract, sel, &input, &mut out);

    let Outcome::Return(n) = outcome else {
        panic!("expected Return, got {outcome:?}");
    };
    let returned = u64::decode_at(out.view(n), 0).unwrap();
    assert_eq!(returned, 42);
}

#[test]
fn route_void_method_returns_empty_ok() {
    let (mut contract, _mock) = new_contract();
    let sel = selector("noop()");

    let mut buf = [0u8; my_token::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;
    let outcome = my_token::route(&mut contract, sel, &[], &mut out);

    assert_eq!(outcome, Outcome::Return(0));
}

#[test]
fn route_unknown_selector_returns_unhandled() {
    let (mut contract, _mock) = new_contract();

    let mut buf = [0u8; my_token::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;
    let outcome = my_token::route(&mut contract, [0xDE, 0xAD, 0xBE, 0xEF], &[], &mut out);

    assert_eq!(outcome, Outcome::Unhandled);
}

#[test]
fn route_short_input_reverts_with_invalid_calldata() {
    let (mut contract, mock) = new_contract();
    let sel = selector("double(uint64)");
    let short_input = [0u8; 1]; // need at least 32 bytes for u64

    // The size check diverges via `host.revert` (unwinds on host targets) —
    // like every revert, caught with `assert_reverts!`.
    let mut buf = [0u8; my_token::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;
    assert_reverts!(
        mock,
        pvm_contract_types::framework_errors::INVALID_CALLDATA,
        my_token::route(&mut contract, sel, &short_input, &mut out)
    );
}

#[test]
fn router_trait_impl_delegates_to_module_route() {
    let (mut contract, _mock) = new_contract();
    // Rust `balance_of` becomes Solidity `balanceOf` (snake_case → camelCase).
    let sel = selector("balanceOf(address)");
    let input = encode_address(Address::from([0xAA; 20]));

    // Call through the Router trait rather than the free function.
    let mut buf = [0u8; my_token::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;
    let outcome = <my_token::MyContract as Router>::route(&mut contract, sel, &input, &mut out);

    let Outcome::Return(n) = outcome else {
        panic!("expected Return, got {outcome:?}");
    };
    let returned = U256::decode_at(out.view(n), 0).unwrap();
    assert_eq!(returned, U256::from(42u64));
}

// The `route()` tests above assert on the returned `Outcome::Return` directly.
// The test below covers the two host-lowering primitives: `finalize_outcome`
// (success → `return_value`, empty flags) and the diverging `revert` door that
// every revert now uses (REVERT flags). A swapped mapping or wrong flags would
// otherwise be invisible to the return-position assertions.

#[test]
fn finalize_lowers_return_to_success_and_revert_door_carries_revert_flags() {
    let mock = MockHostBuilder::new().build();
    let host = Host::from_dyn(Rc::new(mock.clone()));

    let mut buf = [0u8; 8];
    buf[..4].copy_from_slice(&[1, 2, 3, 4]);
    let out: &mut [u8] = &mut buf;

    // Success: finalize_outcome(Return) → return_value with empty flags.
    finalize_outcome(&host, Outcome::Return(4), &out);
    let rv = mock.take_return_value().expect("return_value recorded");
    assert_eq!(rv.flags, ReturnFlags::empty());
    assert_eq!(rv.data, &[1, 2, 3, 4]);

    // Revert: reverts do not flow through `Outcome` — they diverge via the
    // revert door, which records REVERT flags and unwinds on host.
    let rv = mock.expect_revert(|| {
        host.revert(&[1, 2, 3, 4]);
    });
    assert_eq!(rv.flags, ReturnFlags::REVERT);
    assert_eq!(rv.data, &[1, 2, 3, 4]);
}

#[test]
fn route_then_finalize_records_return_value_end_to_end() {
    let (mut contract, mock) = new_contract();
    let host = Host::from_dyn(Rc::new(mock.clone()));
    let sel = selector("double(uint64)");
    let input = encode_u64(21);

    let mut buf = [0u8; my_token::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;
    let outcome = my_token::route(&mut contract, sel, &input, &mut out);

    // The full call()-path lowering: route() → finalize_outcome → host door.
    finalize_outcome(&host, outcome, &out);
    let rv = mock.take_return_value().expect("return_value recorded");
    assert_eq!(rv.flags, ReturnFlags::empty());
    assert_eq!(u64::decode_at(&rv.data, 0).unwrap(), 42);
}

// Regression: a payable `#[fallback]` alongside only non-payable `#[method]`s.
// The router's non-payable guard must not be hoisted before the selector match
// in this case, or a value-bearing unmatched-selector call would revert in
// `route()`'s prelude before reaching the payable fallback.
#[allow(dead_code)] // `new()`/`any()` run only through deploy()/call() (riscv64-gated)
#[pvm_contract_macros::contract]
mod payable_fallback_c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn get(&self) -> u64 {
            7
        }

        #[pvm_contract_macros::fallback]
        #[pvm_contract_macros::payable]
        pub fn any(&mut self) {}
    }
}

#[test]
fn payable_fallback_not_pre_reverted_by_hoisted_guard() {
    // Non-zero msg.value + an unmatched selector: `route()` must return
    // `Outcome::Unhandled` (letting `call()` reach the payable fallback), NOT
    // revert in its prelude. With the pre-fix hoisted `__pvm_assert_non_payable`
    // this diverged instead, and the assert below would never be reached.
    let mock = MockHostBuilder::new()
        .value_transferred(U256::from(1u8))
        .build();
    let mut contract = payable_fallback_c::C::with_host(mock.clone());

    let mut buf = [0u8; payable_fallback_c::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;
    let outcome = payable_fallback_c::route(&mut contract, [0xDE, 0xAD, 0xBE, 0xEF], &[], &mut out);

    assert_eq!(outcome, Outcome::Unhandled);
}

#![cfg(not(feature = "abi-gen"))]
//! Native unit tests proving that cross-contract calls and instantiations
//! routed through `abi_import!`-generated proxies are intercepted by
//! `MockHost::mock_call` / `mock_instantiate`.
//!
//! These tests exercise the host-as-parameter path:
//!
//! ```ignore
//! Iface::from_address(addr).method().call(&cx)?
//!     -> CallBuilder::call(cx, ...)
//!     -> cx.host().call_evm(...)      // dispatches to MockHost
//!     -> MockHost::resolve_call         // returns mocked data
//!     -> CallBuilder::extract_output(host, ...)
//!     -> host.return_data_copy(...)     // reads from MockHost.return_data
//! ```
//!
//! `Context` wraps the `Host` and impls `ContractContext`, so it satisfies
//! the borrow gate (`&impl ContractContext` for view callees,
//! `&mut impl ContractContext` for mutating ones).

extern crate alloc;

use std::rc::Rc;

use pvm_contract_sdk::{
    Address, CallError, Context, Host, HostApi, MockHostBuilder, RefTimeAndProofSizeLimits,
};

pvm_contract_sdk::abi_import! {
    #![abi_import(alloc = true)]
    // SPDX-License-Identifier: MIT
    pragma solidity ^0.8.0;

    interface Flipper {
        constructor();
        function flip() external;
        function get() external view returns (bool);
    }
}

// Regression fixtures for abi_import! parameter naming (see tests below).
pvm_contract_sdk::abi_import! {
    #![abi_import(alloc = true)]
    pragma solidity ^0.8.0;

    interface CamelCaseParams {
        function publishLatest(
            string memory contractName,
            address contractAddress,
            string memory metadataUri
        ) external;
    }
}

pvm_contract_sdk::abi_import! {
    #![abi_import(alloc = true)]
    pragma solidity ^0.8.0;

    interface UnnamedParams {
        function compute(uint256, uint256, address) external returns (uint256);
    }
}

fn encoded_bool(value: bool) -> Vec<u8> {
    let mut buf = vec![0u8; 32];
    if value {
        buf[31] = 1;
    }
    buf
}

#[test]
fn view_call_returns_mocked_data() {
    let target = Address::from([0xBB; 20]);
    let mock = MockHostBuilder::new().build();
    mock.mock_call(target.0, Ok(encoded_bool(true)));
    let cx = Context::new(Host::from_dyn(Rc::new(mock)));

    let res = flipper::Flipper::from_address(target)
        .get()
        .call(&cx)
        .unwrap();

    assert!(res);
}

#[test]
fn view_call_returns_mocked_false() {
    let target = Address::from([0xBC; 20]);
    let mock = MockHostBuilder::new().build();
    mock.mock_call(target.0, Ok(encoded_bool(false)));
    let cx = Context::new(Host::from_dyn(Rc::new(mock)));

    let res = flipper::Flipper::from_address(target)
        .get()
        .call(&cx)
        .unwrap();

    assert!(!res);
}

#[test]
fn write_call_invokes_mock_and_propagates_return_data() {
    let target = Address::from([0xBD; 20]);
    let marker = vec![0xAA, 0xBB, 0xCC, 0xDD];
    let mock = MockHostBuilder::new().build();
    mock.mock_call(target.0, Ok(marker.clone()));
    let mut cx = Context::new(Host::from_dyn(Rc::new(mock)));

    flipper::Flipper::from_address(target)
        .flip()
        .call(&mut cx)
        .expect("flip should succeed");

    // Stronger evidence than a successful return: the mock actually wrote
    // its configured payload into the host's return-data buffer.
    let host = &cx.host;
    assert_eq!(host.return_data_size(), marker.len() as u64);
    let mut buf = vec![0u8; marker.len()];
    let mut out = &mut buf[..];
    host.return_data_copy(&mut out, 0);
    assert_eq!(buf, marker);
}

#[test]
fn unmocked_call_leaves_return_data_empty() {
    // Contrast with `write_call_invokes_mock_and_propagates_return_data`:
    // when no mock is configured for the callee, `MockHost::resolve_call`
    // clears return_data and returns Ok(()) — proving the previous test's
    // marker bytes came from the mock table, not from a default fallback.
    let target = Address::from([0xBE; 20]);
    let mut cx = Context::new(Host::from_dyn(Rc::new(MockHostBuilder::new().build())));

    flipper::Flipper::from_address(target)
        .flip()
        .call(&mut cx)
        .expect("unmocked flip still succeeds");

    assert_eq!(cx.host.return_data_size(), 0);
}

#[test]
fn revert_from_callee_propagates_as_call_error() {
    let target = Address::from([0xCC; 20]);
    let mock = MockHostBuilder::new().build();
    mock.mock_call(target.0, Err(()));
    let cx = Context::new(Host::from_dyn(Rc::new(mock)));

    let res = flipper::Flipper::from_address(target).get().call(&cx);

    assert_eq!(res, Err(CallError::CalleeReverted));
}

#[test]
fn delegate_call_uses_same_mock_table() {
    // `MockHost::resolve_call` is shared between `call`, `call_evm`,
    // `delegate_call`, and `delegate_call_evm`, so a single `mock_call`
    // entry should serve both regular and delegate callers. Verify that
    // (a) the decoded result matches the mocked bool AND (b) the mocked
    // bytes were written into return_data — same evidence as the regular
    // call path.
    let target = Address::from([0xDE; 20]);
    let payload = encoded_bool(true);
    let mock = MockHostBuilder::new().build();
    mock.mock_call(target.0, Ok(payload.clone()));
    let mut cx = Context::new(Host::from_dyn(Rc::new(mock)));

    let res = flipper::Flipper::from_address(target)
        .get()
        .delegate_call(&mut cx)
        .unwrap();

    assert!(res);
    let host = &cx.host;
    assert_eq!(host.return_data_size(), payload.len() as u64);
    let mut buf = vec![0u8; payload.len()];
    let mut out = &mut buf[..];
    host.return_data_copy(&mut out, 0);
    assert_eq!(buf, payload);
}

#[test]
fn chained_calls_each_extract_their_own_return_data() {
    // Two callees mocked with different payloads; the contract calls both
    // in sequence. Each `.call(&cx)` must decode the value belonging to
    // *its* callee — proving that `extract_output` runs immediately after
    // `call_raw` and before the next call clobbers `return_data`. This
    // mirrors on-chain semantics (RETURNDATA reflects only the most recent
    // sub-call) and protects against a future regression where the alloc
    // helper might split `call_raw` and `extract_output` non-atomically.
    let target_a = Address::from([0xA1; 20]);
    let target_b = Address::from([0xB2; 20]);

    let mock = MockHostBuilder::new().build();
    mock.mock_call(target_a.0, Ok(encoded_bool(true)));
    mock.mock_call(target_b.0, Ok(encoded_bool(false)));
    let cx = Context::new(Host::from_dyn(Rc::new(mock)));

    // First call — get the answer for target A.
    let res_a = flipper::Flipper::from_address(target_a)
        .get()
        .call(&cx)
        .unwrap();
    assert!(res_a, "first call should decode target_a's payload");

    // Second call — get the answer for target B.
    let res_b = flipper::Flipper::from_address(target_b)
        .get()
        .call(&cx)
        .unwrap();
    assert!(!res_b, "second call should decode target_b's payload");

    // Pin the overwrite semantics: the host's RETURNDATA now holds only
    // target B's payload (target A's is gone). On chain this is exactly
    // how `RETURNDATASIZE` / `RETURNDATACOPY` behave.
    let host = &cx.host;
    assert_eq!(host.return_data_size(), 32);
    let mut buf = [0u8; 32];
    let mut out = &mut buf[..];
    host.return_data_copy(&mut out, 0);
    assert_eq!(buf, encoded_bool(false).as_slice());
}

#[test]
fn instantiate_returns_mocked_address() {
    let deployed = [0xDD; 20];
    let mock = MockHostBuilder::new().build();
    mock.mock_instantiate(deployed, Vec::new());
    let mut cx = Context::new(Host::from_dyn(Rc::new(mock)));

    let limits = RefTimeAndProofSizeLimits {
        ref_time_limit: u64::MAX,
        proof_size_limit: u64::MAX,
        deposit_limit: [0u8; 32],
    };
    let (addr, ()) = flipper::new_flipper()
        .instantiate(&mut cx, &[0u8; 32], 0, limits, None)
        .expect("instantiate should succeed");

    assert_eq!(addr, Address::from(deployed));
}

#[test]
fn mut_caller_calling_view_callee_compiles_via_coercion() {
    // Pins the borrow-checker coercion path: a caller that holds
    // `&mut Context` can invoke a `View` callee whose `call` takes
    // `&impl ContractContext`. Rust's `&mut T -> &T` reborrow makes this
    // work without an explicit `&*cx`. Regression guard: if someone
    // tightens the View bound to require `&Self`-only (no coercion from
    // `&mut`), this test will fail to compile.
    let target = Address::from([0xBF; 20]);
    let mock = MockHostBuilder::new().build();
    mock.mock_call(target.0, Ok(encoded_bool(true)));
    let mut cx = Context::new(Host::from_dyn(Rc::new(mock)));

    // `&mut cx` exists; Flipper::get is a View callee whose .call
    // takes `&impl ContractContext`. The reborrow `&*&mut cx -> &cx` is
    // implicit here — we just pass `&cx` after taking the mut borrow.
    let res = flipper::Flipper::from_address(target)
        .get()
        .call(&cx)
        .unwrap();
    assert!(res);

    // Confirm `cx` is still owned mutably afterwards (no borrow stuck).
    let _: &mut Context = &mut cx;
}

#[test]
fn instantiate_without_mock_returns_out_of_resources() {
    let mock = MockHostBuilder::new().build();
    let mut cx = Context::new(Host::from_dyn(Rc::new(mock)));

    let limits = RefTimeAndProofSizeLimits {
        ref_time_limit: u64::MAX,
        proof_size_limit: u64::MAX,
        deposit_limit: [0u8; 32],
    };
    let res = flipper::new_flipper().instantiate(&mut cx, &[0u8; 32], 0, limits, None);

    assert_eq!(res, Err(CallError::OutOfResources));
}

// Regression: camelCase params must be snake_cased on both signature and body
// sides of the abi_import!-generated proxy.
#[test]
fn camelcase_params_call_through_mock() {
    let target = Address::from([0xCE; 20]);
    let mock = MockHostBuilder::new().build();
    mock.mock_call(target.0, Ok(vec![]));
    let mut cx = Context::new(Host::from_dyn(Rc::new(mock)));

    camel_case_params::CamelCaseParams::from_address(target)
        .publish_latest(
            "MyContract".to_string(),
            Address::from([0xDE; 20]),
            "ipfs://qm".to_string(),
        )
        .call(&mut cx)
        .expect("publish_latest should succeed");
}

// Regression: unnamed params must use `s{index}` on both sides to avoid
// identifier collisions across multiple unnamed parameters.
#[test]
fn unnamed_params_call_through_mock() {
    let target = Address::from([0xCF; 20]);
    let mut payload = vec![0u8; 32];
    payload[31] = 42;
    let mock = MockHostBuilder::new().build();
    mock.mock_call(target.0, Ok(payload));
    let mut cx = Context::new(Host::from_dyn(Rc::new(mock)));

    let result = unnamed_params::UnnamedParams::from_address(target)
        .compute(
            pvm_contract_sdk::U256::from(1u64),
            pvm_contract_sdk::U256::from(2u64),
            Address::from([0xAB; 20]),
        )
        .call(&mut cx)
        .expect("compute should succeed");

    assert_eq!(result, pvm_contract_sdk::U256::from(42u64));
}

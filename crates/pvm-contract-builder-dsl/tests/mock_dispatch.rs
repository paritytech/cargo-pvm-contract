//! End-to-end DSL dispatch tests against `MockHost`.
//!
//! Builds a tiny contract with the builder DSL, drives `dispatch_impl` against
//! a `MockHost` instance, and asserts on the returned `DispatchOutcome`.
//! No thread-locals, no `catch_unwind`, no panic capture — plain method calls
//! and value assertions.

use pvm_contract_builder_dsl::{
    ContractBuilder, DispatchOutcome, HandlerResult, solidity_selector,
};
use pvm_contract_types::{
    HostApi, MockHost, MockHostBuilder, SolDecode, SolEncode, StaticEncodedLen,
};

const DOUBLE_SELECTOR: [u8; 4] = solidity_selector("double(uint32)");
const PING_SELECTOR: [u8; 4] = solidity_selector("ping()");

fn double_handler<H: HostApi>(_host: &H, input: &[u8], output: &mut [u8]) -> HandlerResult {
    let n = u32::decode_at(input, 0);
    let result = n.wrapping_mul(2);
    let len = <u32 as StaticEncodedLen>::ENCODED_SIZE;
    result.encode_to(&mut output[..len]);
    HandlerResult::Ok(len)
}

fn ping_handler<H: HostApi>(_host: &H, _input: &[u8], _output: &mut [u8]) -> HandlerResult {
    HandlerResult::Ok(0)
}

fn builder() -> ContractBuilder<MockHost> {
    ContractBuilder::<MockHost>::new()
        .method(DOUBLE_SELECTOR, double_handler::<MockHost>)
        .method(PING_SELECTOR, ping_handler::<MockHost>)
}

fn encode_call_double(n: u32) -> Vec<u8> {
    let mut calldata = DOUBLE_SELECTOR.to_vec();
    let mut arg = [0u8; <u32 as StaticEncodedLen>::ENCODED_SIZE];
    n.encode_to(&mut arg);
    calldata.extend_from_slice(&arg);
    calldata
}

fn assert_ok(outcome: &DispatchOutcome<256>) {
    assert!(
        outcome.is_ok(),
        "expected Ok, got revert with data {:?}",
        outcome.data()
    );
}

fn assert_revert(outcome: &DispatchOutcome<256>) {
    assert!(
        outcome.is_revert(),
        "expected Revert, got Ok with data {:?}",
        outcome.data()
    );
}

#[test]
fn double_returns_doubled_value() {
    let host = MockHostBuilder::new()
        .calldata(encode_call_double(21))
        .build();
    let outcome = builder().dispatch_impl::<256>(&host);
    assert_ok(&outcome);
    assert_eq!(u32::decode_at(outcome.data(), 0), 42);
}

#[test]
fn ping_returns_empty_success() {
    let host = MockHostBuilder::new()
        .calldata(PING_SELECTOR.to_vec())
        .build();
    let outcome = builder().dispatch_impl::<256>(&host);
    assert_ok(&outcome);
    assert_eq!(outcome.data().len(), 0);
}

#[test]
fn unknown_selector_reverts() {
    let host = MockHostBuilder::new()
        .calldata(vec![0xde, 0xad, 0xbe, 0xef])
        .build();
    let outcome = builder().dispatch_impl::<256>(&host);
    assert_revert(&outcome);
}

#[test]
fn short_calldata_reverts() {
    let host = MockHostBuilder::new().calldata(vec![0x00]).build();
    let outcome = builder().dispatch_impl::<256>(&host);
    assert_revert(&outcome);
}

#[test]
fn storage_is_observable_from_handler() {
    // Register a handler that reads from storage and returns the value.
    fn read_slot<H: HostApi>(host: &H, _input: &[u8], output: &mut [u8]) -> HandlerResult {
        use pvm_contract_types::StorageFlags;
        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        let _ = host.get_storage(StorageFlags::empty(), &[0u8; 32], &mut out);
        output[..32].copy_from_slice(&buf);
        HandlerResult::Ok(32)
    }
    const READ_SELECTOR: [u8; 4] = solidity_selector("read()");

    let mut preset = [0u8; 32];
    preset[31] = 0x42;
    let host = MockHostBuilder::new()
        .calldata(READ_SELECTOR.to_vec())
        .storage(vec![(vec![0u8; 32], preset.to_vec())])
        .build();

    let outcome = ContractBuilder::<MockHost>::new()
        .method(READ_SELECTOR, read_slot::<MockHost>)
        .dispatch_impl::<256>(&host);
    assert_ok(&outcome);
    assert_eq!(outcome.data()[31], 0x42);
}

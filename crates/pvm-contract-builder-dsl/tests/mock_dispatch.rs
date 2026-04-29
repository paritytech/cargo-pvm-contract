//! End-to-end DSL dispatch tests against `MockHost`.
//!
//! Builds a tiny contract with the builder DSL, drives `dispatch_impl` against
//! a `MockHost` instance, and reads the captured `ReturnValue` via
//! `MockHost::take_return_value()`. Mirrors the `#[contract]` macro's test
//! pattern — same shape across both dispatch paths.

use pvm_contract_builder_dsl::{ContractBuilder, HandlerResult, solidity_selector};
use pvm_contract_types::{
    HostApi, MockHost, MockHostBuilder, ReturnFlags, ReturnValue, SolDecode, SolEncode,
    StaticEncodedLen,
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

fn drive(host: &MockHost) -> ReturnValue {
    builder().dispatch_impl::<256>(host);
    host.take_return_value()
        .expect("dispatch should call return_value")
}

#[test]
fn double_returns_doubled_value() {
    let host = MockHostBuilder::new()
        .calldata(encode_call_double(21))
        .build();
    let rv = drive(&host);
    assert_eq!(rv.flags, ReturnFlags::empty());
    assert_eq!(u32::decode_at(&rv.data, 0), 42);
}

#[test]
fn ping_returns_empty_success() {
    let host = MockHostBuilder::new()
        .calldata(PING_SELECTOR.to_vec())
        .build();
    let rv = drive(&host);
    assert_eq!(rv.flags, ReturnFlags::empty());
    assert_eq!(rv.data.len(), 0);
}

#[test]
fn unknown_selector_reverts() {
    let host = MockHostBuilder::new()
        .calldata(vec![0xde, 0xad, 0xbe, 0xef])
        .build();
    let rv = drive(&host);
    assert_eq!(rv.flags, ReturnFlags::REVERT);
    assert_eq!(
        rv.data,
        pvm_contract_types::framework_errors::UNKNOWN_SELECTOR.as_slice()
    );
}

#[test]
fn short_calldata_reverts() {
    let host = MockHostBuilder::new().calldata(vec![0x00]).build();
    let rv = drive(&host);
    assert_eq!(rv.flags, ReturnFlags::REVERT);
    assert_eq!(
        rv.data,
        pvm_contract_types::framework_errors::NO_SELECTOR.as_slice()
    );
}

#[test]
fn handler_revert_is_reflected_in_outcome() {
    // A handler that returns `HandlerResult::Revert(n)` must surface as
    // `flags == REVERT` with the handler-written payload intact.
    const FAIL_SELECTOR: [u8; 4] = solidity_selector("fail()");

    fn always_fails<H: HostApi>(_host: &H, _input: &[u8], output: &mut [u8]) -> HandlerResult {
        let msg = b"not allowed";
        output[..msg.len()].copy_from_slice(msg);
        HandlerResult::Revert(msg.len())
    }

    let host = MockHostBuilder::new()
        .calldata(FAIL_SELECTOR.to_vec())
        .build();

    ContractBuilder::<MockHost>::new()
        .method(FAIL_SELECTOR, always_fails::<MockHost>)
        .dispatch_impl::<256>(&host);

    let rv = host
        .take_return_value()
        .expect("dispatch called return_value");
    assert_eq!(rv.flags, ReturnFlags::REVERT);
    assert_eq!(rv.data, b"not allowed");
}

#[test]
fn handler_returning_oversize_len_is_clamped() {
    // A buggy handler that returns `Ok(n)` where `n` exceeds the buffer must
    // be clamped by the dispatcher rather than panicking on slice access.
    const BAD_SELECTOR: [u8; 4] = solidity_selector("bad()");

    fn bogus_len<H: HostApi>(_host: &H, _input: &[u8], output: &mut [u8]) -> HandlerResult {
        // Only wrote 4 bytes but claims 9999.
        output[..4].copy_from_slice(&[1, 2, 3, 4]);
        HandlerResult::Ok(9999)
    }

    let host = MockHostBuilder::new()
        .calldata(BAD_SELECTOR.to_vec())
        .build();

    ContractBuilder::<MockHost>::new()
        .method(BAD_SELECTOR, bogus_len::<MockHost>)
        .dispatch_impl::<256>(&host);

    let rv = host
        .take_return_value()
        .expect("dispatch called return_value");
    assert_eq!(rv.flags, ReturnFlags::empty());
    // Must not panic; len is clamped to BUF_SIZE (256).
    assert_eq!(rv.data.len(), 256);
    assert_eq!(&rv.data[..4], &[1, 2, 3, 4]);
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

    ContractBuilder::<MockHost>::new()
        .method(READ_SELECTOR, read_slot::<MockHost>)
        .dispatch_impl::<256>(&host);

    let rv = host
        .take_return_value()
        .expect("dispatch called return_value");
    assert_eq!(rv.flags, ReturnFlags::empty());
    assert_eq!(rv.data[31], 0x42);
}

//! Mock host backend for native unit testing of PVM contracts.
//!
//! [`MockHost`] implements [`HostApi`](super::HostApi) using thread-local state,
//! allowing contract logic to be tested with `cargo test` on the host target.
//!
//! # Quick start
//!
//! ```ignore
//! use pvm_contract_types::{MockHost, MockHostBuilder, HostApi, StorageFlags};
//!
//! // Builder pattern for clean test setup (inspired by Stylus TestVMBuilder)
//! MockHostBuilder::new()
//!     .caller([0xAA; 20])
//!     .calldata(vec![0x01, 0x02, 0x03, 0x04])
//!     .balance([0u8; 32])
//!     .install();
//!
//! // Contract logic can now call HostApi methods:
//! // MockHost::get_storage, set_storage, caller, deposit_event, etc.
//!
//! let events = MockHost::events();
//! assert_eq!(events.len(), 1);
//! ```
//!
//! # Mock external calls
//!
//! ```ignore
//! // Register a mock return value for cross-contract calls
//! MockHost::mock_call([0xBB; 20], Ok(vec![0, 0, 0, 1]));
//!
//! // When contract code calls HostApi::call with callee=[0xBB; 20],
//! // it will return Ok(()) and write the mock data to the output buffer.
//! ```
//!
//! # Capturing return values
//!
//! [`HostApi::return_value`] diverges (`-> !`), so MockHost panics.
//! Use [`std::panic::catch_unwind`] + [`MockHost::take_return_value`]:
//!
//! ```ignore
//! let result = std::panic::catch_unwind(|| {
//!     // ... code that eventually calls HostApi::return_value ...
//! });
//! let (flags, data) = MockHost::take_return_value().unwrap();
//! ```

use std::cell::RefCell;
use std::collections::HashMap;

use super::host::{CallFlags, HostApi, HostResult, ReturnErrorCode, ReturnFlags, StorageFlags};

/// Return value for mocked external calls.
///
/// `Ok(data)` — call succeeds, `data` is written to the output buffer.
/// `Err(())` — call reverts with `ReturnErrorCode::CalleeReverted`.
type MockCallReturn = Result<Vec<u8>, ()>;

#[derive(Default)]
struct MockState {
    // --- Input state (configured before contract execution) ---
    caller: [u8; 20],
    origin: [u8; 20],
    address: [u8; 20],
    balance: [u8; 32],
    chain_id: [u8; 32],
    block_number: [u8; 32],
    block_timestamp: [u8; 32],
    block_author: [u8; 20],
    value_transferred: [u8; 32],
    calldata: Vec<u8>,

    // --- Persistent state (read/written during execution) ---
    storage: HashMap<Vec<u8>, Vec<u8>>,

    // --- Mock call returns (keyed by callee address) ---
    call_returns: HashMap<[u8; 20], MockCallReturn>,

    // --- Output state (captured during execution) ---
    events: Vec<(Vec<[u8; 32]>, Vec<u8>)>,
    return_value: Option<(ReturnFlags, Vec<u8>)>,
}

thread_local! {
    static MOCK_STATE: RefCell<MockState> = RefCell::new(MockState::default());
}

/// Mock host backend for native testing.
///
/// All state is stored in a thread-local, so tests using `MockHost` are safe
/// to run in parallel (each thread gets its own state).
///
/// Use [`MockHostBuilder`] for clean test setup, or call `MockHost::reset()`
/// and the individual `set_*` methods.
pub struct MockHost;

// ---------------------------------------------------------------------------
// State management
// ---------------------------------------------------------------------------

impl MockHost {
    /// Reset all mock state to defaults.
    pub fn reset() {
        MOCK_STATE.with(|s| *s.borrow_mut() = MockState::default());
    }

    // --- Input state setters ---

    /// Set the caller address returned by [`HostApi::caller`].
    pub fn set_caller(caller: [u8; 20]) {
        MOCK_STATE.with(|s| s.borrow_mut().caller = caller);
    }

    /// Set the origin address returned by [`HostApi::origin`].
    pub fn set_origin(origin: [u8; 20]) {
        MOCK_STATE.with(|s| s.borrow_mut().origin = origin);
    }

    /// Set the contract address returned by [`HostApi::address`].
    pub fn set_address(address: [u8; 20]) {
        MOCK_STATE.with(|s| s.borrow_mut().address = address);
    }

    /// Set the balance returned by [`HostApi::balance`].
    pub fn set_balance(balance: [u8; 32]) {
        MOCK_STATE.with(|s| s.borrow_mut().balance = balance);
    }

    /// Set the chain ID returned by [`HostApi::chain_id`].
    pub fn set_chain_id(chain_id: [u8; 32]) {
        MOCK_STATE.with(|s| s.borrow_mut().chain_id = chain_id);
    }

    /// Set the block number returned by [`HostApi::block_number`].
    pub fn set_block_number(block_number: [u8; 32]) {
        MOCK_STATE.with(|s| s.borrow_mut().block_number = block_number);
    }

    /// Set the block timestamp returned by [`HostApi::now`].
    pub fn set_block_timestamp(timestamp: [u8; 32]) {
        MOCK_STATE.with(|s| s.borrow_mut().block_timestamp = timestamp);
    }

    /// Set the value transferred returned by [`HostApi::value_transferred`].
    pub fn set_value_transferred(value: [u8; 32]) {
        MOCK_STATE.with(|s| s.borrow_mut().value_transferred = value);
    }

    /// Set the calldata that [`HostApi::call_data_size`] and
    /// [`HostApi::call_data_copy`] will return.
    pub fn set_calldata(data: Vec<u8>) {
        MOCK_STATE.with(|s| s.borrow_mut().calldata = data);
    }

    // --- Mock call returns ---

    /// Register a mock return value for [`HostApi::call`] to a given callee.
    ///
    /// When contract code calls `HostApi::call` with the matching `callee`
    /// address, the mock will return `Ok(())` and write `data` to the output
    /// buffer (for `Ok(data)`), or return `Err(CalleeReverted)` (for `Err(())`).
    pub fn mock_call(callee: [u8; 20], result: MockCallReturn) {
        MOCK_STATE.with(|s| {
            s.borrow_mut().call_returns.insert(callee, result);
        });
    }

    // --- Storage helpers ---

    /// Read raw storage for test assertions.
    pub fn get_raw_storage(key: &[u8]) -> Option<Vec<u8>> {
        MOCK_STATE.with(|s| s.borrow().storage.get(key).cloned())
    }

    /// Write raw storage for test setup.
    pub fn set_raw_storage(key: Vec<u8>, value: Vec<u8>) {
        MOCK_STATE.with(|s| {
            s.borrow_mut().storage.insert(key, value);
        });
    }

    // --- Output state readers ---

    /// Get all events emitted via [`HostApi::deposit_event`].
    /// Each event is a `(topics, data)` pair.
    pub fn events() -> Vec<(Vec<[u8; 32]>, Vec<u8>)> {
        MOCK_STATE.with(|s| s.borrow().events.clone())
    }

    /// Take the return value set by [`HostApi::return_value`].
    /// Returns `None` if `return_value` was never called.
    pub fn take_return_value() -> Option<(ReturnFlags, Vec<u8>)> {
        MOCK_STATE.with(|s| s.borrow_mut().return_value.take())
    }
}

// ---------------------------------------------------------------------------
// Builder (inspired by Stylus TestVMBuilder)
// ---------------------------------------------------------------------------

/// Fluent builder for configuring [`MockHost`] state before a test.
///
/// # Example
///
/// ```ignore
/// MockHostBuilder::new()
///     .caller([0xAA; 20])
///     .value_transferred([0u8; 32])
///     .storage(vec![([1u8; 32].to_vec(), [42u8; 32].to_vec())])
///     .install();
/// ```
pub struct MockHostBuilder {
    state: MockState,
}

impl MockHostBuilder {
    /// Create a new builder with default state.
    pub fn new() -> Self {
        Self {
            state: MockState::default(),
        }
    }

    /// Set the caller address.
    pub fn caller(mut self, caller: [u8; 20]) -> Self {
        self.state.caller = caller;
        self
    }

    /// Set the origin address.
    pub fn origin(mut self, origin: [u8; 20]) -> Self {
        self.state.origin = origin;
        self
    }

    /// Set the contract address.
    pub fn address(mut self, address: [u8; 20]) -> Self {
        self.state.address = address;
        self
    }

    /// Set the contract balance.
    pub fn balance(mut self, balance: [u8; 32]) -> Self {
        self.state.balance = balance;
        self
    }

    /// Set the chain ID.
    pub fn chain_id(mut self, chain_id: [u8; 32]) -> Self {
        self.state.chain_id = chain_id;
        self
    }

    /// Set the block number.
    pub fn block_number(mut self, block_number: [u8; 32]) -> Self {
        self.state.block_number = block_number;
        self
    }

    /// Set the block timestamp.
    pub fn block_timestamp(mut self, timestamp: [u8; 32]) -> Self {
        self.state.block_timestamp = timestamp;
        self
    }

    /// Set the block author.
    pub fn block_author(mut self, author: [u8; 20]) -> Self {
        self.state.block_author = author;
        self
    }

    /// Set the value transferred (msg.value).
    pub fn value_transferred(mut self, value: [u8; 32]) -> Self {
        self.state.value_transferred = value;
        self
    }

    /// Set the calldata.
    pub fn calldata(mut self, data: Vec<u8>) -> Self {
        self.state.calldata = data;
        self
    }

    /// Pre-populate storage with key-value pairs.
    pub fn storage(mut self, entries: Vec<(Vec<u8>, Vec<u8>)>) -> Self {
        for (key, value) in entries {
            self.state.storage.insert(key, value);
        }
        self
    }

    /// Install this builder's state into the thread-local [`MockHost`].
    ///
    /// This replaces all existing state (equivalent to `MockHost::reset()`
    /// followed by setting each field).
    pub fn install(self) {
        MOCK_STATE.with(|s| *s.borrow_mut() = self.state);
    }
}

impl Default for MockHostBuilder {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// HostApi implementation
// ---------------------------------------------------------------------------

impl HostApi for MockHost {
    fn address(output: &mut [u8; 20]) {
        MOCK_STATE.with(|s| output.copy_from_slice(&s.borrow().address));
    }

    fn get_immutable_data(_output: &mut &mut [u8]) {
        unimplemented!("MockHost::get_immutable_data")
    }

    fn set_immutable_data(_data: &[u8]) {
        unimplemented!("MockHost::set_immutable_data")
    }

    fn balance(output: &mut [u8; 32]) {
        MOCK_STATE.with(|s| output.copy_from_slice(&s.borrow().balance));
    }

    fn balance_of(_addr: &[u8; 20], _output: &mut [u8; 32]) {
        unimplemented!("MockHost::balance_of")
    }

    fn chain_id(output: &mut [u8; 32]) {
        MOCK_STATE.with(|s| output.copy_from_slice(&s.borrow().chain_id));
    }

    fn gas_price() -> u64 {
        0
    }

    fn base_fee(_output: &mut [u8; 32]) {
        unimplemented!("MockHost::base_fee")
    }

    fn call_data_size() -> u64 {
        MOCK_STATE.with(|s| s.borrow().calldata.len() as u64)
    }

    fn call(
        _flags: CallFlags,
        callee: &[u8; 20],
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit: &[u8; 32],
        _value: &[u8; 32],
        _input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        MOCK_STATE.with(|s| {
            let state = s.borrow();
            match state.call_returns.get(callee) {
                Some(Ok(data)) => {
                    if let Some(out) = output {
                        let len = data.len().min(out.len());
                        out[..len].copy_from_slice(&data[..len]);
                    }
                    Ok(())
                }
                Some(Err(())) => Err(ReturnErrorCode::CalleeReverted),
                None => Ok(()),
            }
        })
    }

    fn call_evm(
        _flags: CallFlags,
        callee: &[u8; 20],
        _gas: u64,
        _value: &[u8; 32],
        _input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        MOCK_STATE.with(|s| {
            let state = s.borrow();
            match state.call_returns.get(callee) {
                Some(Ok(data)) => {
                    if let Some(out) = output {
                        let len = data.len().min(out.len());
                        out[..len].copy_from_slice(&data[..len]);
                    }
                    Ok(())
                }
                Some(Err(())) => Err(ReturnErrorCode::CalleeReverted),
                None => Ok(()),
            }
        })
    }

    fn caller(output: &mut [u8; 20]) {
        MOCK_STATE.with(|s| output.copy_from_slice(&s.borrow().caller));
    }

    fn origin(output: &mut [u8; 20]) {
        MOCK_STATE.with(|s| output.copy_from_slice(&s.borrow().origin));
    }

    fn code_hash(_addr: &[u8; 20], _output: &mut [u8; 32]) {
        unimplemented!("MockHost::code_hash")
    }

    fn code_size(_addr: &[u8; 20]) -> u64 {
        unimplemented!("MockHost::code_size")
    }

    fn delegate_call(
        _flags: CallFlags,
        _address: &[u8; 20],
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit_limit: &[u8; 32],
        _input_data: &[u8],
        _output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        unimplemented!("MockHost::delegate_call")
    }

    fn delegate_call_evm(
        _flags: CallFlags,
        _address: &[u8; 20],
        _gas: u64,
        _input_data: &[u8],
        _output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        unimplemented!("MockHost::delegate_call_evm")
    }

    fn deposit_event(topics: &[[u8; 32]], data: &[u8]) {
        MOCK_STATE.with(|s| {
            s.borrow_mut().events.push((topics.to_vec(), data.to_vec()));
        });
    }

    fn get_storage(flags: StorageFlags, key: &[u8], output: &mut &mut [u8]) -> HostResult {
        let _ = flags;
        MOCK_STATE.with(|s| {
            let state = s.borrow();
            match state.storage.get(key) {
                Some(value) => {
                    let len = value.len().min(output.len());
                    output[..len].copy_from_slice(&value[..len]);
                    Ok(())
                }
                None => Err(ReturnErrorCode::KeyNotFound),
            }
        })
    }

    fn hash_keccak_256(input: &[u8], output: &mut [u8; 32]) {
        output.copy_from_slice(&tiny_keccak(input));
    }

    fn call_data_copy(output: &mut [u8], offset: u32) {
        MOCK_STATE.with(|s| {
            let state = s.borrow();
            let start = offset as usize;
            let len = output.len().min(state.calldata.len().saturating_sub(start));
            output[..len].copy_from_slice(&state.calldata[start..start + len]);
        });
    }

    fn call_data_load(output: &mut [u8; 32], offset: u32) {
        MOCK_STATE.with(|s| {
            let state = s.borrow();
            let start = offset as usize;
            output.fill(0);
            let len = 32.min(state.calldata.len().saturating_sub(start));
            output[..len].copy_from_slice(&state.calldata[start..start + len]);
        });
    }

    fn instantiate(
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit: &[u8; 32],
        _value: &[u8; 32],
        _input: &[u8],
        _address: Option<&mut [u8; 20]>,
        _output: Option<&mut &mut [u8]>,
        _salt: Option<&[u8; 32]>,
    ) -> HostResult {
        unimplemented!("MockHost::instantiate")
    }

    fn now(output: &mut [u8; 32]) {
        MOCK_STATE.with(|s| output.copy_from_slice(&s.borrow().block_timestamp));
    }

    fn gas_limit() -> u64 {
        u64::MAX
    }

    fn return_value(flags: ReturnFlags, return_value: &[u8]) -> ! {
        MOCK_STATE.with(|s| {
            s.borrow_mut().return_value = Some((flags, return_value.to_vec()));
        });
        panic!("MockHost::return_value called")
    }

    fn set_storage(flags: StorageFlags, key: &[u8], value: &[u8]) -> Option<u32> {
        let _ = flags;
        MOCK_STATE.with(|s| {
            let mut state = s.borrow_mut();
            let prev = state.storage.insert(key.to_vec(), value.to_vec());
            prev.map(|v| v.len() as u32)
        })
    }

    fn set_storage_or_clear(flags: StorageFlags, key: &[u8; 32], value: &[u8; 32]) -> Option<u32> {
        Self::set_storage(flags, key.as_slice(), value.as_slice())
    }

    fn get_storage_or_zero(flags: StorageFlags, key: &[u8; 32], output: &mut [u8; 32]) {
        let _ = flags;
        MOCK_STATE.with(|s| {
            let state = s.borrow();
            match state.storage.get(key.as_slice()) {
                Some(value) => {
                    output.fill(0);
                    let len = value.len().min(32);
                    output[..len].copy_from_slice(&value[..len]);
                }
                None => output.fill(0),
            }
        });
    }

    fn value_transferred(output: &mut [u8; 32]) {
        MOCK_STATE.with(|s| output.copy_from_slice(&s.borrow().value_transferred));
    }

    fn return_data_size() -> u64 {
        0
    }

    fn return_data_copy(_output: &mut &mut [u8], _offset: u32) {
        unimplemented!("MockHost::return_data_copy")
    }

    fn gas_left() -> u64 {
        u64::MAX
    }

    fn block_author(output: &mut [u8; 20]) {
        MOCK_STATE.with(|s| output.copy_from_slice(&s.borrow().block_author));
    }

    fn block_number(output: &mut [u8; 32]) {
        MOCK_STATE.with(|s| output.copy_from_slice(&s.borrow().block_number));
    }

    fn block_hash(_block_number: &[u8; 32], output: &mut [u8; 32]) {
        output.fill(0);
    }

    fn consume_all_gas() -> ! {
        panic!("MockHost::consume_all_gas called")
    }

    fn terminate(_beneficiary: &[u8; 20]) -> ! {
        panic!("MockHost::terminate called")
    }
}

// ---------------------------------------------------------------------------
// Keccak-256 (minimal implementation for mock use)
// ---------------------------------------------------------------------------

/// Minimal keccak-256 for test use. Not optimized.
fn tiny_keccak(input: &[u8]) -> [u8; 32] {
    const ROUND_CONSTANTS: [u64; 24] = [
        0x0000000000000001,
        0x0000000000008082,
        0x800000000000808a,
        0x8000000080008000,
        0x000000000000808b,
        0x0000000080000001,
        0x8000000080008081,
        0x8000000000008009,
        0x000000000000008a,
        0x0000000000000088,
        0x0000000080008009,
        0x000000008000000a,
        0x000000008000808b,
        0x800000000000008b,
        0x8000000000008089,
        0x8000000000008003,
        0x8000000000008002,
        0x8000000000000080,
        0x000000000000800a,
        0x800000008000000a,
        0x8000000080008081,
        0x8000000000008080,
        0x0000000080000001,
        0x8000000080008008,
    ];

    const ROTATION_OFFSETS: [u32; 25] = [
        0, 1, 62, 28, 27, 36, 44, 6, 55, 20, 3, 10, 43, 25, 39, 41, 45, 15, 21, 8, 18, 2, 61, 56,
        14,
    ];

    const PI: [usize; 25] = [
        0, 10, 20, 5, 15, 16, 1, 11, 21, 6, 7, 17, 2, 12, 22, 23, 8, 18, 3, 13, 14, 24, 9, 19, 4,
    ];

    let rate = 136;
    let mut state = [0u64; 25];

    let mut padded = input.to_vec();
    padded.push(0x01);
    while !padded.len().is_multiple_of(rate) {
        padded.push(0x00);
    }
    let last = padded.len() - 1;
    padded[last] ^= 0x80;

    for block in padded.chunks(rate) {
        for (i, chunk) in block.chunks(8).enumerate() {
            if i < 25 {
                let mut bytes = [0u8; 8];
                bytes[..chunk.len()].copy_from_slice(chunk);
                state[i] ^= u64::from_le_bytes(bytes);
            }
        }

        for round_constant in &ROUND_CONSTANTS {
            let mut c = [0u64; 5];
            for x in 0..5 {
                c[x] = state[x] ^ state[x + 5] ^ state[x + 10] ^ state[x + 15] ^ state[x + 20];
            }
            let mut d = [0u64; 5];
            for x in 0..5 {
                d[x] = c[(x + 4) % 5] ^ c[(x + 1) % 5].rotate_left(1);
            }
            for i in 0..25 {
                state[i] ^= d[i % 5];
            }

            let mut b = [0u64; 25];
            for i in 0..25 {
                b[PI[i]] = state[i].rotate_left(ROTATION_OFFSETS[i]);
            }

            for y in 0..5 {
                for x in 0..5 {
                    state[y * 5 + x] =
                        b[y * 5 + x] ^ (!b[y * 5 + (x + 1) % 5] & b[y * 5 + (x + 2) % 5]);
                }
            }

            state[0] ^= round_constant;
        }
    }

    let mut output = [0u8; 32];
    for (i, chunk) in output.chunks_mut(8).enumerate() {
        let bytes = state[i].to_le_bytes();
        chunk.copy_from_slice(&bytes[..chunk.len()]);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keccak256_empty() {
        let hash = tiny_keccak(b"");
        assert_eq!(
            hash,
            [
                0xc5, 0xd2, 0x46, 0x01, 0x86, 0xf7, 0x23, 0x3c, 0x92, 0x7e, 0x7d, 0xb2, 0xdc, 0xc7,
                0x03, 0xc0, 0xe5, 0x00, 0xb6, 0x53, 0xca, 0x82, 0x27, 0x3b, 0x7b, 0xfa, 0xd8, 0x04,
                0x5d, 0x85, 0xa4, 0x70,
            ]
        );
    }

    #[test]
    fn mock_storage_roundtrip() {
        MockHost::reset();
        let key = [1u8; 32];
        let value = [42u8; 32];

        MockHost::set_storage(StorageFlags::empty(), &key, &value);

        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        let result = MockHost::get_storage(StorageFlags::empty(), &key, &mut out);
        assert!(result.is_ok());
        assert_eq!(buf, value);
    }

    #[test]
    fn mock_storage_key_not_found() {
        MockHost::reset();
        let key = [99u8; 32];
        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        let result = MockHost::get_storage(StorageFlags::empty(), &key, &mut out);
        assert_eq!(result, Err(ReturnErrorCode::KeyNotFound));
    }

    #[test]
    fn mock_caller() {
        MockHost::reset();
        MockHost::set_caller([0xAA; 20]);
        let mut output = [0u8; 20];
        MockHost::caller(&mut output);
        assert_eq!(output, [0xAA; 20]);
    }

    #[test]
    fn mock_calldata() {
        MockHost::reset();
        MockHost::set_calldata(vec![1, 2, 3, 4, 5]);
        assert_eq!(MockHost::call_data_size(), 5);

        let mut buf = [0u8; 5];
        MockHost::call_data_copy(&mut buf, 0);
        assert_eq!(buf, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn mock_events() {
        MockHost::reset();
        let topics = [[1u8; 32], [2u8; 32]];
        let data = [3u8; 64];
        MockHost::deposit_event(&topics, &data);

        let events = MockHost::events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, topics.to_vec());
        assert_eq!(events[0].1, data.to_vec());
    }

    #[test]
    fn mock_return_value_panics() {
        MockHost::reset();
        let result = std::panic::catch_unwind(|| {
            MockHost::return_value(ReturnFlags::empty(), &[1, 2, 3]);
        });
        assert!(result.is_err());
        let (flags, data) = MockHost::take_return_value().unwrap();
        assert_eq!(flags, ReturnFlags::empty());
        assert_eq!(data, vec![1, 2, 3]);
    }

    #[test]
    fn builder_sets_all_fields() {
        MockHostBuilder::new()
            .caller([0xAA; 20])
            .origin([0xBB; 20])
            .address([0xCC; 20])
            .block_number([0u8; 32])
            .calldata(vec![1, 2, 3, 4])
            .install();

        let mut caller = [0u8; 20];
        MockHost::caller(&mut caller);
        assert_eq!(caller, [0xAA; 20]);

        let mut origin = [0u8; 20];
        MockHost::origin(&mut origin);
        assert_eq!(origin, [0xBB; 20]);

        let mut address = [0u8; 20];
        MockHost::address(&mut address);
        assert_eq!(address, [0xCC; 20]);

        assert_eq!(MockHost::call_data_size(), 4);
    }

    #[test]
    fn builder_with_pre_populated_storage() {
        let key = [7u8; 32];
        let value = [99u8; 32];

        MockHostBuilder::new()
            .storage(vec![(key.to_vec(), value.to_vec())])
            .install();

        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        assert!(MockHost::get_storage(StorageFlags::empty(), &key, &mut out).is_ok());
        assert_eq!(buf, value);
    }

    #[test]
    fn mock_call_returns_configured_data() {
        MockHost::reset();
        let callee = [0xBB; 20];
        MockHost::mock_call(callee, Ok(vec![0, 0, 0, 1]));

        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        let result = MockHost::call(
            CallFlags::empty(),
            &callee,
            0,
            0,
            &[0u8; 32],
            &[0u8; 32],
            &[],
            Some(&mut out),
        );
        assert!(result.is_ok());
        assert_eq!(&buf[..4], &[0, 0, 0, 1]);
    }

    #[test]
    fn mock_call_returns_revert() {
        MockHost::reset();
        let callee = [0xCC; 20];
        MockHost::mock_call(callee, Err(()));

        let result = MockHost::call(
            CallFlags::empty(),
            &callee,
            0,
            0,
            &[0u8; 32],
            &[0u8; 32],
            &[],
            None,
        );
        assert_eq!(result, Err(ReturnErrorCode::CalleeReverted));
    }

    #[test]
    fn mock_call_unknown_callee_returns_ok() {
        MockHost::reset();
        let callee = [0xDD; 20];
        let result = MockHost::call(
            CallFlags::empty(),
            &callee,
            0,
            0,
            &[0u8; 32],
            &[0u8; 32],
            &[],
            None,
        );
        assert!(result.is_ok());
    }

    #[test]
    fn mock_now_returns_timestamp() {
        let mut ts = [0u8; 32];
        ts[31] = 42;
        MockHostBuilder::new().block_timestamp(ts).install();

        let mut output = [0u8; 32];
        MockHost::now(&mut output);
        assert_eq!(output[31], 42);
    }
}

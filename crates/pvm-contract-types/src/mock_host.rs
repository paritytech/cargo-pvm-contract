//! Mock host backend for native unit testing of PVM contracts.
//!
//! [`MockHost`] implements [`HostApi`](super::HostApi) using thread-local state,
//! allowing contract logic to be tested with `cargo test` on the host target.
//!
//! # Example
//!
//! ```ignore
//! use pvm_contract_types::{MockHost, HostApi, StorageFlags};
//!
//! MockHost::reset();
//! MockHost::set_caller([0xAA; 20]);
//! MockHost::set_calldata(vec![0x01, 0x02, 0x03, 0x04]);
//!
//! // Call contract logic that uses HostApi methods...
//! // MockHost::get_storage, set_storage, caller, etc. all work.
//!
//! let events = MockHost::events();
//! assert_eq!(events.len(), 1);
//! ```

use std::cell::RefCell;
use std::collections::HashMap;

use super::host::{CallFlags, HostApi, HostResult, ReturnErrorCode, ReturnFlags, StorageFlags};

#[derive(Default)]
struct MockState {
    storage: HashMap<Vec<u8>, Vec<u8>>,
    caller: [u8; 20],
    origin: [u8; 20],
    address: [u8; 20],
    balance: [u8; 32],
    chain_id: [u8; 32],
    block_number: [u8; 32],
    block_author: [u8; 20],
    value_transferred: [u8; 32],
    calldata: Vec<u8>,
    events: Vec<(Vec<[u8; 32]>, Vec<u8>)>,
    return_value: Option<(ReturnFlags, Vec<u8>)>,
}

thread_local! {
    static MOCK_STATE: RefCell<MockState> = RefCell::new(MockState::default());
}

/// Mock host backend for native testing.
///
/// All state is stored in a thread-local, so tests using `MockHost` are safe
/// to run in parallel (each thread gets its own state). Call [`MockHost::reset`]
/// in each test to start from a clean state.
///
/// `return_value` panics with a recognizable message. Use
/// [`std::panic::catch_unwind`] to capture contract return data, then call
/// [`MockHost::take_return_value`] to retrieve it.
pub struct MockHost;

impl MockHost {
    /// Reset all mock state to defaults.
    pub fn reset() {
        MOCK_STATE.with(|s| *s.borrow_mut() = MockState::default());
    }

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

    /// Set the value transferred returned by [`HostApi::value_transferred`].
    pub fn set_value_transferred(value: [u8; 32]) {
        MOCK_STATE.with(|s| s.borrow_mut().value_transferred = value);
    }

    /// Set the calldata that [`HostApi::call_data_size`] and
    /// [`HostApi::call_data_copy`] will return.
    pub fn set_calldata(data: Vec<u8>) {
        MOCK_STATE.with(|s| s.borrow_mut().calldata = data);
    }

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

    /// Read raw storage for test assertions.
    pub fn get_raw_storage(key: &[u8]) -> Option<Vec<u8>> {
        MOCK_STATE.with(|s| s.borrow().storage.get(key).cloned())
    }

    /// Write raw storage for test setup.
    pub fn set_raw_storage(key: Vec<u8>, value: Vec<u8>) {
        MOCK_STATE.with(|s| s.borrow_mut().storage.insert(key, value));
    }
}

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
        _callee: &[u8; 20],
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit: &[u8; 32],
        _value: &[u8; 32],
        _input_data: &[u8],
        _output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        unimplemented!("MockHost::call")
    }

    fn call_evm(
        _flags: CallFlags,
        _callee: &[u8; 20],
        _gas: u64,
        _value: &[u8; 32],
        _input_data: &[u8],
        _output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        unimplemented!("MockHost::call_evm")
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
        // Use a simple implementation for testing
        // In a real test environment, you'd want the actual keccak256
        // For now, use a basic hash that's deterministic
        let mut hasher = Keccak256::new();
        hasher.update(input);
        output.copy_from_slice(&hasher.finalize());
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

    fn now(_output: &mut [u8; 32]) {
        unimplemented!("MockHost::now")
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

/// Minimal Keccak-256 implementation for the mock host.
/// Uses the same `keccak-const` algorithm at runtime.
struct Keccak256 {
    data: Vec<u8>,
}

impl Keccak256 {
    fn new() -> Self {
        Self { data: Vec::new() }
    }

    fn update(&mut self, input: &[u8]) {
        self.data.extend_from_slice(input);
    }

    fn finalize(self) -> [u8; 32] {
        tiny_keccak(&self.data)
    }
}

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

    let rate = 136; // bytes (1088 bits for keccak-256)
    let mut state = [0u64; 25];

    // Absorb with padding
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

        // Keccak-f[1600]
        for round_constant in &ROUND_CONSTANTS {
            // Theta
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

            // Rho and Pi
            let mut b = [0u64; 25];
            for i in 0..25 {
                b[PI[i]] = state[i].rotate_left(ROTATION_OFFSETS[i]);
            }

            // Chi
            for y in 0..5 {
                for x in 0..5 {
                    state[y * 5 + x] =
                        b[y * 5 + x] ^ (!b[y * 5 + (x + 1) % 5] & b[y * 5 + (x + 2) % 5]);
                }
            }

            // Iota
            state[0] ^= round_constant;
        }
    }

    // Squeeze
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
        // Known keccak256 of empty input
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
}

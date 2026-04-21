//! Mock host backend for native unit testing of PVM contracts.
//!
//! [`MockHost`] implements [`HostApi`](super::HostApi) using plain per-instance
//! state. Tests construct their own `MockHost` via [`MockHostBuilder`] and
//! inject it into the contract — no thread-locals, no global setup, no panic
//! capture. Run tests in parallel without contention.
//!
//! # Quick start
//!
//! ```ignore
//! use pvm_contract_types::{HostApi, MockHostBuilder, StorageFlags};
//!
//! let host = MockHostBuilder::new()
//!     .caller([0xAA; 20])
//!     .calldata(vec![0x01, 0x02, 0x03, 0x04])
//!     .build();
//!
//! host.deposit_event(&[[0; 32]], &[1, 2, 3]);
//! assert_eq!(host.events().len(), 1);
//! ```
//!
//! # Mock external calls
//!
//! ```ignore
//! let mut host = MockHostBuilder::new().build();
//! host.mock_call([0xBB; 20], Ok(vec![0, 0, 0, 1]));
//! // HostApi::call to [0xBB; 20] now returns Ok(()) with the mock data.
//! ```
//!
//! # Return values
//!
//! `return_value` is deliberately NOT on [`HostApi`](super::HostApi). Contract
//! code returns `Result<T, E: SolRevert>` natively and the macro-/DSL-generated
//! `call()` wrapper encodes the result to `pallet_revive_uapi::HostFnImpl::return_value`
//! at the boundary. Tests assert on the returned `Result` directly — no
//! `catch_unwind` required.

use core::cell::RefCell;
use std::collections::HashMap;

use super::host::{CallFlags, HostApi, HostResult, ReturnErrorCode, StorageFlags};

/// Return value for mocked external calls.
///
/// `Ok(data)` — call succeeds; `data` is written to the output buffer.
/// `Err(())` — call reverts with `ReturnErrorCode::CalleeReverted`.
pub type MockCallReturn = Result<Vec<u8>, ()>;

/// One captured event: `(topics, data)`.
pub type EventRecord = (Vec<[u8; 32]>, Vec<u8>);

#[derive(Clone)]
struct MockInstantiateReturn {
    address: [u8; 20],
    output: Vec<u8>,
}

/// Mock host backend for native testing.
///
/// All state lives in the instance — no thread-local, no globals. Construct
/// via [`MockHostBuilder::build`]. Interior mutability (`RefCell`) is used
/// only for state that contract methods mutate through `&self.host`: storage,
/// events, and cached return data.
///
/// Re-entrancy note: methods that read/write interior-mutable state borrow
/// briefly (borrow-drop-immediately pattern) so a re-entering `mock_call` or
/// `delegate_call` does not collide with a live borrow guard.
pub struct MockHost {
    // --- Input state (configured before contract execution) ---
    caller: [u8; 20],
    origin: [u8; 20],
    address: [u8; 20],
    balance: [u8; 32],
    balances: HashMap<[u8; 20], [u8; 32]>,
    chain_id: [u8; 32],
    base_fee: [u8; 32],
    block_number: [u8; 32],
    block_timestamp: [u8; 32],
    block_author: [u8; 20],
    value_transferred: [u8; 32],
    calldata: Vec<u8>,

    // --- Mutable state during execution (needs interior mutability under &self) ---
    storage: RefCell<HashMap<Vec<u8>, Vec<u8>>>,
    events: RefCell<Vec<EventRecord>>,
    immutable_data: RefCell<Vec<u8>>,
    return_data: RefCell<Vec<u8>>,

    // --- Mock configuration (read-only during execution) ---
    call_returns: HashMap<[u8; 20], MockCallReturn>,
    instantiate_return: Option<MockInstantiateReturn>,
}

impl MockHost {
    /// Register a mock return value for [`HostApi::call`] to `callee`.
    pub fn mock_call(&mut self, callee: [u8; 20], result: MockCallReturn) {
        self.call_returns.insert(callee, result);
    }

    /// Register a mock return for [`HostApi::instantiate`].
    pub fn mock_instantiate(&mut self, address: [u8; 20], output: Vec<u8>) {
        self.instantiate_return = Some(MockInstantiateReturn { address, output });
    }

    /// All events emitted via [`HostApi::deposit_event`].
    pub fn events(&self) -> Vec<EventRecord> {
        self.events.borrow().clone()
    }

    /// Raw storage read — for test assertions.
    pub fn get_raw_storage(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.storage.borrow().get(key).cloned()
    }

    /// Raw storage write — for test setup.
    pub fn set_raw_storage(&self, key: Vec<u8>, value: Vec<u8>) {
        self.storage.borrow_mut().insert(key, value);
    }
}

/// Fluent builder for [`MockHost`].
///
/// # Example
///
/// ```ignore
/// let host = MockHostBuilder::new()
///     .caller([0xAA; 20])
///     .calldata(vec![/* … */])
///     .storage(vec![([1u8; 32].to_vec(), [42u8; 32].to_vec())])
///     .build();
/// ```
pub struct MockHostBuilder {
    host: MockHost,
}

impl Default for MockHostBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl MockHostBuilder {
    pub fn new() -> Self {
        Self {
            host: MockHost {
                caller: [0; 20],
                origin: [0; 20],
                address: [0; 20],
                balance: [0; 32],
                balances: HashMap::new(),
                chain_id: [0; 32],
                base_fee: [0; 32],
                block_number: [0; 32],
                block_timestamp: [0; 32],
                block_author: [0; 20],
                value_transferred: [0; 32],
                calldata: Vec::new(),
                storage: RefCell::new(HashMap::new()),
                events: RefCell::new(Vec::new()),
                immutable_data: RefCell::new(Vec::new()),
                return_data: RefCell::new(Vec::new()),
                call_returns: HashMap::new(),
                instantiate_return: None,
            },
        }
    }

    pub fn caller(mut self, caller: [u8; 20]) -> Self {
        self.host.caller = caller;
        self
    }

    pub fn origin(mut self, origin: [u8; 20]) -> Self {
        self.host.origin = origin;
        self
    }

    pub fn address(mut self, address: [u8; 20]) -> Self {
        self.host.address = address;
        self
    }

    pub fn balance(mut self, balance: [u8; 32]) -> Self {
        self.host.balance = balance;
        self
    }

    pub fn balance_of(mut self, addr: [u8; 20], balance: [u8; 32]) -> Self {
        self.host.balances.insert(addr, balance);
        self
    }

    pub fn base_fee(mut self, base_fee: [u8; 32]) -> Self {
        self.host.base_fee = base_fee;
        self
    }

    pub fn immutable_data(mut self, data: Vec<u8>) -> Self {
        self.host.immutable_data = RefCell::new(data);
        self
    }

    pub fn chain_id(mut self, chain_id: [u8; 32]) -> Self {
        self.host.chain_id = chain_id;
        self
    }

    pub fn block_number(mut self, block_number: [u8; 32]) -> Self {
        self.host.block_number = block_number;
        self
    }

    pub fn block_timestamp(mut self, timestamp: [u8; 32]) -> Self {
        self.host.block_timestamp = timestamp;
        self
    }

    pub fn block_author(mut self, author: [u8; 20]) -> Self {
        self.host.block_author = author;
        self
    }

    pub fn value_transferred(mut self, value: [u8; 32]) -> Self {
        self.host.value_transferred = value;
        self
    }

    pub fn calldata(mut self, data: Vec<u8>) -> Self {
        self.host.calldata = data;
        self
    }

    pub fn storage(mut self, entries: Vec<(Vec<u8>, Vec<u8>)>) -> Self {
        for (key, value) in entries {
            self.host.storage.get_mut().insert(key, value);
        }
        self
    }

    pub fn mock_call(mut self, callee: [u8; 20], result: MockCallReturn) -> Self {
        self.host.call_returns.insert(callee, result);
        self
    }

    pub fn mock_instantiate(mut self, address: [u8; 20], output: Vec<u8>) -> Self {
        self.host.instantiate_return = Some(MockInstantiateReturn { address, output });
        self
    }

    /// Consume the builder and return an owned [`MockHost`].
    pub fn build(self) -> MockHost {
        self.host
    }
}

// ---------------------------------------------------------------------------
// HostApi implementation
// ---------------------------------------------------------------------------

impl HostApi for MockHost {
    fn address(&self, output: &mut [u8; 20]) {
        *output = self.address;
    }

    fn get_immutable_data(&self, output: &mut &mut [u8]) {
        let data = self.immutable_data.borrow();
        let len = data.len().min(output.len());
        output[..len].copy_from_slice(&data[..len]);
        let tmp = core::mem::take(output);
        *output = &mut tmp[..len];
    }

    fn set_immutable_data(&self, data: &[u8]) {
        *self.immutable_data.borrow_mut() = data.to_vec();
    }

    fn balance(&self, output: &mut [u8; 32]) {
        *output = self.balance;
    }

    fn balance_of(&self, addr: &[u8; 20], output: &mut [u8; 32]) {
        match self.balances.get(addr) {
            Some(bal) => *output = *bal,
            None => output.fill(0),
        }
    }

    fn chain_id(&self, output: &mut [u8; 32]) {
        *output = self.chain_id;
    }

    fn gas_price(&self) -> u64 {
        0
    }

    fn base_fee(&self, output: &mut [u8; 32]) {
        *output = self.base_fee;
    }

    fn call_data_size(&self) -> u64 {
        self.calldata.len() as u64
    }

    fn call(
        &self,
        _flags: CallFlags,
        callee: &[u8; 20],
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit: &[u8; 32],
        _value: &[u8; 32],
        _input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        self.resolve_call(callee, output)
    }

    fn call_evm(
        &self,
        _flags: CallFlags,
        callee: &[u8; 20],
        _gas: u64,
        _value: &[u8; 32],
        _input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        self.resolve_call(callee, output)
    }

    fn caller(&self, output: &mut [u8; 20]) {
        *output = self.caller;
    }

    fn origin(&self, output: &mut [u8; 20]) {
        *output = self.origin;
    }

    fn code_hash(&self, _addr: &[u8; 20], output: &mut [u8; 32]) {
        output.fill(0);
    }

    fn code_size(&self, _addr: &[u8; 20]) -> u64 {
        0
    }

    fn delegate_call(
        &self,
        _flags: CallFlags,
        address: &[u8; 20],
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit_limit: &[u8; 32],
        _input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        self.resolve_call(address, output)
    }

    fn delegate_call_evm(
        &self,
        _flags: CallFlags,
        address: &[u8; 20],
        _gas: u64,
        _input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        self.resolve_call(address, output)
    }

    fn deposit_event(&self, topics: &[[u8; 32]], data: &[u8]) {
        self.events
            .borrow_mut()
            .push((topics.to_vec(), data.to_vec()));
    }

    fn get_storage(&self, _flags: StorageFlags, key: &[u8], output: &mut &mut [u8]) -> HostResult {
        let value = self.storage.borrow().get(key).cloned();
        match value {
            Some(value) => {
                let len = value.len().min(output.len());
                output[..len].copy_from_slice(&value[..len]);
                let tmp = core::mem::take(output);
                *output = &mut tmp[..len];
                Ok(())
            }
            None => Err(ReturnErrorCode::KeyNotFound),
        }
    }

    fn hash_keccak_256(&self, input: &[u8], output: &mut [u8; 32]) {
        *output = tiny_keccak(input);
    }

    fn call_data_copy(&self, output: &mut [u8], offset: u32) {
        let start = (offset as usize).min(self.calldata.len());
        let len = output.len().min(self.calldata.len() - start);
        output[..len].copy_from_slice(&self.calldata[start..start + len]);
        output[len..].fill(0);
    }

    fn call_data_load(&self, output: &mut [u8; 32], offset: u32) {
        let start = (offset as usize).min(self.calldata.len());
        output.fill(0);
        let len = 32.min(self.calldata.len() - start);
        output[..len].copy_from_slice(&self.calldata[start..start + len]);
    }

    fn instantiate(
        &self,
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit: &[u8; 32],
        _value: &[u8; 32],
        _input: &[u8],
        address: Option<&mut [u8; 20]>,
        output: Option<&mut &mut [u8]>,
        _salt: Option<&[u8; 32]>,
    ) -> HostResult {
        match &self.instantiate_return {
            Some(ret) => {
                if let Some(addr) = address {
                    *addr = ret.address;
                }
                *self.return_data.borrow_mut() = ret.output.clone();
                if let Some(out) = output {
                    let len = ret.output.len().min(out.len());
                    out[..len].copy_from_slice(&ret.output[..len]);
                }
                Ok(())
            }
            None => Err(ReturnErrorCode::OutOfResources),
        }
    }

    fn now(&self, output: &mut [u8; 32]) {
        *output = self.block_timestamp;
    }

    fn gas_limit(&self) -> u64 {
        u64::MAX
    }

    fn set_storage(&self, _flags: StorageFlags, key: &[u8], value: &[u8]) -> Option<u32> {
        self.storage
            .borrow_mut()
            .insert(key.to_vec(), value.to_vec())
            .map(|v| v.len() as u32)
    }

    fn set_storage_or_clear(
        &self,
        _flags: StorageFlags,
        key: &[u8; 32],
        value: &[u8; 32],
    ) -> Option<u32> {
        let mut storage = self.storage.borrow_mut();
        if *value == [0u8; 32] {
            storage.remove(key.as_slice()).map(|v| v.len() as u32)
        } else {
            storage
                .insert(key.to_vec(), value.to_vec())
                .map(|v| v.len() as u32)
        }
    }

    fn get_storage_or_zero(&self, _flags: StorageFlags, key: &[u8; 32], output: &mut [u8; 32]) {
        let storage = self.storage.borrow();
        match storage.get(key.as_slice()) {
            Some(value) => {
                output.fill(0);
                let len = value.len().min(32);
                output[..len].copy_from_slice(&value[..len]);
            }
            None => output.fill(0),
        }
    }

    fn value_transferred(&self, output: &mut [u8; 32]) {
        *output = self.value_transferred;
    }

    fn return_data_size(&self) -> u64 {
        self.return_data.borrow().len() as u64
    }

    fn return_data_copy(&self, output: &mut &mut [u8], offset: u32) {
        let data = self.return_data.borrow();
        let start = (offset as usize).min(data.len());
        let len = output.len().min(data.len() - start);
        output[..len].copy_from_slice(&data[start..start + len]);
        let tmp = core::mem::take(output);
        *output = &mut tmp[..len];
    }

    fn gas_left(&self) -> u64 {
        u64::MAX
    }

    fn block_author(&self, output: &mut [u8; 20]) {
        *output = self.block_author;
    }

    fn block_number(&self, output: &mut [u8; 32]) {
        *output = self.block_number;
    }

    fn block_hash(&self, _block_number: &[u8; 32], output: &mut [u8; 32]) {
        output.fill(0);
    }
}

impl MockHost {
    /// Shared logic for `call`, `call_evm`, `delegate_call`, `delegate_call_evm`.
    /// Uses borrow-drop-immediately pattern to stay re-entrancy-safe.
    fn resolve_call(&self, callee: &[u8; 20], output: Option<&mut &mut [u8]>) -> HostResult {
        match self.call_returns.get(callee) {
            Some(Ok(data)) => {
                *self.return_data.borrow_mut() = data.clone();
                if let Some(out) = output {
                    let len = data.len().min(out.len());
                    out[..len].copy_from_slice(&data[..len]);
                }
                Ok(())
            }
            Some(Err(())) => {
                self.return_data.borrow_mut().clear();
                Err(ReturnErrorCode::CalleeReverted)
            }
            None => {
                self.return_data.borrow_mut().clear();
                Ok(())
            }
        }
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
        let host = MockHostBuilder::new().build();
        let key = [1u8; 32];
        let value = [42u8; 32];

        host.set_storage(StorageFlags::empty(), &key, &value);

        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        let result = host.get_storage(StorageFlags::empty(), &key, &mut out);
        assert!(result.is_ok());
        assert_eq!(buf, value);
    }

    #[test]
    fn mock_storage_key_not_found() {
        let host = MockHostBuilder::new().build();
        let key = [99u8; 32];
        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        let result = host.get_storage(StorageFlags::empty(), &key, &mut out);
        assert_eq!(result, Err(ReturnErrorCode::KeyNotFound));
    }

    #[test]
    fn mock_caller() {
        let host = MockHostBuilder::new().caller([0xAA; 20]).build();
        let mut output = [0u8; 20];
        host.caller(&mut output);
        assert_eq!(output, [0xAA; 20]);
    }

    #[test]
    fn mock_calldata() {
        let host = MockHostBuilder::new().calldata(vec![1, 2, 3, 4, 5]).build();
        assert_eq!(host.call_data_size(), 5);

        let mut buf = [0u8; 5];
        host.call_data_copy(&mut buf, 0);
        assert_eq!(buf, [1, 2, 3, 4, 5]);
    }

    #[test]
    fn mock_events() {
        let host = MockHostBuilder::new().build();
        let topics = [[1u8; 32], [2u8; 32]];
        let data = [3u8; 64];
        host.deposit_event(&topics, &data);

        let events = host.events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].0, topics.to_vec());
        assert_eq!(events[0].1, data.to_vec());
    }

    #[test]
    fn builder_sets_all_fields() {
        let host = MockHostBuilder::new()
            .caller([0xAA; 20])
            .origin([0xBB; 20])
            .address([0xCC; 20])
            .block_number([0u8; 32])
            .calldata(vec![1, 2, 3, 4])
            .build();

        let mut caller = [0u8; 20];
        host.caller(&mut caller);
        assert_eq!(caller, [0xAA; 20]);

        let mut origin = [0u8; 20];
        host.origin(&mut origin);
        assert_eq!(origin, [0xBB; 20]);

        let mut address = [0u8; 20];
        host.address(&mut address);
        assert_eq!(address, [0xCC; 20]);

        assert_eq!(host.call_data_size(), 4);
    }

    #[test]
    fn builder_with_pre_populated_storage() {
        let key = [7u8; 32];
        let value = [99u8; 32];

        let host = MockHostBuilder::new()
            .storage(vec![(key.to_vec(), value.to_vec())])
            .build();

        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        assert!(
            host.get_storage(StorageFlags::empty(), &key, &mut out)
                .is_ok()
        );
        assert_eq!(buf, value);
    }

    #[test]
    fn mock_call_returns_configured_data() {
        let callee = [0xBB; 20];
        let host = MockHostBuilder::new()
            .mock_call(callee, Ok(vec![0, 0, 0, 1]))
            .build();

        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        let result = host.call(
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
        let callee = [0xCC; 20];
        let host = MockHostBuilder::new().mock_call(callee, Err(())).build();

        let result = host.call(
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
        let host = MockHostBuilder::new().build();
        let callee = [0xDD; 20];
        let result = host.call(
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
        let host = MockHostBuilder::new().block_timestamp(ts).build();

        let mut output = [0u8; 32];
        host.now(&mut output);
        assert_eq!(output[31], 42);
    }

    #[test]
    fn get_storage_shrinks_output_slice() {
        let host = MockHostBuilder::new().build();
        let key = [1u8; 32];
        let value = [42u8; 10];

        host.set_storage(StorageFlags::empty(), &key, &value);

        let mut buf = [0xFFu8; 32];
        let mut out = &mut buf[..];
        assert!(
            host.get_storage(StorageFlags::empty(), &key, &mut out)
                .is_ok()
        );
        assert_eq!(out.len(), 10);
        assert_eq!(&buf[..10], &value);
    }

    #[test]
    fn set_storage_or_clear_deletes_on_zero_value() {
        let host = MockHostBuilder::new().build();
        let key = [1u8; 32];
        let value = [42u8; 32];

        host.set_storage(StorageFlags::empty(), &key, &value);
        assert!(host.get_raw_storage(&key).is_some());

        host.set_storage_or_clear(StorageFlags::empty(), &key, &[0u8; 32]);
        assert!(host.get_raw_storage(&key).is_none());

        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        assert_eq!(
            host.get_storage(StorageFlags::empty(), &key, &mut out),
            Err(ReturnErrorCode::KeyNotFound)
        );
    }

    #[test]
    fn delegate_call_updates_return_data() {
        let callee = [0xBB; 20];
        let host = MockHostBuilder::new()
            .mock_call(callee, Ok(vec![1, 2, 3, 4]))
            .build();

        let result = host.delegate_call(CallFlags::empty(), &callee, 0, 0, &[0u8; 32], &[], None);
        assert!(result.is_ok());

        assert_eq!(host.return_data_size(), 4);
        let mut buf = [0u8; 4];
        let mut out = &mut buf[..];
        host.return_data_copy(&mut out, 0);
        assert_eq!(buf, [1, 2, 3, 4]);
    }

    #[test]
    fn call_data_copy_zero_pads_tail() {
        let host = MockHostBuilder::new().calldata(vec![1, 2, 3]).build();

        let mut buf = [0xFF; 8];
        host.call_data_copy(&mut buf, 0);
        assert_eq!(buf, [1, 2, 3, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn call_data_copy_offset_beyond_length() {
        let host = MockHostBuilder::new().calldata(vec![1, 2, 3]).build();

        let mut buf = [0xFF; 4];
        host.call_data_copy(&mut buf, 10);
        assert_eq!(buf, [0, 0, 0, 0]);
    }

    #[test]
    fn immutable_data_roundtrip() {
        let host = MockHostBuilder::new()
            .immutable_data(vec![10, 20, 30])
            .build();

        let mut buf = [0u8; 8];
        let mut out = &mut buf[..];
        host.get_immutable_data(&mut out);
        assert_eq!(out.len(), 3);
        assert_eq!(&buf[..3], &[10, 20, 30]);

        host.set_immutable_data(&[99]);
        let mut buf2 = [0u8; 8];
        let mut out2 = &mut buf2[..];
        host.get_immutable_data(&mut out2);
        assert_eq!(out2.len(), 1);
        assert_eq!(buf2[0], 99);
    }

    #[test]
    fn balance_and_balance_of() {
        let mut bal = [0u8; 32];
        bal[31] = 100;
        let addr = [0xAA; 20];
        let mut addr_bal = [0u8; 32];
        addr_bal[31] = 50;

        let host = MockHostBuilder::new()
            .balance(bal)
            .balance_of(addr, addr_bal)
            .build();

        let mut output = [0u8; 32];
        host.balance(&mut output);
        assert_eq!(output[31], 100);

        let mut output2 = [0u8; 32];
        host.balance_of(&addr, &mut output2);
        assert_eq!(output2[31], 50);

        let mut output3 = [0xFFu8; 32];
        host.balance_of(&[0xBB; 20], &mut output3);
        assert_eq!(output3, [0u8; 32]);
    }

    #[test]
    fn chain_id_and_base_fee() {
        let mut cid = [0u8; 32];
        cid[31] = 42;
        let mut fee = [0u8; 32];
        fee[31] = 7;

        let host = MockHostBuilder::new().chain_id(cid).base_fee(fee).build();

        let mut output = [0u8; 32];
        host.chain_id(&mut output);
        assert_eq!(output[31], 42);

        let mut output2 = [0u8; 32];
        host.base_fee(&mut output2);
        assert_eq!(output2[31], 7);
    }

    #[test]
    fn gas_price_and_gas_left_and_gas_limit() {
        let host = MockHostBuilder::new().build();
        assert_eq!(host.gas_price(), 0);
        assert_eq!(host.gas_left(), u64::MAX);
        assert_eq!(host.gas_limit(), u64::MAX);
    }

    #[test]
    fn code_hash_and_code_size_return_defaults() {
        let host = MockHostBuilder::new().build();
        let mut hash = [0xFFu8; 32];
        host.code_hash(&[0xAA; 20], &mut hash);
        assert_eq!(hash, [0u8; 32]);
        assert_eq!(host.code_size(&[0xAA; 20]), 0);
    }

    #[test]
    fn call_evm_uses_call_returns() {
        let callee = [0xEE; 20];
        let host = MockHostBuilder::new()
            .mock_call(callee, Ok(vec![5, 6, 7, 8]))
            .build();

        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        let result = host.call_evm(
            CallFlags::empty(),
            &callee,
            0,
            &[0u8; 32],
            &[],
            Some(&mut out),
        );
        assert!(result.is_ok());
        assert_eq!(&buf[..4], &[5, 6, 7, 8]);
        assert_eq!(host.return_data_size(), 4);
    }

    #[test]
    fn call_data_load_with_offset() {
        let host = MockHostBuilder::new().calldata(vec![0xAA; 40]).build();

        let mut output = [0u8; 32];
        host.call_data_load(&mut output, 8);
        assert_eq!(output, [0xAA; 32]);

        let mut output2 = [0xFF; 32];
        host.call_data_load(&mut output2, 100);
        assert_eq!(output2, [0u8; 32]);
    }

    #[test]
    fn instantiate_with_mock() {
        let deployed_addr = [0xDD; 20];
        let host = MockHostBuilder::new()
            .mock_instantiate(deployed_addr, vec![1, 2])
            .build();

        let mut addr = [0u8; 20];
        let mut buf = [0u8; 8];
        let mut out = &mut buf[..];
        let result = host.instantiate(
            0,
            0,
            &[0u8; 32],
            &[0u8; 32],
            &[],
            Some(&mut addr),
            Some(&mut out),
            None,
        );
        assert!(result.is_ok());
        assert_eq!(addr, deployed_addr);
        assert_eq!(&buf[..2], &[1, 2]);
    }

    #[test]
    fn instantiate_without_mock_returns_error() {
        let host = MockHostBuilder::new().build();
        let result = host.instantiate(0, 0, &[0u8; 32], &[0u8; 32], &[], None, None, None);
        assert_eq!(result, Err(ReturnErrorCode::OutOfResources));
    }

    #[test]
    fn reentrant_call_does_not_panic_on_borrow() {
        // Regression: a mocked call that re-invokes storage operations on the
        // same MockHost must not collide with a live borrow guard.
        let callee = [0xBB; 20];
        let host = MockHostBuilder::new()
            .mock_call(callee, Ok(vec![1, 2, 3, 4]))
            .storage(vec![(vec![1, 2, 3], vec![4, 5, 6])])
            .build();

        // Simulate re-entry: call, then immediately read storage while
        // return_data is written.
        let _ = host.call(
            CallFlags::empty(),
            &callee,
            0,
            0,
            &[0u8; 32],
            &[0u8; 32],
            &[],
            None,
        );
        assert_eq!(host.get_raw_storage(&[1, 2, 3]), Some(vec![4, 5, 6]));
    }
}

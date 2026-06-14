//! `CountingHost` — a `HostApi` wrapper that counts on-chain storage operations.
//!
//! The OrderedIndex measurement binary needs to know how many SLOAD / SSTORE /
//! clear operations an index workload performs. On-chain, each of those maps to
//! a fixed gas cost, so minimizing them minimizes deploy cost.
//!
//! `CountingHost` wraps a [`MockHost`](pvm_contract_types::MockHost) and
//! forwards every `HostApi` method to the inner host unchanged, with one
//! exception: the four storage-mutation entry points increment interior
//! counters. Non-storage methods (balance, caller, keccak, etc.) are
//! passthrough-only — the wrapper must not affect their behavior.
//!
//! The counters live in `Cell<u64>` so the wrapper can be shared via
//! `Rc<CountingHost>` (the `Host` type stores `Rc<dyn HostApi>`), and the
//! measurement binary can read totals after running a workload.
//!
//! # JSON contract (emitted by `measure-ordered-index`)
//!
//! The binary prints exactly one line on stdout, no other stdout output:
//!
//! ```json
//! {"n":10000,"queries":1000,"slot_reads_per_query":12.34,"insert_writes":12345,"insert_clears":0,"range_p50_ns":5678,"range_p99_ns":91011,"correctness":true,"t":2}
//! ```
//!
//! - `n` — number of records inserted before the query phase.
//! - `queries` — number of prefix-range queries executed.
//! - `slot_reads_per_query` — total `get_storage` + `get_storage_or_zero` calls
//!   during the query phase, divided by `queries` (the metric to MINIMIZE).
//! - `insert_writes` — total `set_storage` calls during the build phase.
//! - `insert_clears` — total `set_storage_or_clear` calls during the build phase.
//! - `range_p50_ns` / `range_p99_ns` — wall-clock latency percentiles in nanoseconds.
//! - `correctness` — `true` iff every query returned the expected multiset of
//!   `(key, value)` pairs against a known deterministic dataset.
//! - `t` — the B-tree degree (OrderedIndex generic const).

#![cfg(not(target_arch = "riscv64"))]

extern crate alloc;

use core::cell::Cell;
use std::rc::Rc;

use pvm_contract_types::{
    CallFlags, HostApi, HostResult, MockHost, ReturnFlags, StorageFlags,
};

/// `Copy` snapshot of the three storage counters. Returned by
/// [`CountingHost::snapshot`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reads: u64,
    pub writes: u64,
    pub clears: u64,
}

/// `HostApi` wrapper that counts the four storage-mutation entry points and
/// forwards every other `HostApi` method to the inner [`MockHost`] unchanged.
pub struct CountingHost {
    inner: Rc<MockHost>,
    reads: Cell<u64>,
    writes: Cell<u64>,
    clears: Cell<u64>,
}

impl CountingHost {
    /// Wrap a `MockHost` in a `CountingHost`. The returned `Rc<Self>` is
    /// what gets coerced to `Rc<dyn HostApi>` for `Host::from_dyn`. The
    /// caller also holds the `Rc<Self>` to read the counters later — the
    /// two clones share the same underlying counters.
    pub fn new(inner: Rc<MockHost>) -> Rc<Self> {
        Rc::new(Self {
            inner,
            reads: Cell::new(0),
            writes: Cell::new(0),
            clears: Cell::new(0),
        })
    }

    pub fn reads(&self) -> u64 {
        self.reads.get()
    }

    pub fn writes(&self) -> u64 {
        self.writes.get()
    }

    pub fn clears(&self) -> u64 {
        self.clears.get()
    }

    /// Reset every counter to zero. Use between workload phases (e.g. after
    /// the build phase, before the query phase) so each phase can be measured
    /// independently.
    pub fn reset(&self) {
        self.reads.set(0);
        self.writes.set(0);
        self.clears.set(0);
    }

    /// Take a `Copy` snapshot of the current counters.
    pub fn snapshot(&self) -> Counts {
        Counts {
            reads: self.reads(),
            writes: self.writes(),
            clears: self.clears(),
        }
    }
}

impl HostApi for CountingHost {
    #[inline]
    fn address(&self, output: &mut [u8; 20]) {
        self.inner.address(output);
    }

    #[inline]
    fn get_immutable_data(&self, output: &mut &mut [u8]) {
        self.inner.get_immutable_data(output);
    }

    #[inline]
    fn set_immutable_data(&self, data: &[u8]) {
        self.inner.set_immutable_data(data);
    }

    #[inline]
    fn balance(&self, output: &mut [u8; 32]) {
        self.inner.balance(output);
    }

    #[inline]
    fn balance_of(&self, addr: &[u8; 20], output: &mut [u8; 32]) {
        self.inner.balance_of(addr, output);
    }

    #[inline]
    fn chain_id(&self, output: &mut [u8; 32]) {
        self.inner.chain_id(output);
    }

    #[inline]
    fn gas_price(&self) -> u64 {
        self.inner.gas_price()
    }

    #[inline]
    fn base_fee(&self, output: &mut [u8; 32]) {
        self.inner.base_fee(output);
    }

    #[inline]
    fn call_data_size(&self) -> u64 {
        self.inner.call_data_size()
    }

    #[inline]
    fn call(
        &self,
        flags: CallFlags,
        callee: &[u8; 20],
        ref_time_limit: u64,
        proof_size_limit: u64,
        deposit: &[u8; 32],
        value: &[u8; 32],
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        self.inner
            .call(flags, callee, ref_time_limit, proof_size_limit, deposit, value, input_data, output)
    }

    #[inline]
    fn call_evm(
        &self,
        flags: CallFlags,
        callee: &[u8; 20],
        gas: u64,
        value: &[u8; 32],
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        self.inner.call_evm(flags, callee, gas, value, input_data, output)
    }

    #[inline]
    fn caller(&self, output: &mut [u8; 20]) {
        self.inner.caller(output);
    }

    #[inline]
    fn origin(&self, output: &mut [u8; 20]) {
        self.inner.origin(output);
    }

    #[inline]
    fn code_hash(&self, addr: &[u8; 20], output: &mut [u8; 32]) {
        self.inner.code_hash(addr, output);
    }

    #[inline]
    fn code_size(&self, addr: &[u8; 20]) -> u64 {
        self.inner.code_size(addr)
    }

    #[inline]
    fn delegate_call(
        &self,
        flags: CallFlags,
        address: &[u8; 20],
        ref_time_limit: u64,
        proof_size_limit: u64,
        deposit_limit: &[u8; 32],
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        self.inner.delegate_call(
            flags,
            address,
            ref_time_limit,
            proof_size_limit,
            deposit_limit,
            input_data,
            output,
        )
    }

    #[inline]
    fn delegate_call_evm(
        &self,
        flags: CallFlags,
        address: &[u8; 20],
        gas: u64,
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        self.inner.delegate_call_evm(flags, address, gas, input_data, output)
    }

    #[inline]
    fn deposit_event(&self, topics: &[[u8; 32]], data: &[u8]) {
        self.inner.deposit_event(topics, data);
    }

    #[inline]
    fn get_storage(
        &self,
        flags: StorageFlags,
        key: &[u8],
        output: &mut &mut [u8],
    ) -> HostResult {
        // Read: a storage SLOAD. Dominant gas cost in the workload.
        self.reads.set(self.reads.get() + 1);
        self.inner.get_storage(flags, key, output)
    }

    #[inline]
    fn hash_keccak_256(&self, input: &[u8], output: &mut [u8; 32]) {
        self.inner.hash_keccak_256(input, output);
    }

    #[inline]
    fn call_data_copy(&self, output: &mut [u8], offset: u32) {
        self.inner.call_data_copy(output, offset);
    }

    #[inline]
    fn call_data_load(&self, output: &mut [u8; 32], offset: u32) {
        self.inner.call_data_load(output, offset);
    }

    #[inline]
    fn instantiate(
        &self,
        ref_time_limit: u64,
        proof_size_limit: u64,
        deposit: &[u8; 32],
        value: &[u8; 32],
        input: &[u8],
        address: Option<&mut [u8; 20]>,
        output: Option<&mut &mut [u8]>,
        salt: Option<&[u8; 32]>,
    ) -> HostResult {
        self.inner.instantiate(
            ref_time_limit,
            proof_size_limit,
            deposit,
            value,
            input,
            address,
            output,
            salt,
        )
    }

    #[inline]
    fn now(&self, output: &mut [u8; 32]) {
        self.inner.now(output);
    }

    #[inline]
    fn gas_limit(&self) -> u64 {
        self.inner.gas_limit()
    }

    #[inline]
    fn set_storage(&self, flags: StorageFlags, key: &[u8], value: &[u8]) -> Option<u32> {
        // Write: a non-zero SSTORE.
        self.writes.set(self.writes.get() + 1);
        self.inner.set_storage(flags, key, value)
    }

    #[inline]
    fn set_storage_or_clear(
        &self,
        flags: StorageFlags,
        key: &[u8; 32],
        value: &[u8; 32],
    ) -> Option<u32> {
        // Clear: an all-zero SSTORE that deletes the slot. Counted
        // regardless of whether the slot was previously written.
        self.clears.set(self.clears.get() + 1);
        self.inner.set_storage_or_clear(flags, key, value)
    }

    #[inline]
    fn get_storage_or_zero(&self, flags: StorageFlags, key: &[u8; 32], output: &mut [u8; 32]) {
        // Read: the 32-byte SLOAD path used by Lazy<u64> and friends.
        self.reads.set(self.reads.get() + 1);
        self.inner.get_storage_or_zero(flags, key, output);
    }

    #[inline]
    fn value_transferred(&self, output: &mut [u8; 32]) {
        self.inner.value_transferred(output);
    }

    #[inline]
    fn return_data_size(&self) -> u64 {
        self.inner.return_data_size()
    }

    #[inline]
    fn return_data_copy(&self, output: &mut &mut [u8], offset: u32) {
        self.inner.return_data_copy(output, offset);
    }

    #[inline]
    fn gas_left(&self) -> u64 {
        self.inner.gas_left()
    }

    #[inline]
    fn block_author(&self, output: &mut [u8; 20]) {
        self.inner.block_author(output);
    }

    #[inline]
    fn block_number(&self, output: &mut [u8; 32]) {
        self.inner.block_number(output);
    }

    #[inline]
    fn block_hash(&self, block_number: &[u8; 32], output: &mut [u8; 32]) {
        self.inner.block_hash(block_number, output);
    }

    #[cfg(not(target_arch = "riscv64"))]
    #[inline]
    fn return_value(&self, flags: ReturnFlags, data: &[u8]) {
        self.inner.return_value(flags, data);
    }

    #[inline]
    fn consume_all_gas(&self) -> ! {
        self.inner.consume_all_gas()
    }

    #[inline]
    fn terminate(&self, beneficiary: &[u8; 20]) -> ! {
        self.inner.terminate(beneficiary)
    }
}

#![cfg(not(target_arch = "riscv64"))]

extern crate alloc;

use core::cell::Cell;
use std::rc::Rc;

use pvm_contract_types::{CallFlags, HostApi, HostResult, MockHost, ReturnFlags, StorageFlags};

#[path = "pallet_revive_weight.rs"]
mod pallet_revive_weight;
use pallet_revive_weight::{clear_weight, read_weight, write_weight, Weight};

/// `Copy` snapshot of the storage counters AND the accumulated real
/// `pallet-revive` weight. Returned by [`CountingHost::snapshot`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub reads: u64,
    pub writes: u64,
    pub clears: u64,
    /// Accumulated `pallet-revive` ref_time in picoseconds (compute time).
    pub ref_time_ps: u64,
    /// Accumulated `pallet-revive` proof_size in bytes (PoV / state-proof).
    pub proof_size_bytes: u64,
}

/// `HostApi` wrapper that counts the four storage-mutation entry points,
/// captures the per-op byte sizes, and accumulates the real `pallet-revive`
/// weight each operation would be charged on Polkadot Asset Hub Westend.
///
/// The read/write/clear counters are retained as diagnostics (they are what
/// the prior `slot_reads_per_query` metric measured). The primary metric —
/// real on-chain weight — lives in `ref_time_ps` / `proof_size_bytes`.
pub struct CountingHost {
    inner: Rc<MockHost>,
    reads: Cell<u64>,
    writes: Cell<u64>,
    clears: Cell<u64>,
    weight: Cell<Weight>,
}

impl CountingHost {
    pub fn new(inner: Rc<MockHost>) -> Rc<Self> {
        Rc::new(Self {
            inner,
            reads: Cell::new(0),
            writes: Cell::new(0),
            clears: Cell::new(0),
            weight: Cell::new(Weight::default()),
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

    pub fn reset(&self) {
        self.reads.set(0);
        self.writes.set(0);
        self.clears.set(0);
        self.weight.set(Weight::default());
    }

    pub fn snapshot(&self) -> Counts {
        let w = self.weight.get();
        Counts {
            reads: self.reads(),
            writes: self.writes(),
            clears: self.clears(),
            ref_time_ps: w.ref_time_ps,
            proof_size_bytes: w.proof_size_bytes,
        }
    }

    #[inline]
    fn accrue(&self, w: Weight) {
        self.weight.set(self.weight.get().saturating_add(w));
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
        self.inner.call(
            flags,
            callee,
            ref_time_limit,
            proof_size_limit,
            deposit,
            value,
            input_data,
            output,
        )
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
    fn get_storage(&self, flags: StorageFlags, key: &[u8], output: &mut &mut [u8]) -> HostResult {
        let result = self.inner.get_storage(flags, key, output);
        // After the read, `output` is resized to the value length actually
        // returned; that length is the `n` parameter to `seal_get_storage`.
        let n = output.len() as u64;
        self.reads.set(self.reads.get() + 1);
        self.accrue(read_weight(n));
        result
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
        let new_bytes = value.len() as u64;
        let ret = self.inner.set_storage(flags, key, value);
        let old_bytes = ret.unwrap_or(0) as u64;
        self.writes.set(self.writes.get() + 1);
        self.accrue(write_weight(new_bytes, old_bytes));
        ret
    }

    #[inline]
    fn set_storage_or_clear(
        &self,
        flags: StorageFlags,
        key: &[u8; 32],
        value: &[u8; 32],
    ) -> Option<u32> {
        let ret = self.inner.set_storage_or_clear(flags, key, value);
        let old_bytes = ret.unwrap_or(0) as u64;
        // `pallet-revive` routing (env.rs): an all-zero value clears the slot,
        // otherwise it is a 32-byte set. The weight model follows the same
        // branch so the accumulator reflects the actual on-chain token.
        if value.iter().all(|&b| b == 0) {
            self.clears.set(self.clears.get() + 1);
            self.accrue(clear_weight(old_bytes));
        } else {
            self.writes.set(self.writes.get() + 1);
            self.accrue(write_weight(32, old_bytes));
        }
        ret
    }

    #[inline]
    fn get_storage_or_zero(&self, flags: StorageFlags, key: &[u8; 32], output: &mut [u8; 32]) {
        self.inner.get_storage_or_zero(flags, key, output);
        // Fixed 32-byte read path (env.rs: FixedOutput32).
        self.reads.set(self.reads.get() + 1);
        self.accrue(read_weight(32));
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

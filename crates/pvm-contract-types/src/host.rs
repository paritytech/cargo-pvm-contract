//! Host backend abstraction for PVM smart contracts.
//!
//! [`HostApi`] is the receiver-based trait that both the production
//! [`PolkaVmHost`] (riscv64-only) and the testing [`MockHost`](super::MockHost)
//! implement. Contracts call host operations through an injected handle
//! (`self.host().caller(...)` in the macro path, `host.caller(...)` in the DSL
//! path) so tests can inject a `MockHost` instance per test.
//!
//! Diverging host operations split by role:
//!
//! - **Boundary operations** ([`HostApi::return_value`]) are called only from
//!   macro/DSL dispatch glue at the end of a method invocation. The signature
//!   is cfg-gated: `-> !` on `riscv64` and `-> ()` on host targets, where
//!   [`MockHost`](super::MockHost) captures the encoded result instead of
//!   terminating. Tests inspect the captured result via
//!   [`MockHost::take_return_value`](super::MockHost::take_return_value).
//!
//! - **Mid-execution operations** ([`HostApi::consume_all_gas`],
//!   [`HostApi::terminate`]) can be called from arbitrary positions in user
//!   method bodies. The signature is `-> !` on both targets — a syscall on
//!   `riscv64`, and a typed-payload panic on host targets. Tests recover the
//!   captured halt via [`MockHost::run_until_halt`](super::MockHost::run_until_halt),
//!   which downcasts the panic payload and re-throws non-halt panics so
//!   contract bugs aren't silently swallowed.

pub use pallet_revive_uapi::{CallFlags, ReturnErrorCode, ReturnFlags, StorageFlags};

/// Result type for host operations that can fail.
pub type HostResult = core::result::Result<(), ReturnErrorCode>;

/// Marker trait identifying the contract storage root.
///
/// The `#[contract]` macro auto-implements this on the generated storage
/// struct; DSL handlers wrap their host in [`Context`] (`Context::new(host.clone())`)
/// to satisfy the bound. Cross-contract call
/// builders are bound `&impl ContractContext` (for `View`/`Pure` callees) or
/// `&mut impl ContractContext` (for `NonPayable`/`Payable` callees), so the
/// borrow checker — not just the runtime — rejects view methods that try to
/// initiate a state-mutating cross-contract call.
///
/// Sealed via [`crate::__private::Sealed`]: external code cannot implement
/// `ContractContext` for arbitrary types, so the gate cannot be smuggled past
/// by user-provided "fake roots".
pub trait ContractContext: crate::__private::Sealed {
    /// Borrow the contract's host handle.
    ///
    /// The borrow on `Self` is the load-bearing piece of the gate; the host
    /// returned here is then used internally by the call builder.
    fn host(&self) -> &Host;
}

/// Stateless [`ContractContext`] root.
///
/// Wraps a [`Host`] and implements [`ContractContext`] so cross-contract call
/// builders (which require `&impl ContractContext` / `&mut impl ContractContext`)
/// can be invoked outside the `#[contract]` macro's storage struct — from DSL
/// handlers (wrap the dispatcher-provided `&Host` via `Context::new(host.clone())`)
/// and from `#[test]` functions backed by a `MockHost`.
///
/// `Host` is `Copy` on `riscv64` (ZST) and `Clone` on host targets (one
/// `Rc::clone`), so the owned shape costs nothing in production.
///
/// **Not `Clone`** — same gating contract as the macro-generated storage
/// struct: a `&self` method that gets `&Context` cannot smuggle out a
/// `&mut Context` via cloning. The DSL path is still the "manual control"
/// surface: a handler holds the owned `Context` locally, so it can freely
/// construct both `&cx` and `&mut cx` from the same binding. If you need
/// the static view-vs-mutating guarantee, use the `#[contract]` macro path.
pub struct Context {
    pub host: Host,
}

impl Context {
    /// Construct a new context from an owned host handle.
    #[inline(always)]
    pub fn new(host: Host) -> Self {
        Self { host }
    }
}

impl crate::__private::Sealed for Context {}

impl ContractContext for Context {
    #[inline(always)]
    fn host(&self) -> &Host {
        &self.host
    }
}

/// Receiver-based host API.
///
/// Every method takes `&self` — `PolkaVmHost` is a zero-sized type, so this
/// compiles to identical instructions as a static call. `MockHost` uses
/// interior mutability (`RefCell`) only where it actually mutates shared state
/// (storage, events).
///
/// `return_value` has a cfg-gated signature: it diverges (`-> !`) on `riscv64`
/// and returns `()` on host targets, where `MockHost` captures the encoded
/// result instead of terminating. The mid-execution diverging operations
/// `consume_all_gas` and `terminate` are `-> !` on both targets — a syscall
/// on `riscv64`, a typed-payload panic on host targets that
/// [`MockHost::run_until_halt`](super::MockHost::run_until_halt) catches.
#[allow(clippy::too_many_arguments)]
pub trait HostApi {
    fn address(&self, output: &mut [u8; 20]);
    fn get_immutable_data(&self, output: &mut &mut [u8]);
    fn set_immutable_data(&self, data: &[u8]);
    fn balance(&self, output: &mut [u8; 32]);
    fn balance_of(&self, addr: &[u8; 20], output: &mut [u8; 32]);
    fn chain_id(&self, output: &mut [u8; 32]);
    fn gas_price(&self) -> u64;
    fn base_fee(&self, output: &mut [u8; 32]);
    fn call_data_size(&self) -> u64;
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
    ) -> HostResult;
    fn call_evm(
        &self,
        flags: CallFlags,
        callee: &[u8; 20],
        gas: u64,
        value: &[u8; 32],
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult;
    fn caller(&self, output: &mut [u8; 20]);
    fn origin(&self, output: &mut [u8; 20]);
    fn code_hash(&self, addr: &[u8; 20], output: &mut [u8; 32]);
    fn code_size(&self, addr: &[u8; 20]) -> u64;
    fn delegate_call(
        &self,
        flags: CallFlags,
        address: &[u8; 20],
        ref_time_limit: u64,
        proof_size_limit: u64,
        deposit_limit: &[u8; 32],
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult;
    fn delegate_call_evm(
        &self,
        flags: CallFlags,
        address: &[u8; 20],
        gas: u64,
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult;
    fn deposit_event(&self, topics: &[[u8; 32]], data: &[u8]);
    fn get_storage(&self, flags: StorageFlags, key: &[u8], output: &mut &mut [u8]) -> HostResult;
    fn hash_keccak_256(&self, input: &[u8], output: &mut [u8; 32]);
    fn call_data_copy(&self, output: &mut [u8], offset: u32);
    fn call_data_load(&self, output: &mut [u8; 32], offset: u32);
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
    ) -> HostResult;
    fn now(&self, output: &mut [u8; 32]);
    fn gas_limit(&self) -> u64;
    fn set_storage(&self, flags: StorageFlags, key: &[u8], value: &[u8]) -> Option<u32>;
    fn set_storage_or_clear(
        &self,
        flags: StorageFlags,
        key: &[u8; 32],
        value: &[u8; 32],
    ) -> Option<u32>;
    fn get_storage_or_zero(&self, flags: StorageFlags, key: &[u8; 32], output: &mut [u8; 32]);
    fn value_transferred(&self, output: &mut [u8; 32]);
    fn return_data_size(&self) -> u64;
    fn return_data_copy(&self, output: &mut &mut [u8], offset: u32);
    fn gas_left(&self) -> u64;
    fn block_author(&self, output: &mut [u8; 20]);
    fn block_number(&self, output: &mut [u8; 32]);
    fn block_hash(&self, block_number: &[u8; 32], output: &mut [u8; 32]);

    /// Terminate execution with the given flags and encoded data.
    ///
    /// On `riscv64` this is a syscall and never returns. On host targets
    /// the test mock captures the call as a [`ReturnValue`](super::ReturnValue)
    /// and returns control to the caller — see
    /// [`MockHost::take_return_value`](super::MockHost::take_return_value).
    #[cfg(target_arch = "riscv64")]
    fn return_value(&self, flags: ReturnFlags, data: &[u8]) -> !;

    /// Capture the return value (host-target equivalent of the `riscv64`
    /// diverging syscall). Implementations on host targets should record the
    /// `(flags, data)` pair (typically into a [`ReturnValue`](super::ReturnValue))
    /// for the test to inspect after the dispatch returns.
    #[cfg(not(target_arch = "riscv64"))]
    fn return_value(&self, flags: ReturnFlags, data: &[u8]);

    /// Halt execution and consume all remaining gas.
    ///
    /// On `riscv64` this is a syscall and never returns. On host targets the
    /// mock implementation panics with a typed payload that
    /// [`MockHost::run_until_halt`](super::MockHost::run_until_halt) catches.
    fn consume_all_gas(&self) -> !;

    /// Terminate the contract, transferring its remaining balance to
    /// `beneficiary`.
    ///
    /// Same divergence semantics as [`Self::consume_all_gas`].
    fn terminate(&self, beneficiary: &[u8; 20]) -> !;
}

/// Real host backend for PolkaVM contracts.
///
/// Zero-sized type — `&self` is free; `struct MyContract<PolkaVmHost>` is itself
/// zero-sized. On `riscv64`, each method delegates to `pallet_revive_uapi::HostFnImpl`.
/// On other targets, methods `unimplemented!()` — `PolkaVmHost` must only be
/// constructed inside the riscv64-gated entry-point wrappers in production.
#[derive(Clone, Copy)]
pub struct PolkaVmHost;

#[cfg(target_arch = "riscv64")]
use pallet_revive_uapi::HostFn as _;

#[cfg(target_arch = "riscv64")]
impl HostApi for PolkaVmHost {
    #[inline(always)]
    fn address(&self, output: &mut [u8; 20]) {
        pallet_revive_uapi::HostFnImpl::address(output)
    }
    #[inline(always)]
    fn get_immutable_data(&self, output: &mut &mut [u8]) {
        pallet_revive_uapi::HostFnImpl::get_immutable_data(output)
    }
    #[inline(always)]
    fn set_immutable_data(&self, data: &[u8]) {
        pallet_revive_uapi::HostFnImpl::set_immutable_data(data)
    }
    #[inline(always)]
    fn balance(&self, output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::balance(output)
    }
    #[inline(always)]
    fn balance_of(&self, addr: &[u8; 20], output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::balance_of(addr, output)
    }
    #[inline(always)]
    fn chain_id(&self, output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::chain_id(output)
    }
    #[inline(always)]
    fn gas_price(&self) -> u64 {
        pallet_revive_uapi::HostFnImpl::gas_price()
    }
    #[inline(always)]
    fn base_fee(&self, output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::base_fee(output)
    }
    #[inline(always)]
    fn call_data_size(&self) -> u64 {
        pallet_revive_uapi::HostFnImpl::call_data_size()
    }
    #[inline(always)]
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
        pallet_revive_uapi::HostFnImpl::call(
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
    #[inline(always)]
    fn call_evm(
        &self,
        flags: CallFlags,
        callee: &[u8; 20],
        gas: u64,
        value: &[u8; 32],
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        pallet_revive_uapi::HostFnImpl::call_evm(flags, callee, gas, value, input_data, output)
    }
    #[inline(always)]
    fn caller(&self, output: &mut [u8; 20]) {
        pallet_revive_uapi::HostFnImpl::caller(output)
    }
    #[inline(always)]
    fn origin(&self, output: &mut [u8; 20]) {
        pallet_revive_uapi::HostFnImpl::origin(output)
    }
    #[inline(always)]
    fn code_hash(&self, addr: &[u8; 20], output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::code_hash(addr, output)
    }
    #[inline(always)]
    fn code_size(&self, addr: &[u8; 20]) -> u64 {
        pallet_revive_uapi::HostFnImpl::code_size(addr)
    }
    #[inline(always)]
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
        pallet_revive_uapi::HostFnImpl::delegate_call(
            flags,
            address,
            ref_time_limit,
            proof_size_limit,
            deposit_limit,
            input_data,
            output,
        )
    }
    #[inline(always)]
    fn delegate_call_evm(
        &self,
        flags: CallFlags,
        address: &[u8; 20],
        gas: u64,
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        pallet_revive_uapi::HostFnImpl::delegate_call_evm(flags, address, gas, input_data, output)
    }
    #[inline(always)]
    fn deposit_event(&self, topics: &[[u8; 32]], data: &[u8]) {
        pallet_revive_uapi::HostFnImpl::deposit_event(topics, data)
    }
    #[inline(always)]
    fn get_storage(&self, flags: StorageFlags, key: &[u8], output: &mut &mut [u8]) -> HostResult {
        pallet_revive_uapi::HostFnImpl::get_storage(flags, key, output)
    }
    #[inline(always)]
    fn hash_keccak_256(&self, input: &[u8], output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::hash_keccak_256(input, output)
    }
    #[inline(always)]
    fn call_data_copy(&self, output: &mut [u8], offset: u32) {
        pallet_revive_uapi::HostFnImpl::call_data_copy(output, offset)
    }
    #[inline(always)]
    fn call_data_load(&self, output: &mut [u8; 32], offset: u32) {
        pallet_revive_uapi::HostFnImpl::call_data_load(output, offset)
    }
    #[inline(always)]
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
        pallet_revive_uapi::HostFnImpl::instantiate(
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
    #[inline(always)]
    fn now(&self, output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::now(output)
    }
    #[inline(always)]
    fn gas_limit(&self) -> u64 {
        pallet_revive_uapi::HostFnImpl::gas_limit()
    }
    #[inline(always)]
    fn set_storage(&self, flags: StorageFlags, key: &[u8], value: &[u8]) -> Option<u32> {
        pallet_revive_uapi::HostFnImpl::set_storage(flags, key, value)
    }
    #[inline(always)]
    fn set_storage_or_clear(
        &self,
        flags: StorageFlags,
        key: &[u8; 32],
        value: &[u8; 32],
    ) -> Option<u32> {
        pallet_revive_uapi::HostFnImpl::set_storage_or_clear(flags, key, value)
    }
    #[inline(always)]
    fn get_storage_or_zero(&self, flags: StorageFlags, key: &[u8; 32], output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::get_storage_or_zero(flags, key, output)
    }
    #[inline(always)]
    fn value_transferred(&self, output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::value_transferred(output)
    }
    #[inline(always)]
    fn return_data_size(&self) -> u64 {
        pallet_revive_uapi::HostFnImpl::return_data_size()
    }
    #[inline(always)]
    fn return_data_copy(&self, output: &mut &mut [u8], offset: u32) {
        pallet_revive_uapi::HostFnImpl::return_data_copy(output, offset)
    }
    #[inline(always)]
    fn gas_left(&self) -> u64 {
        pallet_revive_uapi::HostFnImpl::gas_left()
    }
    #[inline(always)]
    fn block_author(&self, output: &mut [u8; 20]) {
        pallet_revive_uapi::HostFnImpl::block_author(output)
    }
    #[inline(always)]
    fn block_number(&self, output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::block_number(output)
    }
    #[inline(always)]
    fn block_hash(&self, block_number: &[u8; 32], output: &mut [u8; 32]) {
        pallet_revive_uapi::HostFnImpl::block_hash(block_number, output)
    }
    #[inline(always)]
    fn return_value(&self, flags: ReturnFlags, data: &[u8]) -> ! {
        pallet_revive_uapi::HostFnImpl::return_value(flags, data)
    }
    #[inline(always)]
    fn consume_all_gas(&self) -> ! {
        pallet_revive_uapi::HostFnImpl::consume_all_gas()
    }
    #[inline(always)]
    fn terminate(&self, beneficiary: &[u8; 20]) -> ! {
        pallet_revive_uapi::HostFnImpl::terminate(beneficiary)
    }
}

#[cfg(not(target_arch = "riscv64"))]
impl HostApi for PolkaVmHost {
    fn address(&self, _output: &mut [u8; 20]) {
        unimplemented!("PolkaVmHost::address is only available on PolkaVM")
    }
    fn get_immutable_data(&self, _output: &mut &mut [u8]) {
        unimplemented!("PolkaVmHost::get_immutable_data is only available on PolkaVM")
    }
    fn set_immutable_data(&self, _data: &[u8]) {
        unimplemented!("PolkaVmHost::set_immutable_data is only available on PolkaVM")
    }
    fn balance(&self, _output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::balance is only available on PolkaVM")
    }
    fn balance_of(&self, _addr: &[u8; 20], _output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::balance_of is only available on PolkaVM")
    }
    fn chain_id(&self, _output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::chain_id is only available on PolkaVM")
    }
    fn gas_price(&self) -> u64 {
        unimplemented!("PolkaVmHost::gas_price is only available on PolkaVM")
    }
    fn base_fee(&self, _output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::base_fee is only available on PolkaVM")
    }
    fn call_data_size(&self) -> u64 {
        unimplemented!("PolkaVmHost::call_data_size is only available on PolkaVM")
    }
    fn call(
        &self,
        _flags: CallFlags,
        _callee: &[u8; 20],
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit: &[u8; 32],
        _value: &[u8; 32],
        _input_data: &[u8],
        _output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        unimplemented!("PolkaVmHost::call is only available on PolkaVM")
    }
    fn call_evm(
        &self,
        _flags: CallFlags,
        _callee: &[u8; 20],
        _gas: u64,
        _value: &[u8; 32],
        _input_data: &[u8],
        _output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        unimplemented!("PolkaVmHost::call_evm is only available on PolkaVM")
    }
    fn caller(&self, _output: &mut [u8; 20]) {
        unimplemented!("PolkaVmHost::caller is only available on PolkaVM")
    }
    fn origin(&self, _output: &mut [u8; 20]) {
        unimplemented!("PolkaVmHost::origin is only available on PolkaVM")
    }
    fn code_hash(&self, _addr: &[u8; 20], _output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::code_hash is only available on PolkaVM")
    }
    fn code_size(&self, _addr: &[u8; 20]) -> u64 {
        unimplemented!("PolkaVmHost::code_size is only available on PolkaVM")
    }
    fn delegate_call(
        &self,
        _flags: CallFlags,
        _address: &[u8; 20],
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit_limit: &[u8; 32],
        _input_data: &[u8],
        _output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        unimplemented!("PolkaVmHost::delegate_call is only available on PolkaVM")
    }
    fn delegate_call_evm(
        &self,
        _flags: CallFlags,
        _address: &[u8; 20],
        _gas: u64,
        _input_data: &[u8],
        _output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        unimplemented!("PolkaVmHost::delegate_call_evm is only available on PolkaVM")
    }
    fn deposit_event(&self, _topics: &[[u8; 32]], _data: &[u8]) {
        unimplemented!("PolkaVmHost::deposit_event is only available on PolkaVM")
    }
    fn get_storage(
        &self,
        _flags: StorageFlags,
        _key: &[u8],
        _output: &mut &mut [u8],
    ) -> HostResult {
        unimplemented!("PolkaVmHost::get_storage is only available on PolkaVM")
    }
    fn hash_keccak_256(&self, _input: &[u8], _output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::hash_keccak_256 is only available on PolkaVM")
    }
    fn call_data_copy(&self, _output: &mut [u8], _offset: u32) {
        unimplemented!("PolkaVmHost::call_data_copy is only available on PolkaVM")
    }
    fn call_data_load(&self, _output: &mut [u8; 32], _offset: u32) {
        unimplemented!("PolkaVmHost::call_data_load is only available on PolkaVM")
    }
    fn instantiate(
        &self,
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit: &[u8; 32],
        _value: &[u8; 32],
        _input: &[u8],
        _address: Option<&mut [u8; 20]>,
        _output: Option<&mut &mut [u8]>,
        _salt: Option<&[u8; 32]>,
    ) -> HostResult {
        unimplemented!("PolkaVmHost::instantiate is only available on PolkaVM")
    }
    fn now(&self, _output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::now is only available on PolkaVM")
    }
    fn gas_limit(&self) -> u64 {
        unimplemented!("PolkaVmHost::gas_limit is only available on PolkaVM")
    }
    fn set_storage(&self, _flags: StorageFlags, _key: &[u8], _value: &[u8]) -> Option<u32> {
        unimplemented!("PolkaVmHost::set_storage is only available on PolkaVM")
    }
    fn set_storage_or_clear(
        &self,
        _flags: StorageFlags,
        _key: &[u8; 32],
        _value: &[u8; 32],
    ) -> Option<u32> {
        unimplemented!("PolkaVmHost::set_storage_or_clear is only available on PolkaVM")
    }
    fn get_storage_or_zero(&self, _flags: StorageFlags, _key: &[u8; 32], _output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::get_storage_or_zero is only available on PolkaVM")
    }
    fn value_transferred(&self, _output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::value_transferred is only available on PolkaVM")
    }
    fn return_data_size(&self) -> u64 {
        unimplemented!("PolkaVmHost::return_data_size is only available on PolkaVM")
    }
    fn return_data_copy(&self, _output: &mut &mut [u8], _offset: u32) {
        unimplemented!("PolkaVmHost::return_data_copy is only available on PolkaVM")
    }
    fn gas_left(&self) -> u64 {
        unimplemented!("PolkaVmHost::gas_left is only available on PolkaVM")
    }
    fn block_author(&self, _output: &mut [u8; 20]) {
        unimplemented!("PolkaVmHost::block_author is only available on PolkaVM")
    }
    fn block_number(&self, _output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::block_number is only available on PolkaVM")
    }
    fn block_hash(&self, _block_number: &[u8; 32], _output: &mut [u8; 32]) {
        unimplemented!("PolkaVmHost::block_hash is only available on PolkaVM")
    }
    fn return_value(&self, _flags: ReturnFlags, _data: &[u8]) {
        unimplemented!("PolkaVmHost::return_value is only available on PolkaVM")
    }
    fn consume_all_gas(&self) -> ! {
        unimplemented!("PolkaVmHost::consume_all_gas is only available on PolkaVM")
    }
    fn terminate(&self, _beneficiary: &[u8; 20]) -> ! {
        unimplemented!("PolkaVmHost::terminate is only available on PolkaVM")
    }
}

// ---------------------------------------------------------------------------
// Concrete `Host` wrapper — cfg-gated internals, uniform surface
// ---------------------------------------------------------------------------
//
// Contracts always hold a concrete `Host`; the field type swaps under cfg.
// On riscv64, `Host { inner: PolkaVmHost }` is zero-sized
// and method calls inline to `HostFnImpl::*` — byte-equivalent to the previous
// `<H: HostApi>` monomorphization. On host targets, `Host { inner: Rc<dyn
// HostApi> }` enables test harnesses to inject a shared `MockHost` without
// the contract struct carrying a generic.

/// Concrete host handle held by every macro-path contract.
///
/// Internals are cfg-gated:
/// - `target_arch = "riscv64"`: contains a zero-sized [`PolkaVmHost`] — methods
///   inline to `pallet_revive_uapi::HostFnImpl::*`, no runtime overhead.
/// - host target with `feature = "alloc"`: contains `Rc<dyn HostApi>` —
///   tests inject a mock via [`Host::from_dyn`].
/// - host target without `alloc`: uninhabited (no constructor) — the type
///   name exists so contract structs declaring `host: Host` parse on any
///   target, but constructing one is impossible until `alloc` is enabled.
///
/// The [`HostApi`] trait is implemented for `Host`, so generic DSL code and
/// contract bodies can treat it uniformly.
#[cfg(target_arch = "riscv64")]
#[derive(Clone, Copy)]
pub struct Host {
    pub(crate) inner: PolkaVmHost,
}

#[cfg(all(not(target_arch = "riscv64"), feature = "alloc"))]
#[derive(Clone)]
pub struct Host {
    pub(crate) inner: alloc::rc::Rc<dyn HostApi>,
}

#[cfg(all(not(target_arch = "riscv64"), not(feature = "alloc")))]
#[derive(Clone, Copy)]
pub struct Host {
    _never: core::convert::Infallible,
}

#[cfg(target_arch = "riscv64")]
impl Host {
    /// Construct the production host (zero-sized type).
    #[inline(always)]
    pub const fn new() -> Self {
        Self { inner: PolkaVmHost }
    }
}

#[cfg(target_arch = "riscv64")]
impl Default for Host {
    #[inline(always)]
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(all(not(target_arch = "riscv64"), feature = "alloc"))]
impl Host {
    /// Wrap any [`HostApi`] implementor for host-target tests.
    ///
    /// Storage types (`Lazy`, `Mapping`) clone the `Host` handle, so the
    /// inner backing is `Rc<dyn HostApi>` — cheap to clone, all clones
    /// observe the same underlying state.
    ///
    /// Typical use: `Host::from_dyn(alloc::rc::Rc::new(mock_host.clone()))`.
    pub fn from_dyn(inner: alloc::rc::Rc<dyn HostApi>) -> Self {
        Self { inner }
    }
}

#[cfg(any(target_arch = "riscv64", feature = "alloc"))]
impl HostApi for Host {
    #[inline(always)]
    fn address(&self, output: &mut [u8; 20]) {
        self.inner.address(output)
    }
    #[inline(always)]
    fn get_immutable_data(&self, output: &mut &mut [u8]) {
        self.inner.get_immutable_data(output)
    }
    #[inline(always)]
    fn set_immutable_data(&self, data: &[u8]) {
        self.inner.set_immutable_data(data)
    }
    #[inline(always)]
    fn balance(&self, output: &mut [u8; 32]) {
        self.inner.balance(output)
    }
    #[inline(always)]
    fn balance_of(&self, addr: &[u8; 20], output: &mut [u8; 32]) {
        self.inner.balance_of(addr, output)
    }
    #[inline(always)]
    fn chain_id(&self, output: &mut [u8; 32]) {
        self.inner.chain_id(output)
    }
    #[inline(always)]
    fn gas_price(&self) -> u64 {
        self.inner.gas_price()
    }
    #[inline(always)]
    fn base_fee(&self, output: &mut [u8; 32]) {
        self.inner.base_fee(output)
    }
    #[inline(always)]
    fn call_data_size(&self) -> u64 {
        self.inner.call_data_size()
    }
    #[inline(always)]
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
    #[inline(always)]
    fn call_evm(
        &self,
        flags: CallFlags,
        callee: &[u8; 20],
        gas: u64,
        value: &[u8; 32],
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        self.inner
            .call_evm(flags, callee, gas, value, input_data, output)
    }
    #[inline(always)]
    fn caller(&self, output: &mut [u8; 20]) {
        self.inner.caller(output)
    }
    #[inline(always)]
    fn origin(&self, output: &mut [u8; 20]) {
        self.inner.origin(output)
    }
    #[inline(always)]
    fn code_hash(&self, addr: &[u8; 20], output: &mut [u8; 32]) {
        self.inner.code_hash(addr, output)
    }
    #[inline(always)]
    fn code_size(&self, addr: &[u8; 20]) -> u64 {
        self.inner.code_size(addr)
    }
    #[inline(always)]
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
    #[inline(always)]
    fn delegate_call_evm(
        &self,
        flags: CallFlags,
        address: &[u8; 20],
        gas: u64,
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        self.inner
            .delegate_call_evm(flags, address, gas, input_data, output)
    }
    #[inline(always)]
    fn deposit_event(&self, topics: &[[u8; 32]], data: &[u8]) {
        self.inner.deposit_event(topics, data)
    }
    #[inline(always)]
    fn get_storage(&self, flags: StorageFlags, key: &[u8], output: &mut &mut [u8]) -> HostResult {
        self.inner.get_storage(flags, key, output)
    }
    #[inline(always)]
    fn hash_keccak_256(&self, input: &[u8], output: &mut [u8; 32]) {
        self.inner.hash_keccak_256(input, output)
    }
    #[inline(always)]
    fn call_data_copy(&self, output: &mut [u8], offset: u32) {
        self.inner.call_data_copy(output, offset)
    }
    #[inline(always)]
    fn call_data_load(&self, output: &mut [u8; 32], offset: u32) {
        self.inner.call_data_load(output, offset)
    }
    #[inline(always)]
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
    #[inline(always)]
    fn now(&self, output: &mut [u8; 32]) {
        self.inner.now(output)
    }
    #[inline(always)]
    fn gas_limit(&self) -> u64 {
        self.inner.gas_limit()
    }
    #[inline(always)]
    fn set_storage(&self, flags: StorageFlags, key: &[u8], value: &[u8]) -> Option<u32> {
        self.inner.set_storage(flags, key, value)
    }
    #[inline(always)]
    fn set_storage_or_clear(
        &self,
        flags: StorageFlags,
        key: &[u8; 32],
        value: &[u8; 32],
    ) -> Option<u32> {
        self.inner.set_storage_or_clear(flags, key, value)
    }
    #[inline(always)]
    fn get_storage_or_zero(&self, flags: StorageFlags, key: &[u8; 32], output: &mut [u8; 32]) {
        self.inner.get_storage_or_zero(flags, key, output)
    }
    #[inline(always)]
    fn value_transferred(&self, output: &mut [u8; 32]) {
        self.inner.value_transferred(output)
    }
    #[inline(always)]
    fn return_data_size(&self) -> u64 {
        self.inner.return_data_size()
    }
    #[inline(always)]
    fn return_data_copy(&self, output: &mut &mut [u8], offset: u32) {
        self.inner.return_data_copy(output, offset)
    }
    #[inline(always)]
    fn gas_left(&self) -> u64 {
        self.inner.gas_left()
    }
    #[inline(always)]
    fn block_author(&self, output: &mut [u8; 20]) {
        self.inner.block_author(output)
    }
    #[inline(always)]
    fn block_number(&self, output: &mut [u8; 32]) {
        self.inner.block_number(output)
    }
    #[inline(always)]
    fn block_hash(&self, block_number: &[u8; 32], output: &mut [u8; 32]) {
        self.inner.block_hash(block_number, output)
    }
    #[cfg(target_arch = "riscv64")]
    #[inline(always)]
    fn return_value(&self, flags: ReturnFlags, data: &[u8]) -> ! {
        self.inner.return_value(flags, data)
    }
    #[cfg(not(target_arch = "riscv64"))]
    #[inline(always)]
    fn return_value(&self, flags: ReturnFlags, data: &[u8]) {
        self.inner.return_value(flags, data)
    }
    #[inline(always)]
    fn consume_all_gas(&self) -> ! {
        self.inner.consume_all_gas()
    }
    #[inline(always)]
    fn terminate(&self, beneficiary: &[u8; 20]) -> ! {
        self.inner.terminate(beneficiary)
    }
}

// `Host` on a non-riscv64 target without `alloc` is uninhabited — every
// method dispatch is `match self._never {}`. This exists so contract code
// that names `Host` still compiles on this configuration, even though no
// `Host` value can ever be constructed.
#[cfg(all(not(target_arch = "riscv64"), not(feature = "alloc")))]
impl HostApi for Host {
    fn address(&self, _output: &mut [u8; 20]) {
        match self._never {}
    }
    fn get_immutable_data(&self, _output: &mut &mut [u8]) {
        match self._never {}
    }
    fn set_immutable_data(&self, _data: &[u8]) {
        match self._never {}
    }
    fn balance(&self, _output: &mut [u8; 32]) {
        match self._never {}
    }
    fn balance_of(&self, _addr: &[u8; 20], _output: &mut [u8; 32]) {
        match self._never {}
    }
    fn chain_id(&self, _output: &mut [u8; 32]) {
        match self._never {}
    }
    fn gas_price(&self) -> u64 {
        match self._never {}
    }
    fn base_fee(&self, _output: &mut [u8; 32]) {
        match self._never {}
    }
    fn call_data_size(&self) -> u64 {
        match self._never {}
    }
    fn call(
        &self,
        _flags: CallFlags,
        _callee: &[u8; 20],
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit: &[u8; 32],
        _value: &[u8; 32],
        _input_data: &[u8],
        _output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        match self._never {}
    }
    fn call_evm(
        &self,
        _flags: CallFlags,
        _callee: &[u8; 20],
        _gas: u64,
        _value: &[u8; 32],
        _input_data: &[u8],
        _output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        match self._never {}
    }
    fn caller(&self, _output: &mut [u8; 20]) {
        match self._never {}
    }
    fn origin(&self, _output: &mut [u8; 20]) {
        match self._never {}
    }
    fn code_hash(&self, _addr: &[u8; 20], _output: &mut [u8; 32]) {
        match self._never {}
    }
    fn code_size(&self, _addr: &[u8; 20]) -> u64 {
        match self._never {}
    }
    fn delegate_call(
        &self,
        _flags: CallFlags,
        _address: &[u8; 20],
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit_limit: &[u8; 32],
        _input_data: &[u8],
        _output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        match self._never {}
    }
    fn delegate_call_evm(
        &self,
        _flags: CallFlags,
        _address: &[u8; 20],
        _gas: u64,
        _input_data: &[u8],
        _output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        match self._never {}
    }
    fn deposit_event(&self, _topics: &[[u8; 32]], _data: &[u8]) {
        match self._never {}
    }
    fn get_storage(
        &self,
        _flags: StorageFlags,
        _key: &[u8],
        _output: &mut &mut [u8],
    ) -> HostResult {
        match self._never {}
    }
    fn hash_keccak_256(&self, _input: &[u8], _output: &mut [u8; 32]) {
        match self._never {}
    }
    fn call_data_copy(&self, _output: &mut [u8], _offset: u32) {
        match self._never {}
    }
    fn call_data_load(&self, _output: &mut [u8; 32], _offset: u32) {
        match self._never {}
    }
    fn instantiate(
        &self,
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit: &[u8; 32],
        _value: &[u8; 32],
        _input: &[u8],
        _address: Option<&mut [u8; 20]>,
        _output: Option<&mut &mut [u8]>,
        _salt: Option<&[u8; 32]>,
    ) -> HostResult {
        match self._never {}
    }
    fn now(&self, _output: &mut [u8; 32]) {
        match self._never {}
    }
    fn gas_limit(&self) -> u64 {
        match self._never {}
    }
    fn set_storage(&self, _flags: StorageFlags, _key: &[u8], _value: &[u8]) -> Option<u32> {
        match self._never {}
    }
    fn set_storage_or_clear(
        &self,
        _flags: StorageFlags,
        _key: &[u8; 32],
        _value: &[u8; 32],
    ) -> Option<u32> {
        match self._never {}
    }
    fn get_storage_or_zero(&self, _flags: StorageFlags, _key: &[u8; 32], _output: &mut [u8; 32]) {
        match self._never {}
    }
    fn value_transferred(&self, _output: &mut [u8; 32]) {
        match self._never {}
    }
    fn return_data_size(&self) -> u64 {
        match self._never {}
    }
    fn return_data_copy(&self, _output: &mut &mut [u8], _offset: u32) {
        match self._never {}
    }
    fn gas_left(&self) -> u64 {
        match self._never {}
    }
    fn block_author(&self, _output: &mut [u8; 20]) {
        match self._never {}
    }
    fn block_number(&self, _output: &mut [u8; 32]) {
        match self._never {}
    }
    fn block_hash(&self, _block_number: &[u8; 32], _output: &mut [u8; 32]) {
        match self._never {}
    }
    fn return_value(&self, _flags: ReturnFlags, _data: &[u8]) {
        match self._never {}
    }
    fn consume_all_gas(&self) -> ! {
        match self._never {}
    }
    fn terminate(&self, _beneficiary: &[u8; 20]) -> ! {
        match self._never {}
    }
}

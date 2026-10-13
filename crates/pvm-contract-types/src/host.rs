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
//!
//! # Where a new host operation belongs
//!
//! Three layers, and the distinction is worth keeping because the first one is
//! also the mock seam — anything added there has to be implemented, or correctly
//! re-forwarded, by every implementor:
//!
//! 1. **[`HostApi`] — one method per pallet-revive syscall.** Byte-level
//!    signatures, `&self` receiver. The only permitted departures from
//!    `pallet_revive_uapi::HostFn` are those the seam forces: the
//!    `return_value`/`revert` split (one syscall, projected by its flag value, so
//!    success can be non-diverging on host targets while revert diverges on
//!    both) and the `-> !` cfg-gating described above. **No defaulted methods and
//!    no derived queries** — a default body is a silent correctness trap for
//!    wrapper implementors like [`Host`], which would re-derive it from the
//!    primitives and ignore whatever the backing `HostApi` said.
//! 2. **Free functions generic over `H: HostApi`** for cheap predicates the
//!    framework itself needs — see
//!    [`value_transferred_is_nonzero`](crate::value_transferred_is_nonzero),
//!    which the payable guard uses. These are dispatch plumbing, not user
//!    surface.
//! 3. **[`Env`] for user-facing typed reads.** Anything a contract author should
//!    reach for: typed returns instead of raw buffers, and the little-endian
//!    decoding done for them. [`Env::has_code`] is the model for a derived read —
//!    no syscall of its own, built on [`HostApi::code_size`].

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

    /// Read-only transaction/block context — the typed equivalent of Solidity's
    /// `msg.*` / `block.*` globals. See [`Env`].
    ///
    /// Provided rather than required: every context root already exposes a
    /// [`Host`], and [`Env`] holds nothing else. Defining it here means it is
    /// reachable through the `&impl ContractContext` bound that cross-contract
    /// call builders impose, so a DSL helper written against that bound can
    /// read context without going back to `cx.host()`.
    ///
    /// The `#[contract]` macro also emits an *inherent* `env()` on the storage
    /// struct. Inherent methods win method resolution, so that one applies
    /// inside contract bodies and this trait need not be in scope there — the
    /// same arrangement as [`ContractContext::host`].
    ///
    /// `unreachable_code` is allowed for the same reason as on [`Host::env`]:
    /// on a host target without `alloc` both `Host` and `Env` are uninhabited,
    /// so the body is vacuous there and a no-op everywhere else.
    #[inline(always)]
    #[allow(unreachable_code)]
    fn env(&self) -> Env {
        self.host().env()
    }
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
/// Context reads go through [`ContractContext::env`], so `cx.env().caller()`
/// works on a `Context` (and on any `&impl ContractContext`) without reaching
/// for the `host` field.
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
///
/// # Byte order of 32-byte buffers
///
/// Two categories, and mixing them up is silent corruption rather than a
/// compile error, so it is worth stating once:
///
/// **Numbers are little-endian.** `balance`, `balance_of`, `chain_id`,
/// `base_fee`, `value_transferred`, `now`, and `block_number` write their
/// output little-endian, because that is what pallet-revive writes
/// (`to_little_endian` in the runtime's `vm/pvm/env.rs`). Inputs are symmetric —
/// every 32-byte numeric argument is read back with `U256::from_little_endian`:
/// `value` on [`HostApi::call`], [`HostApi::call_evm`] and
/// [`HostApi::instantiate`]; `deposit` on [`HostApi::call`] and
/// [`HostApi::instantiate`]; `deposit_limit` on [`HostApi::delegate_call`]; and
/// `block_number` on [`HostApi::block_hash`]. (The `_evm` variants carry no
/// deposit, and `delegate_call` carries no value — a delegatecall runs in the
/// caller's frame.) [`Env`] has a typed accessor for every one of
/// those seven outputs — [`balance`](Env::balance),
/// [`balance_of`](Env::balance_of), [`chain_id`](Env::chain_id),
/// [`base_fee`](Env::base_fee), [`value`](Env::value),
/// [`timestamp`](Env::timestamp), [`block_number`](Env::block_number) — so
/// prefer it over decoding the raw buffers by hand.
///
/// **Identifiers are opaque bytes.** `address`, `caller`, `origin`,
/// `block_author`, `code_hash`, and `block_hash` outputs are `H160`/`H256`
/// values copied verbatim. They are not numbers and are not byte-swapped, so
/// "endianness" does not apply to them.
///
/// Note this is a different convention from contract *storage* slots and *ABI*
/// encoding, both of which are big-endian to match solc. The little-endian rule
/// covers only the host-call boundary described above.
///
/// Any implementor — including test mocks — must uphold this. `MockHost`'s
/// numeric builder setters take typed values and encode little-endian for
/// exactly this reason.
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

    /// Return successfully with the ABI-encoded `data`.
    ///
    /// `return_value` and [`Self::revert`] are the two halves of pallet-revive's
    /// single `return_value(flags, data)` exit syscall: on `riscv64` both inline
    /// to that one syscall and differ only by [`ReturnFlags`] (empty here,
    /// `REVERT` there). They are split into two methods only so the host mock can
    /// give success a non-diverging capture seam (see below).
    ///
    /// This is the **success** door — it never carries a revert, and it is
    /// **internal**: only the single-exit lowering (`finalize_outcome`, or the
    /// DSL's `finalize_response`) calls it, from an encoded `Outcome::Return`.
    /// Contract authors fail a frame via [`Self::revert`], not by calling this.
    ///
    /// On `riscv64` this is the `return_value` syscall (with empty flags) and
    /// never returns. On host targets the test mock captures the call as a
    /// [`ReturnValue`](super::ReturnValue) and returns control to the caller
    /// (so tests can inspect the success payload) — see
    /// [`MockHost::take_return_value`](super::MockHost::take_return_value).
    #[cfg(target_arch = "riscv64")]
    fn return_value(&self, data: &[u8]) -> !;

    /// Capture the success return value (host-target equivalent of the
    /// `riscv64` diverging syscall). Implementations on host targets should
    /// record `data` (typically into a [`ReturnValue`](super::ReturnValue) with
    /// empty flags) for the test to inspect after the dispatch returns.
    #[cfg(not(target_arch = "riscv64"))]
    fn return_value(&self, data: &[u8]);

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

    /// Revert the frame with ABI-encoded return `data`.
    ///
    /// This is the **failure** door — the sole way to revert. It **diverges on
    /// both targets** so it can be called from inside a value-returning method
    /// (e.g. a storage getter) that has no value to return on the error path,
    /// as well as from dispatch error arms.
    ///
    /// On `riscv64` this is the `return_value` syscall with the
    /// [`ReturnFlags::REVERT`] flag and never returns. On host targets the mock
    /// records `data` (tagged with [`ReturnFlags::REVERT`]) and then diverges
    /// via a typed panic. Because it diverges, tests recover the payload with
    /// [`MockHost::expect_revert`](super::MockHost::expect_revert) — which
    /// catches the unwind (via
    /// [`MockHost::run_until_halt`](super::MockHost::run_until_halt)) and returns
    /// the captured [`ReturnValue`](super::ReturnValue) — not a bare
    /// `take_return_value()`, which the reverting call unwinds past.
    fn revert(&self, data: &[u8]) -> !;
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
    fn return_value(&self, data: &[u8]) -> ! {
        // Every contract exit routes through `return_value` (both the dispatch's
        // normal return and a raw call in a user body), so release the reentrancy
        // lock here if this frame holds it. This covers a body that exits via a
        // raw `return_value`, which would otherwise skip the codegen's post-body
        // unlock.
        crate::reentrancy::__reentrancy_clear_if_held(self);
        pallet_revive_uapi::HostFnImpl::return_value(ReturnFlags::empty(), data)
    }
    #[inline(always)]
    fn consume_all_gas(&self) -> ! {
        pallet_revive_uapi::HostFnImpl::consume_all_gas()
    }
    #[inline(always)]
    fn terminate(&self, beneficiary: &[u8; 20]) -> ! {
        pallet_revive_uapi::HostFnImpl::terminate(beneficiary)
    }
    #[inline(always)]
    fn revert(&self, data: &[u8]) -> ! {
        pallet_revive_uapi::HostFnImpl::return_value(ReturnFlags::REVERT, data)
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
    fn return_value(&self, _data: &[u8]) {
        unimplemented!("PolkaVmHost::return_value is only available on PolkaVM")
    }
    fn consume_all_gas(&self) -> ! {
        unimplemented!("PolkaVmHost::consume_all_gas is only available on PolkaVM")
    }
    fn terminate(&self, _beneficiary: &[u8; 20]) -> ! {
        unimplemented!("PolkaVmHost::terminate is only available on PolkaVM")
    }
    fn revert(&self, _data: &[u8]) -> ! {
        unimplemented!("PolkaVmHost::revert is only available on PolkaVM")
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
    fn return_value(&self, data: &[u8]) -> ! {
        self.inner.return_value(data)
    }
    #[cfg(not(target_arch = "riscv64"))]
    #[inline(always)]
    fn return_value(&self, data: &[u8]) {
        self.inner.return_value(data)
    }
    #[inline(always)]
    fn consume_all_gas(&self) -> ! {
        self.inner.consume_all_gas()
    }
    #[inline(always)]
    fn terminate(&self, beneficiary: &[u8; 20]) -> ! {
        self.inner.terminate(beneficiary)
    }
    #[inline(always)]
    fn revert(&self, data: &[u8]) -> ! {
        self.inner.revert(data)
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
    fn return_value(&self, _data: &[u8]) {
        match self._never {}
    }
    fn consume_all_gas(&self) -> ! {
        match self._never {}
    }
    fn terminate(&self, _beneficiary: &[u8; 20]) -> ! {
        match self._never {}
    }
    fn revert(&self, _data: &[u8]) -> ! {
        match self._never {}
    }
}

/// Read-only accessor for chain context — the typed equivalent of Solidity's
/// `msg.*` / `block.*` globals and of the `<address>` members that read chain
/// state.
///
/// Obtained from [`Host::env()`]: `self.env()` inside a `#[contract]` method,
/// `host.env()` in a DSL handler. Holds only a cloned `Host` handle (a ZST on
/// riscv64, one `Rc` bump on host targets) and no state of its own, so
/// constructing one per use is free.
///
/// Current-frame reads, all zero-argument:
///
/// | Accessor | Solidity | Returns |
/// |---|---|---|
/// | [`caller`](Env::caller) | `msg.sender` | [`Address`](crate::Address) |
/// | [`origin`](Env::origin) | `tx.origin` | [`Address`](crate::Address) |
/// | [`address`](Env::address) | `address(this)` | [`Address`](crate::Address) |
/// | [`value`](Env::value) | `msg.value` | [`U256`](crate::U256) |
/// | [`balance`](Env::balance) | `address(this).balance` | [`U256`](crate::U256) |
/// | [`base_fee`](Env::base_fee) | `block.basefee` | [`U256`](crate::U256) |
/// | [`block_number`](Env::block_number) | `block.number` | `u64` |
/// | [`timestamp`](Env::timestamp) | `block.timestamp` | `u64` |
/// | [`chain_id`](Env::chain_id) | `block.chainid` | `u64` |
///
/// Address queries, parameterized by the account being asked about:
///
/// | Accessor | Solidity | Returns |
/// |---|---|---|
/// | [`balance_of`](Env::balance_of) | `addr.balance` | [`U256`](crate::U256) |
/// | [`has_code`](Env::has_code) | `addr.code.length != 0` | `bool` |
///
/// Every accessor takes `&self`, so they are all available to `view` methods.
/// A `pure` method has no receiver and therefore no `env()` — the same
/// restriction solc applies.
///
/// These decode the host's little-endian numeric buffers (see the byte-order
/// note on [`HostApi`]); using them is how you avoid getting that wrong by hand.
///
/// # Return widths
///
/// The host reports every one of these as 32 bytes, but that width is
/// EVM-compatibility packaging, not the value's actual range. Each accessor
/// returns the type matching what pallet-revive actually holds:
///
/// - `block.number` is `BlockNumberFor<T>`, widened to 32 bytes for the ABI.
///   Frame bounds that associated type only by `AtLeast32Bit`, so its width is
///   the runtime's choice; every real runtime uses `u32`.
/// - `block.timestamp` is a `u64` millisecond moment divided down to seconds.
/// - `block.chainid` is declared `type ChainId: Get<u64>`.
/// - `msg.value` is a genuine 256-bit balance, so it stays [`U256`](crate::U256).
/// - Balances (`address(this).balance`, `addr.balance`) are reported in EVM
///   units — the native balance scaled by pallet-revive's `NativeToEthRatio` —
///   and so are 256-bit too.
/// - `block.basefee` is `uint256` in Solidity, and pallet-revive guarantees no
///   narrower width for it.
///
/// The three `u64` accessors therefore narrow to the low 8 bytes, and they do it
/// without a range check. Two of the three widths pallet-revive guarantees
/// outright: the timestamp is a `u64` moment, and the chain ID is declared
/// `Get<u64>`. `block.number` rests on the weaker footing above — `u64` there is
/// runtime convention, not a pallet guarantee — so the honest statement is that
/// no reachable runtime puts data in the high 24 bytes, rather than that none
/// can. Paying a check on every read to cover a runtime with more than
/// `u64::MAX` blocks is not a trade worth making, but the reasoning is
/// convention for `block_number` and proof only for the other two.
///
/// The balance-shaped reads — [`value`](Env::value), [`balance`](Env::balance),
/// [`balance_of`](Env::balance_of) and [`base_fee`](Env::base_fee) — genuinely
/// need 256 bits, and they keep them.
pub struct Env(Host);

/// Narrow a little-endian 32-byte host value to `u64`.
///
/// The high 24 bytes are ignored, with no range check. See "Return widths" on
/// [`Env`] for which of the narrowed widths pallet-revive guarantees and which
/// one rests on runtime convention.
#[inline(always)]
fn narrow_to_u64(b: [u8; 32]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

impl Env {
    #[doc(hidden)]
    #[inline(always)]
    pub(crate) fn new(host: Host) -> Self {
        Env(host)
    }

    /// The immediate caller (Solidity `msg.sender`).
    ///
    /// Under `delegatecall` this is the delegating contract's caller, matching
    /// EVM semantics.
    #[inline(always)]
    pub fn caller(&self) -> crate::Address {
        let mut b = [0u8; 20];
        self.0.caller(&mut b);
        crate::Address(b)
    }

    /// The account that signed the transaction (Solidity `tx.origin`).
    ///
    /// Unlike [`Env::caller`] this is the outermost sender and is unchanged by
    /// intermediate contract calls, so it is the same value at every depth of a
    /// call stack.
    ///
    /// **Do not use it for authorization.** `origin() == owner` passes for *any*
    /// contract the owner is tricked into calling, which is the classic
    /// phishing-via-intermediary hole; authorize on [`Env::caller`] instead.
    /// Legitimate uses are narrow — mainly "is this the top-level frame?"
    /// (`caller() == origin()`).
    #[inline(always)]
    pub fn origin(&self) -> crate::Address {
        let mut b = [0u8; 20];
        self.0.origin(&mut b);
        crate::Address(b)
    }

    /// This contract's own address (Solidity `address(this)`).
    ///
    /// Under `delegatecall` this is the *delegating* contract's address — the
    /// executing code's storage context, not the address the code was loaded
    /// from — matching EVM semantics.
    #[inline(always)]
    pub fn address(&self) -> crate::Address {
        let mut b = [0u8; 20];
        self.0.address(&mut b);
        crate::Address(b)
    }

    /// Value transferred with this call (Solidity `msg.value`).
    ///
    /// Always zero in a non-payable method reached through external dispatch —
    /// the dispatch prelude reverts before the body runs if value was attached.
    /// (An internal Rust call from a payable method into another method's body
    /// skips that prelude and observes the frame's actual value.)
    #[inline(always)]
    pub fn value(&self) -> crate::U256 {
        let mut b = [0u8; 32];
        self.0.value_transferred(&mut b);
        crate::U256::from_le_bytes(b)
    }

    /// This contract's own balance (Solidity `address(this).balance`).
    ///
    /// Reported in EVM units — pallet-revive scales the native balance by
    /// `NativeToEthRatio` — so this stays [`U256`](crate::U256); see "Return
    /// widths" on [`Env`].
    ///
    /// Like the EVM, the value transferred with the current call is already
    /// included: pallet-revive performs the transfer before handing control to
    /// the contract, so a payable method sees the post-credit balance.
    ///
    /// It is the *reducible* balance (free, `Preservation::Preserve`), so it
    /// excludes the existential deposit and anything locked or held. That is a
    /// narrower quantity than EVM's `balance`, which has no ED to reserve.
    #[inline(always)]
    pub fn balance(&self) -> crate::U256 {
        let mut b = [0u8; 32];
        self.0.balance(&mut b);
        crate::U256::from_le_bytes(b)
    }

    /// [EIP-1559](https://eips.ethereum.org/EIPS/eip-1559) base fee of the
    /// current block (Solidity `block.basefee`).
    ///
    /// Stays [`U256`](crate::U256): Solidity declares `block.basefee` as
    /// `uint256`, and unlike [`Env::block_number`] / [`Env::timestamp`] /
    /// [`Env::chain_id`], pallet-revive guarantees no narrower width for it.
    #[inline(always)]
    pub fn base_fee(&self) -> crate::U256 {
        let mut b = [0u8; 32];
        self.0.base_fee(&mut b);
        crate::U256::from_le_bytes(b)
    }

    /// Current block number (Solidity `block.number`).
    ///
    /// Narrowed from the host's 32 bytes to the low 8. See "Return widths" on
    /// [`Env`].
    #[inline(always)]
    pub fn block_number(&self) -> u64 {
        let mut b = [0u8; 32];
        self.0.block_number(&mut b);
        narrow_to_u64(b)
    }

    /// Current block timestamp in seconds (Solidity `block.timestamp`).
    ///
    /// Narrowed like [`Env::block_number`].
    #[inline(always)]
    pub fn timestamp(&self) -> u64 {
        let mut b = [0u8; 32];
        self.0.now(&mut b);
        narrow_to_u64(b)
    }

    /// [EIP-155](https://eips.ethereum.org/EIPS/eip-155) chain ID (Solidity
    /// `block.chainid`).
    ///
    /// `u64` because that is pallet-revive's own declared width
    /// (`type ChainId: Get<u64>`); narrowed like [`Env::block_number`].
    #[inline(always)]
    pub fn chain_id(&self) -> u64 {
        let mut b = [0u8; 32];
        self.0.chain_id(&mut b);
        narrow_to_u64(b)
    }

    /// Balance of the account at `addr` (Solidity `addr.balance`).
    ///
    /// Named after the host syscall, which `pallet_revive_uapi`, [`HostApi`]
    /// and `MockHostBuilder` all spell `balance_of`. This is the *chain's*
    /// balance of an account — unrelated to any ERC-20 `balance_of` method the
    /// contract itself defines, which is reached as `self.balance_of(addr)`
    /// rather than `self.env().balance_of(addr)`.
    ///
    /// Same units and same reducible-balance caveat as [`Env::balance`], which
    /// is the dedicated syscall for the contract's own address and cheaper than
    /// passing [`Env::address`] here.
    #[inline(always)]
    pub fn balance_of(&self, addr: crate::Address) -> crate::U256 {
        let mut b = [0u8; 32];
        self.0.balance_of(&addr.0, &mut b);
        crate::U256::from_le_bytes(b)
    }

    /// Whether the account at `addr` has non-empty code (Solidity
    /// `addr.code.length != 0`).
    ///
    /// Returns `code_size(addr) > 0`.
    ///
    /// Not an "is this a contract" test: an address in its own constructor has
    /// no code yet and reads `false`, and an
    /// [EIP-7702](https://eips.ethereum.org/EIPS/eip-7702) delegated EOA carries
    /// code and reads `true`.
    #[inline(always)]
    pub fn has_code(&self, addr: crate::Address) -> bool {
        self.0.code_size(&addr.0) > 0
    }
}

impl Host {
    /// Return a read-only environment accessor.
    ///
    /// Usage: `self.env().caller()` on macro contracts, `host.env().caller()` on DSL handlers.
    ///
    /// On a host target without `alloc` both `Host` and `Env` are uninhabited, so
    /// the body is vacuous — hence the `unreachable_code` allow, which is a no-op
    /// in the configurations where a `Host` can actually be constructed.
    ///
    /// `clone_on_copy` is allowed because `Host` is `Copy` in two of its three
    /// cfg arms (riscv64 ZST, uninhabited) but `Clone`-only in the `Rc`-backed
    /// arm — `.clone()` is the one spelling valid in all three.
    #[inline(always)]
    #[allow(unreachable_code, clippy::clone_on_copy)]
    pub fn env(&self) -> Env {
        Env::new(self.clone())
    }
}

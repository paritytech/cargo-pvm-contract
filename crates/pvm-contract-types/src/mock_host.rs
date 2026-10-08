//! Mock host backend for native unit testing of PVM contracts.
//!
//! [`MockHost`] implements [`HostApi`](super::HostApi) using plain per-instance
//! state. Tests construct their own `MockHost` via [`MockHostBuilder`] and
//! inject it into the contract — no thread-locals, no global setup. Run tests
//! in parallel without contention.
//!
//! # Shared state via `Rc<RefCell<...>>`
//!
//! `MockHost` is `Clone`; all clones share the same underlying `MockState`
//! through an `Rc<RefCell<_>>`. This lets tests keep one handle for setup and
//! assertions while the contract (wrapped in [`super::Host`]) holds a second
//! handle that mutates the same storage, events, and return-data buffers.
//!
//! ```ignore
//! use std::rc::Rc;
//! use pvm_contract_types::{Host, HostApi, MockHostBuilder};
//!
//! let mock = MockHostBuilder::new().caller([0xAA; 20]).build();
//! let host = Host::from_dyn(Rc::new(mock.clone()));
//! // `mock` still observes writes done through `host`.
//! ```
//!
//! # Mock external calls
//!
//! ```ignore
//! let host = MockHostBuilder::new().build();
//! host.mock_call([0xBB; 20], Ok(vec![0, 0, 0, 1]));
//! // `HostApi::call` to [0xBB; 20] now returns Ok(()) with the mock data.
//! ```
//!
//! # Seeding numeric environment values
//!
//! The numeric setters take typed values (`u64` / [`U256`]) and encode them
//! little-endian, which is the byte order pallet-revive itself writes — see the
//! byte-order note on [`HostApi`](super::HostApi). Read them back through
//! `Host::env()`:
//!
//! ```ignore
//! use pvm_contract_types::{Host, MockHostBuilder, U256};
//!
//! let mock = MockHostBuilder::new()
//!     .block_number(258)
//!     .value_transferred(U256::from(1_000_000_000_000_000_000u64))
//!     .build();
//! let env = Host::from_dyn(std::rc::Rc::new(mock)).env();
//! assert_eq!(env.block_number(), 258);
//! ```
//!
//! The `*_raw` variants (`block_number_raw`, `value_transferred_raw`, …) store
//! the 32 bytes verbatim. Reach for those only when the test is asserting byte
//! layout — seeding raw big-endian bytes by hand is the mistake the typed
//! setters exist to prevent: the `u64`-narrowing accessors (`block_number`,
//! `timestamp`, `chain_id`) keep only the low 8 bytes, so a big-endian value
//! reads back as `0`.
//!
//! # Diverging host operations
//!
//! Two different mechanisms, by role:
//!
//! - [`HostApi::return_value`](super::HostApi::return_value) is the **success**
//!   door, called from the single-exit lowering (`finalize_outcome` / DSL
//!   `finalize_response`) on a successful return. On host targets `MockHost`
//!   captures `data` into a [`ReturnValue`] (tagged with empty flags) and
//!   returns normally; tests inspect the result via
//!   [`MockHost::take_return_value`].
//!
//! - [`HostApi::revert`](super::HostApi::revert),
//!   [`HostApi::terminate`](super::HostApi::terminate), and
//!   [`HostApi::consume_all_gas`](super::HostApi::consume_all_gas) can be
//!   called from arbitrary positions in user code (and, for `revert`, from
//!   dispatch error arms). On host targets `MockHost` panics with a typed
//!   payload so user code after the call doesn't run (matching on-chain
//!   semantics); `revert` also records its `data` (tagged [`ReturnFlags::REVERT`])
//!   so the payload survives the unwind. Tests recover the captured [`Halt`]
//!   via [`MockHost::run_until_halt`], which downcasts the panic and re-throws
//!   non-halt panics so contract bugs aren't silently swallowed.

use core::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use super::host::{CallFlags, HostApi, HostResult, ReturnErrorCode, ReturnFlags, StorageFlags};
use crate::U256;

/// Assert that `$body` reverts with exactly `$expected` ABI bytes, returning the
/// captured [`ReturnValue`] for any further inspection.
///
/// A thin, assertion-style wrapper over [`MockHost::expect_revert`] that hides
/// the `catch_unwind` closure so the call site reads like `assert_eq!`.
/// `$expected` is anything sliceable to `[u8]` (a `framework_errors` constant,
/// `&[u8]`, `Vec<u8>`, a byte string). Panics the test — with the actual outcome
/// — if `$body` doesn't revert, and with a byte diff if the data mismatches.
///
/// Use it for any revert — they all diverge: a method's own `Err(e)`, the input
/// size check, a malformed-calldata decode, the payable guard, and storage
/// `Panic`, on both the macro `route()` and DSL `dispatch_impl` paths. (Only a
/// non-reverting result is returned as data — assert on `Outcome::Return` for
/// that.)
///
/// ```ignore
/// // Macro path — a short-calldata size-check revert diverges during route():
/// let mut buf = [0u8; my_token::MAX_RETURN_LEN];
/// let mut out: &mut [u8] = &mut buf;
/// assert_reverts!(mock, framework_errors::INVALID_CALLDATA,
///     my_token::route(&mut c, sel, &short_input, &mut out));
/// ```
#[cfg(feature = "std")]
#[macro_export]
macro_rules! assert_reverts {
    ($mock:expr, $expected:expr, $body:expr $(,)?) => {{
        let rv = $mock.expect_revert(|| {
            // `let _ =` (not a bare `;`) so a `#[must_use]` body (e.g. `Result`)
            // doesn't trip `unused_must_use` at the call site.
            let _ = $body;
        });
        ::core::assert_eq!(rv.data.as_slice(), &($expected)[..], "revert data mismatch");
        rv
    }};
}

/// Assert that `$body` reverts with Solidity `Panic(uint256)` equal to
/// `$expected`, returning the decoded [`Panic`](crate::Panic).
///
/// A thin, assertion-style wrapper over [`MockHost::expect_panic`].
///
/// ```ignore
/// assert_panics!(mock, Panic::OutOfBoundsAccess, v.get(0));
/// ```
#[cfg(feature = "std")]
#[macro_export]
macro_rules! assert_panics {
    ($mock:expr, $expected:expr, $body:expr $(,)?) => {{
        let got = $mock.expect_panic(|| {
            let _ = $body;
        });
        ::core::assert_eq!(got, $expected);
        got
    }};
}

/// Return value for mocked external calls.
///
/// `Ok(data)` — call succeeds; `data` is written to the output buffer.
/// `Err(())` — call reverts with `ReturnErrorCode::CalleeReverted`.
pub type MockCallReturn = Result<Vec<u8>, ()>;

/// One captured event: `(topics, data)`.
pub type EventRecord = (Vec<[u8; 32]>, Vec<u8>);

/// The payload captured by [`MockHost`] for route-driving tests, from a single
/// [`HostApi::return_value`] (success) or [`HostApi::revert`] (failure) call.
///
/// `flags == ReturnFlags::empty()` indicates a successful return (the
/// dispatch arm matched and the method returned `Ok` / a value);
/// `flags == ReturnFlags::REVERT` indicates a revert, with `data` holding
/// the encoded revert payload (4-byte selector + ABI-encoded fields). The
/// flag is set by the mock according to which door was called, not by the
/// caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReturnValue {
    pub flags: ReturnFlags,
    pub data: Vec<u8>,
}

#[derive(Clone)]
struct MockInstantiateReturn {
    address: [u8; 20],
    output: Vec<u8>,
}

/// Captured halt event from a [`HostApi::terminate`] or
/// [`HostApi::consume_all_gas`] call on a [`MockHost`].
///
/// Returned by [`MockHost::run_until_halt`] when the contract method called
/// one of the diverging mid-execution host operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Halt {
    /// Contract called [`HostApi::terminate`] with this beneficiary address.
    Terminate { beneficiary: [u8; 20] },
    /// Contract called [`HostApi::consume_all_gas`].
    ConsumeAllGas,
    /// Contract called [`HostApi::revert`]. The revert `data` is recorded into
    /// the mock's [`ReturnValue`] before the unwind; recover it with
    /// [`MockHost::expect_revert`] (which returns the captured [`ReturnValue`]).
    /// This variant is just the typed marker that distinguishes a revert from
    /// the other halts.
    Revert,
}

/// Typed panic payload used by [`MockHost`] to halt execution on host targets.
///
/// Private — [`MockHost::run_until_halt`] is the only sanctioned way to
/// recover from this panic. Other panics propagate so contract bugs aren't
/// silently swallowed.
struct HaltPanic(Halt);

/// An opaque, cloned copy of a [`MockHost`]'s full state, produced by
/// [`MockHost::snapshot`] and reinstated by [`MockHost::restore`].
///
/// Use it to model pallet-revive's atomic revert explicitly: snapshot before a
/// call, and `restore` afterwards if it reverted (`MockHost` does not roll back
/// automatically — see [`MockHost::run_until_halt`]).
pub struct MockSnapshot(MockState);

/// Shared inner state of a [`MockHost`]. Lives behind `Rc<RefCell<_>>`.
#[derive(Clone)]
struct MockState {
    // --- Input state (typically set before execution, read during) ---
    caller: [u8; 20],
    origin: [u8; 20],
    address: [u8; 20],
    balance: [u8; 32],
    balances: HashMap<[u8; 20], [u8; 32]>,
    chain_id: [u8; 32],
    base_fee: [u8; 32],
    code_sizes: HashMap<[u8; 20], u64>,
    block_number: [u8; 32],
    block_timestamp: [u8; 32],
    block_author: [u8; 20],
    value_transferred: [u8; 32],
    calldata: Vec<u8>,

    // --- Mutable during execution ---
    storage: HashMap<Vec<u8>, Vec<u8>>,
    events: Vec<EventRecord>,
    immutable_data: Vec<u8>,
    return_data: Vec<u8>,
    /// Captured `return_value` (success) or `revert` (failure) payload from the
    /// contract. On host targets, `HostApi::return_value` does not diverge; it
    /// records the encoded success result here so route-driving tests can read
    /// it after the dispatch lowering (`finalize_outcome` / `dispatch_impl`)
    /// runs. `HostApi::revert` records the revert payload here too (before
    /// unwinding) so it survives to `take_return_value()`.
    return_value: Option<ReturnValue>,

    // --- Mock configuration ---
    call_returns: HashMap<[u8; 20], MockCallReturn>,
    instantiate_return: Option<MockInstantiateReturn>,

    /// Calldata captured from each `call`/`call_evm`/`delegate_call*` in order,
    /// so tests can assert on the exact input a wrapper sent to a callee.
    recorded_calls: Vec<([u8; 20], Vec<u8>)>,

    /// Raw `value` bytes captured from each value-bearing host call
    /// (`call`/`call_evm`/`instantiate`) in order. Not index-aligned with
    /// [`MockState::recorded_calls`]: `delegate_call*` has no value argument,
    /// and `instantiate` records no call.
    recorded_call_values: Vec<[u8; 32]>,
}

impl MockState {
    fn new() -> Self {
        Self {
            caller: [0; 20],
            origin: [0; 20],
            address: [0; 20],
            balance: [0; 32],
            balances: HashMap::new(),
            chain_id: [0; 32],
            base_fee: [0; 32],
            code_sizes: HashMap::new(),
            block_number: [0; 32],
            block_timestamp: [0; 32],
            block_author: [0; 20],
            value_transferred: [0; 32],
            calldata: Vec::new(),
            storage: HashMap::new(),
            events: Vec::new(),
            immutable_data: Vec::new(),
            return_data: Vec::new(),
            return_value: None,
            call_returns: HashMap::new(),
            instantiate_return: None,
            recorded_calls: Vec::new(),
            recorded_call_values: Vec::new(),
        }
    }

    /// Resolve one account's balance, the single source of truth behind both
    /// [`HostApi::balance`] and [`HostApi::balance_of`].
    ///
    /// On chain those two are the same query — pallet-revive routes both
    /// through `account_balance` — so `balance_of(address(this))` always equals
    /// `balance()`. The mock keeps two seeding surfaces for ergonomics
    /// ([`MockHostBuilder::balance`] needs no address, and works before
    /// [`MockHostBuilder::address`] is set), but resolves them here so the two
    /// reads can never disagree the way independent fields would.
    ///
    /// An explicit `balances` entry wins, since it names the account outright;
    /// otherwise the contract's own address falls back to the `balance` seed.
    fn resolve_balance(&self, addr: &[u8; 20]) -> [u8; 32] {
        match self.balances.get(addr) {
            Some(bal) => *bal,
            None if *addr == self.address => self.balance,
            None => [0u8; 32],
        }
    }
}

/// Mock host backend for native testing.
///
/// Holds a reference-counted handle to [`MockState`]. Cloning `MockHost` is
/// cheap (an `Rc` bump) and **shares state** — both the clone and the original
/// observe the same storage, events, return-data, and mock configuration.
///
/// Construct via [`MockHostBuilder::build`]. All operations take `&self`:
/// setup (`mock_call`, `mock_instantiate`), contract-facing `HostApi` calls,
/// and test assertions (`events`, `get_raw_storage`).
///
/// Re-entrancy: every state access uses the borrow-drop-immediately pattern —
/// values are copied/cloned out before downstream logic runs, so nested
/// HostApi calls triggered by a mock don't collide with a live borrow guard.
#[derive(Clone)]
pub struct MockHost {
    state: Rc<RefCell<MockState>>,
}

impl MockHost {
    /// Register a mock return value for [`HostApi::call`] to `callee`.
    pub fn mock_call(&self, callee: [u8; 20], result: MockCallReturn) {
        self.state.borrow_mut().call_returns.insert(callee, result);
    }

    /// Calldata captured from each [`HostApi::call`] / [`HostApi::call_evm`] /
    /// [`HostApi::delegate_call`] / [`HostApi::delegate_call_evm`], in call
    /// order, as `(callee, input_data)` pairs.
    ///
    /// Lets tests assert on the exact bytes a wrapper sent to a callee — e.g.
    /// that a precompile wrapper built the spec-mandated input layout.
    ///
    /// Not index-aligned with [`MockHost::recorded_call_values`] —
    /// [`HostApi::instantiate`] appears there but not here, so the two logs
    /// must not be zipped.
    pub fn recorded_calls(&self) -> Vec<([u8; 20], Vec<u8>)> {
        self.state.borrow().recorded_calls.clone()
    }

    /// Same as [`MockHost::recorded_calls`], but drains the log so the next
    /// assertion sees only the calls made after this point. Useful when one
    /// test drives several calls in sequence on the same mock.
    pub fn take_recorded_calls(&self) -> Vec<([u8; 20], Vec<u8>)> {
        core::mem::take(&mut self.state.borrow_mut().recorded_calls)
    }

    /// Raw `value` bytes captured from each [`HostApi::call`] /
    /// [`HostApi::call_evm`] / [`HostApi::instantiate`], in call order.
    ///
    /// Exposed verbatim (not decoded) so tests can assert the **byte order** a
    /// caller used. pallet-revive reads this argument with
    /// `U256::from_little_endian`, so a caller that encodes big-endian asks the
    /// runtime to move a wildly different amount — a mistake that is invisible
    /// to any assertion made on a decoded value.
    ///
    /// Not index-aligned with [`MockHost::recorded_calls`] —
    /// [`HostApi::delegate_call`] / [`HostApi::delegate_call_evm`] appear there
    /// but not here, so the two logs must not be zipped.
    pub fn recorded_call_values(&self) -> Vec<[u8; 32]> {
        self.state.borrow().recorded_call_values.clone()
    }

    /// Record the raw `value` bytes of a value-bearing host call. The borrow is
    /// released before returning so nested `HostApi` calls can't collide.
    fn record_call_value(&self, value: &[u8; 32]) {
        self.state.borrow_mut().recorded_call_values.push(*value);
    }

    /// Register a mock return for [`HostApi::instantiate`].
    pub fn mock_instantiate(&self, address: [u8; 20], output: Vec<u8>) {
        self.state.borrow_mut().instantiate_return =
            Some(MockInstantiateReturn { address, output });
    }

    /// All events emitted via [`HostApi::deposit_event`].
    pub fn events(&self) -> Vec<EventRecord> {
        self.state.borrow().events.clone()
    }

    /// Capture the full mock state (storage, events, balances, …) so it can be
    /// reinstated later with [`Self::restore`]. The primary use is modelling atomic revert
    /// explicitly (snapshot before a call, `restore` if it reverted).
    pub fn snapshot(&self) -> MockSnapshot {
        MockSnapshot(self.state.borrow().clone())
    }

    /// Reinstate a state previously captured by [`Self::snapshot`], discarding
    /// every mutation made since. This is the explicit rollback primitive —
    /// `MockHost` never rolls back on its own.
    pub fn restore(&self, snapshot: MockSnapshot) {
        *self.state.borrow_mut() = snapshot.0;
    }

    /// Raw storage read — for test assertions.
    pub fn get_raw_storage(&self, key: &[u8]) -> Option<Vec<u8>> {
        self.state.borrow().storage.get(key).cloned()
    }

    /// Raw storage write — for test setup.
    pub fn set_raw_storage(&self, key: Vec<u8>, value: Vec<u8>) {
        self.state.borrow_mut().storage.insert(key, value);
    }

    /// Snapshot of the entire storage map as `(key, value)` pairs — for tests
    /// that need to enumerate every written slot (e.g. diffing the full storage
    /// representation against another implementation), not just point-lookup a
    /// known key. Zero-valued slots are already absent: `set_storage_or_clear`
    /// deletes on a zero write, matching solc's SSTORE-of-zero semantics.
    pub fn storage_dump(&self) -> Vec<(Vec<u8>, Vec<u8>)> {
        self.state
            .borrow()
            .storage
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Take the [`ReturnValue`] captured by the most recent
    /// [`HostApi::return_value`] (success) or [`HostApi::revert`] (failure)
    /// call on this mock, leaving the slot empty. Returns `None` if neither
    /// has been called since the last `take_return_value`.
    ///
    /// On host targets, the dispatch lowering (`finalize_outcome` / DSL
    /// `dispatch_impl`) calls `host.return_value(...)`, which records the encoded
    /// result here instead of diverging. For reverts, prefer
    /// [`Self::expect_revert`], which recovers the payload after the unwind.
    /// Consuming the value rather than cloning prevents stale captures from
    /// leaking across calls on the same mock.
    pub fn take_return_value(&self) -> Option<ReturnValue> {
        self.state.borrow_mut().return_value.take()
    }

    /// Run `f`, returning the captured [`Halt`] if it called
    /// [`HostApi::revert`], [`HostApi::terminate`], or
    /// [`HostApi::consume_all_gas`].
    ///
    /// Returns `None` if `f` completed without halting. Non-halt panics from
    /// `f` (overflow, `unwrap`, `BorrowMutError`, etc.) propagate via
    /// [`std::panic::resume_unwind`] so contract bugs surface as test
    /// failures rather than being silently captured as halts.
    ///
    /// `f` is wrapped in [`std::panic::AssertUnwindSafe`] internally so test
    /// authors don't need to thread the bound through their closures.
    ///
    /// **No implicit state rollback.** `MockHost` is a
    /// transparent flat state store: storage (SSTORE) and event (LOG) writes
    /// `f` made before a revert **persist** — this does not model
    /// pallet-revive's on-chain atomic revert. To assert or reset post-revert
    /// state, capture [`MockHost::snapshot`] before the call and
    /// [`MockHost::restore`] it after.
    pub fn run_until_halt<F: FnOnce()>(&self, f: F) -> Option<Halt> {
        use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
        match catch_unwind(AssertUnwindSafe(f)) {
            Ok(()) => None,
            Err(payload) => match payload.downcast::<HaltPanic>() {
                Ok(halt) => Some(halt.0),
                Err(other) => resume_unwind(other),
            },
        }
    }

    /// Run `f`, asserting it reverted via [`HostApi::revert`], and return the
    /// captured [`ReturnValue`] (`flags == REVERT` plus the ABI-encoded revert
    /// `data`).
    ///
    /// Because [`HostApi::revert`] is the sole revert door, this catches **every**
    /// revert: a method's own `Err(e)`, the input size check, a malformed-calldata
    /// decode, the payable guard, and `panic_revert`-driven `Panic(uint256)` from
    /// storage — on both the macro `route()` and DSL `dispatch_impl` paths (they
    /// all diverge). Drive the contract inside `f`, then decode `rv.data`; for the
    /// standard `Panic(uint256)` case prefer [`Self::expect_panic`].
    ///
    /// Panics the test with a clear message if `f` returned normally or halted
    /// some other way (terminate / consume-all-gas). Genuine bug panics from
    /// `f` still propagate (via [`Self::run_until_halt`]). Built on
    /// `run_until_halt`, so an expected revert prints a panic line to stderr —
    /// same as the other halts; this is benign test noise.
    ///
    /// Note: a `#[method]` called *directly* (not through dispatch) returns
    /// `Err(e)` as a plain value and makes no host call — that is not a revert;
    /// assert on the `Result` instead.
    pub fn expect_revert<F: FnOnce()>(&self, f: F) -> ReturnValue {
        match self.run_until_halt(f) {
            Some(Halt::Revert) => self
                .take_return_value()
                .expect("revert should have recorded a ReturnValue"),
            Some(other) => panic!("expected a revert, but the closure halted via {other:?}"),
            None => panic!("expected a revert, but the closure returned normally"),
        }
    }

    /// Run `f`, asserting it returns successfully (records a `return_value`
    /// without reverting or otherwise halting), and return the captured
    /// [`ReturnValue`]. The success counterpart to [`Self::expect_revert`].
    ///
    /// For the DSL `dispatch_impl` this works directly — it records via
    /// [`HostApi::return_value`] then returns normally. For the `#[contract]`
    /// macro, `route()` returns an `Outcome` *without* calling the host, so lower
    /// it via `finalize_outcome(..)` inside `f` (or assert on the returned
    /// `Outcome` directly instead of using this). Panics the test if `f`
    /// unexpectedly halts (e.g. reverts), naming the actual halt, or if it
    /// returned without recording a value (e.g. an unmatched selector).
    pub fn expect_return<F: FnOnce()>(&self, f: F) -> ReturnValue {
        match self.run_until_halt(f) {
            None => self
                .take_return_value()
                .expect("expected a successful return, but no return_value was recorded"),
            Some(halt) => {
                panic!("expected a successful return, but the closure halted via {halt:?}")
            }
        }
    }

    /// Run `f`, asserting it reverted with Solidity `Panic(uint256)` data, and
    /// return the decoded [`Panic`](crate::Panic) variant. Assert on the
    /// variant, e.g. `assert_eq!(mock.expect_panic(|| { let _ = v.get(0); }),
    /// Panic::OutOfBoundsAccess)`.
    ///
    /// Panics the test if `f` did not revert (see [`Self::expect_revert`]), if
    /// the revert flag is missing, or if the data is not a decodable
    /// `Panic(uint256)`.
    pub fn expect_panic<F: FnOnce()>(&self, f: F) -> crate::Panic {
        use crate::SolError;
        let rv = self.expect_revert(f);
        assert!(
            rv.flags.contains(ReturnFlags::REVERT),
            "revert flags missing REVERT: {:?}",
            rv.flags
        );
        crate::Panic::decode_at(&rv.data, 0)
            .expect("malformed Panic(uint256) data")
            .expect("revert data was not a Panic(uint256) selector")
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
    state: MockState,
}

impl Default for MockHostBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl MockHostBuilder {
    pub fn new() -> Self {
        Self {
            state: MockState::new(),
        }
    }

    pub fn caller(mut self, caller: [u8; 20]) -> Self {
        self.state.caller = caller;
        self
    }

    pub fn origin(mut self, origin: [u8; 20]) -> Self {
        self.state.origin = origin;
        self
    }

    pub fn address(mut self, address: [u8; 20]) -> Self {
        self.state.address = address;
        self
    }

    /// Seed the contract's own balance. Encoded little-endian, matching
    /// [`HostApi::balance`]; use [`Self::balance_raw`] for verbatim bytes.
    ///
    /// This also answers [`HostApi::balance_of`] for the contract's own
    /// address, as it does on chain — the two host functions are one query
    /// there. Seeding the same address through [`Self::balance_of`] overrides
    /// it for both.
    pub fn balance(mut self, balance: U256) -> Self {
        self.state.balance = balance.to_le_bytes();
        self
    }

    /// Seed the contract's own balance from verbatim bytes.
    ///
    /// Prefer [`Self::balance`] unless the test deliberately asserts byte
    /// layout — the host writes this value little-endian.
    pub fn balance_raw(mut self, balance: [u8; 32]) -> Self {
        self.state.balance = balance;
        self
    }

    /// Seed an account's balance. Encoded little-endian, matching
    /// [`HostApi::balance_of`]; use [`Self::balance_of_raw`] for verbatim bytes.
    ///
    /// Passing the contract's own address also sets what [`HostApi::balance`]
    /// reports, and takes precedence over [`Self::balance`] regardless of the
    /// order the two are called in.
    pub fn balance_of(mut self, addr: [u8; 20], balance: U256) -> Self {
        self.state.balances.insert(addr, balance.to_le_bytes());
        self
    }

    /// Seed another account's balance from verbatim bytes.
    ///
    /// Prefer [`Self::balance_of`] unless the test deliberately asserts byte
    /// layout — the host writes this value little-endian.
    pub fn balance_of_raw(mut self, addr: [u8; 20], balance: [u8; 32]) -> Self {
        self.state.balances.insert(addr, balance);
        self
    }

    /// Seed the EVM base fee. Encoded little-endian, matching
    /// [`HostApi::base_fee`]; use [`Self::base_fee_raw`] for verbatim bytes.
    pub fn base_fee(mut self, base_fee: U256) -> Self {
        self.state.base_fee = base_fee.to_le_bytes();
        self
    }

    /// Seed the EVM base fee from verbatim bytes.
    ///
    /// Prefer [`Self::base_fee`] unless the test deliberately asserts byte
    /// layout — the host writes this value little-endian.
    pub fn base_fee_raw(mut self, base_fee: [u8; 32]) -> Self {
        self.state.base_fee = base_fee;
        self
    }

    pub fn immutable_data(mut self, data: Vec<u8>) -> Self {
        self.state.immutable_data = data;
        self
    }

    /// Seed the deployed code size of an account, as reported by
    /// [`HostApi::code_size`]. Any address left unseeded reports `0`.
    ///
    /// This is also what makes [`Env::has_code`](crate::Env::has_code)
    /// observable in tests: it is derived from `code_size`, so without a seeded
    /// size it is always `false`. Note that [`Self::mock_call`] does not seed a
    /// size, so a contract that guards a call with `has_code` needs both.
    pub fn code_size(mut self, addr: [u8; 20], len: u64) -> Self {
        self.state.code_sizes.insert(addr, len);
        self
    }

    /// Seed the EIP-155 chain ID, as read by `Env::chain_id`. Encoded
    /// little-endian; use [`Self::chain_id_raw`] for verbatim bytes.
    ///
    /// `u64` to match pallet-revive's `type ChainId: Get<u64>`.
    pub fn chain_id(mut self, chain_id: u64) -> Self {
        self.state.chain_id = U256::from(chain_id).to_le_bytes();
        self
    }

    /// Seed the chain ID from verbatim bytes.
    ///
    /// Prefer [`Self::chain_id`] unless the test deliberately asserts byte
    /// layout — the host writes this value little-endian.
    pub fn chain_id_raw(mut self, chain_id: [u8; 32]) -> Self {
        self.state.chain_id = chain_id;
        self
    }

    /// Seed the block number, as read by `Env::block_number`. Encoded
    /// little-endian; use [`Self::block_number_raw`] for verbatim bytes.
    pub fn block_number(mut self, block_number: u64) -> Self {
        self.state.block_number = U256::from(block_number).to_le_bytes();
        self
    }

    /// Seed the block number from verbatim bytes.
    ///
    /// Prefer [`Self::block_number`] unless the test deliberately asserts byte
    /// layout — the host writes this value little-endian, and `Env`'s
    /// `u64`-narrowing read keeps only the low 8 bytes.
    pub fn block_number_raw(mut self, block_number: [u8; 32]) -> Self {
        self.state.block_number = block_number;
        self
    }

    /// Seed the block timestamp in seconds, as read by `Env::timestamp`.
    /// Encoded little-endian; use [`Self::block_timestamp_raw`] for verbatim
    /// bytes.
    pub fn block_timestamp(mut self, timestamp: u64) -> Self {
        self.state.block_timestamp = U256::from(timestamp).to_le_bytes();
        self
    }

    /// Seed the block timestamp from verbatim bytes.
    ///
    /// Prefer [`Self::block_timestamp`] unless the test deliberately asserts
    /// byte layout — the host writes this value little-endian, and `Env`'s
    /// `u64`-narrowing read keeps only the low 8 bytes.
    pub fn block_timestamp_raw(mut self, timestamp: [u8; 32]) -> Self {
        self.state.block_timestamp = timestamp;
        self
    }

    pub fn block_author(mut self, author: [u8; 20]) -> Self {
        self.state.block_author = author;
        self
    }

    /// Seed `msg.value`, as read by `Env::value`. Encoded little-endian; use
    /// [`Self::value_transferred_raw`] for verbatim bytes.
    pub fn value_transferred(mut self, value: U256) -> Self {
        self.state.value_transferred = value.to_le_bytes();
        self
    }

    /// Seed `msg.value` from verbatim bytes.
    ///
    /// Prefer [`Self::value_transferred`] unless the test deliberately asserts
    /// byte layout — the host writes this value little-endian.
    pub fn value_transferred_raw(mut self, value: [u8; 32]) -> Self {
        self.state.value_transferred = value;
        self
    }

    pub fn calldata(mut self, data: Vec<u8>) -> Self {
        self.state.calldata = data;
        self
    }

    pub fn storage(mut self, entries: Vec<(Vec<u8>, Vec<u8>)>) -> Self {
        for (key, value) in entries {
            self.state.storage.insert(key, value);
        }
        self
    }

    pub fn mock_call(mut self, callee: [u8; 20], result: MockCallReturn) -> Self {
        self.state.call_returns.insert(callee, result);
        self
    }

    pub fn mock_instantiate(mut self, address: [u8; 20], output: Vec<u8>) -> Self {
        self.state.instantiate_return = Some(MockInstantiateReturn { address, output });
        self
    }

    /// Finalize the builder into a [`MockHost`] backed by `Rc<RefCell<_>>`.
    pub fn build(self) -> MockHost {
        MockHost {
            state: Rc::new(RefCell::new(self.state)),
        }
    }
}

// ---------------------------------------------------------------------------
// HostApi implementation
// ---------------------------------------------------------------------------

impl HostApi for MockHost {
    fn address(&self, output: &mut [u8; 20]) {
        *output = self.state.borrow().address;
    }

    fn get_immutable_data(&self, output: &mut &mut [u8]) {
        let data = self.state.borrow().immutable_data.clone();
        let len = data.len().min(output.len());
        output[..len].copy_from_slice(&data[..len]);
        let tmp = core::mem::take(output);
        *output = &mut tmp[..len];
    }

    fn set_immutable_data(&self, data: &[u8]) {
        self.state.borrow_mut().immutable_data = data.to_vec();
    }

    fn balance(&self, output: &mut [u8; 32]) {
        let state = self.state.borrow();
        *output = state.resolve_balance(&state.address);
    }

    fn balance_of(&self, addr: &[u8; 20], output: &mut [u8; 32]) {
        *output = self.state.borrow().resolve_balance(addr);
    }

    fn chain_id(&self, output: &mut [u8; 32]) {
        *output = self.state.borrow().chain_id;
    }

    fn gas_price(&self) -> u64 {
        0
    }

    fn base_fee(&self, output: &mut [u8; 32]) {
        *output = self.state.borrow().base_fee;
    }

    fn call_data_size(&self) -> u64 {
        self.state.borrow().calldata.len() as u64
    }

    fn call(
        &self,
        _flags: CallFlags,
        callee: &[u8; 20],
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit: &[u8; 32],
        value: &[u8; 32],
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        self.record_call_value(value);
        self.resolve_call(callee, input_data, output)
    }

    fn call_evm(
        &self,
        _flags: CallFlags,
        callee: &[u8; 20],
        _gas: u64,
        value: &[u8; 32],
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        self.record_call_value(value);
        self.resolve_call(callee, input_data, output)
    }

    fn caller(&self, output: &mut [u8; 20]) {
        *output = self.state.borrow().caller;
    }

    fn origin(&self, output: &mut [u8; 20]) {
        *output = self.state.borrow().origin;
    }

    fn code_hash(&self, _addr: &[u8; 20], output: &mut [u8; 32]) {
        output.fill(0);
    }

    fn code_size(&self, addr: &[u8; 20]) -> u64 {
        self.state
            .borrow()
            .code_sizes
            .get(addr)
            .copied()
            .unwrap_or(0)
    }

    fn delegate_call(
        &self,
        _flags: CallFlags,
        address: &[u8; 20],
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit_limit: &[u8; 32],
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        self.resolve_call(address, input_data, output)
    }

    fn delegate_call_evm(
        &self,
        _flags: CallFlags,
        address: &[u8; 20],
        _gas: u64,
        input_data: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        self.resolve_call(address, input_data, output)
    }

    fn deposit_event(&self, topics: &[[u8; 32]], data: &[u8]) {
        self.state
            .borrow_mut()
            .events
            .push((topics.to_vec(), data.to_vec()));
    }

    fn get_storage(&self, _flags: StorageFlags, key: &[u8], output: &mut &mut [u8]) -> HostResult {
        let value = self.state.borrow().storage.get(key).cloned();
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
        *output = crate::keccak256(input);
    }

    fn call_data_copy(&self, output: &mut [u8], offset: u32) {
        let calldata = self.state.borrow().calldata.clone();
        let start = (offset as usize).min(calldata.len());
        let len = output.len().min(calldata.len() - start);
        output[..len].copy_from_slice(&calldata[start..start + len]);
        output[len..].fill(0);
    }

    fn call_data_load(&self, output: &mut [u8; 32], offset: u32) {
        let calldata = self.state.borrow().calldata.clone();
        let start = (offset as usize).min(calldata.len());
        output.fill(0);
        let len = 32.min(calldata.len() - start);
        output[..len].copy_from_slice(&calldata[start..start + len]);
    }

    fn instantiate(
        &self,
        _ref_time_limit: u64,
        _proof_size_limit: u64,
        _deposit: &[u8; 32],
        value: &[u8; 32],
        _input: &[u8],
        address: Option<&mut [u8; 20]>,
        output: Option<&mut &mut [u8]>,
        _salt: Option<&[u8; 32]>,
    ) -> HostResult {
        self.record_call_value(value);
        let ret = self.state.borrow().instantiate_return.clone();
        match ret {
            Some(ret) => {
                if let Some(addr) = address {
                    *addr = ret.address;
                }
                self.state.borrow_mut().return_data = ret.output.clone();
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
        *output = self.state.borrow().block_timestamp;
    }

    fn gas_limit(&self) -> u64 {
        u64::MAX
    }

    fn set_storage(&self, _flags: StorageFlags, key: &[u8], value: &[u8]) -> Option<u32> {
        self.state
            .borrow_mut()
            .storage
            .insert(key.to_vec(), value.to_vec())
            .map(|v| v.len() as u32)
    }

    fn set_storage_or_clear(
        &self,
        _flags: StorageFlags,
        key: &[u8; 32],
        value: &[u8; 32],
    ) -> Option<u32> {
        let mut st = self.state.borrow_mut();
        if *value == [0u8; 32] {
            st.storage.remove(key.as_slice()).map(|v| v.len() as u32)
        } else {
            st.storage
                .insert(key.to_vec(), value.to_vec())
                .map(|v| v.len() as u32)
        }
    }

    fn get_storage_or_zero(&self, _flags: StorageFlags, key: &[u8; 32], output: &mut [u8; 32]) {
        let st = self.state.borrow();
        match st.storage.get(key.as_slice()) {
            Some(value) => {
                output.fill(0);
                let len = value.len().min(32);
                output[..len].copy_from_slice(&value[..len]);
            }
            None => output.fill(0),
        }
    }

    fn value_transferred(&self, output: &mut [u8; 32]) {
        *output = self.state.borrow().value_transferred;
    }

    fn return_data_size(&self) -> u64 {
        self.state.borrow().return_data.len() as u64
    }

    fn return_data_copy(&self, output: &mut &mut [u8], offset: u32) {
        let data = self.state.borrow().return_data.clone();
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
        *output = self.state.borrow().block_author;
    }

    fn block_number(&self, output: &mut [u8; 32]) {
        *output = self.state.borrow().block_number;
    }

    fn block_hash(&self, _block_number: &[u8; 32], output: &mut [u8; 32]) {
        output.fill(0);
    }

    fn return_value(&self, data: &[u8]) {
        self.record_return(ReturnFlags::empty(), data);
    }

    fn consume_all_gas(&self) -> ! {
        std::panic::panic_any(HaltPanic(Halt::ConsumeAllGas))
    }

    fn terminate(&self, beneficiary: &[u8; 20]) -> ! {
        std::panic::panic_any(HaltPanic(Halt::Terminate {
            beneficiary: *beneficiary,
        }))
    }

    fn revert(&self, data: &[u8]) -> ! {
        // Record the payload first so `take_return_value()` can assert the exact
        // ABI bytes. `record_return` drops the `MockState` borrow before we
        // return, so no borrow is held across the `panic_any` unwind below.
        self.record_return(ReturnFlags::REVERT, data);
        std::panic::panic_any(HaltPanic(Halt::Revert))
    }
}

impl MockHost {
    /// Record the frame's exit payload into the single `return_value` slot,
    /// tagged with `flags`. Shared by the two exit doors — the success door
    /// ([`HostApi::return_value`], empty flags) and the failure door
    /// ([`HostApi::revert`], `ReturnFlags::REVERT`) — so both record the bytes
    /// byte-identically; they differ only in the flag and whether they diverge.
    ///
    /// Does **not** diverge: `revert` calls this and then panics separately, so
    /// the `borrow_mut` here is released before any unwind.
    fn record_return(&self, flags: ReturnFlags, data: &[u8]) {
        // Single-exit invariant: on-chain the frame halts at the first exit
        // syscall, so a second exit before the previous payload is consumed is a
        // bug (or a test that forgot to `take_return_value()` between runs).
        // A hard `assert!` (not `debug_assert!`) so the check also holds under
        // `cargo test --release`, where a silent second-exit-wins would
        // otherwise invert the on-chain first-exit-wins semantics.
        assert!(
            self.state.borrow().return_value.is_none(),
            "MockHost: exit recorded twice without an intervening \
             take_return_value(); on-chain the frame would have halted at the \
             first exit"
        );
        self.state.borrow_mut().return_value = Some(ReturnValue {
            flags,
            data: data.to_vec(),
        });
    }

    /// Shared logic for `call`, `call_evm`, `delegate_call`, `delegate_call_evm`.
    /// Uses borrow-drop-immediately pattern to stay re-entrancy-safe.
    fn resolve_call(
        &self,
        callee: &[u8; 20],
        input: &[u8],
        output: Option<&mut &mut [u8]>,
    ) -> HostResult {
        self.state
            .borrow_mut()
            .recorded_calls
            .push((*callee, input.to_vec()));
        let resolved = self.state.borrow().call_returns.get(callee).cloned();
        match resolved {
            Some(Ok(data)) => {
                self.state.borrow_mut().return_data = data.clone();
                if let Some(out) = output {
                    let len = data.len().min(out.len());
                    out[..len].copy_from_slice(&data[..len]);
                }
                Ok(())
            }
            Some(Err(())) => {
                self.state.borrow_mut().return_data.clear();
                Err(ReturnErrorCode::CalleeReverted)
            }
            None => {
                self.state.borrow_mut().return_data.clear();
                Ok(())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Keccak-256 (minimal implementation for mock use)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keccak256_empty() {
        let hash = crate::keccak256(b"");
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
            .block_number(0)
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
    fn call_records_input_data() {
        let callee = [0x99; 20];
        let host = MockHostBuilder::new().mock_call(callee, Ok(vec![])).build();
        let input = [1u8, 2, 3, 4, 5];

        let _ = host.call_evm(CallFlags::empty(), &callee, 0, &[0u8; 32], &input, None);

        assert_eq!(host.recorded_calls(), vec![(callee, input.to_vec())]);
    }

    #[test]
    fn take_recorded_calls_drains_the_log() {
        let callee = [0x99; 20];
        let host = MockHostBuilder::new().build();

        let _ = host.call_evm(CallFlags::empty(), &callee, 0, &[0u8; 32], &[0xAA], None);
        assert_eq!(host.take_recorded_calls(), vec![(callee, vec![0xAA])]);
        assert_eq!(host.take_recorded_calls(), vec![]);

        let _ = host.call_evm(CallFlags::empty(), &callee, 0, &[0u8; 32], &[0xBB], None);
        assert_eq!(host.recorded_calls(), vec![(callee, vec![0xBB])]);
    }

    #[test]
    fn call_records_each_input_in_order() {
        let callee = [0x99; 20];
        let host = MockHostBuilder::new().build();

        let _ = host.call_evm(CallFlags::empty(), &callee, 0, &[0u8; 32], &[0xAA], None);
        let _ = host.call(
            CallFlags::empty(),
            &callee,
            0,
            0,
            &[0u8; 32],
            &[0u8; 32],
            &[0xBB, 0xCC],
            None,
        );

        assert_eq!(
            host.recorded_calls(),
            vec![(callee, vec![0xAA]), (callee, vec![0xBB, 0xCC])]
        );
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
        let host = MockHostBuilder::new().block_timestamp_raw(ts).build();

        let mut output = [0u8; 32];
        host.now(&mut output);
        assert_eq!(output, ts);
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
    fn delegate_call_evm_updates_return_data() {
        let callee = [0xCC; 20];
        let host = MockHostBuilder::new()
            .mock_call(callee, Ok(vec![9, 8, 7]))
            .build();

        let result = host.delegate_call_evm(CallFlags::empty(), &callee, 0, &[], None);
        assert!(result.is_ok());
        assert_eq!(host.return_data_size(), 3);
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
            .balance_raw(bal)
            .balance_of_raw(addr, addr_bal)
            .build();

        let mut output = [0u8; 32];
        host.balance(&mut output);
        assert_eq!(output, bal);

        let mut output2 = [0u8; 32];
        host.balance_of(&addr, &mut output2);
        assert_eq!(output2, addr_bal);

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

        let host = MockHostBuilder::new()
            .chain_id_raw(cid)
            .base_fee_raw(fee)
            .build();

        let mut output = [0u8; 32];
        host.chain_id(&mut output);
        assert_eq!(output, cid);

        let mut output2 = [0u8; 32];
        host.base_fee(&mut output2);
        assert_eq!(output2, fee);
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

    /// `Env::has_code` is derived from the `code_size` syscall rather than being
    /// a syscall of its own, so this pins that the derivation reaches the seeded
    /// backend through the `Host` wrapper and reports `false` — not a panic or a
    /// stale `true` — for an account nobody seeded.
    #[test]
    fn env_has_code_follows_seeded_code_size() {
        use crate::host::Host;
        use std::rc::Rc;

        let contract = [0xAA; 20];
        let eoa = [0xBB; 20];
        let mock = MockHostBuilder::new().code_size(contract, 1234).build();
        assert_eq!(mock.code_size(&contract), 1234);
        assert_eq!(mock.code_size(&eoa), 0);

        let env = Host::from_dyn(Rc::new(mock)).env();
        assert!(env.has_code(contract.into()));
        assert!(!env.has_code(eoa.into()));
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

    #[test]
    fn clone_shares_state() {
        // The test keeps one handle, the contract gets a clone via
        // `Host::from_dyn(Box::new(mock.clone()))`. Both must observe the
        // same storage/events/return-data.
        let host = MockHostBuilder::new().build();
        let clone = host.clone();
        clone.set_storage(StorageFlags::empty(), &[1u8; 32], &[42u8; 32]);

        assert_eq!(
            host.get_raw_storage(&[1u8; 32]),
            Some(vec![42u8; 32]),
            "clone writes must be visible through the original handle"
        );

        host.deposit_event(&[[0u8; 32]], &[9, 9, 9]);
        assert_eq!(clone.events().len(), 1);
    }

    #[test]
    fn mock_call_can_be_configured_after_build() {
        // `mock_call` is `&self`, so handles obtained from `build()` (and
        // any clones) can still register mock returns.
        let callee = [0xBB; 20];
        let host = MockHostBuilder::new().build();
        host.mock_call(callee, Ok(vec![7, 7, 7, 7]));

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
        assert_eq!(&buf[..4], &[7, 7, 7, 7]);
    }

    #[test]
    fn value_transferred_roundtrip() {
        let mut val = [0u8; 32];
        val[31] = 77;

        let host = MockHostBuilder::new().value_transferred_raw(val).build();

        let mut output = [0u8; 32];
        host.value_transferred(&mut output);
        assert_eq!(output, val);
    }

    #[test]
    fn get_storage_or_zero_returns_zeros_for_missing_key() {
        let host = MockHostBuilder::new().build();
        let key = [0xAA; 32];

        let mut output = [0xFFu8; 32];
        host.get_storage_or_zero(StorageFlags::empty(), &key, &mut output);
        assert_eq!(output, [0u8; 32]);

        host.set_storage(StorageFlags::empty(), &key, &[42u8; 32]);
        let mut output2 = [0u8; 32];
        host.get_storage_or_zero(StorageFlags::empty(), &key, &mut output2);
        assert_eq!(output2, [42u8; 32]);
    }

    #[test]
    fn block_author_and_block_number_and_block_hash() {
        let mut bn = [0u8; 32];
        bn[31] = 99;

        // Seeds raw bytes deliberately: this is the passthrough pin for
        // `block_number_raw`, asserting the mock hands back exactly what it was
        // given rather than the little-endian encoding the typed setter applies.
        // Byte 31 is arbitrary here — nothing decodes these bytes as a number.
        let host = MockHostBuilder::new()
            .block_author([0xBB; 20])
            .block_number_raw(bn)
            .build();

        let mut author = [0u8; 20];
        host.block_author(&mut author);
        assert_eq!(author, [0xBB; 20]);

        let mut output = [0u8; 32];
        host.block_number(&mut output);
        assert_eq!(output, bn);

        // `MockHost::block_hash` ignores its `block_number` argument and always
        // zero-fills, so this pins that default, not a lookup. The real host
        // reads that argument little-endian (`read_u256`); if the mock ever
        // grows a seedable block-hash map, the key must be the LE-decoded
        // number so a big-endian caller observably misses.
        let mut hash = [0xFFu8; 32];
        host.block_hash(&bn, &mut hash);
        assert_eq!(hash, [0u8; 32]);
    }

    #[test]
    fn mock_terminate_captures_beneficiary() {
        let host = MockHostBuilder::new().build();
        let halt = host.run_until_halt(|| host.terminate(&[0xAB; 20]));
        assert_eq!(
            halt,
            Some(Halt::Terminate {
                beneficiary: [0xAB; 20]
            })
        );
    }

    #[test]
    fn mock_consume_all_gas_captured() {
        let host = MockHostBuilder::new().build();
        let halt = host.run_until_halt(|| host.consume_all_gas());
        assert_eq!(halt, Some(Halt::ConsumeAllGas));
    }

    #[test]
    fn run_until_halt_returns_none_when_closure_completes() {
        let host = MockHostBuilder::new().build();
        let halt = host.run_until_halt(|| {
            // No halt call — closure completes normally.
            let _ = host.events();
        });
        assert_eq!(halt, None);
    }

    #[test]
    fn run_until_halt_preserves_state_written_before_terminate() {
        let host = MockHostBuilder::new().build();
        let key = [7u8; 32];
        let value = [42u8; 32];

        let halt = host.run_until_halt(|| {
            host.set_storage(StorageFlags::empty(), &key, &value);
            host.terminate(&[0xCD; 20]);
        });

        assert_eq!(
            halt,
            Some(Halt::Terminate {
                beneficiary: [0xCD; 20]
            })
        );
        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        let result = host.get_storage(StorageFlags::empty(), &key, &mut out);
        assert!(result.is_ok());
        assert_eq!(buf, value);
    }

    #[test]
    fn run_until_halt_rethrows_non_halt_panic() {
        let host = MockHostBuilder::new().build();
        // Suppress the default panic hook so the expected non-halt panic
        // doesn't pollute test output. Restore it after.
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let outer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            host.run_until_halt(|| panic!("real bug"));
        }));
        std::panic::set_hook(original_hook);
        assert!(
            outer.is_err(),
            "non-halt panic must propagate out of run_until_halt"
        );
    }

    #[test]
    fn expect_panic_decodes_panic_code() {
        let mock = MockHostBuilder::new().build();
        let host = crate::Host::from_dyn(std::rc::Rc::new(mock.clone()));
        let got = mock.expect_panic(|| crate::panic_revert(&host, crate::Panic::Overflow));
        assert_eq!(got, crate::Panic::Overflow);
    }

    #[test]
    fn expect_panic_decodes_generic_0x00() {
        // The `#[contract]` panic handler emits `Panic::Generic` (0x00) for
        // uncaught Rust panics, but that handler is riscv-only — this pins the
        // 0x00 wire byte on a host target without needing the handler.
        let mock = MockHostBuilder::new().build();
        let host = crate::Host::from_dyn(std::rc::Rc::new(mock.clone()));
        let got = mock.expect_panic(|| crate::panic_revert(&host, crate::Panic::Generic));
        assert_eq!(got, crate::Panic::Generic);
    }

    #[test]
    fn expect_revert_returns_abi_bytes() {
        let mock = MockHostBuilder::new().build();
        let host = crate::Host::from_dyn(std::rc::Rc::new(mock.clone()));
        let rv = mock.expect_revert(|| crate::panic_revert(&host, crate::Panic::OutOfBoundsAccess));
        assert!(rv.flags.contains(ReturnFlags::REVERT));
        // Panic(uint256) selector + code byte.
        assert_eq!(&rv.data[0..4], &[0x4e, 0x48, 0x7b, 0x71]);
        assert_eq!(rv.data[35], 0x32);
        assert_eq!(rv.data.len(), 36);
    }

    #[test]
    #[should_panic(expected = "expected a revert")]
    fn expect_revert_panics_when_closure_returns_normally() {
        let mock = MockHostBuilder::new().build();
        mock.expect_revert(|| { /* no revert */ });
    }

    #[test]
    fn revert_does_not_roll_back_state_by_default() {
        // MockHost is a flat store: writes made before a
        // revert persist. The revert data is still observable via expect_panic.
        let mock = MockHostBuilder::new().build();
        let host = crate::Host::from_dyn(std::rc::Rc::new(mock.clone()));
        let key = [7u8; 32];

        let panic = mock.expect_panic(|| {
            host.set_storage(StorageFlags::empty(), &key, &[1u8; 32]);
            crate::panic_revert(&host, crate::Panic::OutOfBoundsAccess);
        });
        assert_eq!(panic, crate::Panic::OutOfBoundsAccess);
        // Not rolled back automatically:
        assert_eq!(mock.get_raw_storage(&key), Some(vec![1u8; 32]));
    }

    #[test]
    fn snapshot_restore_models_atomic_revert() {
        // The explicit pattern for asserting a revert left no trace.
        let mock = MockHostBuilder::new().build();
        let host = crate::Host::from_dyn(std::rc::Rc::new(mock.clone()));
        let key = [7u8; 32];
        mock.set_raw_storage(key.to_vec(), vec![0xaa; 32]);

        let before = mock.snapshot();
        mock.expect_panic(|| {
            host.set_storage(StorageFlags::empty(), &key, &[1u8; 32]);
            host.deposit_event(&[[9u8; 32]], &[1, 2, 3]);
            crate::panic_revert(&host, crate::Panic::OutOfBoundsAccess);
        });
        mock.restore(before);

        // Storage/events are back to the pre-call snapshot.
        assert_eq!(mock.get_raw_storage(&key), Some(vec![0xaa; 32]));
        assert!(mock.events().is_empty());
    }

    /// Pins the **decode** side of the byte-order contract: hand-written
    /// little-endian bytes in, typed `Env` values out.
    ///
    /// The seeds go through the `_raw` setters on purpose. Seeding via the typed
    /// setters would make this a round-trip, which still passes if the setter
    /// and `Env` are *both* big-endian — so one side has to be raw bytes for the
    /// test to pin anything. See `typed_setters_encode_little_endian` for the
    /// encode side.
    ///
    /// Values are multi-byte (`0x0102`, not `1`) so a byte-order flip actually
    /// changes the decoded result.
    #[test]
    fn env_accessors_via_host() {
        use crate::host::Host;
        use std::rc::Rc;

        // 0x0102 = 258, little-endian.
        let le = |lo, hi| {
            let mut b = [0u8; 32];
            b[0] = lo;
            b[1] = hi;
            b
        };

        // The three address accessors get distinct fills, so an accessor wired
        // to the wrong host function fails rather than reading a value that
        // happens to match.
        let mock = MockHostBuilder::new()
            .caller([0xAA; 20])
            .origin([0xBB; 20])
            .address([0xCC; 20])
            .block_number_raw(le(0x02, 0x01))
            .block_timestamp_raw(le(0x04, 0x03))
            .value_transferred_raw(le(0x06, 0x05))
            .chain_id_raw(le(0x08, 0x07))
            .balance_raw(le(0x0A, 0x09))
            .balance_of_raw([0xDD; 20], le(0x0C, 0x0B))
            .base_fee_raw(le(0x0E, 0x0D))
            .build();
        let host = Host::from_dyn(Rc::new(mock));
        let env = host.env();

        assert_eq!(env.caller().0, [0xAA; 20]);
        assert_eq!(env.origin().0, [0xBB; 20]);
        assert_eq!(env.address().0, [0xCC; 20]);
        assert_eq!(env.block_number(), 0x0102);
        assert_eq!(env.timestamp(), 0x0304);
        assert_eq!(env.value(), U256::from(0x0506u64));
        assert_eq!(env.chain_id(), 0x0708);
        assert_eq!(env.balance(), U256::from(0x090Au64));
        assert_eq!(
            env.balance_of(crate::Address([0xDD; 20])),
            U256::from(0x0B0Cu64)
        );
        assert_eq!(env.base_fee(), U256::from(0x0D0Eu64));
        // An unseeded account reads zero rather than inheriting a neighbour's
        // seed — otherwise the assertions above would prove nothing about which
        // account was asked for.
        assert_eq!(env.balance_of(crate::Address([0xEE; 20])), U256::ZERO);
    }

    /// On chain `balance()` and `balance_of(address(this))` are one query
    /// (pallet-revive routes both through `account_balance`), so the mock must
    /// not let them disagree — a contract asserted against a mock where only
    /// one was seeded would pass on a read the real host answers differently.
    ///
    /// Both seeding directions alias, and an explicit per-account seed wins in
    /// either call order.
    #[test]
    fn own_balance_and_balance_of_self_agree() {
        use crate::host::Host;

        let me = crate::Address([0xCC; 20]);

        let seeded_via_balance = MockHostBuilder::new()
            .address(me.0)
            .balance(U256::from(7u64))
            .build();
        let env = Host::from_dyn(Rc::new(seeded_via_balance)).env();
        assert_eq!(env.balance(), U256::from(7u64));
        assert_eq!(env.balance_of(me), U256::from(7u64));

        let seeded_via_balance_of = MockHostBuilder::new()
            .address(me.0)
            .balance_of(me.0, U256::from(9u64))
            .build();
        let env = Host::from_dyn(Rc::new(seeded_via_balance_of)).env();
        assert_eq!(env.balance(), U256::from(9u64));
        assert_eq!(env.balance_of(me), U256::from(9u64));

        // Conflicting seeds resolve to the explicit one, whichever came first.
        for host in [
            MockHostBuilder::new()
                .address(me.0)
                .balance(U256::from(7u64))
                .balance_of(me.0, U256::from(9u64))
                .build(),
            MockHostBuilder::new()
                .address(me.0)
                .balance_of(me.0, U256::from(9u64))
                .balance(U256::from(7u64))
                .build(),
        ] {
            let env = Host::from_dyn(Rc::new(host)).env();
            assert_eq!(env.balance(), U256::from(9u64));
            assert_eq!(env.balance_of(me), U256::from(9u64));
        }
    }

    /// Pins the **encode** side: typed setters must write little-endian, because
    /// that is what pallet-revive's own host functions write and therefore what
    /// `Env` (and any raw `self.host()` reader) decodes.
    ///
    /// Expected bytes are built by hand rather than via `to_le_bytes`, so a
    /// flipped implementation cannot flip the expectation with it. Paired with
    /// `env_accessors_via_host`, which pins the decode side, this covers both
    /// halves of the byte-order contract for all seven little-endian outputs.
    #[test]
    fn typed_setters_encode_little_endian() {
        let le = |lo, hi| {
            let mut b = [0u8; 32];
            b[0] = lo;
            b[1] = hi;
            b
        };
        let addr = [0xAA; 20];

        let host = MockHostBuilder::new()
            .balance(U256::from(0x0102u64))
            .balance_of(addr, U256::from(0x0304u64))
            .base_fee(U256::from(0x0506u64))
            .chain_id(0x0708)
            .block_number(0x090A)
            .block_timestamp(0x0B0C)
            .value_transferred(U256::from(0x0D0Eu64))
            .build();

        let read = |f: &dyn Fn(&mut [u8; 32])| {
            let mut b = [0u8; 32];
            f(&mut b);
            b
        };

        assert_eq!(read(&|b| host.balance(b)), le(0x02, 0x01));
        assert_eq!(read(&|b| host.balance_of(&addr, b)), le(0x04, 0x03));
        assert_eq!(read(&|b| host.base_fee(b)), le(0x06, 0x05));
        assert_eq!(read(&|b| host.chain_id(b)), le(0x08, 0x07));
        assert_eq!(read(&|b| host.block_number(b)), le(0x0A, 0x09));
        assert_eq!(read(&|b| host.now(b)), le(0x0C, 0x0B));
        assert_eq!(read(&|b| host.value_transferred(b)), le(0x0E, 0x0D));
    }

    /// The three `u64` accessors read the low limb and ignore the high 24 bytes.
    /// A conforming host leaves those zero — guaranteed for the timestamp (a
    /// millisecond moment) and the chain ID (`ChainId: Get<u64>`), and by runtime
    /// convention for the block number (`BlockNumberFor<T>` is only bounded
    /// `AtLeast32Bit`). Either way there is no range check, so an over-wide
    /// `_raw` seed narrows silently rather than panicking.
    ///
    /// Only the `_raw` setters can express that state; the typed ones take `u64`.
    #[test]
    fn env_u64_accessors_narrow_to_low_limb() {
        use crate::host::Host;
        use std::rc::Rc;

        // Low limb = 1, plus a high byte a conforming host would never set.
        let mut b = [0u8; 32];
        b[0] = 1;
        b[31] = 0xFF;

        // One (seed, accessor) pair per case, so a failure names the accessor.
        type Accessor = fn(&crate::Env) -> u64;
        let cases: [(MockHost, Accessor); 3] = [
            (MockHostBuilder::new().block_number_raw(b).build(), |env| {
                env.block_number()
            }),
            (
                MockHostBuilder::new().block_timestamp_raw(b).build(),
                |env| env.timestamp(),
            ),
            (MockHostBuilder::new().chain_id_raw(b).build(), |env| {
                env.chain_id()
            }),
        ];
        for (i, (seed, accessor)) in cases.into_iter().enumerate() {
            let env = Host::from_dyn(Rc::new(seed)).env();
            assert_eq!(accessor(&env), 1, "case {i}: must read the low limb");
        }
    }

    /// The idiom contract authors will actually use: typed setters in, typed
    /// `Env` reads out. Redundant with the two pins above by construction, but
    /// it is the test that fails with a readable message if they disagree.
    #[test]
    fn typed_setters_roundtrip_through_env() {
        use crate::host::Host;
        use std::rc::Rc;

        let mock = MockHostBuilder::new()
            .block_number(258)
            .block_timestamp(1_700_000_000)
            .value_transferred(U256::from(10u64).pow(U256::from(18u64)))
            .chain_id(420)
            .build();
        let env = Host::from_dyn(Rc::new(mock)).env();

        assert_eq!(env.block_number(), 258);
        assert_eq!(env.timestamp(), 1_700_000_000);
        assert_eq!(env.value(), U256::from(10u64).pow(U256::from(18u64)));
        assert_eq!(env.chain_id(), 420);
    }

    /// `env()` is a provided method on `ContractContext`, so it must be
    /// reachable both on a concrete `Context` (the DSL handler spelling) and
    /// through the `&impl ContractContext` bound that cross-contract call
    /// builders impose — the latter is the case an inherent method on `Context`
    /// would have missed.
    #[test]
    fn context_reads_env_directly_and_through_the_bound() {
        use crate::host::{Context, ContractContext, Host};
        use std::rc::Rc;

        // The shape a DSL helper takes once it accepts a typed-call context.
        fn read_through_bound(cx: &impl ContractContext) -> (crate::Address, U256) {
            (cx.env().caller(), cx.env().value())
        }

        let mock = MockHostBuilder::new()
            .caller([0xAA; 20])
            .value_transferred(U256::from(7u64))
            .build();
        let cx = Context::new(Host::from_dyn(Rc::new(mock)));

        assert_eq!(cx.env().caller().0, [0xAA; 20]);
        assert_eq!(cx.env().value(), U256::from(7u64));
        assert_eq!(
            read_through_bound(&cx),
            (crate::Address([0xAA; 20]), U256::from(7u64))
        );
    }
}

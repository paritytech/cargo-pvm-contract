#![doc = include_str!("../builder-dsl.md")]
#![no_std]

pub use pallet_revive_uapi;
pub use pallet_revive_uapi::solidity_selector;
pub use polkavm_derive;
pub use polkavm_derive::polkavm_export;
pub use pvm_contract_types;
pub use ruint;

use pvm_contract_types::{Host, HostApi};

/// 4-byte Solidity function selector.
pub type Selector = [u8; 4];

/// Revert the `deploy` entry point if any value was attached.
///
/// Solidity's default constructor is non-payable, and the `#[contract]` macro
/// path auto-injects an equivalent guard. The DSL has no codegen step, so
/// scaffolded `deploy()` functions must call this explicitly. Omit the call
/// only when the constructor is intentionally payable.
#[inline(always)]
pub fn assert_non_payable_deploy(host: &Host) {
    if pvm_contract_types::value_transferred_is_nonzero(host) {
        host.revert(&pvm_contract_types::framework_errors::NON_PAYABLE_VALUE_RECEIVED);
    }
}

/// The result a [`MethodHandler`] returns to the dispatcher.
///
/// `Ok(n)` — success; `n` bytes were written to the caller-supplied output buffer.
/// `Revert(n)` — revert with the `n` bytes written to the output buffer.
///
/// `n` **must** be `<= output.len()`. The dispatcher clamps to the buffer size
/// as a defensive measure, but handlers should slice the output buffer
/// explicitly (`output[..len]`) to avoid Rust's bounds-check panic when
/// encoding into it.
///
/// Using an enum of `usize` instead of a `Vec` keeps the DSL fully `no_std` /
/// no-alloc — the output buffer is owned by the dispatcher's stack frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlerResult {
    Ok(usize),
    Revert(usize),
}

/// A method handler.
///
/// Writes its encoded ABI output into `output`, returning how many bytes were
/// written and whether the call reverted. Does **not** call `return_value` —
/// the dispatcher owns the transition to the runtime.
///
/// **Invariants**:
/// - Handlers must not write past `output.len()` (Rust panics otherwise).
/// - The returned `HandlerResult::Ok(n)` / `Revert(n)` must satisfy
///   `n <= output.len()`; the dispatcher clamps but will not re-read the
///   buffer past `n` bytes.
pub type MethodHandler = fn(host: &Host, input: &[u8], output: &mut [u8]) -> HandlerResult;

/// Maximum number of methods a single contract can register.
const MAX_METHODS: usize = 16;

#[inline(always)]
fn noop_handler(_host: &Host, _input: &[u8], _output: &mut [u8]) -> HandlerResult {
    HandlerResult::Ok(0)
}

/// Pure Rust builder for PVM smart contract dispatch.
///
/// Handlers take a concrete `&Host`; on riscv64 `Host` is a zero-sized wrapper
/// around `PolkaVmHost`, so production builds pay no indirection. In native
/// unit tests `Host` wraps `Rc<dyn HostApi>` (via [`Host::from_dyn`]) so a
/// `MockHost` can back the same handlers.
///
/// Methods registered via [`method`](Self::method) are non-payable: the
/// dispatcher reverts with
/// [`pvm_contract_types::framework_errors::NON_PAYABLE_VALUE_RECEIVED`] when
/// called with a non-zero value transfer. Methods registered via
/// [`payable_method`](Self::payable_method) accept any value.
///
/// # Example
///
/// ```ignore
/// use pvm_contract_builder_dsl::{ContractBuilder, HandlerResult, solidity_selector};
/// use pvm_contract_types::Host;
///
/// const FIB: [u8; 4] = solidity_selector("fibonacci(uint32)");
///
/// fn fibonacci(_host: &Host, _input: &[u8], _output: &mut [u8]) -> HandlerResult {
///     // decode n, compute fib(n), encode into output[..32]
///     HandlerResult::Ok(32)
/// }
///
/// #[cfg(target_arch = "riscv64")]
/// pub extern "C" fn call() {
///     let host = Host::new();
///     ContractBuilder::new()
///         .method(FIB, fibonacci)
///         .dispatch_impl::<256>(&host);
/// }
/// ```
pub struct ContractBuilder {
    methods: [(Selector, MethodHandler); MAX_METHODS],
    len: usize,
    payable_bits: u64,
}

impl Default for ContractBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ContractBuilder {
    /// Create a new empty contract builder.
    #[inline(always)]
    pub fn new() -> Self {
        Self {
            methods: [([0; 4], noop_handler as MethodHandler); MAX_METHODS],
            len: 0,
            payable_bits: 0,
        }
    }

    /// Panic if `selector` is already registered *on this builder*. The DSL
    /// builds its dispatch table at runtime, so it has no compile-time analog of
    /// the macro path's selector-collision check; a duplicate registration is a
    /// runtime panic instead. This guards only within a single builder — clashes
    /// *across* chained extensions (`dispatch_composed`) are not caught here;
    /// they are resolved by first-match-wins.
    #[inline(always)]
    fn assert_unique_selector(&self, selector: Selector) {
        assert!(
            !self.methods[..self.len].iter().any(|(s, _)| *s == selector),
            "ContractBuilder: duplicate selector registration"
        );
    }

    /// Register a non-payable method handler for the given selector.
    ///
    /// Rejects calls carrying a non-zero value transfer at the dispatch
    /// boundary; the handler itself is not called in that case.
    ///
    /// # Panics
    ///
    /// Panics if more than MAX_METHODS methods are registered, or if `selector`
    /// is already registered (duplicate-selector guard).
    #[inline]
    pub fn method(mut self, selector: Selector, handler: MethodHandler) -> Self {
        assert!(
            self.len < MAX_METHODS,
            "ContractBuilder: exceeded MAX_METHODS ({})",
            MAX_METHODS
        );
        self.assert_unique_selector(selector);
        self.methods[self.len] = (selector, handler);
        self.len += 1;
        self
    }

    /// Register a payable method handler for the given selector.
    ///
    /// Unlike [`method`](Self::method), payable handlers accept calls carrying
    /// any value transfer (including zero).
    ///
    /// # Panics
    ///
    /// Panics if more than MAX_METHODS methods are registered, or if `selector`
    /// is already registered (duplicate-selector guard).
    #[inline]
    pub fn payable_method(mut self, selector: Selector, handler: MethodHandler) -> Self {
        assert!(
            self.len < MAX_METHODS,
            "ContractBuilder: exceeded MAX_METHODS ({})",
            MAX_METHODS
        );
        self.assert_unique_selector(selector);
        self.methods[self.len] = (selector, handler);
        self.payable_bits |= 1u64 << self.len;
        self.len += 1;
        self
    }

    /// Attach a non-payable fallback handler, transitioning to
    /// [`ContractBuilderWithHandlers`]. See the type's docs for semantics.
    pub fn fallback(self, handler: MethodHandler) -> ContractBuilderWithHandlers {
        ContractBuilderWithHandlers {
            inner: self,
            fallback: Some(handler),
            fallback_is_payable: false,
            receive: None,
        }
    }

    /// Attach a payable fallback handler, transitioning to
    /// [`ContractBuilderWithHandlers`]. See the type's docs for semantics.
    pub fn payable_fallback(self, handler: MethodHandler) -> ContractBuilderWithHandlers {
        ContractBuilderWithHandlers {
            inner: self,
            fallback: Some(handler),
            fallback_is_payable: true,
            receive: None,
        }
    }

    /// Attach a `#[receive]`-equivalent handler, transitioning to
    /// [`ContractBuilderWithHandlers`]. See the type's docs for semantics.
    pub fn receive(self, handler: MethodHandler) -> ContractBuilderWithHandlers {
        ContractBuilderWithHandlers {
            inner: self,
            fallback: None,
            fallback_is_payable: false,
            receive: Some(handler),
        }
    }

    /// Try to route a call by selector.
    ///
    /// When the matched method is non-payable and `value_transferred` is
    /// non-zero, writes the `NON_PAYABLE_VALUE_RECEIVED` selector into
    /// `output[..4]` and returns `HandlerResult::Revert(4)` — `output` must be
    /// at least 4 bytes long. Otherwise calls the handler and returns its
    /// result.
    #[inline(always)]
    pub fn try_route(
        &self,
        host: &Host,
        selector: Selector,
        input: &[u8],
        output: &mut [u8],
    ) -> Option<HandlerResult> {
        let mut i = 0;
        while i < self.len {
            let (sel, handler) = self.methods[i];
            if sel == selector {
                let is_payable = (self.payable_bits >> i) & 1 == 1;
                if !is_payable && pvm_contract_types::value_transferred_is_nonzero(host) {
                    let err = pvm_contract_types::framework_errors::NON_PAYABLE_VALUE_RECEIVED;
                    output[..4].copy_from_slice(&err);
                    return Some(HandlerResult::Revert(4));
                }
                return Some(handler(host, input, output));
            }
            i += 1;
        }
        None
    }

    /// Read calldata from the host, match the selector, and hand the encoded
    /// result to the host: `host.return_value(data)` on success, `host.revert(data)`
    /// on failure (unmatched selector, short calldata, or a handler revert).
    ///
    /// On `riscv64` both are the `pallet_revive_uapi` syscall (`-> !`) and
    /// dispatch terminates the contract. On host targets, `MockHost` captures a
    /// success into a `ReturnValue` and returns control (inspect via
    /// `MockHost::take_return_value()`), while a revert unwinds (assert via
    /// `MockHost::expect_revert`).
    ///
    /// Mirrors the `#[contract]` macro's `route()` shape: same dispatch
    /// architecture across DSL and macro paths, same test ergonomics.
    ///
    /// `#[inline(always)]` keeps the dispatcher tight when called from a
    /// single `extern "C" fn call()` entry point. Force-inline (not just hint)
    /// is required to preserve the cross-crate constant-folding that the
    /// previous `<H: HostApi>` generic gave us "for free" via monomorphization
    /// — generics are always inlined-visible at the call site, but a plain
    /// `#[inline]` non-generic function is only a hint the inliner may
    /// decline, which produces an indirect-call dispatch and several hundred
    /// extra bytes of bytecode.
    #[inline(always)]
    pub fn dispatch_impl<const BUF_SIZE: usize>(&self, host: &Host) {
        let call_data_len = host.call_data_size() as usize;

        if call_data_len > BUF_SIZE {
            host.revert(&pvm_contract_types::framework_errors::CALLDATA_TOO_LARGE);
        } else {
            let mut calldata = [0u8; BUF_SIZE];
            host.call_data_copy(&mut calldata[..call_data_len], 0);

            if call_data_len < 4 {
                host.revert(&pvm_contract_types::framework_errors::NO_SELECTOR);
            } else {
                let selector: Selector = [calldata[0], calldata[1], calldata[2], calldata[3]];
                let input = &calldata[4..call_data_len];
                let mut output = [0u8; BUF_SIZE];

                if let Some(result) = self.try_route(host, selector, input, &mut output) {
                    finalize_response(host, &mut output, result);
                } else {
                    host.revert(&pvm_contract_types::framework_errors::UNKNOWN_SELECTOR);
                }
            }
        }
    }

    /// Dispatch, trying this builder's methods first and then each extension
    /// builder in order — first match wins.
    ///
    /// This is the builder-DSL analog of the macro's `implements(...)` fold: an
    /// extension exposes its handlers as a `ContractBuilder`, and the composing
    /// contract chains them here. Selector uniqueness *within* a single builder
    /// is enforced at registration ([`method`](Self::method) panics on a
    /// duplicate). *Across* chained builders a selector may legitimately appear
    /// more than once; first match wins, which is how a composing contract
    /// overrides an extension. Unlike the macro fold, that clash is resolved here
    /// at dispatch rather than rejected at compile time, so order the extensions
    /// with precedence in mind.
    ///
    /// Kept as a standalone body rather than sharing one with `dispatch_impl`:
    /// folding the two into a shared helper regressed the common (non-composing)
    /// dispatch path's code size.
    #[inline(always)]
    pub fn dispatch_composed<const BUF_SIZE: usize>(
        &self,
        host: &Host,
        extensions: &[&ContractBuilder],
    ) {
        let call_data_len = host.call_data_size() as usize;

        if call_data_len > BUF_SIZE {
            host.revert(&pvm_contract_types::framework_errors::CALLDATA_TOO_LARGE);
        } else {
            let mut calldata = [0u8; BUF_SIZE];
            host.call_data_copy(&mut calldata[..call_data_len], 0);

            if call_data_len < 4 {
                host.revert(&pvm_contract_types::framework_errors::NO_SELECTOR);
            } else {
                let selector: Selector = [calldata[0], calldata[1], calldata[2], calldata[3]];
                let input = &calldata[4..call_data_len];
                let mut output = [0u8; BUF_SIZE];

                let result = self
                    .try_route(host, selector, input, &mut output)
                    .or_else(|| {
                        extensions
                            .iter()
                            .find_map(|ext| ext.try_route(host, selector, input, &mut output))
                    });
                if let Some(result) = result {
                    finalize_response(host, &mut output, result);
                } else {
                    host.revert(&pvm_contract_types::framework_errors::UNKNOWN_SELECTOR);
                }
            }
        }
    }
}

/// `ContractBuilder` extended with a [`#[fallback]`]-equivalent and/or
/// [`#[receive]`]-equivalent handler.
///
/// Reached only by calling [`ContractBuilder::fallback`],
/// [`ContractBuilder::payable_fallback`], or [`ContractBuilder::receive`].
/// Contracts that never call any of these keep the original
/// `ContractBuilder` type and the original (smaller) dispatch path — no
/// bytecode cost for users who don't want these features.
///
/// # Semantics
///
/// - **receive** fires on empty calldata (`call_data_size() == 0`).
///   Implicitly payable. Handler's `input` slice is always empty.
/// - **fallback** fires on 1..=3 byte calldata (after receive has been
///   considered) or on a selector that didn't match any registered method.
///   It receives the full incoming calldata. Non-payable by default; use
///   [`payable_fallback`](ContractBuilder::payable_fallback) to accept value.
/// - Without a fallback registered, the unmatched-selector path still
///   reverts with `NO_SELECTOR` / `UNKNOWN_SELECTOR` as before.
pub struct ContractBuilderWithHandlers {
    inner: ContractBuilder,
    /// `None` keeps the original "revert with `UNKNOWN_SELECTOR` /
    /// `NO_SELECTOR`" behaviour on unmatched selectors.
    fallback: Option<MethodHandler>,
    /// Ignored when `fallback` is `None`.
    fallback_is_payable: bool,
    /// Implicitly payable; mirrors Solidity's `receive() external payable`.
    receive: Option<MethodHandler>,
}

impl ContractBuilderWithHandlers {
    /// Forward a non-payable method to the inner [`ContractBuilder`].
    pub fn method(mut self, selector: Selector, handler: MethodHandler) -> Self {
        self.inner = self.inner.method(selector, handler);
        self
    }

    /// Forward a payable method to the inner [`ContractBuilder`].
    pub fn payable_method(mut self, selector: Selector, handler: MethodHandler) -> Self {
        self.inner = self.inner.payable_method(selector, handler);
        self
    }

    /// Set (or replace) the non-payable fallback handler.
    pub fn fallback(mut self, handler: MethodHandler) -> Self {
        self.fallback = Some(handler);
        self.fallback_is_payable = false;
        self
    }

    /// Set (or replace) the payable fallback handler.
    pub fn payable_fallback(mut self, handler: MethodHandler) -> Self {
        self.fallback = Some(handler);
        self.fallback_is_payable = true;
        self
    }

    /// Set (or replace) the receive handler.
    pub fn receive(mut self, handler: MethodHandler) -> Self {
        self.receive = Some(handler);
        self
    }

    /// Dispatch entry point — semantics described on
    /// [`ContractBuilderWithHandlers`].
    #[inline]
    pub fn dispatch_impl<const BUF_SIZE: usize>(&self, host: &Host) {
        let call_data_len = host.call_data_size() as usize;

        if call_data_len > BUF_SIZE {
            host.revert(&pvm_contract_types::framework_errors::CALLDATA_TOO_LARGE);
        }

        let mut calldata = [0u8; BUF_SIZE];
        host.call_data_copy(&mut calldata[..call_data_len], 0);
        let mut output = [0u8; BUF_SIZE];

        if call_data_len == 0
            && let Some(receive) = self.receive
        {
            let result = receive(host, &[], &mut output);
            finalize_response(host, &mut output, result);
            return;
        }

        let default_err = if call_data_len < 4 {
            &pvm_contract_types::framework_errors::NO_SELECTOR
        } else {
            let selector: Selector = [calldata[0], calldata[1], calldata[2], calldata[3]];
            let input = &calldata[4..call_data_len];
            if let Some(result) = self.inner.try_route(host, selector, input, &mut output) {
                finalize_response(host, &mut output, result);
                return;
            }
            &pvm_contract_types::framework_errors::UNKNOWN_SELECTOR
        };

        if let Some(handler) = self.fallback {
            if !self.fallback_is_payable && pvm_contract_types::value_transferred_is_nonzero(host) {
                output[..4].copy_from_slice(
                    &pvm_contract_types::framework_errors::NON_PAYABLE_VALUE_RECEIVED,
                );
                host.revert(&output[..4]);
            }
            let result = handler(host, &calldata[..call_data_len], &mut output);
            finalize_response(host, &mut output, result);
            return;
        }

        host.revert(default_err);
    }

    /// Like [`dispatch_impl`](Self::dispatch_impl) but also chains each extension
    /// builder after this contract's own methods (first match wins) before the
    /// fallback — the with-handlers analog of
    /// [`ContractBuilder::dispatch_composed`]. Lets a DSL contract compose
    /// extensions *and* keep a `fallback`/`receive` handler, matching the macro
    /// path's `implements(...)` + `#[fallback]`/`#[receive]`.
    ///
    /// Kept as a standalone body rather than sharing one with `dispatch_impl`:
    /// folding the two into a shared helper regressed the common (non-composing)
    /// dispatch path's code size.
    #[inline]
    pub fn dispatch_composed<const BUF_SIZE: usize>(
        &self,
        host: &Host,
        extensions: &[&ContractBuilder],
    ) {
        let call_data_len = host.call_data_size() as usize;

        if call_data_len > BUF_SIZE {
            host.revert(&pvm_contract_types::framework_errors::CALLDATA_TOO_LARGE);
        }

        let mut calldata = [0u8; BUF_SIZE];
        host.call_data_copy(&mut calldata[..call_data_len], 0);
        let mut output = [0u8; BUF_SIZE];

        if call_data_len == 0
            && let Some(receive) = self.receive
        {
            let result = receive(host, &[], &mut output);
            finalize_response(host, &mut output, result);
            return;
        }

        let default_err = if call_data_len < 4 {
            &pvm_contract_types::framework_errors::NO_SELECTOR
        } else {
            let selector: Selector = [calldata[0], calldata[1], calldata[2], calldata[3]];
            let input = &calldata[4..call_data_len];
            if let Some(result) = self
                .inner
                .try_route(host, selector, input, &mut output)
                .or_else(|| {
                    extensions
                        .iter()
                        .find_map(|ext| ext.try_route(host, selector, input, &mut output))
                })
            {
                finalize_response(host, &mut output, result);
                return;
            }
            &pvm_contract_types::framework_errors::UNKNOWN_SELECTOR
        };

        if let Some(handler) = self.fallback {
            if !self.fallback_is_payable && pvm_contract_types::value_transferred_is_nonzero(host) {
                output[..4].copy_from_slice(
                    &pvm_contract_types::framework_errors::NON_PAYABLE_VALUE_RECEIVED,
                );
                host.revert(&output[..4]);
            }
            let result = handler(host, &calldata[..call_data_len], &mut output);
            finalize_response(host, &mut output, result);
            return;
        }

        host.revert(default_err);
    }
}

/// Hand a [`HandlerResult`] to the host: success is lowered through the shared
/// [`pvm_contract_types::finalize_outcome`] success exit — the same door the
/// `#[contract]` macro uses — while a revert diverges directly through
/// [`HostApi::revert`], matching the macro path (all reverts diverge; only the
/// non-reverting result flows as an `Outcome`).
///
/// The length is clamped to the buffer first: a [`MethodHandler`] is a
/// user-supplied `fn` (untrusted), so a bad `n` must not index past `output`.
/// The macro path skips this clamp because its lengths come from trusted codegen.
#[inline(always)]
fn finalize_response(host: &Host, output: &mut [u8], result: HandlerResult) {
    match result {
        HandlerResult::Ok(n) => {
            let outcome = pvm_contract_types::Outcome::Return(n.min(output.len()));
            pvm_contract_types::finalize_outcome(host, outcome, &output);
        }
        HandlerResult::Revert(n) => {
            let len = n.min(output.len());
            host.revert(&output[..len]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pvm_contract_types::{Host, MockHost, U256};

    const DEPOSIT: Selector = [0xde, 0x00, 0x00, 0x01];
    const TRANSFER: Selector = [0x7f, 0x00, 0x00, 0x02];

    fn dummy_handler(_host: &Host, _input: &[u8], _output: &mut [u8]) -> HandlerResult {
        HandlerResult::Ok(0)
    }

    #[test]
    #[should_panic(expected = "MAX_METHODS")]
    fn method_panics_on_overflow() {
        let mut builder = ContractBuilder::new();
        for i in 0..=MAX_METHODS {
            builder = builder.method([i as u8, 0, 0, 0], dummy_handler);
        }
    }

    #[test]
    #[should_panic(expected = "MAX_METHODS")]
    fn payable_method_panics_on_overflow() {
        let mut builder = ContractBuilder::new();
        for i in 0..=MAX_METHODS {
            builder = builder.payable_method([i as u8, 0, 0, 0], dummy_handler);
        }
    }

    #[test]
    fn payable_bit_set_correctly() {
        let builder = ContractBuilder::new()
            .method(TRANSFER, dummy_handler)
            .payable_method(DEPOSIT, dummy_handler);
        assert_eq!(builder.payable_bits, 0b10);
    }

    #[test]
    fn payable_bit_survives_for_high_index() {
        let mut builder = ContractBuilder::new();
        for i in 0..(MAX_METHODS - 1) {
            builder = builder.method([i as u8, 0, 0, 0xaa], dummy_handler);
        }
        builder = builder.payable_method([(MAX_METHODS - 1) as u8, 0, 0, 0xaa], dummy_handler);
        assert_eq!(builder.payable_bits, 1u64 << (MAX_METHODS - 1));
    }

    #[test]
    fn non_payable_contract_has_zero_payable_bits() {
        let builder = ContractBuilder::new()
            .method(TRANSFER, dummy_handler)
            .method(DEPOSIT, dummy_handler);
        assert_eq!(builder.payable_bits, 0);
    }

    // ---------------------------------------------------------------------
    // Dispatch tests for fallback / receive
    // ---------------------------------------------------------------------

    use pvm_contract_types::MockHostBuilder;
    extern crate alloc;
    use alloc::rc::Rc;
    use alloc::vec;

    fn ok_marker_handler(_host: &Host, _input: &[u8], output: &mut [u8]) -> HandlerResult {
        output[..3].copy_from_slice(b"hit");
        HandlerResult::Ok(3)
    }

    /// Wrap a `Rc<MockHost>` into the concrete `Host` the dispatcher takes.
    /// Mirrors the pattern used in `tests/mock_dispatch.rs`.
    fn wrap(mock: &Rc<MockHost>) -> Host {
        Host::from_dyn(mock.clone())
    }

    #[test]
    fn empty_calldata_invokes_receive() {
        let mock = Rc::new(MockHostBuilder::new().build());
        ContractBuilder::new()
            .method(TRANSFER, dummy_handler)
            .receive(ok_marker_handler)
            .dispatch_impl::<256>(&wrap(&mock));
        let rv = mock
            .take_return_value()
            .expect("receive should call return_value");
        assert!(
            rv.flags.is_empty(),
            "receive must not revert: {:?}",
            rv.flags
        );
        assert_eq!(&rv.data[..], b"hit");
    }

    #[test]
    fn empty_calldata_without_receive_or_fallback_reverts_no_selector() {
        let mock = Rc::new(MockHostBuilder::new().build());
        let rv = mock.expect_revert(|| {
            ContractBuilder::new()
                .method(TRANSFER, dummy_handler)
                .dispatch_impl::<256>(&wrap(&mock));
        });
        assert_eq!(
            &rv.data[..],
            &pvm_contract_types::framework_errors::NO_SELECTOR
        );
    }

    #[test]
    fn empty_calldata_without_receive_routes_to_fallback() {
        let mock = Rc::new(MockHostBuilder::new().build());
        ContractBuilder::new()
            .method(TRANSFER, dummy_handler)
            .fallback(ok_marker_handler)
            .dispatch_impl::<256>(&wrap(&mock));
        let rv = mock.take_return_value().unwrap();
        assert!(rv.flags.is_empty());
        assert_eq!(&rv.data[..], b"hit");
    }

    #[test]
    fn unmatched_selector_routes_to_fallback() {
        // Calldata = a selector that doesn't match TRANSFER.
        let mock = Rc::new(
            MockHostBuilder::new()
                .calldata(vec![0xff, 0xff, 0xff, 0xff])
                .build(),
        );
        ContractBuilder::new()
            .method(TRANSFER, dummy_handler)
            .fallback(ok_marker_handler)
            .dispatch_impl::<256>(&wrap(&mock));
        let rv = mock.take_return_value().unwrap();
        assert!(rv.flags.is_empty());
        assert_eq!(&rv.data[..], b"hit");
    }

    #[test]
    fn unmatched_selector_without_fallback_reverts_unknown_selector() {
        let mock = Rc::new(
            MockHostBuilder::new()
                .calldata(vec![0xff, 0xff, 0xff, 0xff])
                .build(),
        );
        let rv = mock.expect_revert(|| {
            ContractBuilder::new()
                .method(TRANSFER, dummy_handler)
                .dispatch_impl::<256>(&wrap(&mock));
        });
        assert_eq!(
            &rv.data[..],
            &pvm_contract_types::framework_errors::UNKNOWN_SELECTOR
        );
    }

    #[test]
    fn non_payable_fallback_rejects_value() {
        let mock = Rc::new(
            MockHostBuilder::new()
                .calldata(vec![0xff, 0xff, 0xff, 0xff])
                .value_transferred(U256::from(1u8))
                .build(),
        );
        let rv = mock.expect_revert(|| {
            ContractBuilder::new()
                .fallback(ok_marker_handler)
                .dispatch_impl::<256>(&wrap(&mock));
        });
        assert_eq!(
            &rv.data[..],
            &pvm_contract_types::framework_errors::NON_PAYABLE_VALUE_RECEIVED
        );
    }

    #[test]
    fn payable_fallback_accepts_value() {
        let mock = Rc::new(
            MockHostBuilder::new()
                .calldata(vec![0xff, 0xff, 0xff, 0xff])
                .value_transferred(U256::from(1u8))
                .build(),
        );
        ContractBuilder::new()
            .payable_fallback(ok_marker_handler)
            .dispatch_impl::<256>(&wrap(&mock));
        let rv = mock.take_return_value().unwrap();
        assert!(rv.flags.is_empty(), "payable fallback must accept value");
        assert_eq!(&rv.data[..], b"hit");
    }

    #[test]
    fn receive_fires_before_fallback_on_empty_calldata() {
        let mock = Rc::new(MockHostBuilder::new().build());
        ContractBuilder::new()
            .receive(ok_marker_handler)
            .fallback(|_, _, output| {
                output[..8].copy_from_slice(b"fallback");
                HandlerResult::Ok(8)
            })
            .dispatch_impl::<256>(&wrap(&mock));
        let rv = mock.take_return_value().unwrap();
        assert_eq!(
            &rv.data[..],
            b"hit",
            "receive must dispatch before fallback on empty calldata"
        );
    }

    #[test]
    fn one_to_three_byte_calldata_routes_to_fallback() {
        let mock = Rc::new(MockHostBuilder::new().calldata(vec![0xab, 0xcd]).build());
        ContractBuilder::new()
            .receive(|_, _, _| panic!("receive must NOT fire for 1..=3 byte calldata"))
            .fallback(ok_marker_handler)
            .dispatch_impl::<256>(&wrap(&mock));
        let rv = mock.take_return_value().unwrap();
        assert!(rv.flags.is_empty());
        assert_eq!(&rv.data[..], b"hit");
    }

    // --- composition (router chaining) + duplicate-selector guard --------

    #[test]
    fn dispatch_composed_falls_through_to_extension() {
        // The main contract only knows DEPOSIT; TRANSFER lives on an extension.
        let mock = Rc::new(MockHostBuilder::new().calldata(TRANSFER.to_vec()).build());
        let extension = ContractBuilder::new().method(TRANSFER, ok_marker_handler);
        let main = ContractBuilder::new().method(DEPOSIT, dummy_handler);

        main.dispatch_composed::<256>(&wrap(&mock), &[&extension]);

        let rv = mock
            .take_return_value()
            .expect("chained extension should handle TRANSFER");
        assert!(rv.flags.is_empty());
        assert_eq!(&rv.data[..], b"hit");
    }

    #[test]
    fn dispatch_composed_prefers_own_method_over_extension() {
        // Both define DEPOSIT; first-match-wins means the main contract's own
        // handler runs (the extension's is shadowed).
        let mock = Rc::new(MockHostBuilder::new().calldata(DEPOSIT.to_vec()).build());
        let extension =
            ContractBuilder::new().method(DEPOSIT, |_, _, _| panic!("extension must not win"));
        let main = ContractBuilder::new().method(DEPOSIT, ok_marker_handler);

        main.dispatch_composed::<256>(&wrap(&mock), &[&extension]);

        let rv = mock.take_return_value().unwrap();
        assert_eq!(&rv.data[..], b"hit");
    }

    #[test]
    fn dispatch_composed_reverts_unknown_selector() {
        let mock = Rc::new(
            MockHostBuilder::new()
                .calldata(vec![0xaa, 0xbb, 0xcc, 0xdd])
                .build(),
        );
        let extension = ContractBuilder::new().method(TRANSFER, ok_marker_handler);
        let main = ContractBuilder::new().method(DEPOSIT, dummy_handler);

        // An unmatched selector across the whole chain reverts (diverges).
        let rv = mock.expect_revert(|| {
            main.dispatch_composed::<256>(&wrap(&mock), &[&extension]);
        });
        assert_eq!(
            &rv.data[..],
            &pvm_contract_types::framework_errors::UNKNOWN_SELECTOR
        );
    }

    #[test]
    #[should_panic(expected = "duplicate selector")]
    fn method_panics_on_duplicate_selector() {
        ContractBuilder::new()
            .method(TRANSFER, dummy_handler)
            .method(TRANSFER, dummy_handler);
    }

    // --- composition + fallback/receive on the with-handlers builder ---------

    #[test]
    fn composed_with_handlers_routes_to_chained_extension() {
        // Selector lives on the extension; the composed contract also has a
        // fallback — the extension must win over the fallback.
        let mock = Rc::new(MockHostBuilder::new().calldata(TRANSFER.to_vec()).build());
        let extension = ContractBuilder::new().method(TRANSFER, ok_marker_handler);
        let main = ContractBuilder::new()
            .method(DEPOSIT, dummy_handler)
            .fallback(|_, _, _| panic!("fallback must not fire when an extension handles it"));

        main.dispatch_composed::<256>(&wrap(&mock), &[&extension]);

        let rv = mock.take_return_value().unwrap();
        assert!(rv.flags.is_empty());
        assert_eq!(&rv.data[..], b"hit");
    }

    #[test]
    fn composed_with_handlers_unmatched_selector_routes_to_fallback() {
        // No builder handles the selector — the composed contract's fallback
        // fires (rather than reverting), proving composition + fallback coexist.
        let mock = Rc::new(
            MockHostBuilder::new()
                .calldata(vec![0xaa, 0xbb, 0xcc, 0xdd])
                .build(),
        );
        let extension = ContractBuilder::new().method(TRANSFER, dummy_handler);
        let main = ContractBuilder::new()
            .method(DEPOSIT, dummy_handler)
            .fallback(ok_marker_handler);

        main.dispatch_composed::<256>(&wrap(&mock), &[&extension]);

        let rv = mock.take_return_value().unwrap();
        assert!(rv.flags.is_empty());
        assert_eq!(&rv.data[..], b"hit");
    }

    #[test]
    fn composed_with_handlers_empty_calldata_hits_receive() {
        let mock = Rc::new(MockHostBuilder::new().build());
        let extension = ContractBuilder::new().method(TRANSFER, dummy_handler);
        let main = ContractBuilder::new()
            .method(DEPOSIT, dummy_handler)
            .receive(ok_marker_handler);

        main.dispatch_composed::<256>(&wrap(&mock), &[&extension]);

        let rv = mock.take_return_value().unwrap();
        assert!(rv.flags.is_empty());
        assert_eq!(&rv.data[..], b"hit");
    }
}

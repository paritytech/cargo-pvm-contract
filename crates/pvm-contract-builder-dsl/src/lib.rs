#![doc = include_str!("../../../specs/builder-dsl.md")]
#![no_std]

pub use pallet_revive_uapi;
pub use pallet_revive_uapi::solidity_selector;
pub use polkavm_derive;
pub use polkavm_derive::polkavm_export;
pub use pvm_contract_types;
pub use ruint;

use pvm_contract_types::{Host, HostApi, ReturnFlags};

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
        host.return_value(
            ReturnFlags::REVERT,
            &pvm_contract_types::framework_errors::NON_PAYABLE_VALUE_RECEIVED,
        );
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

    /// Register a non-payable method handler for the given selector.
    ///
    /// Rejects calls carrying a non-zero value transfer at the dispatch
    /// boundary; the handler itself is not called in that case.
    ///
    /// # Panics
    ///
    /// Panics if more than MAX_METHODS methods are registered.
    #[inline]
    pub fn method(mut self, selector: Selector, handler: MethodHandler) -> Self {
        assert!(
            self.len < MAX_METHODS,
            "ContractBuilder: exceeded MAX_METHODS ({})",
            MAX_METHODS
        );
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
    /// Panics if more than MAX_METHODS methods are registered.
    #[inline]
    pub fn payable_method(mut self, selector: Selector, handler: MethodHandler) -> Self {
        assert!(
            self.len < MAX_METHODS,
            "ContractBuilder: exceeded MAX_METHODS ({})",
            MAX_METHODS
        );
        self.methods[self.len] = (selector, handler);
        self.payable_bits |= 1u64 << self.len;
        self.len += 1;
        self
    }

    /// Attach a non-payable fallback handler, transitioning to
    /// [`ContractBuilderWithHandlers`]. See the type's docs for semantics.
    pub fn fallback(self, handler: MethodHandler<H>) -> ContractBuilderWithHandlers<H> {
        ContractBuilderWithHandlers {
            inner: self,
            fallback: Some(handler),
            fallback_is_payable: false,
            receive: None,
        }
    }

    /// Attach a payable fallback handler, transitioning to
    /// [`ContractBuilderWithHandlers`]. See the type's docs for semantics.
    pub fn payable_fallback(self, handler: MethodHandler<H>) -> ContractBuilderWithHandlers<H> {
        ContractBuilderWithHandlers {
            inner: self,
            fallback: Some(handler),
            fallback_is_payable: true,
            receive: None,
        }
    }

    /// Attach a `#[receive]`-equivalent handler, transitioning to
    /// [`ContractBuilderWithHandlers`]. See the type's docs for semantics.
    pub fn receive(self, handler: MethodHandler<H>) -> ContractBuilderWithHandlers<H> {
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

    /// Read calldata from the host, match the selector, and call
    /// `host.return_value(flags, data)` directly with the encoded result.
    ///
    /// On `riscv64`, `host.return_value` is the `pallet_revive_uapi` syscall
    /// (`-> !`) and dispatch terminates the contract. On host targets, the
    /// `MockHost` implementation captures `(flags, data)` and returns
    /// control — tests inspect the result via
    /// `MockHost::take_return_value()`.
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
            host.return_value(
                ReturnFlags::REVERT,
                &pvm_contract_types::framework_errors::CALLDATA_TOO_LARGE,
            );
        } else {
            let mut calldata = [0u8; BUF_SIZE];
            host.call_data_copy(&mut calldata[..call_data_len], 0);

            if call_data_len < 4 {
                host.return_value(
                    ReturnFlags::REVERT,
                    &pvm_contract_types::framework_errors::NO_SELECTOR,
                );
            } else {
                let selector: Selector = [calldata[0], calldata[1], calldata[2], calldata[3]];
                let input = &calldata[4..call_data_len];
                let mut output = [0u8; BUF_SIZE];

                if let Some(result) = self.try_route(host, selector, input, &mut output) {
                    let (flags, raw_len) = match result {
                        HandlerResult::Ok(n) => (ReturnFlags::empty(), n),
                        HandlerResult::Revert(n) => (ReturnFlags::REVERT, n),
                    };
                    let len = if raw_len > BUF_SIZE {
                        BUF_SIZE
                    } else {
                        raw_len
                    };
                    host.return_value(flags, &output[..len]);
                } else {
                    host.return_value(
                        ReturnFlags::REVERT,
                        &pvm_contract_types::framework_errors::UNKNOWN_SELECTOR,
                    );
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
pub struct ContractBuilderWithHandlers<H: pvm_contract_types::HostApi> {
    inner: ContractBuilder<H>,
    /// `None` keeps the original "revert with `UNKNOWN_SELECTOR` /
    /// `NO_SELECTOR`" behaviour on unmatched selectors.
    fallback: Option<MethodHandler<H>>,
    /// Ignored when `fallback` is `None`.
    fallback_is_payable: bool,
    /// Implicitly payable; mirrors Solidity's `receive() external payable`.
    receive: Option<MethodHandler<H>>,
}

impl<H: pvm_contract_types::HostApi> ContractBuilderWithHandlers<H> {
    /// Forward a non-payable method to the inner [`ContractBuilder`].
    pub fn method(mut self, selector: Selector, handler: MethodHandler<H>) -> Self {
        self.inner = self.inner.method(selector, handler);
        self
    }

    /// Forward a payable method to the inner [`ContractBuilder`].
    pub fn payable_method(mut self, selector: Selector, handler: MethodHandler<H>) -> Self {
        self.inner = self.inner.payable_method(selector, handler);
        self
    }

    /// Set (or replace) the non-payable fallback handler.
    pub fn fallback(mut self, handler: MethodHandler<H>) -> Self {
        self.fallback = Some(handler);
        self.fallback_is_payable = false;
        self
    }

    /// Set (or replace) the payable fallback handler.
    pub fn payable_fallback(mut self, handler: MethodHandler<H>) -> Self {
        self.fallback = Some(handler);
        self.fallback_is_payable = true;
        self
    }

    /// Set (or replace) the receive handler.
    pub fn receive(mut self, handler: MethodHandler<H>) -> Self {
        self.receive = Some(handler);
        self
    }

    /// Dispatch entry point — semantics described on
    /// [`ContractBuilderWithHandlers`].
    #[inline]
    #[allow(unreachable_code)]
    pub fn dispatch_impl<const BUF_SIZE: usize>(&self, host: &H) {
        let call_data_len = host.call_data_size() as usize;

        if call_data_len > BUF_SIZE {
            host.return_value(
                ReturnFlags::REVERT,
                &pvm_contract_types::framework_errors::CALLDATA_TOO_LARGE,
            );
            return;
        }

        let mut calldata = [0u8; BUF_SIZE];
        host.call_data_copy(&mut calldata[..call_data_len], 0);
        let mut output = [0u8; BUF_SIZE];

        if call_data_len == 0
            && let Some(receive) = self.receive
        {
            let result = receive(host, &[], &mut output);
            finalize_response(host, &output, result);
            return;
        }

        let default_err = if call_data_len < 4 {
            &pvm_contract_types::framework_errors::NO_SELECTOR
        } else {
            let selector: Selector = [calldata[0], calldata[1], calldata[2], calldata[3]];
            let input = &calldata[4..call_data_len];
            if let Some(result) = self.inner.try_route(host, selector, input, &mut output) {
                finalize_response(host, &output, result);
                return;
            }
            &pvm_contract_types::framework_errors::UNKNOWN_SELECTOR
        };

        if let Some(handler) = self.fallback {
            if !self.fallback_is_payable && pvm_contract_types::value_transferred_is_nonzero(host) {
                output[..4].copy_from_slice(
                    &pvm_contract_types::framework_errors::NON_PAYABLE_VALUE_RECEIVED,
                );
                host.return_value(ReturnFlags::REVERT, &output[..4]);
                return;
            }
            let result = handler(host, &calldata[..call_data_len], &mut output);
            finalize_response(host, &output, result);
            return;
        }

        host.return_value(ReturnFlags::REVERT, default_err);
    }
}

#[inline(always)]
fn finalize_response<H: pvm_contract_types::HostApi>(
    host: &H,
    output: &[u8],
    result: HandlerResult,
) {
    let (flags, raw_len) = match result {
        HandlerResult::Ok(n) => (ReturnFlags::empty(), n),
        HandlerResult::Revert(n) => (ReturnFlags::REVERT, n),
    };
    let len = if raw_len > output.len() {
        output.len()
    } else {
        raw_len
    };
    host.return_value(flags, &output[..len]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use pvm_contract_types::Host;

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
    use alloc::vec;

    fn ok_marker_handler<H: pvm_contract_types::HostApi>(
        _host: &H,
        _input: &[u8],
        output: &mut [u8],
    ) -> HandlerResult {
        output[..3].copy_from_slice(b"hit");
        HandlerResult::Ok(3)
    }

    #[test]
    fn empty_calldata_invokes_receive() {
        let host = MockHostBuilder::new().build();
        ContractBuilder::<MockHost>::new()
            .method(TRANSFER, dummy_handler::<MockHost>)
            .receive(ok_marker_handler::<MockHost>)
            .dispatch_impl::<256>(&host);
        let rv = host
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
        let host = MockHostBuilder::new().build();
        ContractBuilder::<MockHost>::new()
            .method(TRANSFER, dummy_handler::<MockHost>)
            .dispatch_impl::<256>(&host);
        let rv = host.take_return_value().unwrap();
        assert!(rv.flags.contains(ReturnFlags::REVERT));
        assert_eq!(
            &rv.data[..],
            &pvm_contract_types::framework_errors::NO_SELECTOR
        );
    }

    #[test]
    fn empty_calldata_without_receive_routes_to_fallback() {
        let host = MockHostBuilder::new().build();
        ContractBuilder::<MockHost>::new()
            .method(TRANSFER, dummy_handler::<MockHost>)
            .fallback(ok_marker_handler::<MockHost>)
            .dispatch_impl::<256>(&host);
        let rv = host.take_return_value().unwrap();
        assert!(rv.flags.is_empty());
        assert_eq!(&rv.data[..], b"hit");
    }

    #[test]
    fn unmatched_selector_routes_to_fallback() {
        // Calldata = a selector that doesn't match TRANSFER.
        let host = MockHostBuilder::new()
            .calldata(vec![0xff, 0xff, 0xff, 0xff])
            .build();
        ContractBuilder::<MockHost>::new()
            .method(TRANSFER, dummy_handler::<MockHost>)
            .fallback(ok_marker_handler::<MockHost>)
            .dispatch_impl::<256>(&host);
        let rv = host.take_return_value().unwrap();
        assert!(rv.flags.is_empty());
        assert_eq!(&rv.data[..], b"hit");
    }

    #[test]
    fn unmatched_selector_without_fallback_reverts_unknown_selector() {
        let host = MockHostBuilder::new()
            .calldata(vec![0xff, 0xff, 0xff, 0xff])
            .build();
        ContractBuilder::<MockHost>::new()
            .method(TRANSFER, dummy_handler::<MockHost>)
            .dispatch_impl::<256>(&host);
        let rv = host.take_return_value().unwrap();
        assert!(rv.flags.contains(ReturnFlags::REVERT));
        assert_eq!(
            &rv.data[..],
            &pvm_contract_types::framework_errors::UNKNOWN_SELECTOR
        );
    }

    #[test]
    fn non_payable_fallback_rejects_value() {
        let host = MockHostBuilder::new()
            .calldata(vec![0xff, 0xff, 0xff, 0xff])
            .value_transferred([
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 1,
            ])
            .build();
        ContractBuilder::<MockHost>::new()
            .fallback(ok_marker_handler::<MockHost>)
            .dispatch_impl::<256>(&host);
        let rv = host.take_return_value().unwrap();
        assert!(rv.flags.contains(ReturnFlags::REVERT));
        assert_eq!(
            &rv.data[..],
            &pvm_contract_types::framework_errors::NON_PAYABLE_VALUE_RECEIVED
        );
    }

    #[test]
    fn payable_fallback_accepts_value() {
        let host = MockHostBuilder::new()
            .calldata(vec![0xff, 0xff, 0xff, 0xff])
            .value_transferred([
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 1,
            ])
            .build();
        ContractBuilder::<MockHost>::new()
            .payable_fallback(ok_marker_handler::<MockHost>)
            .dispatch_impl::<256>(&host);
        let rv = host.take_return_value().unwrap();
        assert!(rv.flags.is_empty(), "payable fallback must accept value");
        assert_eq!(&rv.data[..], b"hit");
    }

    #[test]
    fn receive_fires_before_fallback_on_empty_calldata() {
        let host = MockHostBuilder::new().build();
        ContractBuilder::<MockHost>::new()
            .receive(ok_marker_handler::<MockHost>)
            .fallback(|_, _, output| {
                output[..8].copy_from_slice(b"fallback");
                HandlerResult::Ok(8)
            })
            .dispatch_impl::<256>(&host);
        let rv = host.take_return_value().unwrap();
        assert_eq!(
            &rv.data[..],
            b"hit",
            "receive must dispatch before fallback on empty calldata"
        );
    }

    #[test]
    fn one_to_three_byte_calldata_routes_to_fallback() {
        let host = MockHostBuilder::new().calldata(vec![0xab, 0xcd]).build();
        ContractBuilder::<MockHost>::new()
            .receive(|_, _, _| panic!("receive must NOT fire for 1..=3 byte calldata"))
            .fallback(ok_marker_handler::<MockHost>)
            .dispatch_impl::<256>(&host);
        let rv = host.take_return_value().unwrap();
        assert!(rv.flags.is_empty());
        assert_eq!(&rv.data[..], b"hit");
    }
}

#![doc = include_str!("../../../specs/builder-dsl.md")]
#![no_std]

use core::marker::PhantomData;

pub use pallet_revive_uapi;
pub use pallet_revive_uapi::solidity_selector;
pub use polkavm_derive;
pub use polkavm_derive::polkavm_export;
pub use pvm_contract_types;
pub use ruint;

use pvm_contract_types::ReturnFlags;

/// 4-byte Solidity function selector.
pub type Selector = [u8; 4];

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
pub type MethodHandler<H> = fn(host: &H, input: &[u8], output: &mut [u8]) -> HandlerResult;

/// Maximum number of methods a single contract can register.
const MAX_METHODS: usize = 16;

fn noop_handler<H: pvm_contract_types::HostApi>(
    _host: &H,
    _input: &[u8],
    _output: &mut [u8],
) -> HandlerResult {
    HandlerResult::Ok(0)
}

/// Pure Rust builder for PVM smart contract dispatch.
///
/// Generic over the host type so only one monomorphization lands in any given
/// binary. In production that's `PolkaVmHost` (a zero-sized type — the builder plus
/// dispatch loop is byte-equivalent to today's static-call version). In unit
/// tests it's `MockHost`, compiled into the host-target test binary only.
///
/// # Example
///
/// ```ignore
/// use pvm_contract_builder_dsl::{ContractBuilder, HandlerResult, solidity_selector};
/// use pvm_contract_types::{HostApi, PolkaVmHost};
///
/// const FIB: [u8; 4] = solidity_selector("fibonacci(uint32)");
///
/// fn fibonacci<H: HostApi>(_host: &H, input: &[u8], output: &mut [u8]) -> HandlerResult {
///     // decode n, compute fib(n), encode into output[..32]
///     HandlerResult::Ok(32)
/// }
///
/// #[cfg(target_arch = "riscv64")]
/// pub extern "C" fn call() {
///     let host = PolkaVmHost;
///     ContractBuilder::<PolkaVmHost>::new()
///         .method(FIB, fibonacci::<PolkaVmHost>)
///         .dispatch_impl::<256>(&host);
///     // unreachable on riscv64: every dispatch path calls
///     // host.return_value(...) which is `-> !` (the syscall).
/// }
/// ```
pub struct ContractBuilder<H: pvm_contract_types::HostApi> {
    methods: [(Selector, MethodHandler<H>); MAX_METHODS],
    len: usize,
    _marker: PhantomData<fn(&H)>,
}

impl<H: pvm_contract_types::HostApi> Default for ContractBuilder<H> {
    fn default() -> Self {
        Self::new()
    }
}

impl<H: pvm_contract_types::HostApi> ContractBuilder<H> {
    /// Create a new empty contract builder.
    pub fn new() -> Self {
        Self {
            methods: [([0; 4], noop_handler::<H> as MethodHandler<H>); MAX_METHODS],
            len: 0,
            _marker: PhantomData,
        }
    }

    /// Register a method handler for the given selector.
    ///
    /// # Panics
    ///
    /// Panics if more than 16 methods are registered.
    pub fn method(mut self, selector: Selector, handler: MethodHandler<H>) -> Self {
        assert!(
            self.len < MAX_METHODS,
            "ContractBuilder: exceeded MAX_METHODS ({})",
            MAX_METHODS
        );
        self.methods[self.len] = (selector, handler);
        self.len += 1;
        self
    }

    /// Try to route a call by selector without reading calldata.
    ///
    /// Returns `Some((flags, bytes_written))` if a handler matched. The caller
    /// owns `output` and sees the handler's writes in-place.
    #[inline(always)]
    pub fn try_route(
        &self,
        host: &H,
        selector: Selector,
        input: &[u8],
        output: &mut [u8],
    ) -> Option<HandlerResult> {
        let mut i = 0;
        while i < self.len {
            let (sel, handler) = self.methods[i];
            if sel == selector {
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
    /// `#[inline]` keeps the dispatcher tight when called from a single
    /// `extern "C" fn call()` entry point.
    #[inline]
    pub fn dispatch_impl<const BUF_SIZE: usize>(&self, host: &H) {
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
                    // Clamp to BUF_SIZE so a buggy handler returning a bogus
                    // length cannot panic on the slice that follows.
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

#[cfg(test)]
mod tests {
    use super::*;
    use pvm_contract_types::MockHost;

    fn dummy_handler<H: pvm_contract_types::HostApi>(
        _host: &H,
        _input: &[u8],
        _output: &mut [u8],
    ) -> HandlerResult {
        HandlerResult::Ok(0)
    }

    #[test]
    #[should_panic(expected = "MAX_METHODS")]
    fn method_panics_on_overflow() {
        let mut builder = ContractBuilder::<MockHost>::new();
        for i in 0..=MAX_METHODS {
            builder = builder.method([i as u8, 0, 0, 0], dummy_handler::<MockHost>);
        }
    }
}

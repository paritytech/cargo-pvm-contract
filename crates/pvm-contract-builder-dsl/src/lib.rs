#![doc = include_str!("../../../specs/builder-dsl.md")]
#![no_std]

pub use pallet_revive_uapi;
pub use pallet_revive_uapi::solidity_selector;
pub use polkavm_derive;
pub use pvm_contract_types;
pub use ruint;

/// 4-byte Solidity function selector.
pub type Selector = [u8; 4];

/// Fixed-size stack buffer for encoding ABI-compatible revert data.
///
/// Encodes errors via [`pvm_contract_types::SolRevert::revert_data`] and
/// returns a slice of the encoded bytes. Works with both single
/// [`pvm_contract_types::SolError`] types and error enums from
/// [`pvm_contract_types::sol_revert_enum!`].
///
/// # Example
///
/// ```ignore
/// let mut buf = RevertBuffer::<64>::new();
/// let payload = buf.encode(&error);
/// PolkaVmHost::return_value(ReturnFlags::REVERT, payload);
/// ```
pub struct RevertBuffer<const N: usize> {
    buf: [u8; N],
}

impl<const N: usize> Default for RevertBuffer<N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const N: usize> RevertBuffer<N> {
    pub fn new() -> Self {
        Self { buf: [0; N] }
    }

    pub fn encode<'a, E: pvm_contract_types::SolRevert>(&'a mut self, e: &E) -> &'a [u8] {
        let len = e.revert_data(&mut self.buf);
        &self.buf[..len]
    }
}

/// A method handler receives the calldata bytes after the 4-byte selector.
///
/// The handler is responsible for decoding inputs, executing logic, and calling
/// [`pvm_contract_types::HostApi::return_value`] to return encoded output.
/// If the handler returns normally (without diverging), the dispatcher treats
/// it as a successful call with no return data.
pub type MethodHandler = fn(&[u8]);

/// Maximum number of methods a single contract can register.
const MAX_METHODS: usize = 16;

/// Pure Rust builder for PVM smart contract dispatch.
///
/// Provides a non-macro alternative to `#[contract]` for authoring PVM contracts.
/// Each method is registered as a `(Selector, MethodHandler)` pair. When
/// [`dispatch`](ContractBuilder::dispatch) is called, the builder reads calldata
/// from the host, extracts the 4-byte selector, and routes to the matching handler.
///
/// # Example
///
/// ```ignore
/// #[no_mangle]
/// #[polkavm_derive::polkavm_export]
/// pub extern "C" fn call() {
///     const FIBONACCI_SELECTOR: [u8; 4] = solidity_selector("fibonacci(uint32)");
///     ContractBuilder::new()
///         .method(FIBONACCI_SELECTOR, fibonacci_handler)
///         .dispatch::<PolkaVmHost, 256>()
/// }
///
/// fn fibonacci_handler(input: &[u8]) {
///     use pvm_contract_types::{HostApi as _, PolkaVmHost, ReturnFlags};
///     use pvm_contract_types::{SolDecode, SolEncode, StaticEncodedLen};
///
///     let n = u32::decode_at(input, 0);
///     let result = fibonacci(n);
///     let mut buf = [0u8; <u32 as StaticEncodedLen>::ENCODED_SIZE];
///     result.encode_to(&mut buf);
///     PolkaVmHost::return_value(ReturnFlags::empty(), &buf);
/// }
/// ```
pub struct ContractBuilder {
    methods: [(Selector, MethodHandler); MAX_METHODS],
    len: usize,
}

fn noop_handler(_: &[u8]) {}

impl Default for ContractBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ContractBuilder {
    /// Create a new empty contract builder.
    pub fn new() -> Self {
        Self {
            methods: [([0; 4], noop_handler as MethodHandler); MAX_METHODS],
            len: 0,
        }
    }

    /// Register a method handler for the given selector.
    ///
    /// # Panics
    ///
    /// Panics if more than 16 methods are registered.
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

    /// Try to route a call by selector without reading calldata.
    ///
    /// Returns `Some(())` if a handler matched (the handler may diverge via
    /// `return_value`). Returns `None` if no selector matched, allowing the
    /// caller to try another router or fall back.
    #[inline(always)]
    pub fn try_route(&self, selector: [u8; 4], input: &[u8]) -> Option<()> {
        let mut i = 0;
        while i < self.len {
            let (sel, handler) = self.methods[i];
            if sel == selector {
                handler(input);
                return Some(());
            }
            i += 1;
        }
        None
    }

    /// Read calldata from the host, match the selector, and dispatch.
    ///
    /// `BUF_SIZE` is the fixed stack buffer size for calldata (e.g. 256).
    /// `H` is the host backend implementation (use `PolkaVmHost` or `MockHost`).
    /// Reverts if calldata exceeds the buffer or no selector matches.
    pub fn dispatch<H: pvm_contract_types::HostApi, const BUF_SIZE: usize>(self) -> ! {
        use pvm_contract_types::ReturnFlags;

        let call_data_len = H::call_data_size() as usize;

        let mut buf = [0u8; BUF_SIZE];
        if call_data_len > BUF_SIZE {
            H::return_value(
                ReturnFlags::REVERT,
                &pvm_contract_types::framework_errors::CALLDATA_TOO_LARGE,
            );
        }
        H::call_data_copy(&mut buf[..call_data_len], 0);

        if call_data_len < 4 {
            H::return_value(
                ReturnFlags::REVERT,
                &pvm_contract_types::framework_errors::NO_SELECTOR,
            );
        }

        let selector: [u8; 4] = [buf[0], buf[1], buf[2], buf[3]];
        let input = &buf[4..call_data_len];

        if self.try_route(selector, input).is_some() {
            H::return_value(ReturnFlags::empty(), &[]);
        }

        H::return_value(
            ReturnFlags::REVERT,
            &pvm_contract_types::framework_errors::UNKNOWN_SELECTOR,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_handler(_: &[u8]) {}

    #[test]
    #[should_panic(expected = "MAX_METHODS")]
    fn method_panics_on_overflow() {
        let mut builder = ContractBuilder::new();
        for i in 0..=MAX_METHODS {
            builder = builder.method([i as u8, 0, 0, 0], dummy_handler);
        }
    }
}

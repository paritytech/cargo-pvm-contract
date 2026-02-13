#![no_std]

pub use pallet_revive_uapi;
pub use pallet_revive_uapi::solidity_selector;
pub use polkavm_derive;
pub use pvm_contract_types;
pub use ruint;

/// 4-byte Solidity function selector.
pub type Selector = [u8; 4];

/// A method handler receives the calldata bytes after the 4-byte selector.
///
/// The handler is responsible for decoding inputs, executing logic, and calling
/// [`pallet_revive_uapi::HostFn::return_value`] to return encoded output.
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
///         .dispatch::<HostFnImpl, 256>()
/// }
///
/// fn fibonacci_handler(input: &[u8]) {
///     use pallet_revive_uapi::{HostFn as _, HostFnImpl, ReturnFlags};
///     use pvm_contract_types::{SolDecode, SolEncode, StaticEncodedLen};
///
///     let n = u32::decode_at(input, 0);
///     let result = fibonacci(n);
///     let mut buf = [0u8; <u32 as StaticEncodedLen>::ENCODED_SIZE];
///     result.encode_to(&mut buf);
///     HostFnImpl::return_value(ReturnFlags::empty(), &buf);
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
        self.methods[self.len] = (selector, handler);
        self.len += 1;
        self
    }

    /// Read calldata from the host, match the selector, and dispatch.
    ///
    /// `BUF_SIZE` is the fixed stack buffer size for calldata (e.g. 256).
    /// `H` is the host function implementation (use `HostFnImpl`).
    /// Reverts if calldata exceeds the buffer or no selector matches.
    pub fn dispatch<H: pallet_revive_uapi::HostFn, const BUF_SIZE: usize>(self) -> ! {
        use pallet_revive_uapi::ReturnFlags;

        let call_data_len = H::call_data_size() as usize;

        let mut buf = [0u8; BUF_SIZE];
        if call_data_len > BUF_SIZE {
            H::return_value(ReturnFlags::REVERT, b"CalldataTooLarge");
        }
        H::call_data_copy(&mut buf[..call_data_len], 0);

        if call_data_len < 4 {
            H::return_value(ReturnFlags::REVERT, b"NoSelector");
        }

        let selector: [u8; 4] = [buf[0], buf[1], buf[2], buf[3]];
        let input = &buf[4..call_data_len];

        let mut i = 0;
        while i < self.len {
            let (sel, handler) = self.methods[i];
            if sel == selector {
                handler(input);
                H::return_value(ReturnFlags::empty(), &[]);
            }
            i += 1;
        }

        H::return_value(ReturnFlags::REVERT, b"UnknownSelector")
    }
}

#![doc = include_str!("../../../specs/abi.md")]
#![cfg_attr(not(feature = "std"), no_std)]

extern crate self as pvm_contract_types;

#[cfg(feature = "alloc")]
extern crate alloc;

#[cfg(feature = "alloc")]
mod alloc_types;
use core::mem::MaybeUninit;

#[cfg(feature = "alloc")]
pub use alloc_types::Bytes;

#[cfg(feature = "abi-gen")]
mod abi_gen;
#[cfg(feature = "abi-gen")]
pub use abi_gen::{
    AbiItem, AbiJson, AbiParam, StorageLayout, StorageLayoutEntry, abi_to_json, parse_type_str,
    storage_layout_to_json,
};

use framework_errors::INVALID_CALLDATA;
#[cfg(feature = "abi-gen")]
#[doc(hidden)]
pub use serde_json;

mod host;
pub use host::{
    CallFlags, Host, HostApi, HostResult, PolkaVmHost, ReturnErrorCode, ReturnFlags, StorageFlags,
};

/// Re-exported so macro-generated `call()` / `deploy()` wrappers can reach it
/// without the user's `Cargo.toml` depending on `pallet-revive-uapi` directly.
#[doc(hidden)]
pub use pallet_revive_uapi;

#[cfg(feature = "std")]
mod mock_host;
#[cfg(feature = "std")]
pub use mock_host::{Halt, MockHost, MockHostBuilder, ReturnValue};

mod i256;
pub use i256::{I256, ParseI256Error};

#[doc(hidden)]
pub use const_format;
pub use ruint::aliases::U256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DecodeError;

impl SolError for DecodeError {
    const SELECTOR: [u8; 4] = INVALID_CALLDATA;

    const SIGNATURE: &'static str = framework_errors::NAMES[0];

    fn encode_params(&self, _buf: &mut [u8]) -> usize {
        0
    }

    fn encoded_size(&self) -> usize {
        4
    }
}

/// Fixed-size buffer for compile-time string concatenation.
///
/// Use [`ConstStr::new`] to concatenate two `&str` values in a `const`
/// context, then call [`ConstStr::as_str`] to obtain the resulting `&str`.
pub struct ConstStr {
    buf: [u8; 256],
    len: usize,
}

impl ConstStr {
    /// Concatenates `a` and `b` into a new [`ConstStr`].
    pub const fn new(a: &str, b: &str) -> Self {
        let a = a.as_bytes();
        let b = b.as_bytes();
        let len = a.len() + b.len();
        assert!(len <= 256, "concatenated string exceeds 256 bytes");

        let mut buf = [0u8; 256];
        let mut i = 0;
        while i < a.len() {
            buf[i] = a[i];
            i += 1;
        }
        let mut j = 0;
        while j < b.len() {
            buf[i + j] = b[j];
            j += 1;
        }
        Self { buf, len }
    }

    /// Appends `s` to this [`ConstStr`], returning a new [`ConstStr`].
    pub const fn append(self, s: &str) -> Self {
        let s = s.as_bytes();
        let new_len = self.len + s.len();
        assert!(new_len <= 256, "appended string exceeds 256 bytes");

        let mut buf = self.buf;
        let mut i = 0;
        while i < s.len() {
            buf[self.len + i] = s[i];
            i += 1;
        }
        Self { buf, len: new_len }
    }

    /// Appends the decimal representation of `n` to this [`ConstStr`].
    pub const fn append_usize(self, n: usize) -> Self {
        if n == 0 {
            return self.append("0");
        }
        let mut digits = [0u8; 20];
        let mut num_digits = 0;
        let mut val = n;
        while val > 0 {
            digits[num_digits] = b'0' + (val % 10) as u8;
            val /= 10;
            num_digits += 1;
        }
        let mut buf = self.buf;
        let mut new_len = self.len;
        let mut i = num_digits;
        while i > 0 {
            i -= 1;
            assert!(new_len < 256, "appended usize exceeds 256 bytes");
            buf[new_len] = digits[i];
            new_len += 1;
        }
        Self { buf, len: new_len }
    }

    /// Returns the concatenated string as a `&str`.
    pub const fn as_str(&self) -> &str {
        let (used, _) = self.buf.split_at(self.len);
        match core::str::from_utf8(used) {
            Ok(s) => s,
            Err(_) => panic!("invalid UTF-8"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Address(pub [u8; 20]);

impl Address {
    pub const ZERO: Self = Self([0u8; 20]);
}

impl From<[u8; 20]> for Address {
    fn from(value: [u8; 20]) -> Self {
        Self(value)
    }
}

impl From<Address> for [u8; 20] {
    fn from(value: Address) -> Self {
        value.0
    }
}

impl AsRef<[u8; 20]> for Address {
    fn as_ref(&self) -> &[u8; 20] {
        &self.0
    }
}

impl AsRef<[u8]> for Address {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Marker trait for types that can be elements of `[T; N]` fixed arrays.
///
/// All types implementing `SolEncode` should also implement this trait,
/// **except `u8`**: bare `[u8; N]` arrays encode as Solidity `bytesN`
/// (a single left-aligned word), matching alloy's behavior. Use wrapper
/// types like `Address` or `#[derive(SolType)]` structs for other semantics.
pub trait SolArrayElement: SolEncode {}

/// Computes the 4-byte Solidity function selector at compile time.
pub const fn const_selector(sig: &str) -> [u8; 4] {
    let hash = keccak_const::Keccak256::new()
        .update(sig.as_bytes())
        .finalize();
    [hash[0], hash[1], hash[2], hash[3]]
}

/// ABI-compatible parameterless custom errors for framework-level reverts.
///
/// Each constant is `keccak256("ErrorName()")[0..4]`. Contracts revert with
/// these 4-byte selectors instead of raw byte strings, so Ethereum tooling
/// (Foundry, ethers, block explorers) can decode them.
pub mod framework_errors {
    use super::const_selector;

    /// Calldata is shorter than the minimum required by the dispatched method.
    pub const INVALID_CALLDATA: [u8; 4] = const_selector("InvalidCalldata()");
    /// Calldata exceeds the fixed buffer size (no-alloc mode only).
    pub const CALLDATA_TOO_LARGE: [u8; 4] = const_selector("CalldataTooLarge()");
    /// Calldata is shorter than 4 bytes (no selector present).
    pub const NO_SELECTOR: [u8; 4] = const_selector("NoSelector()");
    /// The 4-byte selector does not match any method in the contract.
    pub const UNKNOWN_SELECTOR: [u8; 4] = const_selector("UnknownSelector()");
    /// A non-payable entry point received a non-zero value transfer.
    pub const NON_PAYABLE_VALUE_RECEIVED: [u8; 4] = const_selector("NonPayableValueReceived()");

    /// Error names for ABI JSON generation. Single source of truth used by both
    /// the proc macro (`abi_gen.rs`) and the builder (`abi.rs`).
    pub const NAMES: &[&str] = &[
        "InvalidCalldata",
        "CalldataTooLarge",
        "NoSelector",
        "UnknownSelector",
        "NonPayableValueReceived",
    ];
}

/// Read `value_transferred` from the host and report whether any byte is
/// non-zero. On riscv64 the buffer is backed by `[u64; 4]` so the
/// non-zero check is a 4-word OR-fold rather than a 32-byte memcmp against a
/// zero constant — meaningfully smaller bytecode for the payable enforcement
/// hot path.
#[inline]
pub fn value_transferred_is_nonzero<H: HostApi>(host: &H) -> bool {
    #[cfg(target_arch = "riscv64")]
    {
        let mut words = [0u64; 4];
        // SAFETY: `[u64; 4]` and `[u8; 32]` have the same size; `[u8]` has
        // looser alignment than `[u64]`, so casting to `&mut [u8; 32]` is
        // sound and the host's byte writes produce valid `u64` values.
        let buf: &mut [u8; 32] = unsafe { &mut *(words.as_mut_ptr() as *mut [u8; 32]) };
        host.value_transferred(buf);
        words[0] | words[1] | words[2] | words[3] != 0
    }
    #[cfg(not(target_arch = "riscv64"))]
    {
        let mut buf = [0u8; 32];
        host.value_transferred(&mut buf);
        buf != [0u8; 32]
    }
}

/// Selector-based dispatch trait for composable `#[contract]` routing.
///
/// Each contract module gets a generated `impl Router<Host> for Contract`
/// that delegates to a free `mod_name::route(this, selector, input)` function.
/// Dispatch arms call `host.return_value(...)` directly — `-> !` on `riscv64`
/// (terminates execution), `-> ()` on host targets (captures into
/// [`MockHost`](super::MockHost) for tests to inspect via
/// [`MockHost::take_return_value`](super::MockHost::take_return_value)).
///
/// # Composition and inheritance
///
/// Chain routers via `Option::or_else` — the same idiom as `main`:
///
/// ```ignore
/// pub extern "C" fn call() {
///     let mut this = Composed::default();
///     if my_extension::route(&mut this, sel, input).is_some() { return; }
///     if erc20::route(&mut this.parent, sel, input).is_some() { return; }
///     // fallback or revert
/// }
/// ```
pub trait Router<H: HostApi> {
    /// Dispatch `selector` against `input`. Returns `Some(())` if the selector
    /// was handled (the dispatch arm has already called `host.return_value(...)`,
    /// which on `riscv64` means execution has terminated). Returns `None` if
    /// the selector did not match — the caller can try parent routers or
    /// fall back to revert.
    fn route(&mut self, selector: [u8; 4], input: &[u8]) -> Option<()>;
}

/// Trait for encoding Rust types to Solidity ABI-encoded bytes.
///
/// Two encoding surfaces:
/// - [`encode_body_to`](SolEncode::encode_body_to) — field body encoding without offset wrapper.
///   Used internally by parent types (tuples, arrays, structs) when composing fields.
/// - [`encode_to`](SolEncode::encode_to) — smart top-level encoding suitable for ABI return data.
///   Checks [`IS_TUPLE`](SolEncode::IS_TUPLE) and [`IS_DYNAMIC`](SolEncode::IS_DYNAMIC) to
///   produce correct output: tuples encode as flat body (multi-return), dynamic non-tuples
///   get a 32-byte offset wrapper, static non-tuples pass through.
pub trait SolEncode {
    const IS_DYNAMIC: bool;

    /// The canonical Solidity type name (e.g. "uint256", "address", "(uint64,uint64)").
    const SOL_NAME: &'static str;

    /// Size of the head portion in ABI encoding. Defaults to 32 (one ABI word).
    /// Overridden by structs to the sum of their field HEAD_SIZEs.
    const HEAD_SIZE: usize = 32;

    /// Size of the slot this type occupies in a parent tuple/struct head.
    /// Dynamic types always use 32 bytes (an offset pointer); static types
    /// use their full `HEAD_SIZE`.
    const SLOT_SIZE: usize = if Self::IS_DYNAMIC {
        32
    } else {
        Self::HEAD_SIZE
    };

    /// Build an ABI parameter description for this type.
    /// Only available when the `abi-gen` feature is enabled.
    /// Structs override this to return `"type": "tuple"` with `components`.
    #[cfg(feature = "abi-gen")]
    fn abi_param(name: &str) -> AbiParam {
        AbiParam {
            name: name.into(),
            param_type: Self::SOL_NAME.into(),
            components: alloc::vec![],
        }
    }

    /// Whether this type is a Rust tuple `(T1, T2, ...)`.
    /// Tuples represent multiple return values and skip the `enc((T))` wrapping
    /// in [`encode_to`](SolEncode::encode_to). Only set to `true` by tuple impls.
    const IS_TUPLE: bool = false;

    /// Byte length of the field body encoding.
    fn encode_body_len(&self) -> usize;

    /// Encode the field body into `buf` (must be at least `encode_body_len()` bytes).
    /// No offset wrapping — this is what parent types call when composing fields.
    fn encode_body_to(&self, buf: &mut [u8]);

    /// Byte length of the smart top-level encoding.
    fn encode_len(&self) -> usize {
        if Self::IS_TUPLE || !Self::IS_DYNAMIC {
            self.encode_body_len()
        } else {
            32 + self.encode_body_len()
        }
    }

    /// Smart top-level encoding suitable for ABI return data and calldata.
    ///
    /// Per the Solidity ABI spec, function return values are encoded as
    /// `enc((v_1, ..., v_k))`. For a single dynamic return value this means
    /// a 32-byte offset pointer is prepended before the body data, telling
    /// the decoder where the actual content starts. This wrapping is what
    /// makes `abi.decode` work on the caller side.
    ///
    /// The three cases:
    /// - **Tuples** (`IS_TUPLE=true`): flat body directly — represents
    ///   multiple return values, the wrapping is the tuple itself.
    /// - **Dynamic non-tuples** (`IS_DYNAMIC=true`): `[offset=32]` prefix
    ///   followed by the body from [`encode_body_to`](SolEncode::encode_body_to).
    /// - **Static non-tuples**: body directly — no offset needed since the
    ///   size is known at compile time.
    fn encode_to(&self, buf: &mut [u8]) {
        if Self::IS_TUPLE || !Self::IS_DYNAMIC {
            self.encode_body_to(buf);
        } else {
            // Dynamic non-tuple: prepend a 32-byte offset pointer.
            // The offset value is always 32 (0x20) — "data starts at byte 32".
            buf[..24].fill(0);
            buf[24..32].copy_from_slice(&32u64.to_be_bytes());
            self.encode_body_to(&mut buf[32..]);
        }
    }
}

/// Marker trait for types with compile-time known encoded size.
pub trait StaticEncodedLen: SolEncode + Sized {
    const ENCODED_SIZE: usize;
}

/// Trait for decoding Solidity ABI-encoded bytes into Rust types.
pub trait SolDecode: SolEncode + Sized {
    /// Decode from top-level ABI encoding produced by [`SolEncode::encode_to`].
    /// Symmetric with `encode_to`:
    /// - Tuples (IS_TUPLE=true): decode body directly
    /// - Dynamic non-tuples: read offset pointer at position 0, decode body at offset
    /// - Static non-tuples: decode body directly
    fn decode(input: &[u8]) -> Result<Self, DecodeError> {
        if Self::IS_TUPLE || !Self::IS_DYNAMIC {
            Self::decode_at(input, 0)
        } else {
            // Dynamic non-tuple: encode_to wrote [offset=32][body]
            // Read offset, then decode the body at that position
            let offset = input
                .get(24..32)
                .and_then(|x| TryInto::<[u8; 8]>::try_into(x).ok())
                .ok_or(DecodeError)
                .map(u64::from_be_bytes)? as usize;
            Self::decode_tail(input, offset)
        }
    }

    /// Offset-based decode helper used by generated code and custom decoders.
    fn decode_at(input: &[u8], offset: usize) -> Result<Self, DecodeError>;

    /// Tail decode helper used by dynamic container decoding.
    #[inline(always)]
    fn decode_tail(input: &[u8], offset: usize) -> Result<Self, DecodeError> {
        Self::decode_at(input, offset)
    }
}

pub trait StaticDecode: SolDecode + SolEncode + StaticEncodedLen + Sized {
    /// # Safety
    ///
    /// safety contract: caller guarantees `input.len() >= offset + ENCODED_SIZE`.
    /// Caller is the dispatch codegen that checks total size once at entry.
    unsafe fn decode_unchecked(input: &[u8], offset: usize) -> Self;
}

// ---------------------------------------------------------------------------
// Error traits and types for Solidity-compatible ABI-encoded reverts
// ---------------------------------------------------------------------------

/// Trait for Solidity-compatible ABI-encoded revert errors.
///
/// Each implementor represents a single Solidity error type with its own
/// 4-byte selector. Implementors produce error data that all Ethereum tools can decode.
///
/// For error enums (multiple error types), see [`SolRevert`].
///
/// The wire format is: selector (4 bytes) + ABI-encoded parameters.
/// This matches what the Solidity compiler produces for custom errors.
///
/// # Buffer allocation
///
/// In **no-alloc mode** (default stack), errors are encoded into a fixed
/// 256-byte stack buffer. Static error fields are always safe. Errors with
/// dynamic fields (String, Vec) must keep the total payload under 252 bytes
/// or the encoding may panic. [`RevertString`] handles this with truncation.
///
/// In **alloc mode** (`allocator = "pico"` / `"bump"`), the dispatch uses
/// [`SolRevert::revert_data_len`] to allocate an exact-size `Vec<u8>`,
/// so errors with dynamic fields work regardless of payload size.
pub trait SolError {
    /// The 4-byte error selector: `keccak256(SIGNATURE)[0:4]`.
    /// Computed at compile time.
    const SELECTOR: [u8; 4];

    /// The canonical Solidity error signature.
    /// Example: `"InsufficientBalance(address,uint256,uint256)"`
    /// Used for ABI JSON generation.
    const SIGNATURE: &'static str;

    /// Encode the error parameters (without selector) into `buf`.
    /// Returns the number of bytes written.
    /// The caller prepends the 4-byte selector.
    fn encode_params(&self, buf: &mut [u8]) -> usize;

    /// Total encoded size: 4 (selector) + parameter bytes.
    fn encoded_size(&self) -> usize;
}

/// Trait for anything that can produce ABI-encoded revert data.
///
/// [`SolError`] types get this automatically via a blanket impl.
/// Error enums need a manual impl or can use [`sol_revert_enum!`].
pub trait SolRevert {
    /// Write the full revert data (selector + params) into `buf`.
    /// Returns the number of bytes written.
    fn revert_data(&self, buf: &mut [u8]) -> usize;

    /// Total byte length of the revert data (selector + params).
    /// Used by alloc-mode dispatch to allocate an exact-size buffer.
    /// Defaults to 256 (the stack buffer size) if not overridden.
    fn revert_data_len(&self) -> usize {
        256
    }

    /// Return the Solidity error signatures for all error types that
    /// this type can produce. Used by abi-gen to emit ABI JSON error entries.
    ///
    /// For single `SolError` types, returns the one signature.
    /// For error enums (`sol_revert_enum!`), returns all inner error signatures.
    fn error_signatures() -> impl Iterator<Item = &'static &'static str>
    where
        Self: Sized,
    {
        let arr: &'static [&'static str] = &[];
        arr.iter()
    }
}

/// Blanket impl: any `SolError` is automatically `SolRevert`.
impl<E: SolError> SolRevert for E {
    fn revert_data(&self, buf: &mut [u8]) -> usize {
        if buf.len() < 4 {
            return 0;
        }
        buf[0..4].copy_from_slice(&E::SELECTOR);
        4 + self.encode_params(&mut buf[4..])
    }

    fn revert_data_len(&self) -> usize {
        self.encoded_size()
    }

    fn error_signatures() -> impl Iterator<Item = &'static &'static str> {
        let arr = &[E::SIGNATURE];
        arr.iter()
    }
}

/// Standard Solidity `Error(string)` revert.
///
/// Selector: `0x08c379a0` = `keccak256("Error(string)")[0:4]`
///
/// This is what Solidity's `require(condition, "message")` produces.
/// It's the most common error type in the Ethereum ecosystem.
pub struct RevertString<'a>(pub &'a str);

impl SolError for RevertString<'_> {
    const SELECTOR: [u8; 4] = [0x08, 0xc3, 0x79, 0xa0];
    const SIGNATURE: &'static str = "Error(string)";

    fn encode_params(&self, buf: &mut [u8]) -> usize {
        let str_bytes = self.0.as_bytes();

        // ABI string encoding: [offset: 32 bytes][length: 32 bytes][data: padded to 32]
        // Need at least 64 bytes for the offset + length words
        if buf.len() < 64 {
            let n = buf.len().min(32);
            buf[..n].fill(0);
            return n;
        }

        // Truncate string to fit buffer, aligned down to 32-byte boundary
        let max_data = (buf.len() - 64) & !31;
        let actual_len = str_bytes.len().min(max_data);
        let padding = (32 - (actual_len % 32)) % 32;
        let total = 64 + actual_len + padding;

        buf[..total].fill(0);
        buf[24..32].copy_from_slice(&32u64.to_be_bytes()); // offset
        buf[56..64].copy_from_slice(&(actual_len as u64).to_be_bytes()); // length
        buf[64..64 + actual_len].copy_from_slice(&str_bytes[..actual_len]); // data

        total
    }

    fn encoded_size(&self) -> usize {
        // 68 = 4 (selector) + 32 (offset word) + 32 (length word)
        68 + ((self.0.len() + 31) & !31)
    }
}

/// Standard Solidity `Panic(uint256)` revert.
///
/// Selector: `0x4e487b71` = `keccak256("Panic(uint256)")[0:4]`
///
/// The Solidity compiler emits these for runtime failures.
/// Each variant maps to a well-known panic code that Ethereum tools recognize.
///
/// Solidity defines 10 panic codes (0x00-0x51). We implement the two
/// needed for safe math. Likely future additions: 0x01 (assert failure)
/// and 0x32 (out-of-bounds access).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panic {
    /// 0x11 — arithmetic overflow/underflow
    Overflow,
    /// 0x12 — division or modulo by zero
    DivisionByZero,
}

impl Panic {
    fn code(&self) -> u8 {
        match self {
            Panic::Overflow => 0x11,
            Panic::DivisionByZero => 0x12,
        }
    }
}

impl SolError for Panic {
    const SELECTOR: [u8; 4] = [0x4e, 0x48, 0x7b, 0x71];
    const SIGNATURE: &'static str = "Panic(uint256)";

    fn encode_params(&self, buf: &mut [u8]) -> usize {
        buf[..32].fill(0);
        buf[31] = self.code();
        32
    }

    fn encoded_size(&self) -> usize {
        36
    }
}

/// Pre-built error enum for methods that only use standard Solidity errors.
///
/// Wraps [`Panic`] (overflow, div-by-zero) and [`RevertString`] (require-style messages).
/// Use this when your method doesn't define custom errors:
///
/// ```ignore
/// fn transfer(&mut self, to: Address, amount: U256) -> Result<(), SolDefaultError> {
///     let new_balance = balance.checked_sub(amount).ok_or(Panic::Overflow)?;
///     Ok(())
/// }
/// ```
pub enum SolDefaultError {
    Panic(Panic),
    Revert(RevertString<'static>),
}

impl SolRevert for SolDefaultError {
    fn revert_data(&self, buf: &mut [u8]) -> usize {
        match self {
            Self::Panic(e) => e.revert_data(buf),
            Self::Revert(e) => e.revert_data(buf),
        }
    }

    fn revert_data_len(&self) -> usize {
        match self {
            Self::Panic(e) => e.revert_data_len(),
            Self::Revert(e) => e.revert_data_len(),
        }
    }

    fn error_signatures() -> impl Iterator<Item = &'static &'static str> {
        let arr = &[Panic::SIGNATURE, RevertString::SIGNATURE];
        arr.iter()
    }
}

impl From<Panic> for SolDefaultError {
    fn from(e: Panic) -> Self {
        Self::Panic(e)
    }
}

impl From<RevertString<'static>> for SolDefaultError {
    fn from(e: RevertString<'static>) -> Self {
        Self::Revert(e)
    }
}

/// Zero-cost error type for contracts that never produce errors.
///
/// This is an uninhabited enum since `match *self {}` compiles to zero code.
/// Use this when constructor/fallback return `Result` but never actually
/// fail. Unlike [`SolDefaultError`], this adds zero bytes to the contract
/// binary since no error encoding code is generated.
///
/// **When to use which:**
/// - No error paths → `EmptyError`
/// - Custom errors → [`sol_revert_enum!`] (auto-injects `Panic` + `RevertString`)
/// - Standard errors only → [`SolDefaultError`]
///
/// ```ignore
/// type Error = pvm_contract_types::EmptyError;
///
/// pub fn new() -> Result<(), Error> { Ok(()) }
/// pub fn fallback() -> Result<(), Error> { Ok(()) }
/// ```
pub enum EmptyError {}

impl SolRevert for EmptyError {
    fn revert_data(&self, _buf: &mut [u8]) -> usize {
        match *self {}
    }

    fn revert_data_len(&self) -> usize {
        match *self {}
    }
}

/// Generate an error enum with [`SolRevert`] impl and [`From`] conversions
/// for `?` propagation.
///
/// The standard error variants [`Panic`] and [`RevertString`] are
/// auto-injected — you only list your custom error types.
///
/// We recommend one contract-wide error enum rather than per-method enums,
/// to avoid duplicating match arms and optimize bytecode size.
///
/// # Example
///
/// ```ignore
/// sol_revert_enum! {
///     pub enum TokenError {
///         InsufficientBalance(InsufficientBalance),
///         Unauthorized(Unauthorized),
///     }
/// }
/// ```
#[macro_export]
macro_rules! sol_revert_enum {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $($variant:ident($inner:ty)),+ $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis enum $name {
            $($variant($inner),)+
            Panic($crate::Panic),
            Revert($crate::RevertString<'static>),
        }

        impl $crate::SolRevert for $name {
            fn revert_data(&self, buf: &mut [u8]) -> usize {
                match self {
                    $(Self::$variant(e) => <$inner as $crate::SolRevert>::revert_data(e, buf),)+
                    Self::Panic(e) => $crate::SolRevert::revert_data(e, buf),
                    Self::Revert(e) => $crate::SolRevert::revert_data(e, buf),
                }
            }

            fn revert_data_len(&self) -> usize {
                match self {
                    $(Self::$variant(e) => <$inner as $crate::SolRevert>::revert_data_len(e),)+
                    Self::Panic(e) => $crate::SolRevert::revert_data_len(e),
                    Self::Revert(e) => $crate::SolRevert::revert_data_len(e),
                }
            }

            fn error_signatures() -> impl Iterator<Item = &'static &'static str> {
               let arr =  &[
                    <$crate::Panic as $crate::SolError>::SIGNATURE,
                    <$crate::RevertString<'static> as $crate::SolError>::SIGNATURE,
                ];
                let arr = arr.into_iter();
                let arr = arr$(.chain(<$inner as $crate::SolRevert>::error_signatures()))+;
                arr.into_iter()
            }
        }

        $(
            impl From<$inner> for $name {
                fn from(e: $inner) -> Self {
                    Self::$variant(e)
                }
            }
        )+

        impl From<$crate::Panic> for $name {
            fn from(e: $crate::Panic) -> Self {
                Self::Panic(e)
            }
        }

        impl From<$crate::RevertString<'static>> for $name {
            fn from(e: $crate::RevertString<'static>) -> Self {
                Self::Revert(e)
            }
        }
    };
}

// ---------------------------------------------------------------------------
// Primitive type impls
// ---------------------------------------------------------------------------

macro_rules! impl_static_type {
    ($ty:ty, $sol_name:expr, $encode_fn:expr, $decode_fn:expr) => {
        impl SolEncode for $ty {
            const IS_DYNAMIC: bool = false;
            const SOL_NAME: &'static str = $sol_name;

            #[inline]
            fn encode_body_len(&self) -> usize {
                32
            }

            #[inline]
            fn encode_body_to(&self, buf: &mut [u8]) {
                $encode_fn(self, buf)
            }
        }

        impl StaticEncodedLen for $ty {
            const ENCODED_SIZE: usize = 32;
        }

        impl SolDecode for $ty {
            #[inline]
            fn decode_at(input: &[u8], offset: usize) -> Result<Self, DecodeError> {
                $decode_fn(input, offset)
            }
        }
    };
    // Variant that also emits SolArrayElement
    ($ty:ty, $sol_name:expr, $encode_fn:expr, $decode_fn:expr, array_element) => {
        impl_static_type!($ty, $sol_name, $encode_fn, $decode_fn);
        impl SolArrayElement for $ty {}
    };
}

impl_static_type!(
    U256,
    "uint256",
    |val: &U256, buf: &mut [u8]| buf[..32].copy_from_slice(&val.to_be_bytes::<32>()),
    |input: &[u8], offset: usize| input
        .get(offset..offset + 32)
        .ok_or(DecodeError)
        .map(U256::from_be_slice),
    array_element
);

impl_static_type!(
    I256,
    "int256",
    |val: &I256, buf: &mut [u8]| buf[..32].copy_from_slice(&val.to_be_bytes()),
    |input: &[u8], offset: usize| input
        .get(offset..offset + 32)
        .ok_or(DecodeError)
        .map(I256::from_be_slice),
    array_element
);

impl_static_type!(
    u128,
    "uint128",
    |val: &u128, buf: &mut [u8]| {
        buf[..16].fill(0);
        buf[16..32].copy_from_slice(&val.to_be_bytes());
    },
    |input: &[u8], offset: usize| {
        input
            .get(offset + 16..offset + 32)
            .and_then(|x| TryInto::<[u8; 16]>::try_into(x).ok())
            .ok_or(DecodeError)
            .map(u128::from_be_bytes)
    },
    array_element
);

impl_static_type!(
    u64,
    "uint64",
    |val: &u64, buf: &mut [u8]| {
        buf[..24].fill(0);
        buf[24..32].copy_from_slice(&val.to_be_bytes());
    },
    |input: &[u8], offset: usize| {
        input
            .get(offset + 24..offset + 32)
            .and_then(|x| TryInto::<[u8; 8]>::try_into(x).ok())
            .ok_or(DecodeError)
            .map(u64::from_be_bytes)
    },
    array_element
);

impl_static_type!(
    u32,
    "uint32",
    |val: &u32, buf: &mut [u8]| {
        buf[..28].fill(0);
        buf[28..32].copy_from_slice(&val.to_be_bytes());
    },
    |input: &[u8], offset: usize| {
        input
            .get(offset + 28..offset + 32)
            .and_then(|x| TryInto::<[u8; 4]>::try_into(x).ok())
            .ok_or(DecodeError)
            .map(u32::from_be_bytes)
    },
    array_element
);

impl_static_type!(
    u16,
    "uint16",
    |val: &u16, buf: &mut [u8]| {
        buf[..30].fill(0);
        buf[30..32].copy_from_slice(&val.to_be_bytes());
    },
    |input: &[u8], offset: usize| {
        input
            .get(offset + 30)
            .zip(input.get(offset + 31))
            .map(|x| u16::from_be_bytes([*x.0, *x.1]))
            .ok_or(DecodeError)
    },
    array_element
);

impl_static_type!(
    u8,
    "uint8",
    |val: &u8, buf: &mut [u8]| {
        buf[..31].fill(0);
        buf[31] = *val;
    },
    |input: &[u8], offset: usize| input.get(offset + 31).ok_or(DecodeError).copied()
);

impl_static_type!(
    i128,
    "int128",
    |val: &i128, buf: &mut [u8]| {
        let fill = if *val < 0 { 0xff } else { 0 };
        buf[..16].fill(fill);
        buf[16..32].copy_from_slice(&val.to_be_bytes());
    },
    |input: &[u8], offset: usize| {
        input
            .get(offset + 16..offset + 32)
            .and_then(|x| TryInto::<[u8; 16]>::try_into(x).ok())
            .ok_or(DecodeError)
            .map(i128::from_be_bytes)
    },
    array_element
);

impl_static_type!(
    i64,
    "int64",
    |val: &i64, buf: &mut [u8]| {
        let fill = if *val < 0 { 0xff } else { 0 };
        buf[..24].fill(fill);
        buf[24..32].copy_from_slice(&val.to_be_bytes());
    },
    |input: &[u8], offset: usize| {
        input
            .get(offset + 24..offset + 32)
            .and_then(|x| TryInto::<[u8; 8]>::try_into(x).ok())
            .ok_or(DecodeError)
            .map(i64::from_be_bytes)
    },
    array_element
);

impl_static_type!(
    i32,
    "int32",
    |val: &i32, buf: &mut [u8]| {
        let fill = if *val < 0 { 0xff } else { 0 };
        buf[..28].fill(fill);
        buf[28..32].copy_from_slice(&val.to_be_bytes());
    },
    |input: &[u8], offset: usize| {
        input
            .get(offset + 28..offset + 32)
            .and_then(|x| TryInto::<[u8; 4]>::try_into(x).ok())
            .ok_or(DecodeError)
            .map(i32::from_be_bytes)
    },
    array_element
);

impl_static_type!(
    i16,
    "int16",
    |val: &i16, buf: &mut [u8]| {
        let fill = if *val < 0 { 0xff } else { 0 };
        buf[..30].fill(fill);
        buf[30..32].copy_from_slice(&val.to_be_bytes());
    },
    |input: &[u8], offset: usize| input
        .get(offset + 30)
        .zip(input.get(offset + 31))
        .map(|x| i16::from_be_bytes([*x.0, *x.1]))
        .ok_or(DecodeError),
    array_element
);

impl_static_type!(
    i8,
    "int8",
    |val: &i8, buf: &mut [u8]| {
        let fill = if *val < 0 { 0xff } else { 0 };
        buf[..31].fill(fill);
        buf[31] = *val as u8;
    },
    |input: &[u8], offset: usize| input
        .get(offset + 31)
        .copied()
        .map(|x| i8::from_be_bytes([x]))
        .ok_or(DecodeError),
    array_element
);

impl_static_type!(
    bool,
    "bool",
    |val: &bool, buf: &mut [u8]| {
        buf[..31].fill(0);
        buf[31] = if *val { 1 } else { 0 };
    },
    |input: &[u8], offset: usize| input.get(offset + 31).ok_or(DecodeError).map(|x| *x != 0),
    array_element
);

impl_static_type!(
    Address,
    "address",
    |val: &Address, buf: &mut [u8]| {
        buf[..12].fill(0);
        buf[12..32].copy_from_slice(&val.0);
    },
    |input: &[u8], offset: usize| {
        input
            .get(offset + 12..offset + 32)
            .map(|x| {
                let mut result = [0u8; 20];
                result.copy_from_slice(x);
                Address(result)
            })
            .ok_or(DecodeError)
    },
    array_element
);

macro_rules! impl_static_type_decode {
    ($ty:ty,  $decode_fn:expr) => {
        impl StaticDecode for $ty {
            #[inline]
            unsafe fn decode_unchecked(input: &[u8], offset: usize) -> Self {
                $decode_fn(input, offset)
            }
        }
    };
}

impl_static_type_decode!(U256, |input: &[u8], offset: usize| unsafe {
    U256::from_be_slice(input.get_unchecked(offset..offset + 32))
});

impl_static_type_decode!(I256, |input: &[u8], offset: usize| unsafe {
    I256::from_be_slice(input.get_unchecked(offset..offset + 32))
});

impl_static_type_decode!(u128, |input: &[u8], offset: usize| {
    unsafe {
        TryInto::<[u8; 16]>::try_into(input.get_unchecked(offset + 16..offset + 32))
            .map(u128::from_be_bytes)
            .unwrap()
    }
});

impl_static_type_decode!(u64, |input: &[u8], offset: usize| {
    unsafe {
        TryInto::<[u8; 8]>::try_into(input.get_unchecked(offset + 24..offset + 32))
            .map(u64::from_be_bytes)
            .unwrap()
    }
});

impl_static_type_decode!(u32, |input: &[u8], offset: usize| {
    unsafe {
        TryInto::<[u8; 4]>::try_into(input.get_unchecked(offset + 28..offset + 32))
            .map(u32::from_be_bytes)
            .unwrap()
    }
});

impl_static_type_decode!(u16, |input: &[u8], offset: usize| {
    unsafe {
        u16::from_be_bytes([
            *input.get_unchecked(offset + 30),
            *input.get_unchecked(offset + 31),
        ])
    }
});

impl_static_type_decode!(u8, |input: &[u8], offset: usize| unsafe {
    *input.get_unchecked(offset + 31)
});

impl_static_type_decode!(i128, |input: &[u8], offset: usize| {
    unsafe {
        TryInto::<[u8; 16]>::try_into(input.get_unchecked(offset + 16..offset + 32))
            .map(i128::from_be_bytes)
            .unwrap()
    }
});

impl_static_type_decode!(i64, |input: &[u8], offset: usize| {
    unsafe {
        TryInto::<[u8; 8]>::try_into(input.get_unchecked(offset + 24..offset + 32))
            .map(i64::from_be_bytes)
            .unwrap()
    }
});

impl_static_type_decode!(i32, |input: &[u8], offset: usize| {
    input
        .get(offset + 28..offset + 32)
        .and_then(|x| TryInto::<[u8; 4]>::try_into(x).ok())
        .map(i32::from_be_bytes)
        .unwrap()
});

impl_static_type_decode!(i16, |input: &[u8], offset: usize| {
    unsafe {
        i16::from_be_bytes([
            *input.get_unchecked(offset + 30),
            *input.get_unchecked(offset + 31),
        ])
    }
});

impl_static_type_decode!(i8, |input: &[u8], offset: usize| unsafe {
    i8::from_be_bytes([*input.get_unchecked(offset + 31)])
});

impl_static_type_decode!(bool, |input: &[u8], offset: usize| unsafe {
    *input.get_unchecked(offset + 31) != 0
});

impl_static_type_decode!(Address, |input: &[u8], offset: usize| {
    unsafe {
        let mut result = [0u8; 20];
        result.copy_from_slice(input.get_unchecked(offset + 12..offset + 32));
        Address(result)
    }
});

impl SolEncode for &str {
    const IS_DYNAMIC: bool = true;
    const SOL_NAME: &'static str = "string";

    fn encode_body_len(&self) -> usize {
        let data_len = self.len();
        let padding = (32 - (data_len % 32)) % 32;
        32 + data_len + padding
    }

    fn encode_body_to(&self, buf: &mut [u8]) {
        let bytes = self.as_bytes();
        let data_len = bytes.len();
        let padding = (32 - (data_len % 32)) % 32;

        buf[..32].fill(0);
        buf[24..32].copy_from_slice(&(data_len as u64).to_be_bytes());

        buf[32..32 + data_len].copy_from_slice(bytes);
        buf[32 + data_len..32 + data_len + padding].fill(0);
    }
}

impl SolEncode for () {
    const IS_DYNAMIC: bool = false;
    const SOL_NAME: &'static str = "unit";

    fn encode_body_len(&self) -> usize {
        0
    }

    fn encode_body_to(&self, _buf: &mut [u8]) {}
}

impl SolDecode for () {
    fn decode_at(_input: &[u8], _offset: usize) -> Result<Self, DecodeError> {
        Ok(())
    }
}

impl StaticEncodedLen for () {
    const ENCODED_SIZE: usize = 0;
}

impl StaticDecode for () {
    unsafe fn decode_unchecked(_input: &[u8], _offset: usize) -> Self {}
}

// ---------------------------------------------------------------------------
// [u8; N] impl — encodes as Solidity `bytesN` (left-aligned in one word),
// matching alloy's behavior. For `T[N]` array semantics, see the
// `SolArrayElement`-bounded blanket impl below.
// ---------------------------------------------------------------------------

impl<const N: usize> SolEncode for [u8; N] {
    const IS_DYNAMIC: bool = false;
    const SOL_NAME: &'static str = {
        struct H<const N: usize>;
        impl<const N: usize> H<N> {
            const V: ConstStr = ConstStr::new("bytes", "").append_usize(N);
        }
        H::<N>::V.as_str()
    };

    #[inline]
    fn encode_body_len(&self) -> usize {
        32
    }

    fn encode_body_to(&self, buf: &mut [u8]) {
        const { assert!(N >= 1 && N <= 32, "bytesN only valid for N in 1..=32") };
        buf[..N].copy_from_slice(self);
        buf[N..32].fill(0);
    }
}

impl<const N: usize> StaticEncodedLen for [u8; N] {
    const ENCODED_SIZE: usize = 32;
}

impl<const N: usize> SolDecode for [u8; N] {
    fn decode_at(input: &[u8], offset: usize) -> Result<Self, DecodeError> {
        const { assert!(N >= 1 && N <= 32, "bytesN only valid for N in 1..=32") };

        input
            .get(offset..offset + N)
            .map(|x| {
                let mut result = [0u8; N];
                result.copy_from_slice(x);
                result
            })
            .ok_or(DecodeError)
    }
}

impl<const N: usize> StaticDecode for [u8; N] {
    unsafe fn decode_unchecked(input: &[u8], offset: usize) -> Self {
        const { assert!(N >= 1 && N <= 32, "bytesN only valid for N in 1..=32") };

        let mut result = [0u8; N];
        unsafe {
            result.copy_from_slice(input.get_unchecked(offset..offset + N));
        }
        result
    }
}

impl<const N: usize> SolArrayElement for [u8; N] {}
impl<T: SolArrayElement, const N: usize> SolArrayElement for [T; N] {}

// ---------------------------------------------------------------------------
// Blanket impl for fixed-size arrays [T; N] where T: SolArrayElement
// (excludes [u8; N] which has its own bytesN impl above)
// ---------------------------------------------------------------------------

impl<T: SolArrayElement, const N: usize> SolEncode for [T; N] {
    const IS_DYNAMIC: bool = T::IS_DYNAMIC;
    const SOL_NAME: &'static str = {
        struct H<T, const N: usize>(core::marker::PhantomData<T>);
        impl<T: SolEncode, const N: usize> H<T, N> {
            const V: ConstStr = ConstStr::new(T::SOL_NAME, "[").append_usize(N).append("]");
        }
        H::<T, N>::V.as_str()
    };
    const HEAD_SIZE: usize = T::SLOT_SIZE * N;

    fn encode_body_len(&self) -> usize {
        if T::IS_DYNAMIC {
            N * 32 + self.iter().map(|e| e.encode_body_len()).sum::<usize>()
        } else {
            T::HEAD_SIZE * N
        }
    }

    fn encode_body_to(&self, buf: &mut [u8]) {
        if T::IS_DYNAMIC {
            let mut tail_offset = N * T::SLOT_SIZE;
            for (i, elem) in self.iter().enumerate() {
                let ho = i * T::SLOT_SIZE;
                buf[ho..ho + 24].fill(0);
                buf[ho + 24..ho + 32].copy_from_slice(&(tail_offset as u64).to_be_bytes());
                let tl = elem.encode_body_len();
                elem.encode_body_to(&mut buf[tail_offset..tail_offset + tl]);
                tail_offset += tl;
            }
        } else {
            let mut offset = 0;
            for elem in self.iter() {
                elem.encode_body_to(&mut buf[offset..]);
                offset += T::SLOT_SIZE;
            }
        }
    }

    /// For `[T; N]`, ABI type is `T_abi_type[N]` and components come from T.
    #[cfg(feature = "abi-gen")]
    fn abi_param(name: &str) -> AbiParam {
        let inner = T::abi_param("");
        AbiParam {
            name: alloc::string::String::from(name),
            param_type: alloc::format!("{}[{}]", inner.param_type, N),
            components: inner.components,
        }
    }
}

impl<T: SolArrayElement + StaticEncodedLen, const N: usize> StaticEncodedLen for [T; N] {
    const ENCODED_SIZE: usize = T::ENCODED_SIZE * N;
}

impl<T: SolArrayElement + SolDecode, const N: usize> SolDecode for [T; N] {
    fn decode_at(input: &[u8], offset: usize) -> Result<Self, DecodeError> {
        let mut array: [MaybeUninit<T>; N] = [const { MaybeUninit::uninit() }; N];
        let mut written = 0usize;

        for (i, item) in array.iter_mut().enumerate() {
            let res: Result<T, DecodeError> = (|| {
                if T::IS_DYNAMIC {
                    let ho = offset + i * T::SLOT_SIZE;
                    let field_offset = input
                        .get(ho + 24..ho + 32)
                        .and_then(|x| x.try_into().ok())
                        .map(u64::from_be_bytes)
                        .ok_or(DecodeError)? as usize;
                    T::decode_tail(input, offset + field_offset)
                } else {
                    T::decode_at(input, offset + i * T::SLOT_SIZE)
                }
            })();

            match res {
                Ok(v) => {
                    item.write(v);
                    written = i + 1;
                }
                Err(e) => {
                    for slot in &mut array[..written] {
                        unsafe {
                            slot.assume_init_drop();
                        }
                    }
                    return Err(e);
                }
            }
        }

        let ptr = &array as *const _ as *const [T; N];
        Ok(unsafe { ptr.read() })
    }
}

impl<T: SolArrayElement + StaticDecode + StaticEncodedLen, const N: usize> StaticDecode for [T; N] {
    unsafe fn decode_unchecked(input: &[u8], offset: usize) -> Self {
        let mut array: [MaybeUninit<T>; N] = [const { MaybeUninit::uninit() }; N];

        for (i, item) in array.iter_mut().enumerate() {
            let res: T = {
                if T::IS_DYNAMIC {
                    let ho = offset + i * T::SLOT_SIZE;
                    let field_offset = input
                        .get(ho + 24..ho + 32)
                        .and_then(|x| x.try_into().ok())
                        .map(u64::from_be_bytes)
                        .ok_or(DecodeError)
                        .expect("failed to parse offset")
                        as usize;
                    unsafe { T::decode_unchecked(input, offset + field_offset) }
                } else {
                    unsafe { T::decode_unchecked(input, i * T::SLOT_SIZE) }
                }
            };

            item.write(res);
        }

        let ptr = &array as *const _ as *const [T; N];
        unsafe { ptr.read() }
    }
}

// ---------------------------------------------------------------------------
// Tuple impls for arities 1-12
// ---------------------------------------------------------------------------

macro_rules! impl_tuple_sol {
    // We need to build SOL_NAME via ConstStr chain. Since macro repetition
    // can't incrementally build a chain, we use a two-macro approach:
    // the outer macro generates the impl, the inner builds the name.
    (@sol_name $first:ident $(, $rest:ident)*) => {{
        struct H<$first $(, $rest)*>(core::marker::PhantomData<($first, $($rest,)*)>);
        impl<$first: SolEncode $(, $rest: SolEncode)*> H<$first $(, $rest)*> {
            const V: ConstStr = ConstStr::new("(", $first::SOL_NAME)
                $(.append(",").append($rest::SOL_NAME))*
                .append(")");
        }
        H::<$first $(, $rest)*>::V.as_str()
    }};

    ($(($idx:tt : $T:ident)),+) => {
        impl<$($T: SolEncode),+> SolEncode for ($($T,)+) {
            const IS_DYNAMIC: bool = false $(|| $T::IS_DYNAMIC)+;
            const SOL_NAME: &'static str = impl_tuple_sol!(@sol_name $($T),+);
            const HEAD_SIZE: usize = 0 $(+ $T::SLOT_SIZE)+;
            const IS_TUPLE: bool = true;

            fn encode_body_len(&self) -> usize {
                Self::HEAD_SIZE
                    $(+ if $T::IS_DYNAMIC { self.$idx.encode_body_len() } else { 0 })+
            }

            fn encode_body_to(&self, buf: &mut [u8]) {
                let mut __ho = 0usize;
                let mut __to = Self::HEAD_SIZE;
                $(
                    if $T::IS_DYNAMIC {
                        buf[__ho..__ho + 24].fill(0);
                        buf[__ho + 24..__ho + 32]
                            .copy_from_slice(&(__to as u64).to_be_bytes());
                        let __tl = self.$idx.encode_body_len();
                        self.$idx.encode_body_to(&mut buf[__to..__to + __tl]);
                        __to += __tl;
                    } else {
                        self.$idx.encode_body_to(&mut buf[__ho..]);
                    }
                    __ho += $T::SLOT_SIZE;
                )+
            }

            #[cfg(feature = "abi-gen")]
            fn abi_param(name: &str) -> AbiParam {
                let mut __idx = 0u32;
                AbiParam {
                    name: alloc::string::String::from(name),
                    param_type: alloc::string::String::from("tuple"),
                    components: alloc::vec![
                        $({
                            let _ = __idx;
                            let p = $T::abi_param("");
                            __idx += 1;
                            p
                        }),+
                    ],
                }
            }
        }

        impl<$($T: SolEncode),+> SolArrayElement for ($($T,)+) {}

        impl<$($T: StaticEncodedLen),+> StaticEncodedLen for ($($T,)+) {
            const ENCODED_SIZE: usize = 0 $(+ $T::ENCODED_SIZE)+;

        }

        impl<$($T: SolDecode),+> SolDecode for ($($T,)+) {
            fn decode_at(input: &[u8], offset: usize) -> Result<Self, DecodeError> {
                let mut __ho = offset;
                fn fo(input: &[u8], offset: usize) -> Result<usize, DecodeError> {
                    Ok(TryInto::<[u8; 8]>::try_into(
                        input
                            .get(offset + 24..offset + 32)
                            .ok_or(DecodeError)?,
                    )
                    .map_err(|_| DecodeError)
                    .map(u64::from_be_bytes)? as usize)
                }
                Ok(($(
                    {
                        let __val = if $T::IS_DYNAMIC {
                            let __fo =  fo(&input, __ho)?;

                            $T::decode_tail(input, offset + __fo)?
                        } else {
                            $T::decode_at(input, __ho)?
                        };
                        __ho += $T::SLOT_SIZE;
                        __val
                    },
                )+))

            }
        }
        impl<$($T: StaticDecode + StaticEncodedLen + SolDecode),+> StaticDecode for ($($T,)+) {
            unsafe fn decode_unchecked(input: &[u8], offset: usize) -> Self {
                let mut __ho = offset;
                ($(
                    {

                        let __val = unsafe {
                            $T::decode_unchecked(input, __ho)
                        };
                        __ho += $T::SLOT_SIZE;
                        __val
                    },
                )+)

            }
        }
    };
}

impl_tuple_sol!((0: A));
impl_tuple_sol!((0: A), (1: B));
impl_tuple_sol!((0: A), (1: B), (2: C));
impl_tuple_sol!((0: A), (1: B), (2: C), (3: D));
impl_tuple_sol!((0: A), (1: B), (2: C), (3: D), (4: E));
impl_tuple_sol!((0: A), (1: B), (2: C), (3: D), (4: E), (5: F));
impl_tuple_sol!((0: A), (1: B), (2: C), (3: D), (4: E), (5: F), (6: G));
impl_tuple_sol!((0: A), (1: B), (2: C), (3: D), (4: E), (5: F), (6: G), (7: H_));
impl_tuple_sol!((0: A), (1: B), (2: C), (3: D), (4: E), (5: F), (6: G), (7: H_), (8: I));
impl_tuple_sol!((0: A), (1: B), (2: C), (3: D), (4: E), (5: F), (6: G), (7: H_), (8: I), (9: J));
impl_tuple_sol!((0: A), (1: B), (2: C), (3: D), (4: E), (5: F), (6: G), (7: H_), (8: I), (9: J), (10: K));
impl_tuple_sol!((0: A), (1: B), (2: C), (3: D), (4: E), (5: F), (6: G), (7: H_), (8: I), (9: J), (10: K), (11: L));

#[cfg(test)]
mod tests;

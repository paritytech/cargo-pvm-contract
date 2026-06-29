//! SDK for building Rust smart contracts targeting PolkaVM.
//!
//! This is the primary user-facing entry point for the macro-based contract API.
//! It re-exports proc macros, ABI encoding traits, host API wrappers, and
//! primitive types so that `pvm-contract-types` does not need to appear in the
//! consumer's `Cargo.toml`. The proc-macro expansion routes all generated code
//! through `::pvm_contract_sdk::` (the same pattern as ink!).
//!
//! `polkavm-derive` is still required as a direct dependency because the
//! `#[polkavm_export]` attribute macro generates code that references its
//! own crate internally.
//!
//! # Quick start
//!
//! ```toml
//! [dependencies]
//! pvm-contract-sdk = "0.3"
//! polkavm-derive = "0.34"
//! ```
//!
//! ```ignore
//! use pvm_contract_sdk::prelude::*;
//!
//! #[pvm_contract_sdk::contract("MyToken.sol")]
//! mod my_token {
//!     use super::*;
//!
//!     #[pvm_contract_sdk::constructor]
//!     pub fn new() -> Result<(), Error> { Ok(()) }
//!
//!     #[pvm_contract_sdk::method]
//!     pub fn total_supply() -> U256 { U256::ZERO }
//! }
//! ```
#![cfg_attr(not(feature = "std"), no_std)]

// Ensure `::pvm_contract_sdk` resolves everywhere — including inside this
// crate's own doc-tests.  Same pattern as ink!'s `extern crate self as ink;`.
extern crate self as pvm_contract_sdk;

// ---------------------------------------------------------------------------
// Proc macro re-exports
// ---------------------------------------------------------------------------

pub use pvm_contract_macros::{
    SolError, SolEvent, SolStorage, SolType, abi_import, constructor, contract, fallback, method,
    payable, receive, storage,
};

// ---------------------------------------------------------------------------
// Dependency re-exports for user code
// ---------------------------------------------------------------------------

/// Re-exported for `#[pvm_contract_sdk::polkavm_export]` in advanced use cases.
pub use polkavm_derive;
pub use polkavm_derive::polkavm_export;

/// Re-exported for direct access to `ruint` types beyond `U256`.
pub use ruint;

// ---------------------------------------------------------------------------
// Types and traits
// ---------------------------------------------------------------------------

pub use pvm_contract_types::{
    // Primitives
    Address,
    // Host API
    CallFlags,
    // Encoding / decoding
    ConstStr,
    Context,
    // Mutation gating
    ContractContext,
    DecodeError,
    // Error traits and types
    EmptyError,
    EventTopics,
    Host,
    HostApi,
    HostResult,
    I256,
    Panic,
    ParseI256Error,
    PolkaVmHost,
    ReturnErrorCode,
    ReturnFlags,
    RevertString,
    // Dispatch
    Router,
    SolArrayElement,
    SolDecode,
    SolDefaultError,
    SolEncode,
    SolError,
    SolEvent,
    StaticDecode,
    StaticEncodedLen,
    StaticStorageDecode,
    StaticStorageEncode,
    StorageArrayElement,
    StorageDecode,
    StorageEncode,
    StorageFlags,
    StoragePackable,
    U256,
    // Checked decode-offset helpers (used by `#[derive(SolType)]` codegen)
    checked_sum,
    const_keccak256,
    const_selector,
    // Framework errors
    framework_errors,
    keccak256,
    // Storage-layout walker wrapper (StorageEncode family) used by codegen
    layout_step_encode,
    read_word_offset,
    value_transferred_is_nonzero,
};

/// Sealing module re-exported for the `#[contract]` macro to implement on
/// generated storage structs. External users have no reason to import this.
#[doc(hidden)]
pub use pvm_contract_types::__private;

// Cross-contract calls
pub use pvm_contract_core::call::{
    CallBuilder, CallError, CallLimits, NonPayable, Payable, Pure, RefTimeAndProofSizeLimits,
    StateMutability, View,
};

// Typed storage helpers. `Lazy<T>` / `Mapping<K, V>` cover both static
// 32-byte values (`U256`, `Address`, `[u8; 32]`, …) and dynamic ones
// (`String`, `Bytes`, structs with dynamic fields) through their
// `StorageEncode`/`StorageDecode` impls. `StorageVec<T>` models Solidity's
// `T[]` dynamic arrays. `Vec<u8>` is intentionally not a storage value —
// use `Bytes` for `bytes`-shaped storage (`Vec<u8>` is ABI `uint8[]`, a
// different on-chain layout). `StorageComponent` is the trait typed
// storage helpers implement to participate in auto-numbered slot layout.
pub use pvm_storage::{
    AsStorageKey, LayoutStep, Lazy, MAX_STATIC_SLOTS, Mapping, Ref, RefMut, StorageComponent,
    StorageKey, StorageVec, layout_step, layout_step_component,
};

#[cfg(feature = "abi-gen")]
pub use pvm_storage::{StorageLayoutEmit, join_label};

#[cfg(feature = "alloc")]
pub use pvm_contract_types::Bytes;

#[cfg(feature = "abi-gen")]
pub use pvm_contract_types::{
    AbiEventParam, AbiItem, AbiJson, AbiParam, StorageLayout, StorageLayoutEntry, StorageTypeName,
    abi_to_json, parse_type_str, storage_layout_to_json,
};

#[cfg(feature = "std")]
pub use pvm_contract_types::{Halt, MockHost, MockHostBuilder};

/// Full access to the types crate for advanced use cases.
pub use pvm_contract_types as types;

/// Storage codec helpers used by macro-generated impls (kept under a public
/// path so generated `::pvm_contract_sdk::storage_codec::static_*` calls
/// resolve in downstream crates).
pub use pvm_contract_types::storage_codec;

// ---------------------------------------------------------------------------
// Hidden re-exports for macro-generated code
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Hidden re-exports used by generated code
// ---------------------------------------------------------------------------

#[doc(hidden)]
pub use pvm_contract_types::const_format;

/// Re-exported so macro-generated `call()` / `deploy()` boundaries can call
/// `HostFnImpl::return_value` via the SDK without users depending on
/// `pallet-revive-uapi` directly.
#[doc(hidden)]
pub use pvm_contract_types::pallet_revive_uapi;

#[cfg(feature = "abi-gen")]
#[doc(hidden)]
pub use pvm_contract_types::serde_json;

// ---------------------------------------------------------------------------
// Prelude — flatten the most common imports
// ---------------------------------------------------------------------------

/// Common imports for PVM contract development.
///
/// ```ignore
/// use pvm_contract_sdk::prelude::*;
/// ```
pub mod prelude {
    pub use crate::{
        Address,
        DecodeError,
        // Errors
        EmptyError,
        // Host
        Host,
        HostApi,
        I256,
        PolkaVmHost,
        ReturnFlags,
        // Encoding
        SolDecode,
        SolEncode,
        // Error traits
        SolError,
        StaticEncodedLen,
        StorageFlags,
        U256,
    };
}

#[cfg(test)]
#[cfg(feature = "alloc")]
mod test {
    extern crate alloc;
    use super::*;
    fn assert_decode_at_handles_offset<T: SolError + core::fmt::Debug + PartialEq>(value: T) {
        let needed = value.encoded_size();
        const PREFIX: usize = 16;
        let mut buf = alloc::vec![0xAAu8; PREFIX + needed];
        let written = value.encode_to(&mut buf[PREFIX..]);
        assert_eq!(
            written, needed,
            "encode_to length disagrees with encoded_size"
        );

        let decoded = T::decode_at(&buf, PREFIX)
            .expect("decode_at returned DecodeError")
            .expect("selector did not match at offset");
        assert_eq!(decoded, value, "decode_at(input, offset) did not roundtrip");
    }

    #[test]
    fn panic_offset() {
        assert_decode_at_handles_offset(Panic::Overflow);
    }
    #[test]
    fn call_error_offset() {
        assert_decode_at_handles_offset(CallError::TransferFailed);
    }
    #[test]
    fn decode_error_offset() {
        assert_decode_at_handles_offset(DecodeError);
    }
    #[test]
    fn revert_string_offset() {
        assert_decode_at_handles_offset(RevertString("msg".into()));
    }
    #[test]
    fn custom_struct_offset() {
        #[derive(Debug, PartialEq, SolError)]
        pub struct InsufficientBalance {
            account: Address,
            required: U256,
            available: U256,
        }

        assert_decode_at_handles_offset(InsufficientBalance {
            account: Address([0x42; 20]),
            required: U256::from(1000u64),
            available: U256::from(500u64),
        });
    }
}

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
//! polkavm-derive = "0.31"
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
    SolError, SolType, abi_import, constructor, contract, fallback, method, payable,
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
    // Error traits and types
    EmptyError,
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
    SolRevert,
    StaticEncodedLen,
    StorageFlags,
    U256,
    const_selector,
    // Framework errors
    framework_errors,
    sol_revert_enum,
    value_transferred_is_nonzero,
};

// Cross-contract calls
pub use pvm_contract_core::call::{
    CallBuilder, CallError, CallLimits, NonPayable, Payable, Pure, RefTimeAndProofSizeLimits,
    StateMutability, View,
};

// Typed storage helpers. `Lazy`/`Mapping` are the declarable field types for
// `#[slot(N)]` fields on the contract struct.
pub use pvm_storage::{AsStorageKey, Lazy, Mapping, StorageKey};

#[cfg(feature = "abi-gen")]
pub use pvm_storage::StorageLayoutType;

#[cfg(feature = "alloc")]
pub use pvm_contract_types::Bytes;

#[cfg(feature = "abi-gen")]
pub use pvm_contract_types::{
    AbiItem, AbiJson, AbiParam, StorageLayout, StorageLayoutEntry, abi_to_json, parse_type_str,
    storage_layout_to_json,
};

#[cfg(feature = "std")]
pub use pvm_contract_types::{MockHost, MockHostBuilder};

/// Full access to the types crate for advanced use cases.
pub use pvm_contract_types as types;

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
        SolRevert,
        StaticEncodedLen,
        StorageFlags,
        U256,
    };
}

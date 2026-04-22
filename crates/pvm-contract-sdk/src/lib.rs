//! SDK for building Rust smart contracts targeting PolkaVM.
//!
//! This is the primary user-facing entry point for the macro-based contract API.
//! It re-exports proc macros, ABI encoding traits, host API wrappers, and
//! primitive types. Note, however, that the current proc-macro expansion still
//! references `pvm-contract-types` and `polkavm-derive` by absolute crate path,
//! so contract crates must currently include those dependencies directly in
//! `Cargo.toml` as well.
//!
//! # Quick start
//!
//! ```toml
//! [dependencies]
//! pvm-contract-sdk = "0.3"
//! pvm-contract-types = "0.3"
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

// ---------------------------------------------------------------------------
// Proc macro re-exports
// ---------------------------------------------------------------------------

pub use pvm_contract_macros::{
    SolError, SolType, abi_import, constructor, contract, fallback, method,
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
    SolError as SolErrorTrait,
    SolRevert,
    StaticEncodedLen,
    StorageFlags,
    U256,
    const_selector,
    // Framework errors
    framework_errors,
    sol_revert_enum,
};

#[cfg(feature = "alloc")]
pub use pvm_contract_types::Bytes;

#[cfg(feature = "abi-gen")]
pub use pvm_contract_types::{AbiItem, AbiJson, AbiParam, abi_to_json, parse_type_str};

#[cfg(feature = "std")]
pub use pvm_contract_types::{MockHost, MockHostBuilder};

/// Full access to the types crate for advanced use cases.
pub use pvm_contract_types as types;

// ---------------------------------------------------------------------------
// Hidden re-exports used by generated code
// ---------------------------------------------------------------------------

#[doc(hidden)]
pub use pvm_contract_types::const_format;

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
        // Host
        HostApi,
        I256,
        PolkaVmHost,
        ReturnFlags,
        // Encoding
        SolDecode,
        SolEncode,
        StaticEncodedLen,
        StorageFlags,
        U256,
    };
}

//! Contract Dependency Manager (CDM) integration for `cargo-pvm-contract`.
//!
//! CDM publishes contracts under human-readable names (`@ns/name`) and resolves
//! those names to on-chain addresses via a registry contract. This crate layers
//! that resolution on top of the SDK's [`abi_import!`] output without the SDK
//! having to know CDM exists.
//!
//! # Producer side
//!
//! A contract declares its CDM identity in `Cargo.toml`:
//!
//! ```toml
//! [package.metadata.cdm]
//! package = "@example/hello-world"
//! ```
//!
//! No attribute on the `#[contract]` macro, no ELF symbol, no SDK changes.
//! `cdm deploy` reads the mapping from `Cargo.toml` directly.
//!
//! # Consumer side
//!
//! Attach CDM resolution to an `abi_import!`-ed contract:
//!
//! ```ignore
//! pvm_contract_sdk::abi_import! {
//!     #![abi_import(alloc = true)]
//!     foo,
//!     "../echo/target/release/foo.abi.json"
//! }
//!
//! pvm_cdm::reference!(foo::Foo, "@example/foo");
//!
//! // At call sites:
//! let handle = foo::Foo::cdm_lookup();     // runtime registry call
//! let handle = foo::Foo::cdm_from_env();   // compile-time baked address
//! ```
//!
//! The macro emits two inherent methods on the imported contract type:
//!
//! - `cdm_lookup()` — calls `ContractRegistry.getAddress(string)` via
//!   [`PolkaVmHost::call_evm`]. Registry address comes from the
//!   `CONTRACTS_REGISTRY_ADDR` env var (baked at compile time).
//! - `cdm_from_env()` — bakes the address directly from the `CDM_REGISTRY` env
//!   var (a `name=hex;name=hex;...` mapping). Zero runtime overhead.
//!
//! Both env vars are optional. Each resolver only requires its own env var;
//! contracts using only one of the two still compile when the other is unset.
//!
//! [`abi_import!`]: https://docs.rs/pvm-contract-sdk
//! [`PolkaVmHost::call_evm`]: https://docs.rs/pvm-contract-sdk

#![no_std]

pub use pvm_cdm_macros::reference;

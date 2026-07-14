//! Differential test of the SDK's **storage representation** against real solc.
//!
//! [`storage`] proves an SDK contract writes the same on-chain bytes solc does.
//! For each fixture it declares a real `#[contract]` (whose storage layout the
//! macro computes, exactly as solc does for the equivalent Solidity), builds it
//! with the macro-generated `Contract::with_host(mock)`, runs a `populate()`
//! method, and dumps the backing `MockHost`; it then compiles the equivalent
//! `.sol`, executes it on `revm`, and compares the two `{slot -> 32 bytes}`
//! maps. This validates the macro's field placement AND the byte-level encoding
//! end-to-end — packed read-modify-write, mapping key derivation, dynamic
//! `string`/`bytes` inline-vs-spilled, `StorageVec`, fixed-array striping,
//! clearing/deletion, signed two's-complement, and sub-word spill.
//!
//! The complementary storage-*layout* differential (our emitted `storageLayout`
//! JSON vs solc's) lives in `pvm-contract-macros/tests/solc_differential.rs`.
//!
//! Gated behind the `solc-tests` feature (needs `solc` on PATH; pulls `revm`).
//! Also gated `not(feature = "abi-gen")`: the fixtures call `#[method]`s, which
//! the `#[contract]` macro cfg's out under `abi-gen`. Run:
//!
//! ```text
//! cargo test -p pvm-solc-differential --features solc-tests
//! ```

#[cfg(all(test, feature = "solc-tests", not(feature = "abi-gen")))]
mod common;
#[cfg(all(test, feature = "solc-tests", not(feature = "abi-gen")))]
mod storage;

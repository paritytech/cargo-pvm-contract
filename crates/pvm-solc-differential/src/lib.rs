//! Differential test of the SDK's **storage representation** against real solc.
//!
//! [`storage`] proves `pvm-storage` writes the same on-chain bytes solc does:
//! it drives our storage layer, dumps the backing `MockHost`, then compiles the
//! equivalent Solidity, executes it on `revm`, and compares the resulting
//! `{slot -> 32 bytes}` maps. This catches encoding bugs the static layout JSON
//! can't — packed-field read-modify-write, mapping key derivation, dynamic
//! `string`/`bytes` inline-vs-spilled, `StorageVec`, fixed-array striping,
//! clearing/deletion, and signed two's-complement.
//!
//! The complementary storage-*layout* differential (our emitted `storageLayout`
//! JSON vs solc's) lives in `pvm-contract-macros/tests/solc_differential.rs`.
//!
//! Gated behind the `solc-tests` feature (needs `solc` on PATH; pulls `revm`).
//! Run:
//!
//! ```text
//! cargo test -p pvm-solc-differential --features solc-tests
//! ```

#[cfg(all(test, feature = "solc-tests"))]
mod common;
#[cfg(all(test, feature = "solc-tests"))]
mod storage;

//! Differential tests of the SDK against the real Solidity compiler.
//!
//! One home for every "do we match solc?" check. solc owns the rules; we verify
//! our output against it from two angles:
//!
//! - [`layout`] — **storage layout** (static): the macro's emitted
//!   `storageLayout` JSON vs solc's `storageLayout` JSON (slot / offset / type
//!   per field). Catches a layout bug that would otherwise be baked into both
//!   our walker and a hand-authored golden file. solc only (no execution).
//! - [`storage`] — **storage representation** (dynamic): the actual on-chain
//!   bytes `pvm-storage` writes vs the bytes solc's bytecode writes when
//!   executed on `revm`. Catches encoding bugs the static layout can't —
//!   packed-field read-modify-write, mapping key derivation, dynamic
//!   `string`/`bytes` inline-vs-spilled, `StorageVec`, fixed-array striping.
//!
//! Both share the solc invocation in [`common`]. Gated behind the `solc-tests`
//! feature (needs `solc` on PATH; pulls `revm` for the storage module). Run:
//!
//! ```text
//! cargo test -p pvm-solc-differential --features solc-tests
//! ```
//!
//! A natural third module — `abi` (calldata/return ABI encoding vs solc/alloy)
//! — can slot in here later alongside these two.

#[cfg(all(test, feature = "solc-tests"))]
mod common;
#[cfg(all(test, feature = "solc-tests"))]
mod layout;
#[cfg(all(test, feature = "solc-tests"))]
mod storage;

//! Reentrancy guard primitives backing the `#[non_reentrant]` modifier.
//!
//! The lock lives at a fixed, namespaced storage key
//! (`keccak256("pvm.guards.reentrancy")`, ERC-7201 style) — **not** in the
//! contract's declared storage layout. This keeps it out of the auto-numbered
//! slot chain and the `storageLayout` ABI, and makes it collision-free with
//! user storage. Presence (a non-zero value) means "locked".
//!
//! **The lock must live in storage, not an in-memory flag:** PVM gives each call
//! fresh memory, so only the on-chain write is visible to a re-entrant frame.
//!
//! **The lock is cleared by the dispatch codegen before `return_value`, never via
//! `Drop`.** On-chain `return_value` terminates the call without unwinding, so a
//! `Drop` guard would never run and the lock would stay set forever, bricking the
//! contract. See the `#[non_reentrant]` codegen in `pvm-contract-macros`.

use crate::{DecodeError, Host, HostApi, SolError, StorageFlags, const_keccak256, const_selector};

/// Fixed storage slot for the reentrancy lock (ERC-7201-style namespaced key).
const REENTRANCY_KEY: [u8; 32] = const_keccak256(b"pvm.guards.reentrancy");

/// Non-zero marker written to lock the guard. Reading back non-zero ⇒ locked.
const LOCKED: [u8; 32] = [1u8; 32];

/// All-zero value: `set_storage_or_clear` auto-deletes the slot, so the lock
/// reads back as zero (unlocked).
const UNLOCKED: [u8; 32] = [0u8; 32];

/// OpenZeppelin-compatible reentrancy error.
///
/// Selector matches OZ v5 `error ReentrancyGuardReentrantCall();`, so Foundry /
/// Etherscan decode a `#[non_reentrant]` revert as the familiar OZ error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReentrancyGuardReentrantCall;

impl SolError for ReentrancyGuardReentrantCall {
    const SELECTOR: [u8; 4] = const_selector("ReentrancyGuardReentrantCall()");
    const SIGNATURE: &'static str = "ReentrancyGuardReentrantCall()";

    fn encoded_size(&self) -> usize {
        4
    }

    fn encode_to(&self, buf: &mut [u8]) -> usize {
        buf[0..4].copy_from_slice(&Self::SELECTOR);
        4
    }

    fn decode_at(input: &[u8], offset: usize) -> Result<Option<Self>, DecodeError> {
        if input.len() < 4 {
            return Err(DecodeError);
        }
        if input
            .get(offset..offset + 4)
            .is_some_and(|x| x == Self::SELECTOR)
        {
            Ok(Some(Self))
        } else {
            Ok(None)
        }
    }
}

/// Whether the reentrancy lock is currently held.
///
/// The dispatch codegen calls this and, on `true`, reverts **inline** with
/// `ReentrancyGuardReentrantCall` via `return_value` + an explicit `return`
/// from the route.
#[doc(hidden)]
pub fn __reentrancy_is_locked(host: &Host) -> bool {
    let mut buf = [0u8; 32];
    host.get_storage_or_zero(StorageFlags::empty(), &REENTRANCY_KEY, &mut buf);
    buf != UNLOCKED
}

/// Set the reentrancy lock (full-guard entry, after the not-locked check).
#[doc(hidden)]
pub fn __reentrancy_lock(host: &Host) {
    let _ = host.set_storage_or_clear(StorageFlags::empty(), &REENTRANCY_KEY, &LOCKED);
}

/// Clear the reentrancy lock (full-guard exit).
///
/// The dispatch codegen calls this explicitly after the user body returns and
/// before the `return_value`.
#[doc(hidden)]
pub fn __reentrancy_unlock(host: &Host) {
    let _ = host.set_storage_or_clear(StorageFlags::empty(), &REENTRANCY_KEY, &UNLOCKED);
}

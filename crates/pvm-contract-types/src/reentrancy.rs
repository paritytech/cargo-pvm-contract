//! Reentrancy guard primitives backing the `#[non_reentrant]` modifier.
//!
//! The lock lives in transient storage (EIP-1153) at a fixed, namespaced key
//! (`keccak256("pvm.guards.reentrancy")`, ERC-7201 style), outside the contract's
//! declared storage layout, so it stays out of the auto-numbered slot chain and
//! the `storageLayout` ABI and can't collide with user storage. Presence (a
//! non-zero value) means "locked". Transient is the right fit: it's shared across
//! the call stack within a transaction (a re-entrant frame sees the lock), cheaper
//! than a persistent `SSTORE`, and auto-cleared at transaction end, so a stuck
//! lock can't brick the contract across transactions. It must be on-chain, not an
//! in-memory flag: PVM gives each call fresh memory, so only a storage write is
//! visible to a re-entrant frame.
//!
//! The lock must be released when a guarded call exits: transient storage
//! persists across *sequential* (non-nested) calls within a transaction, so a
//! stale lock would make a later guarded call in the same transaction revert
//! spuriously (as in OpenZeppelin's `ReentrancyGuardTransient`).
//!
//! Two mechanisms release it. The `#[non_reentrant]` codegen emits an explicit
//! unlock after the user body; this covers a normal return, and is the only
//! path on host targets. A body can also exit by calling `return_value` itself
//! (a diverging `-> !` syscall), which skips that trailing unlock; a `Drop`
//! guard can't help either, since the syscall diverges without unwinding so no
//! destructor runs. To cover that on-chain, entry raises a frame-local flag
//! (`REENTRANCY_LOCK_HELD`) and `return_value` releases the lock when the flag
//! is still set, catching a divergent exit that skipped the trailing unlock.
//! PVM gives each call a fresh memory image, so the flag is `false` at the top
//! of every frame and marks exactly the frame that took the lock, with no key
//! derivation or depth counter. This flag path is riscv64-only: on host a
//! process-global `static` would leak across parallel tests, and there
//! `return_value` returns normally so the explicit unlock already suffices.

use crate::{DecodeError, Host, HostApi, SolError, StorageFlags, const_keccak256, const_selector};

/// Frame-local "this frame owns the lock" flag (see module docs). Reset to
/// `false` by PVM's fresh per-call memory image, so no explicit init is needed.
///
/// A plain `static mut` (not an atomic): on-chain execution is single-threaded,
/// and a plain flag stays transparent to the optimizer. In a guardless contract
/// (which never calls [`__reentrancy_lock`]) LTO then dead-code-eliminates the
/// whole clear branch, so it pays nothing.
#[cfg(target_arch = "riscv64")]
static mut REENTRANCY_LOCK_HELD: bool = false;

/// Fixed storage slot for the reentrancy lock (ERC-7201-style namespaced key).
const REENTRANCY_KEY: [u8; 32] = const_keccak256(b"pvm.guards.reentrancy");

/// Non-zero marker written to lock the guard. Reading back non-zero means locked.
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
    host.get_storage_or_zero(StorageFlags::TRANSIENT, &REENTRANCY_KEY, &mut buf);
    buf != UNLOCKED
}

/// Set the reentrancy lock (full-guard entry, after the not-locked check).
#[doc(hidden)]
pub fn __reentrancy_lock(host: &Host) {
    // SAFETY: single-threaded on-chain execution; no concurrent access.
    #[cfg(target_arch = "riscv64")]
    unsafe {
        REENTRANCY_LOCK_HELD = true;
    }
    let _ = host.set_storage_or_clear(StorageFlags::TRANSIENT, &REENTRANCY_KEY, &LOCKED);
}

/// Clear the reentrancy lock (full-guard exit).
///
/// The dispatch codegen calls this explicitly after the user body returns and
/// before the `return_value`.
#[doc(hidden)]
pub fn __reentrancy_unlock(host: &Host) {
    // SAFETY: single-threaded on-chain execution; no concurrent access.
    #[cfg(target_arch = "riscv64")]
    unsafe {
        REENTRANCY_LOCK_HELD = false;
    }
    let _ = host.set_storage_or_clear(StorageFlags::TRANSIENT, &REENTRANCY_KEY, &UNLOCKED);
}

/// On-chain safety net for a body that exits via a raw diverging `return_value`,
/// skipping the codegen's post-body [`__reentrancy_unlock`]. Called from inside
/// `return_value`, which every contract exit routes through: if this frame holds
/// the lock, release it before diverging. A no-op for any frame that never took
/// the lock. In a guardless contract the whole branch is dead-code eliminated
/// under LTO.
#[cfg(target_arch = "riscv64")]
#[doc(hidden)]
pub fn __reentrancy_clear_if_held(host: &impl HostApi) {
    // SAFETY: single-threaded on-chain execution; no concurrent access. In a
    // guardless contract `__reentrancy_lock` is never linked, so LTO proves this
    // is always `false` and eliminates the whole branch.
    if unsafe { REENTRANCY_LOCK_HELD } {
        unsafe {
            REENTRANCY_LOCK_HELD = false;
        }
        let _ = host.set_storage_or_clear(StorageFlags::TRANSIENT, &REENTRANCY_KEY, &UNLOCKED);
    }
}

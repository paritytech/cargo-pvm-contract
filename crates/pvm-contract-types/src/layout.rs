//! Const-folded storage-layout walker, shared across the whole SDK.
//!
//! These items live in `pvm-contract-types` (the leaf crate) so that **every**
//! layout computation goes through one algorithm: the tuple `StorageEncode`
//! impls in this crate, the `#[derive(SolStorage)]` field walker, and the
//! `#[contract]` / `#[storage]` macro chains all consume the same
//! [`layout_step`]. Previously `layout_step` lived in `pvm-storage`, which
//! forced the lower-crate tuple impls to hand-roll a shadow copy of the packing
//! rule that could silently drift.
//!
//! `pvm-storage` re-exports these so existing `pvm_storage::layout_step` /
//! `pvm_storage::MAX_STATIC_SLOTS` paths keep resolving.

/// Maximum number of 32-byte slots a single static `Lazy<T>` / `Mapping<K, V>`
/// value can occupy. 8 slots = 256 bytes is enough for typical record types
/// (e.g. `(Address, U256, U256)`) without allocating heap or requiring
/// `feature(generic_const_exprs)` to size the stack buffer by
/// `T::STORAGE_SLOTS`.
///
/// Increase this if a contract needs larger inline static values, but never
/// raise it beyond `pallet-revive`'s `STORAGE_BYTES` limit (currently 416 bytes
/// = 13 slots) — that's the hard cap the runtime enforces per storage value,
/// so any larger buffer here would fail at host-call time on chain.
pub const MAX_STATIC_SLOTS: usize = 8;

/// One step in the const-folded contract-field layout walker.
///
/// Used by the `#[contract]` and `#[storage]` macros to compute each field's
/// placement at compile time. The walker carries the chain state as a
/// `LayoutStep`: the placement of the current field plus the entry conditions
/// for the next one. See [`layout_step`] for the algorithm.
#[derive(Copy, Clone)]
pub struct LayoutStep {
    /// Slot the current field starts at.
    pub slot: u64,
    /// Byte offset within `slot` where the current field begins.
    pub offset: u8,
    /// Slot the next field's chain step should start from.
    pub next_slot: u64,
    /// Bytes remaining in `next_slot` (32 if `next_slot` is fresh, 0 if
    /// the current field consumed the slot to its end).
    pub next_space: u8,
}

impl LayoutStep {
    /// Sentinel value used to seed the chain for the first field.
    pub const FIRST: LayoutStep = LayoutStep {
        slot: 0,
        offset: 0,
        next_slot: 0,
        next_space: 32,
    };
}

/// Compute one step of the contract-field layout walker, given the chain
/// state from the previous step and this field's `PACKED_BYTES` + `SLOTS`.
///
/// Mirrors solc's layout rule: a field starts on the current slot if it has
/// enough remaining bytes, else advances to the next fresh slot. Multi-slot
/// composites (`SLOTS > 1`) always claim from the start of a fresh slot and
/// consume to its end.
///
/// This is the SHARED const-fn used by every walker site so the
/// contract-field chain (`contract.rs`), the `#[storage]` sub-struct chain
/// (`sol_storage.rs`), the SolType-derive struct walker (`sol_type.rs`), and
/// the tuple `StorageEncode` impls (`storage_codec.rs`) agree on layout
/// byte-for-byte.
pub const fn layout_step(prev: LayoutStep, packed_bytes: usize, slots: u64) -> LayoutStep {
    let bytes = packed_bytes as u8;
    // Decide whether the current field fits in `prev.next_slot` or must
    // advance to a fresh slot.
    let (slot, space) = if prev.next_space < bytes {
        (prev.next_slot + 1, 32u8)
    } else {
        (prev.next_slot, prev.next_space)
    };
    let offset = space - bytes;
    // Multi-slot composites: this field occupies `slots` consecutive slots
    // starting at `slot`, consuming the last one to its end.
    let (next_slot, next_space) = if slots > 1 {
        (slot + slots - 1, 0u8)
    } else {
        (slot, offset)
    };
    LayoutStep {
        slot,
        offset,
        next_slot,
        next_space,
    }
}

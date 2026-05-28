//! Typed storage helpers for PVM smart contracts with Solidity-compatible slot layout.
//!
//! Provides [`Lazy<T>`] for single-value storage and [`Mapping<K, V>`] for key-value
//! storage, both using Solidity-compatible key derivation so tools like `cast storage`
//! and `cast index` work out of the box.
//!
//! Static values use [`Lazy<T>`] and [`Mapping<K, V>`] with `T`/`V` bound to
//! `SolEncode + StaticDecode + StaticEncodedLen`. The value must have a
//! compile-time-known size that's a positive multiple of 32 and at most
//! [`MAX_STATIC_BYTES`]. Single-word values (`U256`, `Address`, `bool`,
//! `[u8; 32]`, …) occupy one slot; multi-word values like `(U256, U256)` or
//! `#[derive(SolType)]` structs are striped across `T::ENCODED_SIZE / 32`
//! consecutive slots, mirroring Solidity's struct-in-storage layout.
//!
//! Dynamic `bytes` / `string` values ride the same `Lazy<T>` / `Mapping<K, V>`
//! accessors as static types — `Lazy<String>`, `Lazy<Bytes>`,
//! `Mapping<K, String>`, `Mapping<K, Bytes>` encode inline when `len < 32` and
//! spill to `keccak256(slot) + i` chunks otherwise, matching `solc`'s storage
//! layout. `Vec<u8>` is intentionally **not** a storage value — its `SolEncode`
//! name is `"uint8[]"` (a different on-chain layout from Solidity `bytes`), so
//! `Lazy<Vec<u8>>` and `Mapping<K, Vec<u8>>` fail to compile. Use [`Bytes`]
//! ([`pvm_contract_types::Bytes`]) for `bytes`-shaped storage. `Vec<u8>` is
//! still a valid mapping *key* (`mapping(bytes => _)`) and a valid ABI param.
//!
//! All accessors implement [`StorageComponent`], so they participate in the
//! auto-numbered slot layout produced by the `#[contract]` and `#[storage]`
//! macros.
//!
//! # Field-level packing
//!
//! Adjacent sub-32-byte primitive fields share a single 32-byte slot,
//! matching solc's `storageLayout`. Two adjacent `Lazy<u128>` fields land
//! at `(slot=0, offset=16)` and `(slot=0, offset=0)` respectively — exactly
//! what solc emits for `uint128 a; uint128 b;`. The macro walker
//! ([`layout_step`]) is the const-fn that decides each field's placement.
//!
//! Packed writes are read-modify-write (one SLOAD + one SSTORE), matching
//! solc/Stylus. Full-slot writes are a single SSTORE — no overhead from the
//! packing infrastructure.
//!
//! Multi-slot composites (`Lazy<(U256, U256)>`, multi-slot
//! `#[derive(SolType)]` structs), mappings, and `#[storage]` sub-structs
//! always start a fresh slot and never pack with neighbours. They report
//! `PACKED_BYTES = 32`.
//!
//! # Usage
//!
//! Inside a `#[contract]` module, declare storage fields on the contract struct.
//! Slot numbers are assigned in declaration order by default; opt out with
//! `#[slot(N)]` if you need to pin a specific slot. The macro constructs each
//! field via the safe [`StorageComponent::new_at`] entry point.
//!
//! ```ignore
//! use pvm_storage::{Lazy, Mapping, StorageComponent};
//!
//! // The `#[contract]` macro emits calls like the lines below. Direct user
//! // code shouldn't need to construct handles by hand — use macro-managed
//! // storage fields and access them via `self.balances.get(&caller)` etc.
//! let mut total_supply = <Lazy<U256> as StorageComponent>::new_at(
//!     StorageKey::from_slot(0), 0, host.clone(),
//! );
//! total_supply.set(&U256::from(1000));
//! assert_eq!(total_supply.get(), U256::from(1000));
//!
//! let mut balances = <Mapping<Address, U256> as StorageComponent>::new_at(
//!     StorageKey::from_slot(1), 0, host,
//! );
//! balances.insert(&caller, &U256::from(500));
//! assert_eq!(balances.get(&caller), U256::from(500));
//! ```
//!
//! `Lazy::new` and `Mapping::new` themselves are `unsafe fn` — direct
//! construction lets a `&self` (view) method bypass the borrow-check
//! mutation gate. See their docs for the safety contract.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

// Alias so that macro-generated `::pvm_contract_sdk::` paths resolve
// within this crate's own tests. Same pattern as pvm-contract-types.
extern crate self as pvm_contract_sdk;

use core::marker::PhantomData;
use pvm_contract_types::{Host, HostApi, SolEncode, StorageDecode, StorageEncode, StorageFlags};

// ---------------------------------------------------------------------------
// Shared inner functions: type-erased helpers that operate on raw [u8; 32].
// Each takes a `&Host` so the instance-based `HostApi` trait dispatch works.
// Benchmarked with/without #[inline(never)]: letting the compiler decide
// produced smaller .polkavm output so we omit the annotation.
// ---------------------------------------------------------------------------

fn storage_get_32(host: &Host, key: &[u8; 32]) -> [u8; 32] {
    let mut buf = [0u8; 32];
    host.get_storage_or_zero(StorageFlags::empty(), key, &mut buf);
    buf
}

fn storage_set_32(host: &Host, key: &[u8; 32], value: &[u8; 32]) {
    host.set_storage_or_clear(StorageFlags::empty(), key, value);
}

fn storage_derive_key(host: &Host, root: &[u8; 32], padded_key: &[u8; 32]) -> [u8; 32] {
    let mut preimage = [0u8; 64];
    preimage[0..32].copy_from_slice(padded_key);
    preimage[32..64].copy_from_slice(root);
    let mut output = [0u8; 32];
    host.hash_keccak_256(&preimage, &mut output);
    output
}

// Dynamic-key variant: preimage is `raw_key ++ pad32(root)` (no key padding).
// Matches Solidity's `mapping(string => _)` / `mapping(bytes => _)` slot
// derivation, where the key bytes are hashed verbatim.
#[cfg(feature = "alloc")]
fn storage_derive_key_unpadded(host: &Host, root: &[u8; 32], key: &[u8]) -> [u8; 32] {
    let mut preimage = alloc::vec::Vec::with_capacity(key.len() + 32);
    preimage.extend_from_slice(key);
    preimage.extend_from_slice(root);
    let mut output = [0u8; 32];
    host.hash_keccak_256(&preimage, &mut output);
    output
}

/// Read a 32-byte slot, treating all-zero as "absent".
///
/// pallet-revive's Fix-keyed uapi only exposes `get_storage_or_zero`, which
/// returns zeros for both deleted and never-written slots. For Solidity-style
/// storage (which `pvm-storage` targets — see `resolc`) that conflation is
/// the correct semantics: SSTORE 0 deletes, SLOAD of missing returns 0,
/// and "set to 0" is indistinguishable from "never written". Dynamic
/// `bytes` / `string` accessors recover the "set empty vs never written"
/// distinction by storing a non-zero sentinel in the inline header.
///
/// Only referenced by dynamic-bytes code (alloc-gated) and tests; the static
/// `Lazy`/`Mapping` paths go through `storage_try_get_static_into` instead.
#[cfg(test)]
fn storage_try_get_32(host: &Host, key: &[u8; 32]) -> Option<[u8; 32]> {
    let buf = storage_get_32(host, key);
    (buf != [0u8; 32]).then_some(buf)
}

/// Hash a 32-byte slot to produce the data root for a dynamic value
/// (`keccak256(slot)`). This matches Solidity's layout for `bytes`, `string`,
/// and arrays.
#[cfg(test)]
fn dynamic_data_root(host: &Host, slot: &[u8; 32]) -> [u8; 32] {
    let mut output = [0u8; 32];
    host.hash_keccak_256(slot, &mut output);
    output
}

/// Increment a 32-byte big-endian integer in-place (used to walk consecutive
/// storage slots — both the body of dynamic values and multi-word static
/// values that span more than one slot).
fn inc_slot(slot: &mut [u8; 32]) {
    for byte in slot.iter_mut().rev() {
        let (next, carry) = byte.overflowing_add(1);
        *byte = next;
        if !carry {
            return;
        }
    }
}

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

/// Read `out.len()` consecutive slots starting at `key` into `out`.
fn read_slots(host: &Host, key: &[u8; 32], out: &mut [[u8; 32]]) {
    let mut k = *key;
    for slot in out.iter_mut() {
        *slot = storage_get_32(host, &k);
        inc_slot(&mut k);
    }
}

/// Read `out.len()` consecutive slots starting at `key`. Returns `None` iff
/// every slot read back as `[0; 32]` — matches Solidity's "value-zero ≡
/// deleted ≡ never-written" semantics aggregated across a multi-slot value.
fn try_read_slots(host: &Host, key: &[u8; 32], out: &mut [[u8; 32]]) -> Option<()> {
    let mut k = *key;
    let mut any_present = false;
    for slot in out.iter_mut() {
        let read = storage_get_32(host, &k);
        if read != [0u8; 32] {
            any_present = true;
        }
        *slot = read;
        inc_slot(&mut k);
    }
    any_present.then_some(())
}

/// Stream-encode `value` slot-by-slot and write to consecutive slots starting
/// at `key`. Uses a 32-byte stack buffer regardless of `T::STORAGE_SLOTS`.
fn write_value<T: StorageEncode>(host: &Host, key: &[u8; 32], value: &T) {
    let mut k = *key;
    for i in 0..T::STORAGE_SLOTS {
        let mut buf = [0u8; 32];
        value.encode_slot(i, &mut buf);
        storage_set_32(host, &k, &buf);
        inc_slot(&mut k);
    }
}

/// Clear `n` consecutive slots starting at `key`.
fn clear_n_slots(host: &Host, key: &[u8; 32], n: usize) {
    let mut k = *key;
    for _ in 0..n {
        host.set_storage_or_clear(StorageFlags::empty(), &k, &[0u8; 32]);
        inc_slot(&mut k);
    }
}

// ---------------------------------------------------------------------------
// StorageKey
// ---------------------------------------------------------------------------

/// A 32-byte storage key for Solidity-compatible slot addressing.
///
/// Use [`from_slot`](StorageKey::from_slot) for root slots and
/// [`derive`](StorageKey::derive) for mapping key derivation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StorageKey([u8; 32]);

impl StorageKey {
    /// Create a key from a slot number.
    ///
    /// Solidity slots are uint256 (32 bytes, big-endian). A u64 slot number is
    /// zero-padded on the left to fill the full 32 bytes, so slot 1 becomes
    /// `[0,0,...,0,0,0,1]`.
    pub const fn from_slot(slot: u64) -> Self {
        let mut key = [0u8; 32];
        let bytes = slot.to_be_bytes();
        let mut i = 0;
        while i < 8 {
            key[24 + i] = bytes[i];
            i += 1;
        }
        StorageKey(key)
    }

    /// Construct from raw 32 bytes. Internal: callers must ensure the bytes
    /// already represent a valid slot identifier.
    #[doc(hidden)]
    pub const fn from_raw(bytes: [u8; 32]) -> Self {
        StorageKey(bytes)
    }

    /// Derive a mapping child key following Solidity's key derivation convention.
    ///
    /// For scalar keys: `keccak256(pad32(key) ++ self)` (one keccak).
    /// For tuple keys: chained derivation matching Solidity's nested mappings.
    /// Uses the host keccak function for native speed.
    pub fn derive<K: AsStorageKey>(&self, host: &Host, map_key: &K) -> Self {
        map_key.derive_slot(host, self)
    }

    /// Raw access to the 32-byte key for debugging and host API interop.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Add a small offset, propagating carries across all 32 bytes.
    ///
    /// Used to position sub-fields of a `#[storage]` struct rooted at a
    /// derived key — e.g. a `Mapping<K, MyStorageStruct>::entry(k)` constructs
    /// the inner struct at `derived = keccak256(pad32(k) ++ root)`, and the
    /// struct's macro-generated constructor positions each field at
    /// `derived.add(N)` where `N` is the field's slot offset within the
    /// struct as computed by the `LayoutStep` walker.
    ///
    /// For a contract-field StorageKey produced by `from_slot(s)`, `add(N)`
    /// equals `from_slot(s + N)` modulo 64-bit wrap of `s + N`. For a
    /// derived key, `add` performs proper 256-bit big-endian addition. Wrap
    /// past `2^256 - 1` is not handled — derived keys are uniformly
    /// distributed in the 256-bit space, so wrap is effectively impossible
    /// at any realistic depth.
    pub fn add(self, n: u64) -> StorageKey {
        let mut out = self.0;
        // Add `n` into the low 8 bytes (bytes 24..32) with carry up.
        let mut carry: u64 = n;
        let mut i = 31i32;
        while i >= 0 && carry > 0 {
            let sum = out[i as usize] as u64 + (carry & 0xff);
            out[i as usize] = sum as u8;
            carry = (carry >> 8) + (sum >> 8);
            i -= 1;
        }
        StorageKey(out)
    }
}

// ---------------------------------------------------------------------------
// AsStorageKey
// ---------------------------------------------------------------------------

/// Trait for types that can be used as mapping keys.
///
/// Each implementation derives the storage slot from a root key and the key
/// material. Scalar types (Address, U256, etc.) do one keccak derivation.
/// Tuple types chain derivations to match Solidity's nested mapping layout.
///
/// Dynamic types (String, bytes) require a different derivation formula
/// (`keccak256(raw_bytes)` before padding) and need their own `AsStorageKey`
/// impl and they cannot be added to `impl_scalar_storage_key!`.
pub trait AsStorageKey {
    /// Derive the storage slot from a root key and this key material.
    ///
    /// For scalars: `keccak256(pad32(self) ++ root)`.
    /// For tuples: chained derivation matching Solidity's nested mappings.
    fn derive_slot(&self, host: &Host, root: &StorageKey) -> StorageKey;
}

/// Implement `AsStorageKey` for static types that ABI-encode to exactly 32 bytes.
///
/// Each type produces one keccak derivation: `keccak256(pad32(value) ++ root_slot)`.
/// The padding uses `SolEncode::encode_body_to`, matching Solidity's convention
/// (integers are right-aligned big-endian, addresses are right-aligned zero-padded).
///
/// Only valid for 32-byte static types. Do NOT add dynamic types (String, Vec)
/// here. They use a different Solidity key derivation formula and need a
/// dedicated `AsStorageKey` impl.
macro_rules! impl_scalar_storage_key {
    ($($ty:ty),* $(,)?) => {$(
        impl AsStorageKey for $ty {
            fn derive_slot(&self, host: &Host, root: &StorageKey) -> StorageKey {
                let mut padded = [0u8; 32];
                SolEncode::encode_body_to(self, &mut padded);
                StorageKey(storage_derive_key(host, root.as_bytes(), &padded))
            }
        }
    )*}
}

// All 32-byte scalar types from pvm-contract-types.
// This list must match the types that implement SolEncode + StaticEncodedLen
// with ENCODED_SIZE == 32 in pvm-contract-types.
//
// Unsigned integers:
//   U256, u128, u64, u32, u16, u8
// Signed integers:
//   I256, i128, i64, i32, i16, i8
// Other:
//   bool, Address
use pvm_contract_types::{Address, I256, U256};

impl_scalar_storage_key!(
    // Unsigned integers
    U256, u128, u64, u32, u16, u8, // Signed integers
    I256, i128, i64, i32, i16, i8, // Other value types
    bool, Address,
);

// Fixed-size byte arrays [u8; N] encode as Solidity `bytesN` (left-aligned, 32 bytes).
// Common key sizes: bytes32 ([u8; 32]) for hashes, bytes20 ([u8; 20]) for raw addresses.
impl<const N: usize> AsStorageKey for [u8; N] {
    fn derive_slot(&self, host: &Host, root: &StorageKey) -> StorageKey {
        let mut padded = [0u8; 32];
        SolEncode::encode_body_to(self, &mut padded);
        StorageKey(storage_derive_key(host, root.as_bytes(), &padded))
    }
}

// Tuple keys for nested mappings.
// `Mapping<(Address, Address), U256>` produces the same slots as
// `Mapping<Address, Mapping<Address, U256>>` with chained get().get().
// Each tuple element derives one level, matching Solidity's Rule 3:
//   slot = keccak256(pad32(k2) ++ keccak256(pad32(k1) ++ root_slot))

macro_rules! impl_tuple_storage_key {
    ($first:ident : $idx0:tt $(, $rest:ident : $idx:tt)+) => {
        impl<$first: AsStorageKey $(, $rest: AsStorageKey)+> AsStorageKey for ($first, $($rest,)+) {
            fn derive_slot(&self, host: &Host, root: &StorageKey) -> StorageKey {
                let slot = self.$idx0.derive_slot(host, root);
                $(let slot = self.$idx.derive_slot(host, &slot);)+
                slot
            }
        }
    };
}

// Tuple key impls for arities 2–5. To support deeper nesting, add a line.
impl_tuple_storage_key!(A: 0, B: 1);
impl_tuple_storage_key!(A: 0, B: 1, C: 2);
impl_tuple_storage_key!(A: 0, B: 1, C: 2, D: 3);
impl_tuple_storage_key!(A: 0, B: 1, C: 2, D: 3, E: 4);

// Dynamic key types: Solidity's `mapping(string => _)` and `mapping(bytes => _)`
// derive slots as `keccak256(raw_bytes ++ pad32(root_slot))` — the key bytes are
// hashed verbatim with no padding. These impls are alloc-gated because building
// the preimage requires a heap buffer of `key.len() + 32` bytes.
//
// `str` and `[u8]` get impls so that future ergonomics (e.g. `Mapping::get_by`)
// can dispatch on them without requiring an owned key. Today, `Mapping<K, V>`
// still requires `K: Sized`, so users will declare `Mapping<String, V>` or
// `Mapping<Vec<u8>, V>`.
#[cfg(feature = "alloc")]
impl AsStorageKey for str {
    fn derive_slot(&self, host: &Host, root: &StorageKey) -> StorageKey {
        StorageKey(storage_derive_key_unpadded(
            host,
            root.as_bytes(),
            self.as_bytes(),
        ))
    }
}

#[cfg(feature = "alloc")]
impl AsStorageKey for [u8] {
    fn derive_slot(&self, host: &Host, root: &StorageKey) -> StorageKey {
        StorageKey(storage_derive_key_unpadded(host, root.as_bytes(), self))
    }
}

#[cfg(feature = "alloc")]
impl AsStorageKey for alloc::string::String {
    fn derive_slot(&self, host: &Host, root: &StorageKey) -> StorageKey {
        <str as AsStorageKey>::derive_slot(self.as_str(), host, root)
    }
}

#[cfg(feature = "alloc")]
impl AsStorageKey for alloc::vec::Vec<u8> {
    fn derive_slot(&self, host: &Host, root: &StorageKey) -> StorageKey {
        <[u8] as AsStorageKey>::derive_slot(self.as_slice(), host, root)
    }
}

/// Sentinel byte injected at `slot[30]` for an empty-inline dynamic value, so
/// the slot stays non-zero and survives `set_storage_or_clear`'s auto-delete.
/// Mirrors the canonical definition in `pvm-contract-types::storage_codec`;
/// duplicated here only for test assertions on the Solidity dynamic layout.
#[cfg(test)]
const EMPTY_INLINE_SENTINEL: u8 = 0x01;

// ---------------------------------------------------------------------------
// StorageComponent: how a typed storage object claims root slots.
// ---------------------------------------------------------------------------

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
/// (`sol_storage.rs`), and the SolType-derive struct walker (`sol_type.rs`)
/// agree on layout byte-for-byte.
pub const fn layout_step(prev: LayoutStep, packed_bytes: usize, slots: u64) -> LayoutStep {
    let bytes = packed_bytes as u8;
    // Decide whether the current field fits in `prev.next_slot` or must
    // advance to a fresh slot.
    let (slot, space) = if prev.next_space < bytes {
        (prev.next_slot + 1, 32u8)
    } else {
        (prev.next_slot, prev.next_space)
    };
    let space_after = space - bytes;
    let offset = space_after;
    // Multi-slot composites: this field occupies `slots` consecutive slots
    // starting at `slot`, consuming the last one to its end.
    let (next_slot, next_space) = if slots > 1 {
        (slot + slots - 1, 0u8)
    } else {
        (slot, space_after)
    };
    LayoutStep {
        slot,
        offset,
        next_slot,
        next_space,
    }
}

/// A typed storage helper that occupies one or more contiguous root slots.
///
/// Implementations:
///
/// - [`Lazy<T>`]      — 1 slot. `T` may be static (e.g. `U256`) or dynamic
///   (e.g. `String`, [`Bytes`](pvm_contract_types::Bytes)) with solc-compatible inline/spilled layout.
/// - [`Mapping<K,V>`] — 1 slot (the root; entries live at derived keys).
///   `V` may likewise be static or dynamic.
/// - user storage structs annotated with `#[storage]` — sum of their fields'
///   `SLOTS`, assigned in declaration order.
///
/// The `#[contract]` macro reads `SLOTS` to assign slot numbers to fields. The
/// macro-generated constructor calls [`StorageComponent::new_at`] with the
/// assigned base slot and a clone of the contract's host handle.
pub trait StorageComponent: Sized {
    /// Number of root storage slots claimed by this component.
    const SLOTS: u64;

    /// Number of bytes consumed within the component's *first* slot when it
    /// participates in field-level packing alongside siblings. `32` means the
    /// component always starts a fresh slot and claims it fully (the case for
    /// composites, mappings, dynamic-bodied types, and full-slot primitives).
    /// `< 32` means the component is a packable sub-word value and may share
    /// a slot with adjacent fields.
    const PACKED_BYTES: usize;

    /// Construct the component at `(key, offset)`, bound to `host`. `key` is
    /// the 32-byte storage key (a contract-field slot via
    /// [`StorageKey::from_slot`], or a derived key produced by a parent
    /// `Mapping`/`StorageVec`/`#[storage]` walker). `offset` is the byte
    /// position within `key`'s slot where the component begins; only
    /// meaningful when `PACKED_BYTES < 32` (the component packs with
    /// siblings). Full-slot components expect `offset == 0`.
    fn new_at(key: StorageKey, offset: u8, host: Host) -> Self;

    /// Clear every storage slot this component owns.
    ///
    /// Semantics per impl:
    ///
    /// - [`Lazy<T>`]: zero the slot(s) the value occupies. For sub-word
    ///   primitives this is a sub-slot RMW that preserves neighbours
    ///   (matches solc/Stylus). For dynamic types (`String`, `Bytes`),
    ///   clears the header AND any spilled body chunks.
    /// - [`Mapping<K, V>`]: **no-op**. Solidity mappings have no header to
    ///   clear — entries live at derived keys that can't be enumerated. If
    ///   you need to clear individual entries, call `delete` /
    ///   `view_mut(k).clear()` on each known key.
    /// - `#[storage]` sub-structs: recursively clear each field — matches
    ///   solc's `delete struct_field` semantics.
    ///
    /// Used by the storage-typed `Mapping<K, V: StorageComponent>::delete`
    /// to clear an entry of arbitrary inner shape.
    fn clear(&mut self);
}

// ---------------------------------------------------------------------------
// StorageLayoutEmit: per-struct hook for emitting layout JSON leaves.
// ---------------------------------------------------------------------------

/// Push flattened storage-layout entries for a composable storage component.
///
/// The `#[contract]` macro generates the top-level `__storage_layout_json()`
/// function by iterating contract-struct fields: leaf fields (`Lazy<T>` /
/// `Mapping<K, V>`) get inlined as single entries via the macro's syntactic
/// type resolver; embedded `#[storage]` sub-structs dispatch through this
/// trait, which recursively flattens their fields and prefixes each entry's
/// label with the field path (`erc20.total_supply`, `metadata.name`, …) to
/// match solc's storage-layout convention.
///
/// `#[storage]` auto-emits this impl. Hand-rolled storage components need to
/// implement it explicitly to participate in abi-gen layout output.
#[cfg(feature = "abi-gen")]
pub trait StorageLayoutEmit {
    /// Append entries for this component into `out`, rooted at `base` and
    /// prefixed by `name_prefix` (empty string at top level).
    fn emit_entries(
        base: u64,
        name_prefix: &str,
        out: &mut Vec<pvm_contract_types::StorageLayoutEntry>,
    );
}

/// Join `prefix` and `name` with a `.` separator, or return `name` alone when
/// `prefix` is empty. Used by macro-generated layout helpers to build dotted
/// field paths like `erc20.balances`.
#[cfg(feature = "abi-gen")]
pub fn join_label(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        String::from(name)
    } else {
        let mut out = String::with_capacity(prefix.len() + 1 + name.len());
        out.push_str(prefix);
        out.push('.');
        out.push_str(name);
        out
    }
}

// ---------------------------------------------------------------------------
// Lazy<T> — static 32-byte value at a fixed storage slot.
// ---------------------------------------------------------------------------

/// A single typed value at a fixed storage slot (or a contiguous range of
/// slots, for multi-word `T`).
///
/// "Lazy" because there is no caching: every [`get`](Lazy::get) reads from
/// host storage, every [`set`](Lazy::set) writes immediately.
///
/// Static `T` must have a compile-time-known size that's a positive multiple
/// of 32 and at most [`MAX_STATIC_BYTES`]. Single-word `T` (`U256`, `Address`,
/// `bool`, `[u8; 32]`, …) occupies one slot; an `N`-word `T` (e.g.
/// `(U256, U256)`, or a `#[derive(SolType)]` struct of static fields) is
/// striped across `N` consecutive slots starting at `self.key`, matching
/// Solidity's struct-in-storage layout.
///
/// Dynamic `T` (`String`, [`Bytes`](pvm_contract_types::Bytes), or
/// `#[derive(SolType)]` structs with dynamic fields) uses the same `Lazy<T>`
/// accessor: the header lives inline at `self.key` and any spilled body sits
/// at `keccak256(key) + i`. `Vec<u8>` is rejected at compile time — use
/// [`Bytes`](pvm_contract_types::Bytes) instead, since `Vec<u8>` is ABI
/// `"uint8[]"` and would disagree with the on-chain `bytes` layout.
pub struct Lazy<T> {
    key: StorageKey,
    /// Byte offset within `key`'s 32-byte slot where this value lives.
    /// `0` for full-slot types (`T::PACKED_BYTES == 32`); non-zero only when
    /// the contract macro places the field after a sub-word neighbour.
    offset: u8,
    host: Host,
    _marker: PhantomData<T>,
}

impl<T: StorageEncode + StorageDecode> Lazy<T> {
    /// Compile-time validation of `T::STORAGE_SLOTS`. Referencing this in
    /// every public method forces the const evaluator to run the check at
    /// each monomorphization, even though the actual check lives in one place.
    const _SIZE_CHECK: () = {
        assert!(
            T::STORAGE_SLOTS > 0,
            "Lazy<T>: T::STORAGE_SLOTS must be positive"
        );
        assert!(
            T::STORAGE_SLOTS <= MAX_STATIC_SLOTS,
            "Lazy<T>: T::STORAGE_SLOTS exceeds MAX_STATIC_SLOTS. \
             Use a dynamic value (String, Bytes) or raise MAX_STATIC_SLOTS."
        );
    };

    /// Create a new `Lazy` at the given storage key, bound to a host handle.
    ///
    /// # Safety
    ///
    /// Fabricating a `Lazy` outside the `#[contract]` / `#[storage]` macro
    /// expansion path bypasses the view-vs-mutating compile-time gate that
    /// the SDK normally enforces. A `&self` (view) method that calls
    /// `unsafe { Lazy::new(slot, 0, self.host().clone()) }` can obtain a writable
    /// handle, call `set`, and mutate storage — defeating Rust's borrow
    /// checker.
    ///
    /// The runtime backstop (pallet-revive's STATICCALL boundary) still
    /// rejects the SSTORE at execution time, so this is not a soundness hole
    /// — only an SDK-level safety contract. Use
    /// [`StorageComponent::new_at`] (safe) from macro-generated code; reach
    /// for this constructor only when you need an arbitrary `StorageKey`
    /// (e.g. a manually-derived key) and you've ensured the resulting handle
    /// is reached only from `&mut self` paths. Contract crates that want
    /// belt-and-braces enforcement should add `#![forbid(unsafe_code)]` at
    /// the crate root.
    pub unsafe fn new(key: StorageKey, offset: u8, host: Host) -> Self {
        let () = Self::_SIZE_CHECK;
        debug_assert!(
            (offset as usize) + T::PACKED_BYTES <= 32,
            "Lazy::new: offset + T::PACKED_BYTES exceeds slot width",
        );
        debug_assert!(
            offset == 0 || T::PACKED_BYTES < 32,
            "Lazy::new: non-zero offset only valid for sub-32-byte (packable) T",
        );
        Lazy {
            key,
            offset,
            host,
            _marker: PhantomData,
        }
    }

    /// Read the value from storage. For multi-slot `T`, reads
    /// `T::STORAGE_SLOTS` consecutive slots starting at `self.key`.
    ///
    /// Returns the zero value for `T` if the slot was never written,
    /// matching Solidity's default-to-zero semantics.
    ///
    /// **Lossy decode for `T = String`:** Rust's `String` must hold valid
    /// UTF-8, so invalid byte sequences in storage are replaced with U+FFFD
    /// (matching Stylus's `StorageString::get_string`). A Solidity contract
    /// reading the same slot sees the raw bytes verbatim — `string` in solc
    /// is just `bytes` with a UTF-8 hint and has no decode step. If you need
    /// byte-exact roundtrips (e.g. on-chain `keccak256` matching an off-chain
    /// hash), use [`Lazy<Bytes>`] instead — it preserves every byte. See
    /// also `Mapping::get` for the same caveat on `V = String`.
    ///
    /// [`Lazy<Bytes>`]: pvm_contract_types::Bytes
    pub fn get(&self) -> T {
        let () = Self::_SIZE_CHECK;
        if T::PACKED_BYTES < 32 {
            // Packed sub-slot path: read the slot, unpack our byte window via
            // the polymorphic dispatch hook. `__unpack_from_dispatched` is a
            // no-zeroing reader; the caller (us) doesn't touch the rest of the
            // buffer, so neighbours stay correct. The hook delegates to
            // `<T as StoragePackable>::unpack_from` for packable T; full-slot
            // T never reaches this branch.
            let buf = storage_get_32(&self.host, self.key.as_bytes());
            T::__unpack_from_dispatched(&buf, self.offset as usize)
        } else if T::HAS_DYNAMIC_BODY {
            // Dispatch to the type's host-aware reader (e.g. LazySlot<String>
            // reads its body from `keccak256(key) + i`).
            T::read_from_storage::<MAX_STATIC_SLOTS>(&self.host, self.key.as_bytes())
        } else if T::STORAGE_SLOTS == 1 {
            // Fast path: skip the loop + multi-slot buffer for single-slot V.
            // The branch is const-folded at monomorphization.
            let one = [storage_get_32(&self.host, self.key.as_bytes())];
            T::from_slots(&one)
        } else {
            let mut slots = [[0u8; 32]; MAX_STATIC_SLOTS];
            read_slots(
                &self.host,
                self.key.as_bytes(),
                &mut slots[..T::STORAGE_SLOTS],
            );
            T::from_slots(&slots[..T::STORAGE_SLOTS])
        }
    }

    /// Read the value, distinguishing "never written" from "has been set."
    ///
    /// Returns `None` if every slot occupied by `T` reads back zero (either
    /// never written or cleared). Returns `Some(value)` if any occupied slot
    /// is present.
    ///
    /// Note: writing an all-zero static value deletes every slot (Solidity
    /// semantics), so `try_get()` returns `None` after writing the zero
    /// value of `T`.
    ///
    /// For `HAS_DYNAMIC_BODY` types, "present" is decided by the **header
    /// slot** at `self.key`: a non-zero header (including the empty-string
    /// sentinel) → `Some(value)` with the full body loaded; a zero header
    /// → `None`.
    ///
    /// **Not available for packed fields:** when `T::PACKED_BYTES < 32`
    /// (sub-32-byte primitives sharing a slot with neighbours), `try_get`
    /// fails to compile with a const-assert message. The semantics would
    /// be misleading — a neighbour's write to the same slot would make
    /// `try_get` indistinguishable from `get`. For packed fields, use
    /// `.get()` and compare to the zero value of `T` instead.
    ///
    /// ```compile_fail,E0080
    /// # use pvm_contract_types::{Host, MockHostBuilder};
    /// # use pvm_storage::{Lazy, StorageKey};
    /// # use std::rc::Rc;
    /// let host = Host::from_dyn(Rc::new(MockHostBuilder::new().build()));
    /// // `u128` has PACKED_BYTES = 16 — try_get is rejected at codegen time.
    /// let lazy = unsafe { Lazy::<u128>::new(StorageKey::from_slot(0), 16, host) };
    /// let _ = lazy.try_get();
    /// ```
    pub fn try_get(&self) -> Option<T> {
        let () = Self::_SIZE_CHECK;
        // try_get is only meaningful for full-slot types. For sub-slot packed
        // fields, "is this written?" cannot be answered honestly — a neighbor
        // writing to the same slot makes our `try_get` return Some(zero) even
        // if we never wrote. Solidity has the same conflation; Stylus avoids
        // it by not exposing try_get at all. We keep it for full-slot and
        // reject it for packed with a clear compile-time message.
        const {
            assert!(
                T::PACKED_BYTES == 32,
                "Lazy::try_get is only available on full-slot types \
                 (PACKED_BYTES == 32). For packed sub-slot fields, use \
                 `.get()` and compare to the zero value of T — a neighbour's \
                 write to the same slot would otherwise make `try_get` \
                 indistinguishable from `get`.",
            );
        }
        if T::HAS_DYNAMIC_BODY {
            // Multi-slot dynamic V: "set" iff any header slot is non-zero.
            // For a single-slot LazySlot<T>, the header itself is the marker.
            // For a struct with a LazyDynamic field, the dynamic field's
            // header may be the only non-zero slot — checking just slot 0
            // would miss it.
            let mut buf = [[0u8; 32]; MAX_STATIC_SLOTS];
            try_read_slots(
                &self.host,
                self.key.as_bytes(),
                &mut buf[..T::STORAGE_SLOTS],
            )?;
            Some(T::read_from_storage::<MAX_STATIC_SLOTS>(
                &self.host,
                self.key.as_bytes(),
            ))
        } else if T::STORAGE_SLOTS == 1 {
            let read = storage_get_32(&self.host, self.key.as_bytes());
            if read == [0u8; 32] {
                None
            } else {
                Some(T::from_slots(&[read]))
            }
        } else {
            let mut slots = [[0u8; 32]; MAX_STATIC_SLOTS];
            try_read_slots(
                &self.host,
                self.key.as_bytes(),
                &mut slots[..T::STORAGE_SLOTS],
            )?;
            Some(T::from_slots(&slots[..T::STORAGE_SLOTS]))
        }
    }

    /// Write a value to storage. Encodes `value` slot-by-slot and writes to
    /// `T::STORAGE_SLOTS` consecutive slots starting at `self.key`.
    ///
    /// Takes `&mut self` so that view methods (which receive `&Storage`)
    /// cannot call this through an immutable borrow.
    ///
    /// **Read-modify-write for packed fields:** when `T::PACKED_BYTES < 32`
    /// (sub-32-byte primitives that share a slot with neighbours via the
    /// macro walker), `set` performs one SLOAD + one SSTORE: it loads the
    /// shared slot, zeros only the field's byte window, writes the new
    /// bytes back, and stores. This matches solc and Stylus's gas profile
    /// for packed `SSTORE`s — neighbours sharing the slot are preserved.
    pub fn set(&mut self, value: &T) {
        let () = Self::_SIZE_CHECK;
        if T::PACKED_BYTES < 32 {
            // Packed sub-slot RMW: load slot, zero our window, write our
            // bytes back via the polymorphic dispatch hook, store. One extra
            // SLOAD on each write vs. the full-slot path — same gas profile
            // as solc / Stylus for adjacent sub-32-byte fields sharing a
            // slot. `__pack_into_dispatched` delegates to
            // `<T as StoragePackable>::pack_into` for packable T; full-slot T
            // never reaches this branch.
            let mut buf = storage_get_32(&self.host, self.key.as_bytes());
            let off = self.offset as usize;
            buf[off..off + T::PACKED_BYTES].fill(0);
            value.__pack_into_dispatched(&mut buf, off);
            storage_set_32(&self.host, self.key.as_bytes(), &buf);
        } else if T::HAS_DYNAMIC_BODY {
            value.write_to_storage(&self.host, self.key.as_bytes());
        } else if T::STORAGE_SLOTS == 1 {
            let mut buf = [0u8; 32];
            value.encode_slot(0, &mut buf);
            storage_set_32(&self.host, self.key.as_bytes(), &buf);
        } else {
            write_value(&self.host, self.key.as_bytes(), value);
        }
    }

}

impl<T: StorageEncode + StorageDecode> StorageComponent for Lazy<T> {
    /// One root slot per slot of `T::STORAGE_SLOTS`. A multi-slot `T` (e.g.
    /// `(U256, U256)`) reserves multiple consecutive slots, mirroring
    /// Solidity's struct-in-storage layout.
    const SLOTS: u64 = T::STORAGE_SLOTS as u64;

    /// Propagates `T::PACKED_BYTES`. A `Lazy<u128>` has `PACKED_BYTES = 16`
    /// (packable); a `Lazy<U256>` or `Lazy<(U256, U256)>` has
    /// `PACKED_BYTES = 32` (full-slot).
    const PACKED_BYTES: usize = T::PACKED_BYTES;

    fn new_at(key: StorageKey, offset: u8, host: Host) -> Self {
        // SAFETY: `new_at` is the safe public entry point for macro-generated
        // storage construction. The macro emits this call inside a contract
        // struct's field initializer, where Rust's borrow check on the
        // surrounding struct then gates `&self` / `&mut self` access to the
        // resulting handle. `Lazy::new` is `unsafe` only because direct
        // user-code calls would let `&self` methods reconstruct a writable
        // handle — that bypass cannot happen through this trait method.
        unsafe { Lazy::new(key, offset, host) }
    }

    /// Clear every slot occupied by this value.
    ///
    /// **Packed sub-word T** (`PACKED_BYTES < 32`): read-modify-write that
    /// zeros only the field's byte window — neighbours sharing the slot are
    /// preserved. If the resulting slot is all-zero (no neighbour written),
    /// the host's `set_storage_or_clear` auto-deletes the slot.
    ///
    /// **Dynamic body** (`String`, `Bytes`): clears the inline header AND
    /// any spilled body chunks at `keccak256(slot) + i`. No storage leak
    /// even after a previously-long value is cleared.
    ///
    /// **Multi-slot static T** (e.g. `(U256, U256)`): clears all
    /// `T::STORAGE_SLOTS` consecutive slots; each auto-deletes individually.
    fn clear(&mut self) {
        let () = Self::_SIZE_CHECK;
        if T::PACKED_BYTES < 32 {
            // Packed sub-slot clear: RMW that zeros only our window. Calling
            // `set_storage_or_clear` with an all-zero buffer would auto-delete
            // the slot and clobber any neighbour bytes — so we load, zero
            // OUR range, write back. If our zeroing leaves the slot all-zero
            // (no neighbour present), the host auto-deletes on store anyway.
            let mut buf = storage_get_32(&self.host, self.key.as_bytes());
            let off = self.offset as usize;
            buf[off..off + T::PACKED_BYTES].fill(0);
            storage_set_32(&self.host, self.key.as_bytes(), &buf);
        } else if T::HAS_DYNAMIC_BODY {
            <T as StorageEncode>::clear_storage(&self.host, self.key.as_bytes(), T::STORAGE_SLOTS);
        } else if T::STORAGE_SLOTS == 1 {
            storage_set_32(&self.host, self.key.as_bytes(), &[0u8; 32]);
        } else {
            clear_n_slots(&self.host, self.key.as_bytes(), T::STORAGE_SLOTS);
        }
    }
}

/// `Lazy<T>` is a storage handle around `T`; in layout JSON it's named by
/// `T` (the same way the macro's syntactic `Lazy<T>` detection unwraps).
///
/// This explicit impl closes a gap: when a contract author aliases a type
/// (`type Counter = Lazy<U256>;`) the syntactic wrapper detection in
/// `sol_storage_type_name` doesn't see "Lazy" — it sees "Counter" — and
/// falls through to `<Counter as StorageTypeName>::name()`. Without this
/// impl, that path would fail (Lazy doesn't implement `SolEncode`, so the
/// blanket impl doesn't cover it).
#[cfg(feature = "abi-gen")]
impl<T: pvm_contract_types::StorageTypeName> pvm_contract_types::StorageTypeName for Lazy<T> {
    fn name() -> alloc::string::String {
        <T as pvm_contract_types::StorageTypeName>::name()
    }
}

/// `Lazy<T>` as a layout-emit leaf. Used when an aliased `Lazy<T>` reaches
/// the macro's "not syntactically a leaf" branch (`type Counter =
/// Lazy<U256>;` declared in a contract field).
#[cfg(feature = "abi-gen")]
impl<T: pvm_contract_types::StorageTypeName> StorageLayoutEmit for Lazy<T> {
    fn emit_entries(
        base: u64,
        name_prefix: &str,
        out: &mut Vec<pvm_contract_types::StorageLayoutEntry>,
    ) {
        out.push(pvm_contract_types::StorageLayoutEntry {
            label: String::from(name_prefix),
            slot: alloc::format!("{}", base),
            offset: 0,
            ty: <T as pvm_contract_types::StorageTypeName>::name(),
        });
    }
}

// ---------------------------------------------------------------------------
// Mapping<K, V>
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Storage handle guards: lifetime-bound wrappers that gate read-vs-write
// access through `Deref` / `DerefMut`.
//
// `Ref<'a, T>` implements `Deref<Target = T>` only. Any method on `T` that
// takes `&self` is callable through it; methods that take `&mut self` are
// not. `RefMut<'a, T>` additionally implements `DerefMut`, so it forwards
// both read and write methods. The `'a` lifetime ties the guard to the
// borrow that produced it (typically a storage helper's `&self` / `&mut self`).
//
// Used to close the view-bypass gap on `Mapping<K1, Mapping<K2, V>>::get`,
// which previously returned an owned writable `Mapping<K2, V>` and let a
// `&self` (view) method call `.insert()` through it.
// ---------------------------------------------------------------------------

/// Read-only handle returned by storage helpers when a callee is invoked
/// through an immutable borrow. Forwards `&self` methods on the inner type
/// via [`Deref`], but never `&mut self` methods (no `DerefMut` impl).
pub struct Ref<'a, T> {
    inner: T,
    _marker: PhantomData<&'a T>,
}

impl<T> Ref<'_, T> {
    fn new(inner: T) -> Self {
        Self {
            inner,
            _marker: PhantomData,
        }
    }
}

impl<T> core::ops::Deref for Ref<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}

/// Mutable handle returned by storage helpers when a callee is invoked
/// through a mutable borrow. Forwards both `&self` and `&mut self` methods
/// via [`Deref`] + [`DerefMut`].
pub struct RefMut<'a, T> {
    inner: T,
    _marker: PhantomData<&'a mut T>,
}

impl<T> RefMut<'_, T> {
    fn new(inner: T) -> Self {
        Self {
            inner,
            _marker: PhantomData,
        }
    }
}

impl<T> core::ops::Deref for RefMut<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.inner
    }
}

impl<T> core::ops::DerefMut for RefMut<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        &mut self.inner
    }
}

/// A key-value mapping backed by on-chain storage.
///
/// Each entry lives at a derived slot: `keccak256(pad32(key) ++ pad32(root_slot))`.
/// The mapping stores nothing at its root slot.
pub struct Mapping<K, V> {
    root: StorageKey,
    host: Host,
    _marker: PhantomData<(K, V)>,
}

impl<K, V> Mapping<K, V> {
    /// Create a new mapping rooted at the given storage key, bound to a host handle.
    ///
    /// # Safety
    ///
    /// See [`Lazy::new`] for the safety contract. Fabricating a `Mapping`
    /// outside macro-generated code lets a `&self` method reconstruct a
    /// writable handle and bypass the borrow-check view gate. Use
    /// [`StorageComponent::new_at`] from macro expansion; reach for this
    /// constructor only when an arbitrary `StorageKey` is required.
    pub unsafe fn new(root: StorageKey, host: Host) -> Self {
        Mapping {
            root,
            host,
            _marker: PhantomData,
        }
    }
}

impl<K, V> StorageComponent for Mapping<K, V> {
    const SLOTS: u64 = 1;
    /// Mappings always claim a full slot for their root header — they never
    /// pack with neighbours. Matches solc's storage layout for mappings.
    const PACKED_BYTES: usize = 32;

    fn new_at(key: StorageKey, offset: u8, host: Host) -> Self {
        debug_assert!(
            offset == 0,
            "Mapping<K, V> always full-slot; offset must be 0"
        );
        let _ = offset;
        // SAFETY: same justification as `Lazy::new_at` — this is the
        // macro-only safe entry point; bypass via direct `Mapping::new` is
        // what the `unsafe` keyword on `new` exists to mark.
        unsafe { Mapping::new(key, host) }
    }

    /// No-op. Solidity's `delete map` for a `mapping(K => V)` field
    /// clears no storage (mappings have no header to zero, and entries at
    /// derived keys can't be enumerated). Matches that behaviour exactly —
    /// any actually-written entries remain at their derived keys.
    ///
    /// To clear individual entries call [`Mapping::delete`] (storage-typed
    /// V) or [`Mapping::remove`] (value-typed V) per key. To clear an
    /// entire mapping by enumeration, the contract must track keys itself.
    fn clear(&mut self) {
        // Intentionally empty. See doc comment.
    }
}

/// `Mapping<K, V>` in layout JSON is `mapping(K_name,V_name)`. This impl
/// is the fallback for callers that didn't take the macro's syntactic
/// `Mapping<K, V>` detection path — for example a type-aliased mapping
/// (`type Balances = Mapping<Address, U256>;`) flowing into the layout
/// emitter as a single ident "Balances". The macro's wrapper detection
/// matches on the syntactic ident "Mapping", which fails for aliases.
///
/// Built via `format!` because `concatcp!` (compile-time `&'static str`
/// concat) can't reference generic parameters from the surrounding impl.
/// Layout JSON is built off-chain under `--features abi-gen`, so heap
/// allocation here is fine.
#[cfg(feature = "abi-gen")]
impl<K: pvm_contract_types::StorageTypeName, V: pvm_contract_types::StorageTypeName>
    pvm_contract_types::StorageTypeName for Mapping<K, V>
{
    fn name() -> alloc::string::String {
        alloc::format!(
            "mapping({},{})",
            <K as pvm_contract_types::StorageTypeName>::name(),
            <V as pvm_contract_types::StorageTypeName>::name(),
        )
    }
}

/// `Mapping<K, V>` as a layout-emit leaf. Used when an aliased mapping
/// (`type Balances = Mapping<Address, U256>;`) reaches the macro's
/// "not syntactically a leaf" branch.
#[cfg(feature = "abi-gen")]
impl<K: pvm_contract_types::StorageTypeName, V: pvm_contract_types::StorageTypeName>
    StorageLayoutEmit for Mapping<K, V>
{
    fn emit_entries(
        base: u64,
        name_prefix: &str,
        out: &mut Vec<pvm_contract_types::StorageLayoutEntry>,
    ) {
        out.push(pvm_contract_types::StorageLayoutEntry {
            label: String::from(name_prefix),
            slot: alloc::format!("{}", base),
            offset: 0,
            ty: <Self as pvm_contract_types::StorageTypeName>::name(),
        });
    }
}

/// V-agnostic accessors: only need the key derivation, independent of
/// whether the value path is value-typed or storage-typed.
impl<K: AsStorageKey, V> Mapping<K, V> {
    /// Compute the raw storage key for a given map key.
    ///
    /// Useful for debugging and cross-checking with `cast index`.
    pub fn slot_of(&self, key: &K) -> StorageKey {
        self.root.derive(&self.host, key)
    }
}

impl<K: AsStorageKey, V: StorageEncode + StorageDecode> Mapping<K, V> {
    /// Derive the slot once and return a [`Lazy`] handle for multiple operations.
    ///
    /// Requires `&mut self` because the returned `Lazy` supports writes.
    /// For read-only access, use [`get`](Mapping::get) or [`try_get`](Mapping::try_get).
    ///
    /// This saves a keccak host call when doing read-then-write on the same key.
    ///
    /// **Canonical offset within the entry slot:** for sub-word `V`
    /// (`PACKED_BYTES < 32` — `u8`..`u128`, `i8`..`i128`, `bool`, `Address`,
    /// `[u8; N<32]`), solc stores the value right-aligned within the derived
    /// slot at byte `32 - PACKED_BYTES`. `insert` / `get` / `remove` route
    /// through `encode_slot` / `from_slots` and observe that convention; the
    /// returned `Lazy` must use the same offset so `entry().set()` / `.get()`
    /// agree byte-for-byte with `insert` / `get`. For full-slot `V`
    /// (`PACKED_BYTES == 32`) this is `0` — identical to the previous behavior.
    pub fn entry(&mut self, key: &K) -> Lazy<V> {
        // SAFETY: `entry` takes `&mut self`, so the caller already has
        // mutating access through the surrounding borrow. The returned
        // `Lazy` is a typed handle to the derived slot; producing it via
        // `Lazy::new` here does not introduce a new bypass surface.
        let offset = (32 - V::PACKED_BYTES) as u8;
        unsafe { Lazy::new(self.slot_of(key), offset, self.host.clone()) }
    }

    /// Read the value at the given key. For multi-slot `V`, reads
    /// `V::STORAGE_SLOTS` consecutive slots starting at the derived key.
    ///
    /// Returns the zero value if the key was never written.
    ///
    /// **Lossy decode for `V = String`:** Rust's `String` must hold valid
    /// UTF-8, so invalid byte sequences in storage are replaced with U+FFFD
    /// (matching Stylus's `StorageString::get_string`). A Solidity contract
    /// reading the same slot sees the raw bytes verbatim — `string` in solc
    /// is just `bytes` with a UTF-8 hint and has no decode step. If you need
    /// byte-exact roundtrips (e.g. on-chain `keccak256` matching an off-chain
    /// hash), use [`Mapping<K, Bytes>`] instead — it preserves every byte.
    ///
    /// [`Mapping<K, Bytes>`]: pvm_contract_types::Bytes
    pub fn get(&self, key: &K) -> V {
        let () = Lazy::<V>::_SIZE_CHECK;
        let slot = self.slot_of(key);
        if V::HAS_DYNAMIC_BODY {
            V::read_from_storage::<MAX_STATIC_SLOTS>(&self.host, slot.as_bytes())
        } else if V::STORAGE_SLOTS == 1 {
            let one = [storage_get_32(&self.host, slot.as_bytes())];
            V::from_slots(&one)
        } else {
            let mut slots = [[0u8; 32]; MAX_STATIC_SLOTS];
            read_slots(&self.host, slot.as_bytes(), &mut slots[..V::STORAGE_SLOTS]);
            V::from_slots(&slots[..V::STORAGE_SLOTS])
        }
    }

    /// Read the value, returning `None` if every slot occupied by the entry
    /// reads back zero (either never written or cleared).
    ///
    /// **Solidity zero-slot semantics:** `insert(k, &V::default())` writes the
    /// zero value, but `set_storage_or_clear` collapses zero writes into a
    /// slot deletion (matching `SSTORE` clears-and-refunds). The next
    /// `try_get(k)` therefore returns `None`, conflating "never written" with
    /// "explicitly set to zero". This matches Solidity, where a slot reading
    /// back zero is indistinguishable from one that was never assigned. Use
    /// [`get`](Self::get) (returns the zero value) when you need a single
    /// definition of "absent".
    pub fn try_get(&self, key: &K) -> Option<V> {
        let () = Lazy::<V>::_SIZE_CHECK;
        let slot = self.slot_of(key);
        if V::HAS_DYNAMIC_BODY {
            let mut buf = [[0u8; 32]; MAX_STATIC_SLOTS];
            try_read_slots(&self.host, slot.as_bytes(), &mut buf[..V::STORAGE_SLOTS])?;
            Some(V::read_from_storage::<MAX_STATIC_SLOTS>(
                &self.host,
                slot.as_bytes(),
            ))
        } else if V::STORAGE_SLOTS == 1 {
            let read = storage_get_32(&self.host, slot.as_bytes());
            if read == [0u8; 32] {
                None
            } else {
                Some(V::from_slots(&[read]))
            }
        } else {
            let mut slots = [[0u8; 32]; MAX_STATIC_SLOTS];
            try_read_slots(&self.host, slot.as_bytes(), &mut slots[..V::STORAGE_SLOTS])?;
            Some(V::from_slots(&slots[..V::STORAGE_SLOTS]))
        }
    }

    /// Write a value at the given key. Encodes `value` slot-by-slot and writes
    /// to `V::STORAGE_SLOTS` consecutive slots beneath the derived key.
    pub fn insert(&mut self, key: &K, value: &V) {
        let () = Lazy::<V>::_SIZE_CHECK;
        let slot = self.slot_of(key);
        if V::HAS_DYNAMIC_BODY {
            value.write_to_storage(&self.host, slot.as_bytes());
        } else if V::STORAGE_SLOTS == 1 {
            let mut buf = [0u8; 32];
            value.encode_slot(0, &mut buf);
            storage_set_32(&self.host, slot.as_bytes(), &buf);
        } else {
            write_value(&self.host, slot.as_bytes(), value);
        }
    }

    /// Delete every slot occupied by the entry at the given key.
    pub fn remove(&mut self, key: &K) {
        let () = Lazy::<V>::_SIZE_CHECK;
        let slot = self.slot_of(key);
        if V::HAS_DYNAMIC_BODY {
            <V as StorageEncode>::clear_storage(&self.host, slot.as_bytes(), V::STORAGE_SLOTS);
        } else if V::STORAGE_SLOTS == 1 {
            storage_set_32(&self.host, slot.as_bytes(), &[0u8; 32]);
        } else {
            clear_n_slots(&self.host, slot.as_bytes(), V::STORAGE_SLOTS);
        }
    }
}

// ---------------------------------------------------------------------------
// Mapping<K, V: StorageComponent>: storage-typed value
//
// Disjoint with the value-typed `Mapping<K, V: StorageEncode + StorageDecode>`
// impl above: no in-tree type implements both bounds (primitives + derived
// structs implement the value codec; `Lazy<T>`, `Mapping`, `StorageVec`,
// `#[storage]` sub-structs implement `StorageComponent`).
//
// `V::new_at(derived_key, 0, host)` positions the sub-component at the
// mapping's derived slot. For nested mappings (V = Mapping<K2, …>) this
// generalizes the previously-hand-rolled `Mapping<K1, Mapping<K2, V>>` impl.
// For `#[storage]` sub-structs as map values, the struct's macro-generated
// `new_at` further walks its fields at `derived_key.add(N)` per field.
// ---------------------------------------------------------------------------

impl<K, V: StorageComponent> Mapping<K, V> {
    /// Read-only view into the sub-component at `key`. The returned `Ref`
    /// inherits this mapping's `&self` borrow, so only `&self` methods on
    /// `V` are reachable through it. Writes through the returned handle
    /// are blocked at compile time.
    ///
    /// Distinct method name (`view`, not `get`) so this storage-typed impl
    /// can coexist with the value-typed `Mapping::get` which returns an
    /// owned `V` for value-shaped V. Use `view` when V is itself a storage
    /// handle: `Lazy<T>`, `Mapping<K2, V'>`, `StorageVec<T>`, or a
    /// `#[storage]` sub-struct.
    ///
    /// **Canonical offset:** for sub-word storage handles (`V::PACKED_BYTES
    /// < 32`, e.g. `Lazy<u128>` with `PACKED_BYTES = 16`), the handle is
    /// placed at `32 - V::PACKED_BYTES` within the derived slot — solc's
    /// right-aligned convention for `mapping(K => sub_word_V)`. Full-slot
    /// V's (`Mapping`, `StorageVec`, `#[storage]` structs) have
    /// `PACKED_BYTES == 32`, yielding offset 0.
    pub fn view(&self, key: &K) -> Ref<'_, V>
    where
        K: AsStorageKey,
    {
        let offset = (32 - V::PACKED_BYTES) as u8;
        Ref::new(V::new_at(self.slot_of(key), offset, self.host.clone()))
    }

    /// Mutable view into the sub-component at `key`. Caller has `&mut self`
    /// on the outer mapping; the returned `RefMut` propagates that
    /// capability into the sub-component, allowing the full mutating API.
    ///
    /// Subsumes the previously-hand-rolled
    /// `Mapping<K1, Mapping<K2, V>>::entry` — when `V = Mapping<K2, V'>`,
    /// `V::new_at` reconstructs the inner mapping at the derived slot.
    /// Canonical-offset rule same as [`view`](Self::view).
    pub fn view_mut(&mut self, key: &K) -> RefMut<'_, V>
    where
        K: AsStorageKey,
    {
        let offset = (32 - V::PACKED_BYTES) as u8;
        RefMut::new(V::new_at(self.slot_of(key), offset, self.host.clone()))
    }

    /// Delete the entry at `key` by clearing every slot the sub-component
    /// owns. Equivalent to Solidity's `delete mapping[key]`:
    ///
    /// - `V = Lazy<T>`: zeros `T`'s storage slot (sub-word RMW preserves
    ///   any neighbour bytes that happen to be there).
    /// - `V = Mapping<K2, V'>`: **no-op**. The inner mapping's entries
    ///   live at derived keys we can't enumerate — matches solc, which
    ///   also leaves them in place.
    /// - `V = #[storage]` sub-struct: recursively clears each field via
    ///   the derived `StorageComponent::clear`.
    ///
    /// Symmetric with the value-typed [`Mapping::remove`]: both delete
    /// the entry; this one dispatches to `V::clear()` so it works for
    /// arbitrary storage-component value shapes.
    pub fn delete(&mut self, key: &K)
    where
        K: AsStorageKey,
    {
        let offset = (32 - V::PACKED_BYTES) as u8;
        let mut handle = V::new_at(self.slot_of(key), offset, self.host.clone());
        <V as StorageComponent>::clear(&mut handle);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;

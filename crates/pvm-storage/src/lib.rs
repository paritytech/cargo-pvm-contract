//! Typed storage helpers for PVM smart contracts with Solidity-compatible slot layout.
//!
//! Provides [`Lazy<T>`] for single-value storage, [`Mapping<K, V>`] for
//! key-value storage, and [`StorageVec<T>`] for dynamic arrays (Solidity's
//! `T[]`). All three use Solidity-compatible key/index derivation so tools
//! like `cast storage` and `cast index` work out of the box.
//!
//! [`Lazy<T>`] and [`Mapping<K, V>`] bind `T`/`V` to
//! [`StorageEncode`](pvm_contract_types::StorageEncode) +
//! [`StorageDecode`](pvm_contract_types::StorageDecode). The value's
//! [`STORAGE_SLOTS`](pvm_contract_types::StorageEncode::STORAGE_SLOTS) is
//! checked at compile time and must be in `1..=MAX_STATIC_SLOTS`. Single-slot
//! values (`U256`, `Address`, `bool`, `[u8; 32]`, …) occupy one slot;
//! multi-slot values like `(U256, U256)` or static
//! `#[derive(SolType, SolStorage)]` structs are striped across
//! `T::STORAGE_SLOTS` consecutive slots, mirroring Solidity's
//! struct-in-storage layout.
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
//! solc. Full-slot writes are a single SSTORE — no overhead from the
//! packing infrastructure.
//!
//! Multi-slot composites (`Lazy<(U256, U256)>`, multi-slot
//! `#[derive(SolType, SolStorage)]` structs), mappings, and `#[storage]`
//! sub-structs always start a fresh slot and never pack with neighbours.
//! They report `PACKED_BYTES = 32`.
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
//! // `alone = true` is the safe choice for contract-field positions where
//! // the macro's layout walker has proven no neighbour shares the slot.
//! let mut total_supply = <Lazy<U256> as StorageComponent>::new_at(
//!     StorageKey::from_slot(0), 0, true, host.clone(),
//! );
//! total_supply.set(&U256::from(1000));
//! assert_eq!(total_supply.get(), U256::from(1000));
//!
//! let mut balances = <Mapping<Address, U256> as StorageComponent>::new_at(
//!     StorageKey::from_slot(1), 0, true, host,
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
pub(crate) use pvm_contract_types::storage_codec::inc_be_32;
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

pub use pvm_contract_types::MAX_STATIC_SLOTS;

// Body-base derivation for a dynamic array (`StorageVec<T>`):
// `keccak256(pad32(slot))`. Element `i` of a full-slot single-slot `T` array
// lives at `body_base + i`; multi-slot/packed shapes scale this stride. The
// formula has no key component — unlike `Mapping`, the array's elements are
// addressed by index, not by hashed key. Matches Solidity's `T[]` layout.
fn storage_derive_body_base(host: &Host, slot_key: &[u8; 32]) -> [u8; 32] {
    let mut output = [0u8; 32];
    host.hash_keccak_256(slot_key, &mut output);
    output
}

/// Add `n` to a 32-byte big-endian integer in-place, propagating carries
/// up through all 32 bytes. Used by `StorageVec` to address element `i`
/// at `body_base + i` without iterating `inc_be_32` `i` times.
fn inc_slot_by(slot: &mut [u8; 32], n: u64) {
    let mut carry: u64 = n;
    for byte in slot.iter_mut().rev() {
        if carry == 0 {
            return;
        }
        let sum = *byte as u64 + (carry & 0xff);
        *byte = sum as u8;
        carry = (carry >> 8) + (sum >> 8);
    }
}

/// Read a u64 length from a storage slot's lower 8 bytes (big-endian).
/// Solidity stores array lengths as `uint256`; we cap support at `u64::MAX`
/// elements (more than enough for any real-world contract) and panic if the
/// upper 24 bytes are non-zero, which would indicate either corrupted state
/// or a length set via raw uAPI that exceeds our supported range.
fn read_len_u64(host: &Host, slot_key: &[u8; 32]) -> u64 {
    let buf = storage_get_32(host, slot_key);
    assert!(
        buf[..24].iter().all(|&b| b == 0),
        "StorageVec length exceeds u64::MAX"
    );
    u64::from_be_bytes([
        buf[24], buf[25], buf[26], buf[27], buf[28], buf[29], buf[30], buf[31],
    ])
}

/// Write a u64 length to a storage slot as a big-endian `uint256` (upper 24
/// bytes zero). When `n == 0` the host's `set_storage_or_clear` deletes the
/// slot, matching Solidity's `delete arr.length` behaviour.
fn write_len_u64(host: &Host, slot_key: &[u8; 32], n: u64) {
    let mut buf = [0u8; 32];
    buf[24..32].copy_from_slice(&n.to_be_bytes());
    storage_set_32(host, slot_key, &buf);
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
    /// matches `from_slot(s + N)` as long as `s + N` fits in `u64`; if it
    /// overflows `u64`, the carry propagates into the higher bytes (i.e. this
    /// is full 256-bit modular addition, not `u64` wrapping).
    /// For a derived key, `add` performs proper 256-bit big-endian addition that
    /// wrap is unreachable because derived keys are uniformly distributed in
    /// the 256-bit space, so no realistic struct/array depth approaches the
    /// wrap boundary.
    ///
    /// Named `add` despite clippy's `should_implement_trait` lint: this is
    /// EVM `uint256` modular addition (matches solc's storage-key derivation),
    /// not host-side `core::ops::Add` arithmetic — a trait impl would mislead.
    #[allow(clippy::should_implement_trait)]
    pub fn add(self, n: u64) -> StorageKey {
        let mut out = self.0;
        inc_slot_by(&mut out, n);
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

// The layout walker (`LayoutStep` + `layout_step`) lives in `pvm-contract-types`
// so the tuple `StorageEncode` impls there consume the same algorithm rather
// than hand-rolling a shadow copy. Re-exported here so existing
// `pvm_storage::{LayoutStep, layout_step}` paths keep resolving.
pub use pvm_contract_types::{LayoutStep, layout_step};

/// `StorageComponent`-family wrapper over [`layout_step`]: reads the component
/// type's `PACKED_BYTES` + `SLOTS` so call sites (the `#[contract]` /
/// `#[storage]` field-layout chains) pass only the type. Mirrors
/// [`pvm_contract_types::layout_step_encode`] for the `StorageEncode` family;
/// both forward to the one trait-agnostic [`layout_step`] primitive.
pub const fn layout_step_component<T: StorageComponent>(prev: LayoutStep) -> LayoutStep {
    layout_step(prev, T::PACKED_BYTES, T::SLOTS)
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
    ///
    /// `alone` declares that no other sub-word component shares this slot —
    /// either the field has no sub-word neighbours in the surrounding
    /// `#[contract]` / `#[storage]` layout, or it lives at a uniquely
    /// derived key (e.g. a `Mapping` entry). For sub-word `Lazy<T>` this
    /// lets `set`/`clear` skip the read-modify-write SLOAD that would
    /// otherwise be needed to preserve neighbour bytes. Full-slot
    /// components (`PACKED_BYTES == 32`) ignore the flag — they always own
    /// their slot.
    fn new_at(key: StorageKey, offset: u8, alone: bool, host: Host) -> Self;

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
// StorageType / SimpleStorageType: unified composition traits (issue #108).
//
// `StorageType` lets a value leaf (`U256`, `bool`, `String`, ...) OR a handle
// container (`Mapping`, `StorageVec`, `#[storage]` struct) sit *inside* a
// container. Its GAT return types decide the access shape: a leaf yields its
// value (`Get<'a> = Self`) and a `Lazy` write cursor (`GetMut<'a> = Lazy<Self>`);
// a container yields a `Ref`/`RefMut` guard over itself. This is the single
// mechanism that lets `StorageVec` and `Mapping` compose to arbitrary depth
// with no per-shape impls.
//
// `StorageType` is intentionally NOT blanket-impl'd over `StorageEncode +
// StorageDecode`: a blanket would overlap the container impls (the E0592
// coherence wall this design exists to dodge — see issue #108). Leaves are
// enumerated explicitly via `impl_leaf_storage_type!`; containers impl it
// directly; `#[derive(SolStorage)]` structs get it from the derive.
//
// `SimpleStorageType` refines it with by-value ops (the container `push`/`pop`/
// `insert`/`set` surface) and is a single always-coherent blanket-free set of
// leaf impls; containers never implement it, which is what gates by-value ops
// to leaves only.
// ---------------------------------------------------------------------------

/// A value or handle that can occupy a slot region as the *element* of a
/// container (`StorageVec`, `Mapping`) or the value of a `#[storage]` field.
///
/// See the module-level notes above for how the `Get`/`GetMut` GATs drive
/// arbitrary-depth composition. Contract *fields* are still constructed through
/// [`StorageComponent::new_at`]; `StorageType` is the element/value role.
pub trait StorageType: Sized {
    /// Slots one element claims at its placement site (stride).
    const SLOTS: u64;
    /// Packing width; `32` = fresh slot, `< 32` = sub-word packable leaf.
    const PACKED_BYTES: usize;
    /// Whether the value spills a body outside its slot range (`String`/`Bytes`).
    const HAS_DYNAMIC_BODY: bool;
    /// Whether clearing this element must recurse — it owns storage at derived
    /// keys (a container) or spills a body (`String`/`Bytes`). `false` for a
    /// static leaf whose whole representation lives in its slot range and can
    /// be bulk-zeroed. A single-slot leaf (`U256`) and a single-slot container
    /// (`StorageVec<U256>`) share every other const, so this is the only signal
    /// [`StorageVec::clear`] has to choose bulk-zero vs. per-element recursion.
    const NEEDS_RECURSIVE_CLEAR: bool;

    /// What an immutable access yields: the value for a leaf, a [`Ref`] guard
    /// for a container.
    type Get<'a>
    where
        Self: 'a;
    /// What a mutable access yields: a [`Lazy`] write cursor for a leaf, a
    /// [`RefMut`] guard for a container.
    type GetMut<'a>
    where
        Self: 'a;

    /// Immutable accessor at `(key, offset)`, borrowing `host` so the returned
    /// guard cannot outlive the parent borrow.
    fn get_at(key: StorageKey, offset: u8, host: &Host) -> Self::Get<'_>;

    /// Mutable accessor at `(key, offset)`.
    ///
    /// # Safety
    ///
    /// Minting a writable accessor bypasses the borrow-check view gate (see
    /// [`Lazy::new`]). The safe callers are the container methods, which gate
    /// access through their `&mut self` receiver and the returned guard's
    /// lifetime.
    unsafe fn get_mut_at(key: StorageKey, offset: u8, alone: bool, host: &Host)
    -> Self::GetMut<'_>;

    /// Clear the storage this element occupies at `(key, offset)`.
    ///
    /// # Safety
    ///
    /// Same contract as [`get_mut_at`](StorageType::get_mut_at).
    unsafe fn clear_at(key: StorageKey, offset: u8, alone: bool, host: &Host);
}

/// A [`StorageType`] leaf that is materialized by value — enables the container
/// by-value surface (`push`/`pop`/`insert`/`set`/value `get`). Containers do
/// not implement this, which is what confines by-value ops to leaves.
pub trait SimpleStorageType: StorageType {
    /// The owned value type (always `Self` for the built-in leaves).
    type Value;
    /// Read the value at `(key, offset)`.
    fn read_value(key: StorageKey, offset: u8, host: &Host) -> Self::Value;
    /// Read the value, or `None` if the slot(s) read back zero.
    fn try_read_value(key: StorageKey, offset: u8, host: &Host) -> Option<Self::Value>;
    /// Write the value at `(key, offset)`.
    fn write_value(value: &Self::Value, key: StorageKey, offset: u8, alone: bool, host: &Host);
}

/// The `StorageType` associated items for a value-leaf `$ty`, delegating to
/// [`Lazy`] so all packed/`alone`/dynamic-body logic is reused. Emitted at
/// item position inside a hand-written `impl ... StorageType for $ty` header
/// (so generic leaves — tuples, arrays — can supply their own generics/bounds).
macro_rules! leaf_storage_type_body {
    ($ty:ty) => {
        const SLOTS: u64 = <$ty as StorageEncode>::STORAGE_SLOTS as u64;
        const PACKED_BYTES: usize = <$ty as StorageEncode>::PACKED_BYTES;
        const HAS_DYNAMIC_BODY: bool = <$ty as StorageEncode>::HAS_DYNAMIC_BODY;
        // A static leaf bulk-zeroes; a dynamic-body leaf (String/Bytes) must
        // recurse to tear down its spilled chunks.
        const NEEDS_RECURSIVE_CLEAR: bool = <$ty as StorageEncode>::HAS_DYNAMIC_BODY;

        type Get<'a>
            = $ty
        where
            Self: 'a;
        type GetMut<'a>
            = Lazy<$ty>
        where
            Self: 'a;

        fn get_at(key: StorageKey, offset: u8, host: &Host) -> $ty {
            // Reads ignore `alone`; a plain (RMW-safe) cursor is correct.
            unsafe { Lazy::<$ty>::new(key, offset, host.clone()) }.get()
        }

        unsafe fn get_mut_at(key: StorageKey, offset: u8, alone: bool, host: &Host) -> Lazy<$ty> {
            if alone {
                unsafe { Lazy::<$ty>::new_alone(key, offset, host.clone()) }
            } else {
                unsafe { Lazy::<$ty>::new(key, offset, host.clone()) }
            }
        }

        unsafe fn clear_at(key: StorageKey, offset: u8, alone: bool, host: &Host) {
            let mut cell = if alone {
                unsafe { Lazy::<$ty>::new_alone(key, offset, host.clone()) }
            } else {
                unsafe { Lazy::<$ty>::new(key, offset, host.clone()) }
            };
            <Lazy<$ty> as StorageComponent>::clear(&mut cell);
        }
    };
}

/// The `SimpleStorageType` associated items for a value-leaf `$ty`.
macro_rules! simple_storage_type_body {
    ($ty:ty) => {
        type Value = $ty;

        fn read_value(key: StorageKey, offset: u8, host: &Host) -> $ty {
            <$ty as StorageType>::get_at(key, offset, host)
        }

        fn try_read_value(key: StorageKey, offset: u8, host: &Host) -> Option<$ty> {
            // Match `Mapping::try_get`: read through the codec at the canonical
            // (right-aligned) offset. Only meaningful for alone slots (mapping
            // entries / vec elements), where offset == canonical.
            let _ = offset;
            <$ty as StorageDecode>::try_read_from_storage(host, key.as_bytes())
        }

        fn write_value(value: &$ty, key: StorageKey, offset: u8, alone: bool, host: &Host) {
            let mut cell = unsafe { <$ty as StorageType>::get_mut_at(key, offset, alone, host) };
            cell.set(value);
        }
    };
}

/// Enumerate `StorageType` + `SimpleStorageType` for concrete value-leaf types.
/// Deliberately per-type (not a blanket over the codec) to stay coherent with
/// the container impls (`StorageVec<S>`, `Mapping<K, V>`).
macro_rules! impl_leaf_storage_type {
    ($($ty:ty),* $(,)?) => {$(
        impl StorageType for $ty { leaf_storage_type_body!($ty); }
        impl SimpleStorageType for $ty { simple_storage_type_body!($ty); }
    )*}
}

// Scalar 32-byte-or-sub-word leaves — same set as `impl_scalar_storage_key!`.
impl_leaf_storage_type!(
    U256, u128, u64, u32, u16, u8, // unsigned
    I256, i128, i64, i32, i16, i8, // signed
    bool, Address,
);

// Dynamic-body leaves (alloc-gated): `String` and `Bytes`.
#[cfg(feature = "alloc")]
impl_leaf_storage_type!(alloc::string::String, pvm_contract_types::Bytes);

/// Enumerate `StorageType` + `SimpleStorageType` for tuple value-leaves,
/// mirroring the codec's tuple impls. Keyed on the concrete `(A, B, ...)`
/// constructor, so no overlap with scalar or container impls.
macro_rules! impl_tuple_storage_type {
    ($($t:ident),+) => {
        impl<$($t),+> StorageType for ($($t,)+)
        where
            ($($t,)+): StorageEncode + StorageDecode,
        {
            leaf_storage_type_body!(($($t,)+));
        }

        impl<$($t),+> SimpleStorageType for ($($t,)+)
        where
            ($($t,)+): StorageEncode + StorageDecode,
        {
            simple_storage_type_body!(($($t,)+));
        }
    };
}

// Tuple arities 1..=8, matching the codec's `impl_storage_tuple!` coverage.
impl_tuple_storage_type!(A);
impl_tuple_storage_type!(A, B);
impl_tuple_storage_type!(A, B, C);
impl_tuple_storage_type!(A, B, C, D);
impl_tuple_storage_type!(A, B, C, D, E);
impl_tuple_storage_type!(A, B, C, D, E, F);
impl_tuple_storage_type!(A, B, C, D, E, F, G);
impl_tuple_storage_type!(A, B, C, D, E, F, G, H);

// Fixed arrays as value leaves. A single impl keyed on the `[T; N]`
// constructor and gated on the codec bound covers BOTH `[u8; N]` (Solidity
// `bytesN`) and `[T; N]` of static elements (Solidity `T[N]`) — the codec
// implements `StorageEncode`/`StorageDecode` for exactly those. Two separate
// `[u8; N]` / `[T: StorageArrayElement; N]` impls would conflict here: unlike
// the codec (co-located with the `StorageArrayElement` marker), this
// downstream crate can't prove `u8: !StorageArrayElement`, so a single
// where-gated impl is used instead.
impl<T, const N: usize> StorageType for [T; N]
where
    [T; N]: StorageEncode + StorageDecode,
{
    leaf_storage_type_body!([T; N]);
}
impl<T, const N: usize> SimpleStorageType for [T; N]
where
    [T; N]: StorageEncode + StorageDecode,
{
    simple_storage_type_body!([T; N]);
}

// ---------------------------------------------------------------------------
// StorageLayoutEmit: per-struct hook for emitting layout JSON leaves.
// ---------------------------------------------------------------------------

/// Push flattened storage-layout entries for a composable storage component.
///
/// The `#[contract]` macro generates the top-level `__storage_layout_json()`
/// function by dispatching **every** storage field through this trait — there
/// is no separate inlined-leaf path. `Lazy<T>` and `Mapping<K, V>` push a
/// single entry; embedded `#[storage]` sub-structs recursively flatten their
/// own fields, prefixing each entry's label with the field path
/// (`erc20.total_supply`, `metadata.name`, …) to match solc's storage-layout
/// convention. Adding a new storage component (e.g. a `StorageVec<T>`) is
/// therefore a pure trait-impl task: implement this (and `StorageTypeName`)
/// and the macro renders it with no codegen changes.
///
/// `#[storage]` auto-emits this impl. Hand-rolled storage components need to
/// implement it explicitly to participate in abi-gen layout output.
#[cfg(feature = "abi-gen")]
pub trait StorageLayoutEmit {
    /// Append entries for this component into `out`, rooted at slot `base`,
    /// byte `offset` within that slot, and prefixed by `name_prefix` (empty
    /// string at top level). `offset` is non-zero only for packed sub-word
    /// leaf fields sharing a slot with neighbours; multi-slot composites and
    /// `#[storage]` sub-structs always start a fresh slot and ignore it.
    fn emit_entries(
        base: u64,
        offset: u8,
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
/// Static `T` must report `STORAGE_SLOTS` in `1..=`[`MAX_STATIC_SLOTS`].
/// Single-slot `T` (`U256`, `Address`, `bool`, `[u8; 32]`, …) occupies one
/// slot; an `N`-slot `T` (e.g. `(U256, U256)`, or a
/// `#[derive(SolType, SolStorage)]` struct of static fields) is striped
/// across `N` consecutive slots starting at `self.key`, matching Solidity's
/// struct-in-storage layout.
///
/// Dynamic `T` (`String`, [`Bytes`](pvm_contract_types::Bytes), or
/// `#[derive(SolType, SolStorage)]` structs with dynamic fields) uses the
/// same `Lazy<T>` accessor: the header lives inline at `self.key` and any
/// spilled body sits at `keccak256(key) + i`. `Vec<u8>` is rejected at
/// compile time — use [`Bytes`](pvm_contract_types::Bytes) instead, since
/// `Vec<u8>` is ABI `"uint8[]"` and would disagree with the on-chain
/// `bytes` layout.
pub struct Lazy<T> {
    key: StorageKey,
    /// Byte offset within `key`'s 32-byte slot where this value lives.
    /// `0` for full-slot types (`T::PACKED_BYTES == 32`); non-zero only when
    /// the contract macro places the field after a sub-word neighbour.
    offset: u8,
    /// True when nothing else in storage shares this slot — set by the
    /// macro's neighbour analysis for fields with no sub-word siblings, and
    /// by `Mapping` for its derived entry slots (every key has a unique
    /// keccak-derived key). Sub-word writes (`set`/`clear`) skip the SLOAD
    /// half of the read-modify-write when this is true. Defaults to `false`
    /// in [`Lazy::new`] — the safe (RMW) choice when the caller can't prove
    /// exclusivity.
    alone: bool,
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
        // Default to `alone = false`: the safe choice when the caller can't
        // statically prove the slot has no neighbours. Forces sub-word writes
        // through the RMW path so any neighbour bytes survive. Callers that
        // *can* prove exclusivity (the macro-driven [`StorageComponent::new_at`]
        // path with `alone = true`, or `Mapping::entry` whose derived slot is
        // unique to the key) get the optimised constructor via
        // [`Lazy::new_alone`].
        unsafe { Self::new_inner(key, offset, false, host) }
    }

    /// Like [`Lazy::new`], but declares that no other sub-word component
    /// shares this slot. Sub-word `T` writes skip the SLOAD half of the
    /// read-modify-write because there are no neighbour bytes to preserve.
    ///
    /// # Safety
    ///
    /// In addition to [`Lazy::new`]'s safety contract, the caller must
    /// guarantee that no other live storage handle writes to this slot.
    /// Violating that invariant clobbers the neighbour on the next `set`.
    /// Safe callers are the macro-generated [`StorageComponent::new_at`]
    /// path (with `alone = true` derived from layout analysis) and the
    /// `Mapping::entry`/`view*`/`delete` family (each derived slot is
    /// uniquely keyed by `keccak256(pad32(key) ++ pad32(slot))`).
    pub unsafe fn new_alone(key: StorageKey, offset: u8, host: Host) -> Self {
        unsafe { Self::new_inner(key, offset, true, host) }
    }

    /// Shared body for [`Lazy::new`] and [`Lazy::new_alone`].
    ///
    /// # Safety
    ///
    /// Same contract as [`Lazy::new`]; additionally, callers passing
    /// `alone = true` must also satisfy [`Lazy::new_alone`]'s contract.
    unsafe fn new_inner(key: StorageKey, offset: u8, alone: bool, host: Host) -> Self {
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
            alone,
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
    /// UTF-8, so invalid byte sequences in storage are replaced with U+FFFD.
    /// A Solidity contract reading the same slot sees the raw bytes verbatim
    /// — `string` in solc is just `bytes` with a UTF-8 hint and has no
    /// decode step. If you need byte-exact roundtrips (e.g. on-chain
    /// `keccak256` matching an off-chain hash), use [`Lazy<Bytes>`] instead
    /// — it preserves every byte. See also `Mapping::get` for the same
    /// caveat on `V = String`.
    ///
    /// [`Lazy<Bytes>`]: pvm_contract_types::Bytes
    pub fn get(&self) -> T {
        let () = Self::_SIZE_CHECK;
        if T::PACKED_BYTES < 32 {
            // Packed sub-slot path: read the slot, unpack our byte window via
            // the polymorphic dispatch hook. The hook delegates to
            // `<T as StoragePackable>::unpack_from` for packable T; full-slot
            // and dynamic T never reach this branch.
            let buf = storage_get_32(&self.host, self.key.as_bytes());
            T::__unpack_from_dispatched(&buf, self.offset as usize)
        } else {
            // Full-slot OR dynamic — each type owns its access pattern.
            // Primitives do one SLOAD; tuples do N SLOADs; String/Bytes do
            // header + body. `Lazy<T>` doesn't need to know which.
            T::read_from_storage(&self.host, self.key.as_bytes())
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
    /// For dynamic types (`String`, `Bytes`, structs with a dynamic field),
    /// "present" is decided by the **header slot** at `self.key`: a non-zero
    /// header (including the empty-string sentinel) → `Some(value)` with
    /// the full body loaded; a zero header → `None`. Each dynamic type
    /// owns its own `StorageDecode::try_read_from_storage` impl that
    /// implements this check.
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
        // if we never wrote. Solidity has the same conflation. We keep
        // `try_get` for full-slot and reject it for packed with a clear
        // compile-time message.
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
        // Each type owns its presence check:
        // - static types: all-zero slots → None (Solidity-compat semantics)
        // - dynamic types (String/Bytes): zero header slot → None
        T::try_read_from_storage(&self.host, self.key.as_bytes())
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
    ///
    /// **Fast path when alone in slot** (`self.alone == true`): when the
    /// caller has proven the slot has no neighbours — every contract field
    /// with no sub-word siblings, and every `Mapping` entry (derived slot
    /// is unique per key) — the SLOAD is skipped. `set` writes a fresh
    /// zero-initialised 32-byte buffer with the value packed at `offset`,
    /// recovering main-branch write cost.
    pub fn set(&mut self, value: &T) {
        let () = Self::_SIZE_CHECK;
        if T::PACKED_BYTES < 32 && self.alone {
            // Sub-word + alone-in-slot: no neighbours to preserve. Write a
            // fresh zero-padded 32-byte slot with the value packed at our
            // offset. Skips the SLOAD the RMW path would do.
            let mut buf = [0u8; 32];
            value.__pack_into_dispatched(&mut buf, self.offset as usize);
            storage_set_32(&self.host, self.key.as_bytes(), &buf);
        } else if T::PACKED_BYTES < 32 {
            // Packed sub-slot RMW: load slot, zero our window, write our
            // bytes back via the polymorphic dispatch hook, store. One extra
            // SLOAD on each write vs. the full-slot path — same gas profile
            // as solc for adjacent sub-32-byte fields sharing a slot.
            // `__pack_into_dispatched` delegates to
            // `<T as StoragePackable>::pack_into` for packable T; full-slot T
            // never reaches this branch.
            let mut buf = storage_get_32(&self.host, self.key.as_bytes());
            let off = self.offset as usize;
            buf[off..off + T::PACKED_BYTES].fill(0);
            value.__pack_into_dispatched(&mut buf, off);
            storage_set_32(&self.host, self.key.as_bytes(), &buf);
        } else {
            // Full-slot OR dynamic — each type owns its write pattern.
            value.write_to_storage(&self.host, self.key.as_bytes());
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

    fn new_at(key: StorageKey, offset: u8, alone: bool, host: Host) -> Self {
        // SAFETY: `new_at` is the safe public entry point for macro-generated
        // storage construction. The macro emits this call inside a contract
        // struct's field initializer, where Rust's borrow check on the
        // surrounding struct then gates `&self` / `&mut self` access to the
        // resulting handle. `Lazy::new`/`new_alone` are `unsafe` only because
        // direct user-code calls would let `&self` methods reconstruct a
        // writable handle — that bypass cannot happen through this trait
        // method. The macro derives `alone` from its layout walker (true iff
        // no sibling field shares this slot); `Mapping` passes
        // `alone = true` because each entry lives at a uniquely keyed slot.
        if alone {
            unsafe { Lazy::new_alone(key, offset, host) }
        } else {
            unsafe { Lazy::new(key, offset, host) }
        }
    }

    /// Clear every slot occupied by this value.
    ///
    /// **Packed sub-word T** (`PACKED_BYTES < 32`): read-modify-write that
    /// zeros only the field's byte window — neighbours sharing the slot are
    /// preserved. If the resulting slot is all-zero (no neighbour written),
    /// the host's `set_storage_or_clear` auto-deletes the slot. When the
    /// handle was constructed with `alone = true` (no possible neighbour),
    /// the SLOAD is skipped and the slot is auto-deleted directly.
    ///
    /// **Dynamic body** (`String`, `Bytes`): clears the inline header AND
    /// any spilled body chunks at `keccak256(slot) + i`. No storage leak
    /// even after a previously-long value is cleared.
    ///
    /// **Multi-slot static T** (e.g. `(U256, U256)`): clears all
    /// `T::STORAGE_SLOTS` consecutive slots; each auto-deletes individually.
    fn clear(&mut self) {
        let () = Self::_SIZE_CHECK;
        if T::PACKED_BYTES < 32 && self.alone {
            // Alone-in-slot sub-word clear: no neighbours to preserve, write
            // all-zero directly. `set_storage_or_clear` auto-deletes the slot.
            storage_set_32(&self.host, self.key.as_bytes(), &[0u8; 32]);
        } else if T::PACKED_BYTES < 32 {
            // Packed sub-slot clear: RMW that zeros only our window. If our
            // zeroing leaves the slot all-zero (no neighbour present), the
            // host auto-deletes on store anyway.
            let mut buf = storage_get_32(&self.host, self.key.as_bytes());
            let off = self.offset as usize;
            buf[off..off + T::PACKED_BYTES].fill(0);
            storage_set_32(&self.host, self.key.as_bytes(), &buf);
        } else {
            // Full-slot OR dynamic — each type owns its clear pattern.
            <T as StorageEncode>::clear_storage(&self.host, self.key.as_bytes());
        }
    }
}

/// `Lazy<T>` as a container value: `Mapping<K, Lazy<T>>` (the simplest
/// storage-typed value) and `StorageVec<Lazy<T>>`. It behaves as a transparent
/// handle — `get`/`entry` hand out a `Ref`/`RefMut<Lazy<T>>` guard (call
/// `.get()`/`.set()` on it), forwarding `T`'s packing/slot/dynamic shape so the
/// on-chain layout matches storing `T` directly. Prefer `Mapping<K, T>` unless
/// you specifically want the handle form.
impl<T: StorageEncode + StorageDecode> StorageType for Lazy<T> {
    const SLOTS: u64 = T::STORAGE_SLOTS as u64;
    const PACKED_BYTES: usize = T::PACKED_BYTES;
    const HAS_DYNAMIC_BODY: bool = T::HAS_DYNAMIC_BODY;
    // A dynamic-body `T` (String/Bytes) must recurse to tear down spilled
    // chunks; a static `T` bulk-zeroes.
    const NEEDS_RECURSIVE_CLEAR: bool = T::HAS_DYNAMIC_BODY;

    type Get<'a>
        = Ref<'a, Lazy<T>>
    where
        Self: 'a;
    type GetMut<'a>
        = RefMut<'a, Lazy<T>>
    where
        Self: 'a;

    fn get_at(key: StorageKey, offset: u8, host: &Host) -> Ref<'_, Lazy<T>> {
        // SAFETY: wrapped in a read-only `Ref`; the caller's `&self` borrow
        // gates mutation.
        Ref::new(unsafe { Lazy::<T>::new(key, offset, host.clone()) })
    }

    unsafe fn get_mut_at(
        key: StorageKey,
        offset: u8,
        alone: bool,
        host: &Host,
    ) -> RefMut<'_, Lazy<T>> {
        let cell = if alone {
            unsafe { Lazy::<T>::new_alone(key, offset, host.clone()) }
        } else {
            unsafe { Lazy::<T>::new(key, offset, host.clone()) }
        };
        RefMut::new(cell)
    }

    unsafe fn clear_at(key: StorageKey, offset: u8, alone: bool, host: &Host) {
        let mut cell = if alone {
            unsafe { Lazy::<T>::new_alone(key, offset, host.clone()) }
        } else {
            unsafe { Lazy::<T>::new(key, offset, host.clone()) }
        };
        <Lazy<T> as StorageComponent>::clear(&mut cell);
    }
}

/// `Lazy<T>` is a storage handle around `T`; in layout JSON it's named by `T`.
///
/// The macro names every field through `<#ty as StorageTypeName>::name()`, so
/// this explicit impl is what gives `Lazy<T>` its name — and it keeps working
/// when a contract author aliases the type (`type Counter = Lazy<U256>;`), where
/// the field's syntactic ident is "Counter", not "Lazy". There is no blanket
/// `StorageTypeName` impl (`Lazy` doesn't implement `SolEncode`), so without
/// this impl that path would fail.
#[cfg(feature = "abi-gen")]
impl<T: pvm_contract_types::StorageTypeName> pvm_contract_types::StorageTypeName for Lazy<T> {
    fn name() -> alloc::string::String {
        <T as pvm_contract_types::StorageTypeName>::name()
    }
}

/// `Lazy<T>` as a layout-emit leaf — a single entry at `(base, offset)`.
/// `offset` carries the packed sub-word placement (e.g. a `Lazy<u128>`
/// sharing a slot lands at offset 16) so the rendered layout matches solc.
#[cfg(feature = "abi-gen")]
impl<T: pvm_contract_types::StorageTypeName> StorageLayoutEmit for Lazy<T> {
    fn emit_entries(
        base: u64,
        offset: u8,
        name_prefix: &str,
        out: &mut Vec<pvm_contract_types::StorageLayoutEntry>,
    ) {
        out.push(pvm_contract_types::StorageLayoutEntry {
            label: String::from(name_prefix),
            slot: alloc::format!("{}", base),
            offset,
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
    /// Wrap `inner` in a read-only guard. `#[doc(hidden)] pub` so
    /// macro-generated container `StorageType` impls (e.g. `#[storage]`
    /// sub-structs) can construct it; not part of the advertised API. Only
    /// exposes `&self` methods on `inner` via `Deref`, so this grants no
    /// capability beyond what the caller already holds.
    #[doc(hidden)]
    pub fn new(inner: T) -> Self {
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
    /// Wrap `inner` in a mutable guard. `#[doc(hidden)] pub` for the same
    /// reason as [`Ref::new`]; the caller already owns `inner`, so this grants
    /// no capability beyond forwarding its methods.
    #[doc(hidden)]
    pub fn new(inner: T) -> Self {
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

    fn new_at(key: StorageKey, offset: u8, alone: bool, host: Host) -> Self {
        debug_assert!(
            offset == 0,
            "Mapping<K, V> always full-slot; offset must be 0"
        );
        // `alone` is meaningless for a full-slot component — the root header
        // always owns its slot byte-for-byte (`PACKED_BYTES == 32`). Accept
        // the argument from the trait signature but discard it.
        let _ = (offset, alone);
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

/// `Mapping<K, V>` in layout JSON is `mapping(K_name => V_name)`. This impl
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
            "mapping({} => {})",
            <K as pvm_contract_types::StorageTypeName>::name(),
            <V as pvm_contract_types::StorageTypeName>::name(),
        )
    }
}

/// `Mapping<K, V>` as a layout-emit leaf — a single `mapping(K => V)` entry.
/// A mapping always claims a fresh slot (`PACKED_BYTES == 32`), so `offset`
/// is always `0` here; it is threaded through for signature uniformity.
#[cfg(feature = "abi-gen")]
impl<K: pvm_contract_types::StorageTypeName, V: pvm_contract_types::StorageTypeName>
    StorageLayoutEmit for Mapping<K, V>
{
    fn emit_entries(
        base: u64,
        offset: u8,
        name_prefix: &str,
        out: &mut Vec<pvm_contract_types::StorageLayoutEntry>,
    ) {
        out.push(pvm_contract_types::StorageLayoutEntry {
            label: String::from(name_prefix),
            slot: alloc::format!("{}", base),
            offset,
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

// ---------------------------------------------------------------------------
// Mapping value access — unified over StorageType (issue #108).
//
// One `get`/`entry`/`delete` surface covers every value shape. For a leaf V,
// `get` returns the value and `entry` a `Lazy<V>` cursor (read-then-write on a
// single keccak). For a container V (`Mapping`, `StorageVec`, `#[storage]`
// struct), `get` returns a read-only `Ref<V>` and `entry` a `RefMut<V>` — this
// subsumes the old `view`/`view_mut`/`delete` block and the dedicated
// `Mapping<K, StorageVec<T>>` block, and composes to any nesting depth.
// By-value `insert`/`try_get`/`remove` live on the `SimpleStorageType` tier.
// ---------------------------------------------------------------------------

impl<K: AsStorageKey, V: StorageType> Mapping<K, V> {
    /// Canonical byte offset for an entry: solc right-aligns a sub-word value
    /// within its derived slot (`32 - PACKED_BYTES`). Full-slot V — all
    /// containers and `U256`-shaped leaves — yields `0`.
    const fn entry_offset() -> u8 {
        (32 - V::PACKED_BYTES) as u8
    }

    /// Read the value at `key`. Returns the value for a leaf V, or a read-only
    /// [`Ref`] guard for a container V — the `Get` GAT chooses.
    ///
    /// The derived slot `keccak256(pad32(key) ++ pad32(root))` is unique to
    /// `key`, so the entry is always alone in its slot.
    ///
    /// **Lossy decode for `V = String`:** invalid UTF-8 in storage is replaced
    /// with U+FFFD; use `Mapping<K, Bytes>` for byte-exact roundtrips.
    pub fn get(&self, key: &K) -> V::Get<'_> {
        V::get_at(self.slot_of(key), Self::entry_offset(), &self.host)
    }

    /// Derive the slot once and return a mutable accessor: a [`Lazy<V>`] cursor
    /// for a leaf V (read-then-write on one keccak — the ERC-20 `entry` idiom),
    /// or a [`RefMut`] guard for a container V. Requires `&mut self`.
    pub fn entry(&mut self, key: &K) -> V::GetMut<'_> {
        // SAFETY: `&mut self` proves mutating access; the derived slot is
        // key-unique so `alone = true`, and the returned cursor/guard is tied
        // to this borrow (or, for a leaf, is an owned `Lazy` handle).
        unsafe { V::get_mut_at(self.slot_of(key), Self::entry_offset(), true, &self.host) }
    }

    /// Delete the entry at `key`, clearing every slot it occupies. Recurses for
    /// a container V; a `Mapping` value is a no-op (its entries live at
    /// underivable keys — matches solc's `delete`).
    pub fn delete(&mut self, key: &K) {
        // SAFETY: `&mut self`; key-unique derived slot.
        unsafe { V::clear_at(self.slot_of(key), Self::entry_offset(), true, &self.host) }
    }
}

impl<K: AsStorageKey, V: SimpleStorageType> Mapping<K, V> {
    /// Read the value, returning `None` if the entry reads back zero (never
    /// written or cleared) — Solidity's zero-slot semantics.
    pub fn try_get(&self, key: &K) -> Option<V::Value> {
        V::try_read_value(self.slot_of(key), Self::entry_offset(), &self.host)
    }

    /// Write a value at `key`.
    pub fn insert(&mut self, key: &K, value: &V::Value) {
        V::write_value(
            value,
            self.slot_of(key),
            Self::entry_offset(),
            true,
            &self.host,
        );
    }

    /// Delete the entry at `key` (leaf-value alias of [`delete`](Mapping::delete)).
    pub fn remove(&mut self, key: &K) {
        self.delete(key);
    }
}

/// A `Mapping<K, V>` can itself be an *element/value* of another container,
/// enabling `Mapping<K1, Mapping<K2, V>>` (nested mappings) and
/// `StorageVec<Mapping<K, V>>` (`mapping(...)[]`) through the generic container
/// impls, with no per-shape code.
impl<K, V: StorageType> StorageType for Mapping<K, V> {
    const SLOTS: u64 = 1;
    const PACKED_BYTES: usize = 32;
    const HAS_DYNAMIC_BODY: bool = false;
    // A mapping stores nothing at its root and its entries live at underivable
    // keys, so there is nothing to recurse into on clear — bulk-zeroing the
    // (empty) root slot is correct and matches solc's `delete` on a mapping.
    const NEEDS_RECURSIVE_CLEAR: bool = false;

    type Get<'a>
        = Ref<'a, Mapping<K, V>>
    where
        Self: 'a;
    type GetMut<'a>
        = RefMut<'a, Mapping<K, V>>
    where
        Self: 'a;

    fn get_at(key: StorageKey, offset: u8, host: &Host) -> Ref<'_, Mapping<K, V>> {
        debug_assert_eq!(offset, 0, "Mapping element always full-slot");
        // SAFETY: wrapped in a read-only `Ref`; the caller's `&self` borrow
        // gates mutation.
        Ref::new(unsafe { Mapping::<K, V>::new(key, host.clone()) })
    }

    unsafe fn get_mut_at(
        key: StorageKey,
        offset: u8,
        alone: bool,
        host: &Host,
    ) -> RefMut<'_, Mapping<K, V>> {
        debug_assert_eq!(offset, 0, "Mapping element always full-slot");
        let _ = alone;
        // SAFETY: the caller holds mutating access; the `RefMut` lifetime ties
        // the handle to that borrow.
        RefMut::new(unsafe { Mapping::<K, V>::new(key, host.clone()) })
    }

    unsafe fn clear_at(_key: StorageKey, _offset: u8, _alone: bool, _host: &Host) {
        // No-op: a mapping has no root header to clear and its entries live at
        // underivable keys — matches solc's `delete mapping`.
    }
}

// ---------------------------------------------------------------------------
// StorageVec<T> — dynamic array with Solidity-compatible storage layout.
// ---------------------------------------------------------------------------

/// A dynamic array backed by on-chain storage, matching Solidity's `T[]`
/// storage layout byte-for-byte.
///
/// The element count lives at the root slot encoded as `uint256`
/// (big-endian). Element `i`'s slot is `keccak256(pad32(slot)) + stride(i)`,
/// where the stride depends on `T`'s shape:
/// - sub-word `T` (`PACKED_BYTES < 32`): `stride(i) = i / per_slot`, where
///   `per_slot = 32 / PACKED_BYTES` (multiple elements share a slot).
/// - single-slot `T` (`PACKED_BYTES == 32, STORAGE_SLOTS == 1`):
///   `stride(i) = i` (one slot per element).
/// - multi-slot static `T` (`STORAGE_SLOTS > 1`):
///   `stride(i) = i * STORAGE_SLOTS` (each element walks `STORAGE_SLOTS`
///   consecutive slots).
///
/// `StorageVec<u8>` corresponds to Solidity's `uint8[]` (one byte per
/// element, 32 elements per slot) — **distinct from**
/// [`Bytes`](pvm_contract_types::Bytes), which models Solidity's `bytes` type
/// (inline header or spilled body). Use `Bytes` when you need `bytes`-shaped
/// storage; use `StorageVec<u8>` when you need a `uint8[]` array.
///
/// # API summary
///
/// - **Read:** `len` / `is_empty`, `get(i)` (panics OOB) / `try_get(i)`
///   (`Option`), `first` / `last`, and [`iter`](Self::iter) (reads the
///   length once, then streams elements — cheaper than a manual
///   `0..len`/`get` loop). All take `&self`, so they work in `view` methods.
/// - **Write:** `push`, `pop`, `set(i, &value)` (direct-write — no
///   per-element handle on flat `StorageVec<T>`), and `clear`. All take
///   `&mut self`.
///
/// # Notable design choices
///
/// - `get(i)` / `pop()` return `T` by value.
/// - Per-element handles only appear on the nested impl
///   (`StorageVec<StorageVec<T>>`), where `entry(i)` / `grow()` return
///   a `RefMut<'_, StorageVec<T>>`.
/// - `pop()` zeros the freed slot only when the freed element was the first
///   packed element in its slot — the gas-optimal policy that matches solc.
///   For full-slot elements, every pop frees a full slot.
/// - Out-of-bounds `get`/`set` revert via a plain trap with a static message
///   (no `core::fmt` in the bytecode), **not** solc's ABI-encoded
///   `Panic(0x32)` — off-chain callers won't see the `0x32` code. Use
///   `try_get` to avoid the trap.
/// - The length is read as a `u64`; a stored length exceeding `u64::MAX`
///   (unreachable through this API — only via corrupted state or raw uAPI)
///   traps intentionally rather than silently truncating to a smaller value.
///
/// # Element shapes supported
///
/// All `T: StorageEncode + StorageDecode` with `T::STORAGE_SLOTS <=
/// MAX_STATIC_SLOTS`. The implementation dispatches on `T`'s properties:
///
/// - **Sub-word multi-pack** (`T::PACKED_BYTES < 32`): elements share a
///   32-byte slot, `per_slot = 32 / PACKED_BYTES` elements per slot, packed
///   right-aligned (solc-compatible). Covers `uint8`..`uint128`,
///   `int8`..`int128`, `bool`, `Address` (`per_slot = 1`), and `[u8; N]` for
///   `N < 32`. `set` does read-modify-write to preserve neighbours; `pop`
///   clears the whole slot only when the freed element was the first one in
///   its slot.
/// - **Single-slot full-word** (`STORAGE_SLOTS == 1, PACKED_BYTES == 32`):
///   one slot per element, fast path with no RMW. Covers `U256`, `I256`,
///   `[u8; 32]` (i.e. `bytes32`), `[T; N]` whose total bytes fit in one
///   slot, and single-slot derived structs.
/// - **Multi-slot static** (`STORAGE_SLOTS > 1, !HAS_DYNAMIC_BODY`):
///   stride of `STORAGE_SLOTS` slots per element. Covers tuples, fixed
///   arrays `[T; N]` that span >1 slot (e.g. `[U256; 3]`, `[u32; 9]`), and
///   derived structs that span 2..=8 slots.
/// - **Dynamic-body** (`HAS_DYNAMIC_BODY`): each element gets its own
///   inline/spilled layout — header lives in the element's slot, spilled
///   body at `keccak256(header_slot) + i`. Covers `String` and `Bytes`.
///
/// Nested arrays (`StorageVec<StorageVec<T>>`, i.e. Solidity's `T[][]`)
/// are supported via the dedicated nested impl block below.
pub struct StorageVec<T> {
    root: StorageKey,
    base: core::cell::OnceCell<[u8; 32]>,
    host: Host,
    _marker: PhantomData<T>,
}

impl<S: StorageType> StorageVec<S> {
    /// Compile-time shape validation. Referencing `_SHAPE_CHECK` in every
    /// public method forces the const evaluator to run the check at each
    /// monomorphization — same pattern as `Lazy::_SIZE_CHECK`.
    ///
    /// Note: the `SLOTS <= MAX_STATIC_SLOTS` bound is NOT asserted here — it
    /// only matters for *value* elements that materialize into a stack buffer,
    /// and that path is re-checked by `Lazy::_SIZE_CHECK` inside every leaf
    /// `get_at`/`get_mut_at`. Handle elements (`Mapping`, `StorageVec`,
    /// multi-slot `#[storage]` structs) never materialize, so they may exceed
    /// `MAX_STATIC_SLOTS` legitimately.
    const _SHAPE_CHECK: () = {
        assert!(S::SLOTS >= 1, "StorageVec<S>: S::SLOTS must be positive");
        // Sub-word multi-pack types always occupy a single slot. solc has no
        // notion of "multi-slot sub-word" — every sub-word value claims at
        // most one slot.
        assert!(
            S::PACKED_BYTES == 32 || S::SLOTS == 1,
            "StorageVec<S>: sub-word S (PACKED_BYTES < 32) must satisfy SLOTS == 1"
        );
    };

    /// Create a new `StorageVec` rooted at the given storage key.
    ///
    /// # Safety
    ///
    /// Same safety contract as [`Lazy::new`] and [`Mapping::new`]. Direct
    /// construction outside macro-generated code lets a `&self` (view)
    /// method reconstruct a writable handle and bypass the borrow-check
    /// view gate. Use [`StorageComponent::new_at`] from macro expansion;
    /// reach for this constructor only when an arbitrary `StorageKey` is
    /// required. Contract crates that want belt-and-braces enforcement
    /// should add `#![forbid(unsafe_code)]` at the crate root.
    pub unsafe fn new(root: StorageKey, host: Host) -> Self {
        let () = Self::_SHAPE_CHECK;
        StorageVec {
            root,
            base: core::cell::OnceCell::new(),
            host,
            _marker: PhantomData,
        }
    }

    /// Lazily compute and cache the body base `keccak256(pad32(slot))`.
    /// View methods that touch only the length (`len`, `is_empty`) skip
    /// this — only element accessors trigger the keccak.
    fn body_base(&self) -> &[u8; 32] {
        self.base
            .get_or_init(|| storage_derive_body_base(&self.host, self.root.as_bytes()))
    }

    /// Elements per storage slot for sub-word packing. Always `1` for
    /// full-slot `S` (`PACKED_BYTES == 32` — every handle, and full-word
    /// leaves); for sub-word `S` returns `32 / PACKED_BYTES`.
    const fn per_slot() -> u64 {
        if S::PACKED_BYTES == 32 {
            1
        } else {
            (32 / S::PACKED_BYTES) as u64
        }
    }

    /// Slot index (offset from `body_base`) for element `i`.
    /// - Sub-word leaf: `i / per_slot` (multiple elements share a slot)
    /// - Multi-slot element (`SLOTS > 1`): `i * SLOTS` (stride)
    /// - Single-slot leaf / single-slot handle: `i`
    fn slot_index_for(i: u64) -> u64 {
        if S::PACKED_BYTES < 32 {
            i / Self::per_slot()
        } else if S::SLOTS > 1 {
            // Multi-slot element. `checked_mul` so a corrupted length /
            // pathologically large `i` surfaces as a clean panic rather than
            // silently wrapping into the wrong slot.
            i.checked_mul(S::SLOTS)
                .expect("StorageVec: element-stride overflow")
        } else {
            i
        }
    }

    /// Byte offset within slot for sub-word element `i`. Solc places the
    /// element at index 0 right-aligned (lowest within the slot):
    /// `offset = 32 - PACKED_BYTES * (within + 1)`. Full-slot elements
    /// (`PACKED_BYTES == 32` — all handles) get offset `0`.
    fn element_offset(i: u64) -> u8 {
        if S::PACKED_BYTES < 32 {
            let within = (i % Self::per_slot()) as usize;
            (32 - S::PACKED_BYTES * (within + 1)) as u8
        } else {
            0
        }
    }

    /// Storage key for element `i`'s base slot.
    fn element_slot(&self, i: u64) -> StorageKey {
        let mut key = *self.body_base();
        inc_slot_by(&mut key, Self::slot_index_for(i));
        StorageKey::from_raw(key)
    }

    /// Return the number of elements.
    pub fn len(&self) -> u64 {
        let () = Self::_SHAPE_CHECK;
        read_len_u64(&self.host, self.root.as_bytes())
    }

    /// Return `true` if the array contains no elements.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Read the element at `index`. Returns the value for a leaf element, or a
    /// read-only [`Ref`] guard for a container element (`StorageVec`,
    /// `Mapping`, `#[storage]` struct) — the `Get` GAT chooses.
    ///
    /// # Panics
    ///
    /// Panics (reverts) if `index >= len()`. The revert is a plain trap, not
    /// solc's ABI-encoded `Panic(0x32)`. Use [`try_get`](Self::try_get) for a
    /// non-panicking read.
    pub fn get(&self, index: u64) -> S::Get<'_> {
        let () = Self::_SHAPE_CHECK;
        assert!(index < self.len(), "StorageVec::get: index out of bounds");
        S::get_at(self.element_slot(index), Self::element_offset(index), &self.host)
    }

    /// Read the element at `index`, returning `None` if out of bounds.
    pub fn try_get(&self, index: u64) -> Option<S::Get<'_>> {
        let () = Self::_SHAPE_CHECK;
        if index >= self.len() {
            return None;
        }
        Some(S::get_at(
            self.element_slot(index),
            Self::element_offset(index),
            &self.host,
        ))
    }

    /// Read-only access to the first element, or `None` if empty.
    pub fn first(&self) -> Option<S::Get<'_>> {
        self.try_get(0)
    }

    /// Read-only access to the last element, or `None` if empty.
    pub fn last(&self) -> Option<S::Get<'_>> {
        let () = Self::_SHAPE_CHECK;
        let len = self.len();
        if len == 0 {
            None
        } else {
            Some(S::get_at(
                self.element_slot(len - 1),
                Self::element_offset(len - 1),
                &self.host,
            ))
        }
    }

    /// Mutable access to the element at `index`. Returns a [`Lazy`] write
    /// cursor for a leaf element, or a [`RefMut`] guard for a container.
    ///
    /// # Panics
    ///
    /// Panics (reverts) if `index >= len()`.
    pub fn entry(&mut self, index: u64) -> S::GetMut<'_> {
        let () = Self::_SHAPE_CHECK;
        assert!(
            index < self.len(),
            "StorageVec::entry: index out of bounds"
        );
        self.elem_mut(index)
    }

    /// Iterate over the elements front to back, yielding each element's `Get`
    /// (value for leaves, [`Ref`] for containers).
    ///
    /// The length is read **once** at construction. The iterator borrows the
    /// vec immutably, so it composes with `view` methods.
    pub fn iter(&self) -> StorageVecIter<'_, S> {
        let () = Self::_SHAPE_CHECK;
        StorageVecIter {
            vec: self,
            pos: 0,
            len: self.len(),
        }
    }

    /// Append a fresh (zero) element and return a mutable handle to it. For a
    /// leaf, populate it via the returned `Lazy` cursor's `set`; for a
    /// container, operate on the returned `RefMut` (`push`, `insert`, ...).
    /// This is the by-handle append that works for *any* element shape;
    /// [`push`](Self::push) is the by-value convenience for leaf elements.
    ///
    /// # Panics
    ///
    /// Panics if the length would overflow `u64::MAX`.
    pub fn grow(&mut self) -> S::GetMut<'_> {
        let () = Self::_SHAPE_CHECK;
        let len = self.len();
        let new_len = len
            .checked_add(1)
            .expect("StorageVec::grow: length overflow");
        write_len_u64(&self.host, self.root.as_bytes(), new_len);
        self.elem_mut(len)
    }

    /// Remove the last element, clearing its storage (recursively for
    /// container elements). Returns `true` if an element was removed.
    ///
    /// Unlike [`pop`](Self::pop) (leaf-only, returns the value), this works
    /// for any element shape and does not return the removed element — a
    /// container element cannot be materialized by value.
    pub fn erase_last(&mut self) -> bool {
        let () = Self::_SHAPE_CHECK;
        let len = self.len();
        if len == 0 {
            return false;
        }
        let new_len = len - 1;
        self.elem_clear(new_len);
        write_len_u64(&self.host, self.root.as_bytes(), new_len);
        true
    }

    /// Remove every element and reset length to zero.
    ///
    /// **O(n) gas.** For static leaf elements this bulk-zeroes the touched
    /// body slots; for container / dynamic-body elements it recurses per
    /// element (`S::clear_at`) so derived sub-slots and spilled bodies are
    /// also cleared — matching solc's `delete arr`.
    pub fn clear(&mut self) {
        let () = Self::_SHAPE_CHECK;
        let len = self.len();
        if len > 0 {
            if S::NEEDS_RECURSIVE_CLEAR {
                // Container or dynamic-body element: each owns storage at
                // derived keys / spilled chunks, so recurse per element.
                for i in 0..len {
                    self.elem_clear(i);
                }
            } else if S::PACKED_BYTES < 32 {
                // Sub-word leaf: clear every touched body slot, ceil(len/per).
                let per = Self::per_slot();
                let total_slots = len.div_ceil(per);
                let mut key = *self.body_base();
                for _ in 0..total_slots {
                    storage_set_32(&self.host, &key, &[0u8; 32]);
                    inc_be_32(&mut key);
                }
            } else {
                // Single-slot / multi-slot static leaf: clear
                // `len * SLOTS` consecutive slots.
                let total_slots = len
                    .checked_mul(S::SLOTS)
                    .expect("StorageVec::clear: total-slots overflow");
                let mut key = *self.body_base();
                for _ in 0..total_slots {
                    storage_set_32(&self.host, &key, &[0u8; 32]);
                    inc_be_32(&mut key);
                }
            }
        }
        storage_set_32(&self.host, self.root.as_bytes(), &[0u8; 32]);
    }

    /// Mutable accessor for element `i` (no bounds check). Sub-word leaves
    /// pack multiple per slot, so `alone` is false unless one element fills
    /// its slot; full-slot elements are always alone.
    fn elem_mut(&mut self, i: u64) -> S::GetMut<'_> {
        let alone = Self::per_slot() == 1;
        // SAFETY: `&mut self` proves mutating access through the parent borrow;
        // the returned guard's lifetime ties it to that borrow.
        unsafe { S::get_mut_at(self.element_slot(i), Self::element_offset(i), alone, &self.host) }
    }

    /// Clear the storage for element `i` (no bounds check). For sub-word
    /// leaves, `alone == (within == 0)` reproduces the whole-slot-vs-RMW
    /// pop policy; container / dynamic elements recurse via `S::clear_at`.
    fn elem_clear(&mut self, i: u64) {
        let alone = if S::PACKED_BYTES < 32 {
            i.is_multiple_of(Self::per_slot())
        } else {
            true
        };
        // SAFETY: `&mut self`; see `elem_mut`.
        unsafe { S::clear_at(self.element_slot(i), Self::element_offset(i), alone, &self.host) }
    }
}

/// By-value element operations — only for leaf (`SimpleStorageType`) elements.
/// Container elements use [`entry`](StorageVec::entry) / [`grow`](StorageVec::grow) /
/// [`erase_last`](StorageVec::erase_last) instead.
impl<S: SimpleStorageType> StorageVec<S> {
    /// Overwrite the element at `index`.
    ///
    /// # Panics
    ///
    /// Panics (reverts) if `index >= len()`.
    pub fn set(&mut self, index: u64, value: &S::Value) {
        let () = Self::_SHAPE_CHECK;
        assert!(index < self.len(), "StorageVec::set: index out of bounds");
        self.write_value_at(index, value);
    }

    /// Append an element by value, then increment the length.
    ///
    /// # Panics
    ///
    /// Panics if the length would overflow `u64::MAX`.
    pub fn push(&mut self, value: &S::Value) {
        let () = Self::_SHAPE_CHECK;
        let len = self.len();
        let new_len = len
            .checked_add(1)
            .expect("StorageVec::push: length overflow");
        self.write_value_at(len, value);
        write_len_u64(&self.host, self.root.as_bytes(), new_len);
    }

    /// Remove and return the last element by value, or `None` if empty. The
    /// freed slot(s) are cleared (SSTORE-to-zero refund), matching solc.
    pub fn pop(&mut self) -> Option<S::Value> {
        let () = Self::_SHAPE_CHECK;
        let len = self.len();
        if len == 0 {
            return None;
        }
        let new_len = len - 1;
        let value = S::read_value(
            self.element_slot(new_len),
            Self::element_offset(new_len),
            &self.host,
        );
        self.elem_clear(new_len);
        write_len_u64(&self.host, self.root.as_bytes(), new_len);
        Some(value)
    }

    /// Write `value` at element `i` (no bounds check).
    fn write_value_at(&mut self, i: u64, value: &S::Value) {
        let alone = Self::per_slot() == 1;
        S::write_value(
            value,
            self.element_slot(i),
            Self::element_offset(i),
            alone,
            &self.host,
        );
    }
}

impl<S: StorageType> StorageComponent for StorageVec<S> {
    /// One root slot for the length header. Elements live at
    /// `keccak256(slot) + i` and consume no additional contract-layout slots.
    const SLOTS: u64 = 1;

    /// Never packs with neighbours — the length header always claims a full
    /// slot. Matches `Mapping`'s `PACKED_BYTES = 32` and solc's storage
    /// layout for dynamic arrays.
    const PACKED_BYTES: usize = 32;

    fn new_at(key: StorageKey, offset: u8, alone: bool, host: Host) -> Self {
        debug_assert_eq!(offset, 0, "StorageVec<S> always full-slot; offset must be 0");
        // Full-slot component: the length header always owns its slot and
        // elements live at derived keys, so `offset` / `alone` are irrelevant.
        let _ = (offset, alone);
        // SAFETY: macro-only safe entry point. See `Lazy::new_at`.
        unsafe { StorageVec::<S>::new(key, host) }
    }

    /// Remove every element and reset the length header to zero — solc's
    /// `delete arr`. O(n) in the element count; see [`StorageVec::clear`].
    fn clear(&mut self) {
        // Inherent `clear` (resolves before the trait method of the same name).
        StorageVec::<S>::clear(self)
    }
}

/// A `StorageVec<S>` can itself be an *element* of another container, enabling
/// `StorageVec<StorageVec<T>>` (`T[][]`), `Mapping<K, StorageVec<T>>`
/// (`mapping(K => T[])`), and deeper nesting — all through the generic
/// container impls, with no per-shape code.
impl<S: StorageType> StorageType for StorageVec<S> {
    const SLOTS: u64 = 1;
    const PACKED_BYTES: usize = 32;
    const HAS_DYNAMIC_BODY: bool = false;
    // A vec owns element storage at derived keys — clearing must recurse.
    const NEEDS_RECURSIVE_CLEAR: bool = true;

    type Get<'a>
        = Ref<'a, StorageVec<S>>
    where
        Self: 'a;
    type GetMut<'a>
        = RefMut<'a, StorageVec<S>>
    where
        Self: 'a;

    fn get_at(key: StorageKey, offset: u8, host: &Host) -> Ref<'_, StorageVec<S>> {
        debug_assert_eq!(offset, 0, "StorageVec element always full-slot");
        // SAFETY: immediately wrapped in a read-only `Ref`, which exposes only
        // `&self` methods; the caller's `&self` borrow gates mutation.
        Ref::new(unsafe { StorageVec::<S>::new(key, host.clone()) })
    }

    unsafe fn get_mut_at(
        key: StorageKey,
        offset: u8,
        alone: bool,
        host: &Host,
    ) -> RefMut<'_, StorageVec<S>> {
        debug_assert_eq!(offset, 0, "StorageVec element always full-slot");
        let _ = alone;
        // SAFETY: the caller (a container `&mut self` method) holds mutating
        // access; the `RefMut` lifetime ties the handle to that borrow.
        RefMut::new(unsafe { StorageVec::<S>::new(key, host.clone()) })
    }

    unsafe fn clear_at(key: StorageKey, offset: u8, alone: bool, host: &Host) {
        let _ = (offset, alone);
        // SAFETY: short-lived handle used only to recursively clear the inner
        // vec's length + body slots.
        let mut inner = unsafe { StorageVec::<S>::new(key, host.clone()) };
        inner.clear();
    }
}

/// `StorageVec<T>` is Solidity's `T[]`; in layout JSON it's named `<T>[]`.
/// Recursing on the element name lets `StorageVec<StorageVec<U256>>` resolve
/// to `uint256[][]` and `Mapping<K, StorageVec<T>>` (whose `name()` calls
/// `V::name()`) nest to `mapping(K => T[])`.
#[cfg(feature = "abi-gen")]
impl<T: pvm_contract_types::StorageTypeName> pvm_contract_types::StorageTypeName for StorageVec<T> {
    fn name() -> alloc::string::String {
        alloc::format!("{}[]", <T as pvm_contract_types::StorageTypeName>::name())
    }
}

/// `StorageVec<T>` as a layout-emit leaf — a single `T[]` entry. The length
/// header always claims a fresh slot (`PACKED_BYTES == 32`), so `offset` is
/// always `0`; it is threaded through for signature uniformity. The generic
/// `T` covers the nested `StorageVec<StorageVec<U256>>` shape too, since the
/// inner `StorageVec<U256>` itself implements `StorageTypeName` above.
#[cfg(feature = "abi-gen")]
impl<T: pvm_contract_types::StorageTypeName> StorageLayoutEmit for StorageVec<T> {
    fn emit_entries(
        base: u64,
        offset: u8,
        name_prefix: &str,
        out: &mut Vec<pvm_contract_types::StorageLayoutEntry>,
    ) {
        out.push(pvm_contract_types::StorageLayoutEntry {
            label: String::from(name_prefix),
            slot: alloc::format!("{}", base),
            offset,
            ty: <Self as pvm_contract_types::StorageTypeName>::name(),
        });
    }
}

/// Iterator over a [`StorageVec<S>`], produced by [`StorageVec::iter`].
///
/// Captures the length at construction and yields each element's `Get` — the
/// value for a leaf element, a [`Ref`] guard for a container element. Holds an
/// immutable borrow of the vec (`'a`), so it composes with `view` methods and
/// the yielded guards are branded with that borrow.
pub struct StorageVecIter<'a, S: StorageType> {
    vec: &'a StorageVec<S>,
    pos: u64,
    len: u64,
}

impl<'a, S: StorageType> StorageVecIter<'a, S> {
    /// Read element `i` through the iterator's `'a` borrow of the vec (not the
    /// shorter `&mut self` borrow of `next`), so the yielded `S::Get<'a>`
    /// (e.g. `Ref<'a, _>`) is tied to the vec's lifetime, not the step's.
    fn get_at_index(&self, i: u64) -> S::Get<'a> {
        S::get_at(
            self.vec.element_slot(i),
            StorageVec::<S>::element_offset(i),
            &self.vec.host,
        )
    }
}

impl<'a, S: StorageType> Iterator for StorageVecIter<'a, S> {
    type Item = S::Get<'a>;

    fn next(&mut self) -> Option<S::Get<'a>> {
        if self.pos >= self.len {
            return None;
        }
        let item = self.get_at_index(self.pos);
        self.pos += 1;
        Some(item)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = (self.len - self.pos) as usize;
        (remaining, Some(remaining))
    }
}

impl<'a, S: StorageType> DoubleEndedIterator for StorageVecIter<'a, S> {
    fn next_back(&mut self) -> Option<S::Get<'a>> {
        if self.pos >= self.len {
            return None;
        }
        self.len -= 1;
        Some(self.get_at_index(self.len))
    }
}

impl<S: StorageType> ExactSizeIterator for StorageVecIter<'_, S> {}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests;

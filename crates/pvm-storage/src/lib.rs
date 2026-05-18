//! Typed storage helpers for PVM smart contracts with Solidity-compatible slot layout.
//!
//! Provides [`Lazy<T>`] for single-value storage and [`Mapping<K, V>`] for key-value
//! storage, both using Solidity-compatible key derivation so tools like `cast storage`
//! and `cast index` work out of the box.
//!
//! Static 32-byte values use [`Lazy<T>`] and [`Mapping<K, V>`] with `T`/`V`
//! bound to `SolEncode + StaticDecode + StaticEncodedLen`. Dynamic `bytes` /
//! `string` values use the dedicated accessors [`LazyBytes`], [`LazyString`],
//! [`MappingBytes`], and [`MappingString`] — they encode inline when
//! `len < 32` and spill to `keccak256(slot) + i` chunks otherwise, matching
//! `solc`'s storage layout.
//!
//! All accessors implement [`StorageComponent`], so they participate in the
//! auto-numbered slot layout produced by the `#[contract]` and `#[storage]`
//! macros.
//!
//! # Usage
//!
//! Inside a `#[contract]` module, declare storage fields on the contract struct.
//! Slot numbers are assigned in declaration order by default; opt out with
//! `#[slot(N)]` if you need to pin a specific slot.
//!
//! ```ignore
//! use pvm_storage::{Lazy, Mapping, StorageKey};
//!
//! let mut total_supply = Lazy::<U256>::new(StorageKey::from_slot(0), host.clone());
//! total_supply.set(&U256::from(1000));
//! assert_eq!(total_supply.get(), U256::from(1000));
//!
//! let mut balances = Mapping::<Address, U256>::new(StorageKey::from_slot(1), host);
//! balances.insert(&caller, &U256::from(500));
//! assert_eq!(balances.get(&caller), U256::from(500));
//! ```

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

// Alias so that macro-generated `::pvm_contract_sdk::` paths resolve
// within this crate's own tests. Same pattern as pvm-contract-types.
extern crate self as pvm_contract_sdk;

use core::marker::PhantomData;
use pvm_contract_types::{Host, HostApi, SolEncode, StaticDecode, StaticEncodedLen, StorageFlags};

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

/// Delete a storage slot.
///
/// pallet-revive's uapi only exposes deletion via `set_storage_or_clear` with
/// an all-zero value; there is no dedicated `clear_storage` host function.
#[cfg(feature = "alloc")]
fn storage_delete_32(host: &Host, key: &[u8; 32]) {
    host.set_storage_or_clear(StorageFlags::empty(), key, &[0u8; 32]);
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
fn storage_try_get_32(host: &Host, key: &[u8; 32]) -> Option<[u8; 32]> {
    let buf = storage_get_32(host, key);
    (buf != [0u8; 32]).then_some(buf)
}

/// Hash a 32-byte slot to produce the data root for a dynamic value
/// (`keccak256(slot)`). This matches Solidity's layout for `bytes`, `string`,
/// and arrays.
#[cfg(feature = "alloc")]
fn dynamic_data_root(host: &Host, slot: &[u8; 32]) -> [u8; 32] {
    let mut output = [0u8; 32];
    host.hash_keccak_256(slot, &mut output);
    output
}

/// Increment a 32-byte big-endian integer in-place (used to walk consecutive
/// storage slots for the body of dynamic values).
#[cfg(feature = "alloc")]
fn inc_slot(slot: &mut [u8; 32]) {
    for byte in slot.iter_mut().rev() {
        let (next, carry) = byte.overflowing_add(1);
        *byte = next;
        if !carry {
            return;
        }
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

// ---------------------------------------------------------------------------
// Dynamic value helpers (alloc-gated): solc-compatible bytes/string layout.
//
// Layout matches Solidity's `bytes` / `string` storage exactly:
//
//   Short form (raw byte length < 32):
//     slot[0..len] = body bytes (left-aligned in high-order positions)
//     slot[31]     = (len as u8) << 1               // low bit = 0
//
//   Long form (raw byte length >= 32):
//     slot         = (len * 2 + 1) as big-endian u256   // low bit = 1
//     keccak256(slot) + i  =  32-byte body chunks (i = 0, 1, ...)
//
// Empty-inline sentinel: a literal `set("")` / `set(vec![])` would otherwise
// produce an all-zero header, which `set_storage_or_clear` (the only Fix-keyed
// write uapi — see [`storage_set_32`]) auto-deletes. To keep the slot present
// in the trie so `try_get` can return `Some(empty)`, we stash a non-zero byte
// at `slot[30]` (an "empty sentinel" — `len = 0` ⇒ body occupies `slot[0..0]`,
// so the decoder never looks at bytes 0..31). For any non-empty inline value
// the length lives in `slot[31]` and the slot is already non-zero, so the
// sentinel is only injected when `len == 0`. Resolc-emitted Solidity has no
// such distinction (`SLOAD` of missing == 0 == set-to-0); we recover it here
// only for `bytes` / `string` because the decoder has a spare 31-byte field.
//
// `clear` deletes the header (and any long-form body chunks) via
// `set_storage_or_clear(_, &[0; 32])` — the only deletion path in uapi.
// ---------------------------------------------------------------------------

/// Decoded short/long header for a solc-compatible dynamic slot.
#[cfg(feature = "alloc")]
enum DynHeader {
    /// `len` bytes packed inline in the slot (`len < 32`). The slot's
    /// high-order bytes hold the data.
    Inline { len: usize },
    /// `len` bytes spilled to keccak-derived chunks (`len >= 32`).
    Spilled { len: usize },
}

#[cfg(feature = "alloc")]
fn decode_dyn_header(slot_bytes: &[u8; 32]) -> DynHeader {
    if slot_bytes[31] & 1 == 0 {
        DynHeader::Inline {
            len: (slot_bytes[31] >> 1) as usize,
        }
    } else {
        // Spilled: full slot encodes `len * 2 + 1` as a u256.
        // Length cannot exceed usize::MAX bytes in practice; the upper 192
        // bits are always zero for any plausible stored value.
        let mut len_be = [0u8; 16];
        len_be.copy_from_slice(&slot_bytes[16..32]);
        let raw = u128::from_be_bytes(len_be);
        DynHeader::Spilled {
            len: (raw >> 1) as usize,
        }
    }
}

#[cfg(feature = "alloc")]
fn encode_long_header(len: usize) -> [u8; 32] {
    // `len * 2 + 1` as a 32-byte big-endian u256. `len` fits in u64 on any
    // realistic chain so the value fits in u128 with room to spare.
    let raw: u128 = (len as u128) * 2 + 1;
    let mut out = [0u8; 32];
    out[16..32].copy_from_slice(&raw.to_be_bytes());
    out
}

#[cfg(feature = "alloc")]
fn read_dyn_body(host: &Host, slot: &StorageKey, len: usize) -> alloc::vec::Vec<u8> {
    let mut out = alloc::vec::Vec::with_capacity(len);
    let mut body_slot = dynamic_data_root(host, slot.as_bytes());
    let mut remaining = len;
    while remaining > 0 {
        let chunk = storage_get_32(host, &body_slot);
        let take = if remaining >= 32 { 32 } else { remaining };
        out.extend_from_slice(&chunk[..take]);
        remaining -= take;
        inc_slot(&mut body_slot);
    }
    out
}

/// Clear `count` body chunks starting at `keccak256(slot) + start_chunk`.
#[cfg(feature = "alloc")]
fn clear_dyn_body_range(host: &Host, slot: &StorageKey, start_chunk: usize, count: usize) {
    if count == 0 {
        return;
    }
    let mut body_slot = dynamic_data_root(host, slot.as_bytes());
    for _ in 0..start_chunk {
        inc_slot(&mut body_slot);
    }
    for _ in 0..count {
        storage_delete_32(host, &body_slot);
        inc_slot(&mut body_slot);
    }
}

#[cfg(feature = "alloc")]
fn dynamic_bytes_set(host: &Host, slot: &StorageKey, data: &[u8]) {
    let new_len = data.len();
    let new_chunks = if new_len < 32 {
        0
    } else {
        new_len.div_ceil(32)
    };

    // Inspect the existing layout so we can free body chunks that the new
    // value no longer needs (otherwise a long → short transition would leak
    // storage at `keccak256(slot) + i`).
    let old_slot_bytes = storage_get_32(host, slot.as_bytes());
    if let DynHeader::Spilled { len: old_len } = decode_dyn_header(&old_slot_bytes) {
        let old_chunks = old_len.div_ceil(32);
        if old_chunks > new_chunks {
            clear_dyn_body_range(host, slot, new_chunks, old_chunks - new_chunks);
        }
    }

    if new_len < 32 {
        // Short form: inline body + length × 2 in byte 31.
        let mut packed = [0u8; 32];
        packed[..new_len].copy_from_slice(data);
        packed[31] = (new_len as u8) << 1;
        if new_len == 0 {
            // Empty-inline sentinel: keeps the slot from being auto-cleared
            // by `set_storage_or_clear`. `slot[30]` is outside both the
            // (zero-length) body and the length byte, so the decoder ignores
            // it. Any non-zero value would do; `EMPTY_INLINE_SENTINEL` is the
            // chosen marker.
            packed[30] = EMPTY_INLINE_SENTINEL;
        }
        storage_set_32(host, slot.as_bytes(), &packed);
        return;
    }

    // Long form: header = len * 2 + 1; body chunks at keccak256(slot) + i.
    let header = encode_long_header(new_len);
    storage_set_32(host, slot.as_bytes(), &header);

    let mut body_slot = dynamic_data_root(host, slot.as_bytes());
    let mut offset = 0usize;
    while offset < new_len {
        let mut chunk = [0u8; 32];
        let remaining = new_len - offset;
        let take = if remaining >= 32 { 32 } else { remaining };
        chunk[..take].copy_from_slice(&data[offset..offset + take]);
        // Body chunks use `set_storage_or_clear`: an all-zero chunk gets
        // auto-deleted, but the read side calls `get_storage_or_zero` which
        // returns zeros for missing slots — so the round-trip is preserved.
        // This also matches Solidity SSTORE semantics.
        storage_set_32(host, &body_slot, &chunk);
        offset += take;
        inc_slot(&mut body_slot);
    }
}

/// Sentinel byte injected at `slot[30]` for an empty-inline dynamic value, so
/// the slot stays non-zero and survives `set_storage_or_clear`'s auto-delete.
/// Decoder ignores `slot[..31]` when `len == 0`, so the value is arbitrary;
/// `0x01` is the smallest non-zero marker.
#[cfg(feature = "alloc")]
const EMPTY_INLINE_SENTINEL: u8 = 0x01;

#[cfg(feature = "alloc")]
fn dynamic_bytes_get(host: &Host, slot: &StorageKey) -> alloc::vec::Vec<u8> {
    let slot_bytes = storage_get_32(host, slot.as_bytes());
    match decode_dyn_header(&slot_bytes) {
        DynHeader::Inline { len } => alloc::vec::Vec::from(&slot_bytes[..len]),
        DynHeader::Spilled { len } => read_dyn_body(host, slot, len),
    }
}

#[cfg(feature = "alloc")]
fn dynamic_bytes_try_get(host: &Host, slot: &StorageKey) -> Option<alloc::vec::Vec<u8>> {
    let slot_bytes = storage_try_get_32(host, slot.as_bytes())?;
    Some(match decode_dyn_header(&slot_bytes) {
        DynHeader::Inline { len } => alloc::vec::Vec::from(&slot_bytes[..len]),
        DynHeader::Spilled { len } => read_dyn_body(host, slot, len),
    })
}

#[cfg(feature = "alloc")]
fn dynamic_bytes_clear(host: &Host, slot: &StorageKey) {
    let slot_bytes = storage_get_32(host, slot.as_bytes());
    if let DynHeader::Spilled { len } = decode_dyn_header(&slot_bytes) {
        let chunks = len.div_ceil(32);
        clear_dyn_body_range(host, slot, 0, chunks);
    }
    storage_delete_32(host, slot.as_bytes());
}

// ---------------------------------------------------------------------------
// StorageComponent: how a typed storage object claims root slots.
// ---------------------------------------------------------------------------

/// A typed storage helper that occupies one or more contiguous root slots.
///
/// Implementations:
///
/// - [`Lazy<T>`]      — 1 slot (static 32-byte value).
/// - [`LazyBytes`] / [`LazyString`] — 1 slot (dynamic `bytes` / `string` with
///   solc-compatible inline/spilled layout).
/// - [`Mapping<K,V>`] — 1 slot (the root; entries live at derived keys).
/// - [`MappingBytes<K>`] / [`MappingString<K>`] — 1 slot, dynamic values.
/// - user storage structs annotated with `#[storage]` — sum of their fields'
///   `SLOTS`, assigned in declaration order.
///
/// The `#[contract]` macro reads `SLOTS` to assign slot numbers to fields. The
/// macro-generated constructor calls [`StorageComponent::new_at`] with the
/// assigned base slot and a clone of the contract's host handle.
pub trait StorageComponent: Sized {
    /// Number of root storage slots claimed by this component.
    const SLOTS: u64;

    /// Construct the component rooted at `slot`, bound to `host`.
    fn new_at(slot: u64, host: Host) -> Self;
}

// ---------------------------------------------------------------------------
// StorageLayoutType: for storageLayout JSON generation (abi-gen only)
// ---------------------------------------------------------------------------

/// Trait for resolving Solidity type names in `storageLayout` JSON.
///
/// Only used at build time (behind `cfg(feature = "abi-gen")`).
/// Implementations use `SolEncode::SOL_NAME` for leaf types and
/// construct mapping type strings for `Mapping`.
#[cfg(feature = "abi-gen")]
pub trait StorageLayoutType {
    /// Returns the Solidity storage type name (e.g., "uint256", "mapping(address,uint256)").
    fn sol_type_name() -> String
    where
        Self: Sized;
}

#[cfg(feature = "abi-gen")]
impl<T: SolEncode + StaticEncodedLen> StorageLayoutType for T {
    fn sol_type_name() -> String {
        String::from(T::SOL_NAME)
    }
}

#[cfg(feature = "abi-gen")]
impl<T: SolEncode + StaticEncodedLen> StorageLayoutType for Lazy<T> {
    fn sol_type_name() -> String {
        String::from(T::SOL_NAME)
    }
}

#[cfg(all(feature = "abi-gen", feature = "alloc"))]
impl StorageLayoutType for LazyBytes {
    fn sol_type_name() -> String {
        String::from("bytes")
    }
}

#[cfg(all(feature = "abi-gen", feature = "alloc"))]
impl StorageLayoutType for LazyString {
    fn sol_type_name() -> String {
        String::from("string")
    }
}

#[cfg(feature = "abi-gen")]
impl<K: SolEncode, V: StorageLayoutType> StorageLayoutType for Mapping<K, V> {
    fn sol_type_name() -> String {
        format!("mapping({},{})", K::SOL_NAME, V::sol_type_name())
    }
}

#[cfg(all(feature = "abi-gen", feature = "alloc"))]
impl<K: SolEncode> StorageLayoutType for MappingBytes<K> {
    fn sol_type_name() -> String {
        format!("mapping({},bytes)", K::SOL_NAME)
    }
}

#[cfg(all(feature = "abi-gen", feature = "alloc"))]
impl<K: SolEncode> StorageLayoutType for MappingString<K> {
    fn sol_type_name() -> String {
        format!("mapping({},string)", K::SOL_NAME)
    }
}

// ---------------------------------------------------------------------------
// Lazy<T> — static 32-byte value at a fixed storage slot.
// ---------------------------------------------------------------------------

/// A single typed value at a fixed storage slot.
///
/// "Lazy" because there is no caching: every [`get`](Lazy::get) reads from host
/// storage, every [`set`](Lazy::set) writes immediately.
///
/// `T` is restricted to static 32-byte types (`U256`, `Address`, `bool`,
/// `[u8; 32]`, …). For dynamic values, use [`LazyBytes`] or [`LazyString`].
///
/// The decode/encode is inlined at the method site (rather than dispatched
/// through a value-type trait) so LLVM sees one direct call chain per type
/// and shares the byte-swap helper with the dispatch encode/decode paths.
pub struct Lazy<T> {
    key: StorageKey,
    host: Host,
    _marker: PhantomData<T>,
}

impl<T: SolEncode + StaticDecode + StaticEncodedLen> Lazy<T> {
    /// Create a new `Lazy` at the given storage key, bound to a host handle.
    pub fn new(key: StorageKey, host: Host) -> Self {
        const {
            assert!(
                T::ENCODED_SIZE == 32,
                "Lazy<T> requires a 32-byte type (U256, Address, bool, [u8; 32], …)"
            )
        };
        Lazy {
            key,
            host,
            _marker: PhantomData,
        }
    }

    /// Read the value from storage.
    ///
    /// Returns the zero value for `T` if the slot was never written,
    /// matching Solidity's default-to-zero semantics.
    pub fn get(&self) -> T {
        let buf = storage_get_32(&self.host, self.key.as_bytes());
        unsafe { T::decode_unchecked(&buf, 0) }
    }

    /// Read the value, distinguishing "never written" from "has been set."
    ///
    /// Returns `None` if the slot was never written or was cleared.
    /// Returns `Some(value)` if the slot is present.
    ///
    /// Note: writing an all-zero static value deletes the key
    /// (Solidity semantics), so `try_get()` returns `None` after writing zero.
    pub fn try_get(&self) -> Option<T> {
        storage_try_get_32(&self.host, self.key.as_bytes())
            .map(|buf| unsafe { T::decode_unchecked(&buf, 0) })
    }

    /// Write a value to storage.
    ///
    /// Takes `&mut self` so that view methods (which receive `&Storage`)
    /// cannot call this through an immutable borrow.
    pub fn set(&mut self, value: &T) {
        let mut buf = [0u8; 32];
        SolEncode::encode_body_to(value, &mut buf);
        storage_set_32(&self.host, self.key.as_bytes(), &buf);
    }

    /// Clear the storage slot.
    ///
    /// Writes all-zero, which the host deletes from storage.
    pub fn clear(&mut self) {
        storage_set_32(&self.host, self.key.as_bytes(), &[0u8; 32]);
    }
}

impl<T: SolEncode + StaticDecode + StaticEncodedLen> StorageComponent for Lazy<T> {
    const SLOTS: u64 = 1;

    fn new_at(slot: u64, host: Host) -> Self {
        Lazy::new(StorageKey::from_slot(slot), host)
    }
}

// ---------------------------------------------------------------------------
// LazyBytes — dynamic Vec<u8> at a fixed storage slot (solc bytes layout).
// ---------------------------------------------------------------------------

/// A `Vec<u8>` storage slot using Solidity's `bytes` layout.
///
/// Short values (`len < 32`) live inline in the header slot; long values use
/// a header at the slot and body chunks at `keccak256(slot) + i`.
#[cfg(feature = "alloc")]
pub struct LazyBytes {
    key: StorageKey,
    host: Host,
}

#[cfg(feature = "alloc")]
impl LazyBytes {
    /// Create a new `LazyBytes` at the given storage key, bound to a host handle.
    pub fn new(key: StorageKey, host: Host) -> Self {
        LazyBytes { key, host }
    }

    /// Read the bytes from storage. Returns an empty vec if never written.
    pub fn get(&self) -> alloc::vec::Vec<u8> {
        dynamic_bytes_get(&self.host, &self.key)
    }

    /// Read the bytes, returning `None` if the slot was never written or cleared.
    ///
    /// A `set(&[])` (empty value) preserves the slot, so this returns
    /// `Some(empty)`. Only `clear()` produces `None`.
    pub fn try_get(&self) -> Option<alloc::vec::Vec<u8>> {
        dynamic_bytes_try_get(&self.host, &self.key)
    }

    /// Write bytes to storage.
    pub fn set(&mut self, value: &[u8]) {
        dynamic_bytes_set(&self.host, &self.key, value);
    }

    /// Clear the storage slot (header and any spilled body chunks).
    pub fn clear(&mut self) {
        dynamic_bytes_clear(&self.host, &self.key);
    }
}

#[cfg(feature = "alloc")]
impl StorageComponent for LazyBytes {
    const SLOTS: u64 = 1;

    fn new_at(slot: u64, host: Host) -> Self {
        LazyBytes::new(StorageKey::from_slot(slot), host)
    }
}

// ---------------------------------------------------------------------------
// LazyString — dynamic String at a fixed storage slot (same layout as LazyBytes).
// ---------------------------------------------------------------------------

/// A `String` storage slot using Solidity's `string` layout (identical to `bytes`).
///
/// Decoded values use `String::from_utf8_lossy` — invalid UTF-8 written by a
/// foreign contract is replaced with U+FFFD. Round-trips of Rust-written
/// strings are byte-exact.
#[cfg(feature = "alloc")]
pub struct LazyString {
    key: StorageKey,
    host: Host,
}

#[cfg(feature = "alloc")]
impl LazyString {
    /// Create a new `LazyString` at the given storage key, bound to a host handle.
    pub fn new(key: StorageKey, host: Host) -> Self {
        LazyString { key, host }
    }

    /// Read the string from storage. Returns an empty string if never written.
    pub fn get(&self) -> alloc::string::String {
        let bytes = dynamic_bytes_get(&self.host, &self.key);
        alloc::string::String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Read the string, returning `None` if the slot was never written or cleared.
    pub fn try_get(&self) -> Option<alloc::string::String> {
        let bytes = dynamic_bytes_try_get(&self.host, &self.key)?;
        Some(alloc::string::String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Write a string to storage.
    pub fn set(&mut self, value: &str) {
        dynamic_bytes_set(&self.host, &self.key, value.as_bytes());
    }

    /// Clear the storage slot (header and any spilled body chunks).
    pub fn clear(&mut self) {
        dynamic_bytes_clear(&self.host, &self.key);
    }
}

#[cfg(feature = "alloc")]
impl StorageComponent for LazyString {
    const SLOTS: u64 = 1;

    fn new_at(slot: u64, host: Host) -> Self {
        LazyString::new(StorageKey::from_slot(slot), host)
    }
}

// ---------------------------------------------------------------------------
// Mapping<K, V>
// ---------------------------------------------------------------------------

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
    pub fn new(root: StorageKey, host: Host) -> Self {
        Mapping {
            root,
            host,
            _marker: PhantomData,
        }
    }
}

impl<K, V> StorageComponent for Mapping<K, V> {
    const SLOTS: u64 = 1;

    fn new_at(slot: u64, host: Host) -> Self {
        Mapping::new(StorageKey::from_slot(slot), host)
    }
}

impl<K: AsStorageKey, V: SolEncode + StaticDecode + StaticEncodedLen> Mapping<K, V> {
    /// Compute the raw storage key for a given map key.
    ///
    /// Useful for debugging and cross-checking with `cast index`.
    pub fn slot_of(&self, key: &K) -> StorageKey {
        self.root.derive(&self.host, key)
    }

    /// Derive the slot once and return a [`Lazy`] handle for multiple operations.
    ///
    /// Requires `&mut self` because the returned `Lazy` supports writes.
    /// For read-only access, use [`get`](Mapping::get) or [`try_get`](Mapping::try_get).
    ///
    /// This saves a keccak host call when doing read-then-write on the same key.
    pub fn entry(&mut self, key: &K) -> Lazy<V> {
        Lazy::new(self.slot_of(key), self.host.clone())
    }

    /// Read the value at the given key.
    ///
    /// Returns the zero value if the key was never written.
    pub fn get(&self, key: &K) -> V {
        let buf = storage_get_32(&self.host, self.slot_of(key).as_bytes());
        unsafe { V::decode_unchecked(&buf, 0) }
    }

    /// Read the value, returning `None` if the key was never written.
    pub fn try_get(&self, key: &K) -> Option<V> {
        storage_try_get_32(&self.host, self.slot_of(key).as_bytes())
            .map(|buf| unsafe { V::decode_unchecked(&buf, 0) })
    }

    /// Write a value at the given key.
    pub fn insert(&mut self, key: &K, value: &V) {
        let mut buf = [0u8; 32];
        SolEncode::encode_body_to(value, &mut buf);
        storage_set_32(&self.host, self.slot_of(key).as_bytes(), &buf);
    }

    /// Delete the value at the given key.
    pub fn remove(&mut self, key: &K) {
        storage_set_32(&self.host, self.slot_of(key).as_bytes(), &[0u8; 32]);
    }
}

// ---------------------------------------------------------------------------
// Mapping<K1, Mapping<K2, V>> (nested)
// ---------------------------------------------------------------------------

/// Nested mappings can also be accessed with tuple keys:
/// `Mapping<(Address, Address), U256>` produces the same slots as
/// `Mapping<Address, Mapping<Address, U256>>`. Tuple key support is
/// implemented via `AsStorageKey` for tuples up to arity 5.
impl<K1: AsStorageKey, K2: AsStorageKey, V: SolEncode + StaticDecode + StaticEncodedLen>
    Mapping<K1, Mapping<K2, V>>
{
    /// Read path for nested mappings: derives the inner mapping root.
    ///
    /// The returned `Mapping` is an owned value with full read/write access.
    /// Mutability enforcement for view methods is handled upstream by the
    /// `#[contract]` macro, which injects `&Storage` (not `&mut Storage`)
    /// for view functions, preventing access to this `&mut self` `entry()`.
    pub fn get(&self, key: &K1) -> Mapping<K2, V> {
        Mapping::new(self.root.derive(&self.host, key), self.host.clone())
    }

    /// Write path for nested mappings: derives the inner mapping root.
    ///
    /// Takes `&mut self`, so this is only available in mutating methods.
    pub fn entry(&mut self, key: &K1) -> Mapping<K2, V> {
        Mapping::new(self.root.derive(&self.host, key), self.host.clone())
    }
}

// ---------------------------------------------------------------------------
// MappingBytes<K> — mapping with dynamic `bytes` values.
// ---------------------------------------------------------------------------

/// A `mapping(K => bytes)` backed by on-chain storage.
///
/// Each entry's header lives at `keccak256(pad32(key) ++ pad32(root_slot))`,
/// with the body at `keccak256(header_slot)` for spilled values — matching
/// `solc`'s nested layout.
#[cfg(feature = "alloc")]
pub struct MappingBytes<K> {
    root: StorageKey,
    host: Host,
    _marker: PhantomData<K>,
}

#[cfg(feature = "alloc")]
impl<K> MappingBytes<K> {
    /// Create a new `MappingBytes` rooted at the given storage key.
    pub fn new(root: StorageKey, host: Host) -> Self {
        MappingBytes {
            root,
            host,
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "alloc")]
impl<K> StorageComponent for MappingBytes<K> {
    const SLOTS: u64 = 1;

    fn new_at(slot: u64, host: Host) -> Self {
        MappingBytes::new(StorageKey::from_slot(slot), host)
    }
}

#[cfg(feature = "alloc")]
impl<K: AsStorageKey> MappingBytes<K> {
    /// Compute the raw storage key for a given map key.
    pub fn slot_of(&self, key: &K) -> StorageKey {
        self.root.derive(&self.host, key)
    }

    /// Derive the slot once and return a [`LazyBytes`] handle for read-then-write.
    pub fn entry(&mut self, key: &K) -> LazyBytes {
        LazyBytes::new(self.slot_of(key), self.host.clone())
    }

    /// Read the bytes at the given key. Returns empty if never written.
    pub fn get(&self, key: &K) -> alloc::vec::Vec<u8> {
        dynamic_bytes_get(&self.host, &self.slot_of(key))
    }

    /// Read the bytes, returning `None` if the key was never written or cleared.
    pub fn try_get(&self, key: &K) -> Option<alloc::vec::Vec<u8>> {
        dynamic_bytes_try_get(&self.host, &self.slot_of(key))
    }

    /// Write bytes at the given key.
    pub fn insert(&mut self, key: &K, value: &[u8]) {
        dynamic_bytes_set(&self.host, &self.slot_of(key), value);
    }

    /// Delete the bytes at the given key.
    pub fn remove(&mut self, key: &K) {
        dynamic_bytes_clear(&self.host, &self.slot_of(key));
    }
}

// ---------------------------------------------------------------------------
// MappingString<K> — mapping with dynamic `string` values.
// ---------------------------------------------------------------------------

/// A `mapping(K => string)` backed by on-chain storage. Same layout as
/// [`MappingBytes`]; decoded values use `String::from_utf8_lossy`.
#[cfg(feature = "alloc")]
pub struct MappingString<K> {
    root: StorageKey,
    host: Host,
    _marker: PhantomData<K>,
}

#[cfg(feature = "alloc")]
impl<K> MappingString<K> {
    /// Create a new `MappingString` rooted at the given storage key.
    pub fn new(root: StorageKey, host: Host) -> Self {
        MappingString {
            root,
            host,
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "alloc")]
impl<K> StorageComponent for MappingString<K> {
    const SLOTS: u64 = 1;

    fn new_at(slot: u64, host: Host) -> Self {
        MappingString::new(StorageKey::from_slot(slot), host)
    }
}

#[cfg(feature = "alloc")]
impl<K: AsStorageKey> MappingString<K> {
    /// Compute the raw storage key for a given map key.
    pub fn slot_of(&self, key: &K) -> StorageKey {
        self.root.derive(&self.host, key)
    }

    /// Derive the slot once and return a [`LazyString`] handle for read-then-write.
    pub fn entry(&mut self, key: &K) -> LazyString {
        LazyString::new(self.slot_of(key), self.host.clone())
    }

    /// Read the string at the given key. Returns empty if never written.
    pub fn get(&self, key: &K) -> alloc::string::String {
        let bytes = dynamic_bytes_get(&self.host, &self.slot_of(key));
        alloc::string::String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Read the string, returning `None` if the key was never written or cleared.
    pub fn try_get(&self, key: &K) -> Option<alloc::string::String> {
        let bytes = dynamic_bytes_try_get(&self.host, &self.slot_of(key))?;
        Some(alloc::string::String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Write a string at the given key.
    pub fn insert(&mut self, key: &K, value: &str) {
        dynamic_bytes_set(&self.host, &self.slot_of(key), value.as_bytes());
    }

    /// Delete the string at the given key.
    pub fn remove(&mut self, key: &K) {
        dynamic_bytes_clear(&self.host, &self.slot_of(key));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate alloc;
    extern crate std;

    use super::*;
    use alloc::rc::Rc;
    #[cfg(feature = "alloc")]
    use alloc::string::String;
    #[cfg(feature = "alloc")]
    use alloc::vec::Vec;
    use pvm_contract_types::Address;
    use pvm_contract_types::MockHostBuilder;
    use ruint::aliases::U256;

    /// Fresh isolated `Host` backed by a new `MockHost` in an `Rc`.
    /// Clone the returned handle to share storage state between cells.
    fn h() -> Host {
        Host::from_dyn(Rc::new(MockHostBuilder::new().build()))
    }

    // --- Lazy roundtrips ---

    #[test]
    fn lazy_roundtrip_u256() {
        let mut lazy = Lazy::<U256>::new(StorageKey::from_slot(0), h());
        lazy.set(&U256::from(42));
        assert_eq!(lazy.get(), U256::from(42));
    }

    #[test]
    fn lazy_roundtrip_address() {
        let addr = Address([0xAA; 20]);
        let mut lazy = Lazy::<Address>::new(StorageKey::from_slot(0), h());
        lazy.set(&addr);
        assert_eq!(lazy.get(), addr);
    }

    #[test]
    fn lazy_roundtrip_bool() {
        let mut lazy = Lazy::<bool>::new(StorageKey::from_slot(0), h());
        lazy.set(&true);
        assert!(lazy.get());
        lazy.set(&false);
        // Writing false = all-zero = deletes the key, so get returns zero = false
        assert!(!lazy.get());
    }

    #[test]
    fn lazy_default_is_zero() {
        let lazy = Lazy::<U256>::new(StorageKey::from_slot(0), h());
        assert_eq!(lazy.get(), U256::ZERO);
    }

    #[test]
    fn lazy_try_get_uninitialized() {
        let lazy = Lazy::<U256>::new(StorageKey::from_slot(0), h());
        assert_eq!(lazy.try_get(), None);
    }

    #[test]
    fn lazy_try_get_nonzero_value() {
        let mut lazy = Lazy::<U256>::new(StorageKey::from_slot(0), h());
        lazy.set(&U256::from(99));
        assert_eq!(lazy.try_get(), Some(U256::from(99)));
    }

    #[test]
    fn lazy_set_zero_deletes() {
        let mut lazy = Lazy::<U256>::new(StorageKey::from_slot(0), h());
        lazy.set(&U256::from(42));
        assert_eq!(lazy.try_get(), Some(U256::from(42)));
        lazy.set(&U256::ZERO);
        // Writing zero triggers set_storage_or_clear deletion
        assert_eq!(lazy.try_get(), None);
    }

    #[test]
    fn lazy_clear_then_try_get() {
        let mut lazy = Lazy::<U256>::new(StorageKey::from_slot(0), h());
        lazy.set(&U256::from(42));
        lazy.clear();
        assert_eq!(lazy.try_get(), None);
    }

    #[test]
    fn lazy_clear() {
        let mut lazy = Lazy::<U256>::new(StorageKey::from_slot(0), h());
        lazy.set(&U256::from(42));
        lazy.clear();
        assert_eq!(lazy.get(), U256::ZERO);
    }

    // --- Mapping operations ---

    #[test]
    fn mapping_insert_get() {
        let mut m = Mapping::<Address, U256>::new(StorageKey::from_slot(0), h());
        let addr = Address([0xBB; 20]);
        m.insert(&addr, &U256::from(100));
        assert_eq!(m.get(&addr), U256::from(100));
    }

    #[test]
    fn mapping_remove() {
        let mut m = Mapping::<Address, U256>::new(StorageKey::from_slot(0), h());
        let addr = Address([0xCC; 20]);
        m.insert(&addr, &U256::from(50));
        m.remove(&addr);
        assert_eq!(m.get(&addr), U256::ZERO);
    }

    #[test]
    fn mapping_remove_then_try_get() {
        let mut m = Mapping::<Address, U256>::new(StorageKey::from_slot(0), h());
        let addr = Address([0xDD; 20]);
        m.insert(&addr, &U256::from(50));
        assert_eq!(m.try_get(&addr), Some(U256::from(50)));
        m.remove(&addr);
        // Key is truly deleted, not just zeroed (#33)
        assert_eq!(m.try_get(&addr), None);
    }

    #[test]
    fn mapping_different_keys_independent() {
        let mut m = Mapping::<Address, U256>::new(StorageKey::from_slot(0), h());
        let a = Address([0x01; 20]);
        let b = Address([0x02; 20]);
        m.insert(&a, &U256::from(10));
        m.insert(&b, &U256::from(20));
        assert_eq!(m.get(&a), U256::from(10));
        assert_eq!(m.get(&b), U256::from(20));
    }

    // --- Nested mappings ---

    #[test]
    fn nested_mapping_allowances() {
        let mut allowances =
            Mapping::<Address, Mapping<Address, U256>>::new(StorageKey::from_slot(2), h());
        let owner = Address([0xAA; 20]);
        let spender = Address([0xBB; 20]);

        allowances.entry(&owner).insert(&spender, &U256::from(500));
        assert_eq!(allowances.get(&owner).get(&spender), U256::from(500));
    }

    // --- Tuple keys ---

    #[test]
    fn tuple_key_matches_chaining() {
        let host = h();
        let owner = Address([0xAA; 20]);
        let spender = Address([0xBB; 20]);
        let amount = U256::from(123);

        // Write via nested mapping chaining
        let mut chained =
            Mapping::<Address, Mapping<Address, U256>>::new(StorageKey::from_slot(2), host.clone());
        chained.entry(&owner).insert(&spender, &amount);

        // Read via tuple key (same slot, same host state)
        let tuple_map =
            Mapping::<(Address, Address), U256>::new(StorageKey::from_slot(2), host.clone());
        assert_eq!(tuple_map.get(&(owner, spender)), amount);
    }

    #[test]
    fn tuple_key_write_and_read() {
        let mut m = Mapping::<(Address, Address), U256>::new(StorageKey::from_slot(0), h());
        let alice = Address([0xAA; 20]);
        let bob = Address([0xBB; 20]);

        m.insert(&(alice, bob), &U256::from(500));
        assert_eq!(m.get(&(alice, bob)), U256::from(500));
        assert_eq!(m.get(&(bob, alice)), U256::ZERO); // different key order
    }

    #[test]
    fn triple_tuple_key_matches_chaining() {
        let host = h();
        let a = Address([0xAA; 20]);
        let b = Address([0xBB; 20]);
        let c = Address([0xCC; 20]);

        // Derive slot via triple nesting
        let root = StorageKey::from_slot(0);
        let chained = root.derive(&host, &a);
        let chained = chained.derive(&host, &b);
        let chained = chained.derive(&host, &c);

        // Derive slot via 3-tuple (must match chaining)
        let tupled = (a, b, c).derive_slot(&host, &root);
        assert_eq!(chained, tupled);
    }

    #[test]
    fn bytes32_as_mapping_key() {
        let mut m = Mapping::<[u8; 32], U256>::new(StorageKey::from_slot(0), h());
        let key = [0xAB; 32];
        m.insert(&key, &U256::from(42));
        assert_eq!(m.get(&key), U256::from(42));
    }

    // --- Dynamic accessors: LazyString / LazyBytes ---

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_roundtrip_string_short() {
        let mut lazy = LazyString::new(StorageKey::from_slot(0), h());
        lazy.set(&String::from("hello"));
        assert_eq!(lazy.get(), "hello");
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_roundtrip_string_long() {
        let mut lazy = LazyString::new(StorageKey::from_slot(0), h());
        let long = "a".repeat(200);
        lazy.set(&long);
        assert_eq!(lazy.get(), long);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_empty_is_default() {
        let lazy = LazyString::new(StorageKey::from_slot(0), h());
        assert_eq!(lazy.get(), "");
        assert_eq!(lazy.try_get(), None);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_clear() {
        let mut lazy = LazyString::new(StorageKey::from_slot(0), h());
        lazy.set(&String::from("payload"));
        assert_eq!(lazy.try_get().as_deref(), Some("payload"));
        lazy.clear();
        assert_eq!(lazy.try_get(), None);
        assert_eq!(lazy.get(), "");
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_overwrite_smaller() {
        let mut lazy = LazyString::new(StorageKey::from_slot(0), h());
        let host = lazy.host.clone();
        let key = lazy.key;
        let long =
            String::from("hello world this is a long string that spills over the inline boundary");
        let long_chunks = long.len().div_ceil(32);
        lazy.set(&long);
        lazy.set(&String::from("short"));
        assert_eq!(lazy.get(), "short");

        // Stale body chunks from the previous long value must have been
        // deleted, otherwise we'd be leaking storage on every long → short
        // transition.
        let mut body_slot = dynamic_data_root(&host, key.as_bytes());
        for _ in 0..long_chunks {
            assert_eq!(
                storage_try_get_32(&host, &body_slot),
                None,
                "stale body chunk not cleared"
            );
            inc_slot(&mut body_slot);
        }
    }

    // --- solc layout invariants ---

    /// "set("") and never written are distinguishable" — the central guarantee
    /// of using raw set_storage (not _or_clear) for the short header.
    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_set_empty_distinct_from_never_written() {
        let mut written = LazyString::new(StorageKey::from_slot(0), h());
        let never = LazyString::new(StorageKey::from_slot(1), written.host.clone());

        written.set("");

        assert_eq!(written.try_get(), Some(String::new()));
        assert_eq!(written.get(), "");
        assert_eq!(never.try_get(), None);
        assert_eq!(never.get(), "");
    }

    /// `set("")` must leave a non-zero header in the slot so that
    /// `set_storage_or_clear` doesn't auto-delete it; the decoder still
    /// reports inline-len-0. The sentinel lives at `slot[30]` (outside the
    /// zero-length body and outside the length byte at `slot[31]`).
    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_set_empty_writes_non_zero_sentinel_header() {
        let mut lazy = LazyString::new(StorageKey::from_slot(0), h());
        let host = lazy.host.clone();
        let key = lazy.key;

        lazy.set("");

        let slot_bytes = storage_get_32(&host, key.as_bytes());
        assert_ne!(
            slot_bytes, [0u8; 32],
            "slot must be non-zero so it persists"
        );
        assert_eq!(slot_bytes[31], 0, "length byte: inline + len 0");
        assert_eq!(slot_bytes[30], EMPTY_INLINE_SENTINEL, "sentinel at byte 30");
        assert!(
            slot_bytes[..30].iter().all(|&b| b == 0),
            "bytes 0..30 must be zero"
        );
    }

    /// Overwriting a sentinel-only empty header with a non-empty value must
    /// clear the sentinel byte (otherwise stale `0x01` at `slot[30]` would
    /// land inside a future 31-byte inline value's body).
    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_overwrite_empty_clears_sentinel() {
        let mut lazy = LazyString::new(StorageKey::from_slot(0), h());
        let host = lazy.host.clone();
        let key = lazy.key;

        lazy.set("");
        lazy.set(&"a".repeat(31));

        let slot_bytes = storage_get_32(&host, key.as_bytes());
        assert_eq!(
            slot_bytes[30], b'a',
            "byte 30 is the last body byte for len=31"
        );
        assert_eq!(slot_bytes[31], 31 * 2, "length × 2");
        assert_eq!(lazy.get(), "a".repeat(31));
    }

    /// Probe the slot bytes directly: short value lives inline with
    /// `byte31 = length * 2` (low bit = 0). This is the solc convention that
    /// `cast storage` decodes natively.
    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_short_inline_layout() {
        let mut lazy = LazyString::new(StorageKey::from_slot(0), h());
        let host = lazy.host.clone();
        let key = lazy.key;
        lazy.set(&String::from("hello"));

        let slot_bytes = storage_get_32(&host, key.as_bytes());
        assert_eq!(&slot_bytes[..5], b"hello");
        assert!(slot_bytes[5..31].iter().all(|&b| b == 0));
        assert_eq!(slot_bytes[31], 5 * 2, "byte31 = length * 2, low bit 0");
    }

    /// 31-byte string is still inline; 32-byte string spills.
    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_boundary_31_bytes_inline() {
        let mut lazy = LazyString::new(StorageKey::from_slot(0), h());
        let host = lazy.host.clone();
        let key = lazy.key;
        let s = "a".repeat(31);
        lazy.set(&s);

        let slot_bytes = storage_get_32(&host, key.as_bytes());
        assert!(slot_bytes[31] & 1 == 0, "low bit 0 -> inline");
        assert_eq!(slot_bytes[31] >> 1, 31);
        assert_eq!(&slot_bytes[..31], s.as_bytes());
        assert_eq!(lazy.get(), s);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_boundary_32_bytes_spilled() {
        let mut lazy = LazyString::new(StorageKey::from_slot(0), h());
        let host = lazy.host.clone();
        let key = lazy.key;
        let s = "b".repeat(32);
        lazy.set(&s);

        let slot_bytes = storage_get_32(&host, key.as_bytes());
        assert!(slot_bytes[31] & 1 == 1, "low bit 1 -> spilled");
        // Header = 32 * 2 + 1 = 65, fits in byte 31.
        assert_eq!(slot_bytes[31], 65);
        assert!(slot_bytes[..31].iter().all(|&b| b == 0));
        assert_eq!(lazy.get(), s);
    }

    /// Long-spill probe: header is `len * 2 + 1` big-endian, body chunks live
    /// at consecutive slots starting from `keccak256(slot)`.
    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_long_spill_layout() {
        let mut lazy = LazyString::new(StorageKey::from_slot(0), h());
        let host = lazy.host.clone();
        let key = lazy.key;
        // 40 bytes spans two 32-byte chunks (8 bytes into the second).
        let s: String = (0..40).map(|i| (b'a' + (i % 26) as u8) as char).collect();
        lazy.set(&s);

        let slot_bytes = storage_get_32(&host, key.as_bytes());
        assert!(slot_bytes[31] & 1 == 1);
        // 40 * 2 + 1 = 81.
        assert_eq!(slot_bytes[31], 81);

        let mut body_slot = dynamic_data_root(&host, key.as_bytes());
        let chunk0 = storage_get_32(&host, &body_slot);
        assert_eq!(&chunk0[..32], &s.as_bytes()[..32]);

        inc_slot(&mut body_slot);
        let chunk1 = storage_get_32(&host, &body_slot);
        assert_eq!(&chunk1[..8], &s.as_bytes()[32..40]);
        assert!(chunk1[8..].iter().all(|&b| b == 0), "trailing chunk pad");

        assert_eq!(lazy.get(), s);
    }

    /// Short → long transition: previously inline data is replaced with
    /// spill-form header and body chunks.
    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_grow_short_to_long() {
        let mut lazy = LazyString::new(StorageKey::from_slot(0), h());
        lazy.set(&String::from("short"));
        assert_eq!(lazy.get(), "short");

        let long = "x".repeat(100);
        lazy.set(&long);
        assert_eq!(lazy.get(), long);
    }

    /// Long → short transition deletes the now-orphaned body chunks. Probes
    /// each previously-occupied keccak slot and asserts it no longer exists.
    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_shrink_long_to_short_clears_chunks() {
        let mut lazy = LazyString::new(StorageKey::from_slot(0), h());
        let host = lazy.host.clone();
        let key = lazy.key;
        let long = "y".repeat(100); // 4 chunks of 32B
        lazy.set(&long);
        lazy.set(&String::from("ok"));
        assert_eq!(lazy.get(), "ok");

        let mut body_slot = dynamic_data_root(&host, key.as_bytes());
        for chunk_idx in 0..4 {
            assert_eq!(
                storage_try_get_32(&host, &body_slot),
                None,
                "body chunk {chunk_idx} not cleared after shrink"
            );
            inc_slot(&mut body_slot);
        }
    }

    /// clear() on a long value must delete header AND every body chunk.
    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_clear_after_long_deletes_chunks() {
        let mut lazy = LazyString::new(StorageKey::from_slot(0), h());
        let host = lazy.host.clone();
        let key = lazy.key;
        let long = "z".repeat(70); // 3 chunks
        lazy.set(&long);
        lazy.clear();

        // Header slot gone.
        assert_eq!(storage_try_get_32(&host, key.as_bytes()), None);
        // All body chunks gone.
        let mut body_slot = dynamic_data_root(&host, key.as_bytes());
        for chunk_idx in 0..3 {
            assert_eq!(
                storage_try_get_32(&host, &body_slot),
                None,
                "body chunk {chunk_idx} survived clear()"
            );
            inc_slot(&mut body_slot);
        }
        assert_eq!(lazy.try_get(), None);
        assert_eq!(lazy.get(), "");
    }

    /// Mapping<Address, String> with a spill-form value round-trips through
    /// the same layout path.
    #[cfg(feature = "alloc")]
    #[test]
    fn mapping_with_long_string_value() {
        let mut m = MappingString::<Address>::new(StorageKey::from_slot(0), h());
        let addr = Address([0x11; 20]);
        let value = "w".repeat(100);
        m.insert(&addr, &value);
        assert_eq!(m.get(&addr), value);
        m.remove(&addr);
        assert_eq!(m.try_get(&addr), None);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_roundtrip_vec_u8() {
        let mut lazy = LazyBytes::new(StorageKey::from_slot(0), h());
        lazy.set(&alloc::vec![1, 2, 3, 4, 5]);
        assert_eq!(lazy.get(), alloc::vec![1, 2, 3, 4, 5]);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_vec_u8_large() {
        let mut lazy = LazyBytes::new(StorageKey::from_slot(0), h());
        let data: Vec<u8> = (0..=255u8).collect();
        lazy.set(&data);
        assert_eq!(lazy.get(), data);
    }

    /// `Vec<u8>` rides the same solc-compatible path as `String`. Cover the
    /// inline / spill boundary explicitly: 31 bytes inline, 32 bytes spills.
    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_vec_u8_boundary() {
        let mut a = LazyBytes::new(StorageKey::from_slot(0), h());
        let host = a.host.clone();
        let key_a = a.key;

        let inline: Vec<u8> = (0..31).collect();
        a.set(&inline);
        let slot_bytes = storage_get_32(&host, key_a.as_bytes());
        assert_eq!(slot_bytes[31], 31 * 2, "31B vec inline, byte31 = 62");
        assert_eq!(a.get(), inline);

        let mut b = LazyBytes::new(StorageKey::from_slot(1), host);
        let spill: Vec<u8> = (0..32).collect();
        b.set(&spill);
        let slot_b = storage_get_32(&b.host, b.key.as_bytes());
        assert_eq!(slot_b[31], 32 * 2 + 1, "32B vec spills, byte31 = 65");
        assert_eq!(b.get(), spill);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn mapping_address_to_string() {
        let mut m = MappingString::<Address>::new(StorageKey::from_slot(0), h());
        let a = Address([0x01; 20]);
        let b = Address([0x02; 20]);
        m.insert(&a, "alice");
        m.insert(&b, "bob");
        assert_eq!(m.get(&a), "alice");
        assert_eq!(m.get(&b), "bob");
        m.remove(&a);
        assert_eq!(m.try_get(&a), None);
        assert_eq!(m.get(&b), "bob");
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn dynamic_data_root_independent_per_slot() {
        // Distinct header slots must hash to distinct data roots so two
        // dynamic values on adjacent slots can't trample each other.
        let mut a = LazyString::new(StorageKey::from_slot(0), h());
        let host = a.host.clone();
        let mut b = LazyString::new(StorageKey::from_slot(1), host);
        a.set("first");
        b.set("second");
        assert_eq!(a.get(), "first");
        assert_eq!(b.get(), "second");
    }

    // --- Solidity compatibility ---

    #[test]
    fn storage_key_from_slot() {
        assert_eq!(StorageKey::from_slot(0).as_bytes(), &[0u8; 32]);
        let mut expected = [0u8; 32];
        expected[31] = 1;
        assert_eq!(StorageKey::from_slot(1).as_bytes(), &expected);
    }

    #[test]
    fn derive_key_matches_solidity() {
        let host = h();
        // cast index address 0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA 1
        // Expected: keccak256(pad32(0xAA..AA) ++ pad32(1))
        let addr = Address([0xAA; 20]);
        let root = StorageKey::from_slot(1);
        let derived = root.derive(&host, &addr);

        // Compute expected: keccak256(0x000..0xAAAA..AA ++ 0x000..001)
        let mut preimage = [0u8; 64];
        // Address is right-aligned: 12 zero bytes + 20 address bytes
        preimage[12..32].copy_from_slice(&[0xAA; 20]);
        // Slot 1: 31 zero bytes + 0x01
        preimage[63] = 1;
        let mut expected = [0u8; 32];
        host.hash_keccak_256(&preimage, &mut expected);

        assert_eq!(derived.as_bytes(), &expected);
    }

    // --- StorageComponent ---

    #[test]
    fn storage_component_slot_count() {
        assert_eq!(<Lazy<U256> as StorageComponent>::SLOTS, 1);
        assert_eq!(<Mapping<Address, U256> as StorageComponent>::SLOTS, 1);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn storage_component_slot_count_dynamic() {
        assert_eq!(<LazyString as StorageComponent>::SLOTS, 1);
        assert_eq!(<LazyBytes as StorageComponent>::SLOTS, 1);
        assert_eq!(<MappingString<Address> as StorageComponent>::SLOTS, 1);
        assert_eq!(<MappingBytes<Address> as StorageComponent>::SLOTS, 1);
    }

    #[test]
    fn storage_component_new_at_matches_new() {
        let host = h();
        let mut a = Lazy::<U256>::new(StorageKey::from_slot(7), host.clone());
        let mut b = <Lazy<U256> as StorageComponent>::new_at(7, host);
        a.set(&U256::from(99));
        // `b` shares the host, so should see the same write.
        assert_eq!(b.get(), U256::from(99));
        b.set(&U256::from(100));
        assert_eq!(a.get(), U256::from(100));
    }

    // --- Entry optimization ---

    #[test]
    fn entry_reuse_for_read_write() {
        let mut m = Mapping::<Address, U256>::new(StorageKey::from_slot(0), h());
        let addr = Address([0xEE; 20]);
        m.insert(&addr, &U256::from(100));

        // Use entry for read-then-write
        let mut cell = m.entry(&addr);
        let val = cell.get();
        assert_eq!(val, U256::from(100));
        cell.set(&(val - U256::from(30)));

        assert_eq!(m.get(&addr), U256::from(70));
    }

    // --- Multi-field storage ---

    #[test]
    fn multi_field_storage() {
        let host = h();
        let mut counter = Lazy::<U256>::new(StorageKey::from_slot(0), host.clone());
        let mut balances = Mapping::<Address, U256>::new(StorageKey::from_slot(1), host);

        counter.set(&U256::from(42));
        assert_eq!(counter.get(), U256::from(42));

        let addr = Address([0xFF; 20]);
        balances.insert(&addr, &U256::from(1000));
        assert_eq!(balances.get(&addr), U256::from(1000));
    }

    /// Full ERC-20-like example showing how storage fields are constructed
    /// and used. This mirrors the `#[contract]` macro's generated code.
    #[test]
    fn erc20_storage_example() {
        let host = h();
        let mut total_supply = Lazy::<U256>::new(StorageKey::from_slot(0), host.clone());
        let mut balances = Mapping::<Address, U256>::new(StorageKey::from_slot(1), host.clone());
        let mut allowances =
            Mapping::<Address, Mapping<Address, U256>>::new(StorageKey::from_slot(2), host);

        let alice = Address([0xAA; 20]);
        let bob = Address([0xBB; 20]);
        let initial_supply = U256::from(10_000);

        // Constructor: set total supply and mint to alice
        total_supply.set(&initial_supply);
        balances.insert(&alice, &initial_supply);

        assert_eq!(total_supply.get(), initial_supply);
        assert_eq!(balances.get(&alice), initial_supply);
        assert_eq!(balances.get(&bob), U256::ZERO);

        // Transfer: alice sends 300 to bob using entry() for read-then-write
        let amount = U256::from(300);
        let mut alice_cell = balances.entry(&alice);
        let alice_bal = alice_cell.get();
        alice_cell.set(&(alice_bal - amount));

        let mut bob_cell = balances.entry(&bob);
        let bob_bal = bob_cell.get();
        bob_cell.set(&(bob_bal + amount));

        assert_eq!(balances.get(&alice), U256::from(9_700));
        assert_eq!(balances.get(&bob), U256::from(300));

        // Approve: alice approves bob for 500
        allowances.entry(&alice).insert(&bob, &U256::from(500));

        // Read allowance via chaining
        assert_eq!(allowances.get(&alice).get(&bob), U256::from(500));
        // Other direction is zero
        assert_eq!(allowances.get(&bob).get(&alice), U256::ZERO);
    }

    #[test]
    fn different_slots_dont_interfere() {
        let host = h();
        let mut value_a = Lazy::<U256>::new(StorageKey::from_slot(5), host.clone());
        let mut value_b = Lazy::<U256>::new(StorageKey::from_slot(10), host);

        value_a.set(&U256::from(111));
        value_b.set(&U256::from(222));
        assert_eq!(value_a.get(), U256::from(111));
        assert_eq!(value_b.get(), U256::from(222));
    }

    // --- Solidity slot cross-checks (hardcoded values from `cast index`) ---

    #[test]
    fn mapping_solidity_slot_compat() {
        // `cast index address 0xBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB 1`
        // → 0x8f22848572deaf321ecb41095a0a57d3f19eda24b92a3f4a8e554a2e56f45bc4
        let m = Mapping::<Address, U256>::new(StorageKey::from_slot(1), h());
        let addr = Address([0xBB; 20]);
        let slot = m.slot_of(&addr);

        let expected = [
            0x8f, 0x22, 0x84, 0x85, 0x72, 0xde, 0xaf, 0x32, 0x1e, 0xcb, 0x41, 0x09, 0x5a, 0x0a,
            0x57, 0xd3, 0xf1, 0x9e, 0xda, 0x24, 0xb9, 0x2a, 0x3f, 0x4a, 0x8e, 0x55, 0x4a, 0x2e,
            0x56, 0xf4, 0x5b, 0xc4,
        ];
        assert_eq!(slot.as_bytes(), &expected, "must match `cast index` output");
    }

    #[test]
    fn nested_mapping_slot_matches_solidity() {
        // allowances[0xAA..AA][0xBB..BB] at root slot 2:
        // inner = cast index address 0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA 2
        //       → 0xe1e81504ed8609a5b03379f97b221e3dede4a62d6d61a87a4ab7ed7b1b9c0553
        // outer = cast index address 0xBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB <inner>
        //       → 0x35815c850ac7d4d0af322824699787b146e33c6cac5d0a52ab3225d6985a27a7
        let allowances =
            Mapping::<Address, Mapping<Address, U256>>::new(StorageKey::from_slot(2), h());
        let owner = Address([0xAA; 20]);
        let spender = Address([0xBB; 20]);

        // Derive via chaining: get(&owner) returns inner Mapping, then slot_of(&spender)
        let inner = allowances.get(&owner);
        let slot = inner.slot_of(&spender);

        let expected = [
            0x35, 0x81, 0x5c, 0x85, 0x0a, 0xc7, 0xd4, 0xd0, 0xaf, 0x32, 0x28, 0x24, 0x69, 0x97,
            0x87, 0xb1, 0x46, 0xe3, 0x3c, 0x6c, 0xac, 0x5d, 0x0a, 0x52, 0xab, 0x32, 0x25, 0xd6,
            0x98, 0x5a, 0x27, 0xa7,
        ];
        assert_eq!(
            slot.as_bytes(),
            &expected,
            "must match chained `cast index` output"
        );
    }

    // --- Dynamic keys (String / Vec<u8>) ---
    // Run with: cargo test -p pvm-storage --features alloc

    #[cfg(feature = "alloc")]
    use alloc::string::ToString;
    #[cfg(feature = "alloc")]
    use alloc::vec;

    #[cfg(feature = "alloc")]
    #[test]
    fn mapping_string_key_roundtrip() {
        let mut m = Mapping::<String, U256>::new(StorageKey::from_slot(0), h());
        m.insert(&"alice".to_string(), &U256::from(100));
        assert_eq!(m.get(&"alice".to_string()), U256::from(100));
        assert_eq!(m.get(&"bob".to_string()), U256::ZERO);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn mapping_bytes_key_roundtrip() {
        let mut m = Mapping::<Vec<u8>, U256>::new(StorageKey::from_slot(0), h());
        m.insert(&vec![1u8, 2, 3], &U256::from(42));
        assert_eq!(m.get(&vec![1u8, 2, 3]), U256::from(42));
        assert_eq!(m.get(&vec![1u8, 2, 4]), U256::ZERO);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn mapping_bytes_key_long_roundtrip() {
        // 100-byte key spans multiple keccak preimage bytes; confirms the
        // unpadded formula handles arbitrary-length keys.
        let mut m = Mapping::<Vec<u8>, U256>::new(StorageKey::from_slot(1), h());
        let key = vec![b'x'; 100];
        m.insert(&key, &U256::from(7));
        assert_eq!(m.get(&key), U256::from(7));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn mapping_string_key_solidity_parity() {
        // cast index string "foo" 1
        // → 0xb770ea6769bbbd870e326681074f882a4d98de2943bbf7a23e8f4b258b1b8ac9
        let m = Mapping::<String, U256>::new(StorageKey::from_slot(1), h());
        let slot = m.slot_of(&"foo".to_string());
        let expected = [
            0xb7, 0x70, 0xea, 0x67, 0x69, 0xbb, 0xbd, 0x87, 0x0e, 0x32, 0x66, 0x81, 0x07, 0x4f,
            0x88, 0x2a, 0x4d, 0x98, 0xde, 0x29, 0x43, 0xbb, 0xf7, 0xa2, 0x3e, 0x8f, 0x4b, 0x25,
            0x8b, 0x1b, 0x8a, 0xc9,
        ];
        assert_eq!(
            slot.as_bytes(),
            &expected,
            "must match `cast index string \"foo\" 1`"
        );
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn mapping_bytes_key_solidity_parity() {
        // cast index bytes "0x010203" 1
        // → 0x4c6b2a1cad5eaf1e4e6556e0d021d6a22514b15458a60294869177950c245b57
        let m = Mapping::<Vec<u8>, U256>::new(StorageKey::from_slot(1), h());
        let slot = m.slot_of(&vec![1u8, 2, 3]);
        let expected = [
            0x4c, 0x6b, 0x2a, 0x1c, 0xad, 0x5e, 0xaf, 0x1e, 0x4e, 0x65, 0x56, 0xe0, 0xd0, 0x21,
            0xd6, 0xa2, 0x25, 0x14, 0xb1, 0x54, 0x58, 0xa6, 0x02, 0x94, 0x86, 0x91, 0x77, 0x95,
            0x0c, 0x24, 0x5b, 0x57,
        ];
        assert_eq!(
            slot.as_bytes(),
            &expected,
            "must match `cast index bytes \"0x010203\" 1`"
        );
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn mapping_string_key_empty() {
        // Empty key: preimage is just the 32-byte root slot.
        // keccak256(b"" ++ pad32(1)) = b10e2d527612073b26eecdfd717e6a320cf44b4afac2b0732d9fcbe2b7fa0cf6
        let mut m = Mapping::<String, U256>::new(StorageKey::from_slot(1), h());
        m.insert(&String::new(), &U256::from(9));
        assert_eq!(m.get(&String::new()), U256::from(9));

        let slot = m.slot_of(&String::new());
        let expected = [
            0xb1, 0x0e, 0x2d, 0x52, 0x76, 0x12, 0x07, 0x3b, 0x26, 0xee, 0xcd, 0xfd, 0x71, 0x7e,
            0x6a, 0x32, 0x0c, 0xf4, 0x4b, 0x4a, 0xfa, 0xc2, 0xb0, 0x73, 0x2d, 0x9f, 0xcb, 0xe2,
            0xb7, 0xfa, 0x0c, 0xf6,
        ];
        assert_eq!(slot.as_bytes(), &expected);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn mapping_string_key_no_padding_collision_safety() {
        // The 1-byte string "a" (raw bytes: [0x61]) and the 32-byte static key
        // [0x61, 0x00*31] both have 0x61 as their first preimage byte. With the
        // padded formula they would collide; with the unpadded formula they
        // must NOT collide.
        let host = h();
        let dyn_map = Mapping::<String, U256>::new(StorageKey::from_slot(0), host.clone());
        let static_map = Mapping::<[u8; 32], U256>::new(StorageKey::from_slot(0), host.clone());

        let dyn_slot = dyn_map.slot_of(&"a".to_string());

        let mut padded_a = [0u8; 32];
        padded_a[0] = 0x61;
        let static_slot = static_map.slot_of(&padded_a);

        assert_ne!(
            dyn_slot.as_bytes(),
            static_slot.as_bytes(),
            "dynamic and static keys with shared prefix must derive distinct slots"
        );
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn mapping_string_key_distinct_lengths() {
        // "a" and "aa" share a prefix; verify distinct slots.
        let m = Mapping::<String, U256>::new(StorageKey::from_slot(0), h());
        assert_ne!(
            m.slot_of(&"a".to_string()).as_bytes(),
            m.slot_of(&"aa".to_string()).as_bytes(),
        );
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn mapping_string_key_matches_str_impl() {
        // The String impl must delegate to the str impl so that derived slots
        // are byte-identical. This guarantee is what would let a future
        // `get_by_str` zero-alloc accessor share storage with the String API.
        let host = h();
        let root = StorageKey::from_slot(3);
        let m = Mapping::<String, U256>::new(root, host.clone());
        let owned_slot = m.slot_of(&"alice".to_string());
        let borrowed_slot = <str as AsStorageKey>::derive_slot("alice", &host, &root);
        assert_eq!(owned_slot.as_bytes(), borrowed_slot.as_bytes());
    }
}

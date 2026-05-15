//! Typed storage helpers for PVM smart contracts with Solidity-compatible slot layout.
//!
//! Provides [`Lazy<T>`] for single-value storage and [`Mapping<K, V>`] for key-value
//! storage, both using Solidity-compatible key derivation so tools like `cast storage`
//! and `cast index` work out of the box.
//!
//! The crate is built on two layered abstractions:
//!
//! - [`SlotValue`] — *how a value lives in storage*. Static-encoded leaves (32-byte
//!   types) put their value directly in the slot; dynamic-encoded leaves
//!   (`String`, `Vec<u8>`) put a length in the slot and the payload at a derived
//!   key, matching the layout `solc` emits for the same Solidity type.
//! - [`StorageComponent`] — *how a typed storage object claims root slots*.
//!   `Lazy<T>` and `Mapping<K, V>` each claim one slot; user-defined storage
//!   structs (marked with `#[storage]`) claim the sum of their fields. The
//!   `#[contract]` macro uses [`StorageComponent::SLOTS`] to auto-number the
//!   contract struct's fields, so explicit `#[slot(N)]` is optional.
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
use pvm_contract_types::{
    Host, HostApi, SolEncode, StaticDecode, StaticEncodedLen, StorageFlags,
};

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

fn storage_try_get_32(host: &Host, key: &[u8; 32]) -> Option<[u8; 32]> {
    let mut buf = [0u8; 32];
    let mut out = &mut buf[..];
    match host.get_storage(StorageFlags::empty(), key, &mut out) {
        Ok(()) => Some(buf),
        Err(_) => None,
    }
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

// ---------------------------------------------------------------------------
// SlotValue: how a Rust type lives in (or under) a single storage slot.
// ---------------------------------------------------------------------------

/// Encode/decode a Rust value into a Solidity-compatible storage cell rooted
/// at a single slot.
///
/// Two layout families share this trait:
///
/// - **Static** (`U256`, `Address`, `bool`, `[u8; 32]`, etc.): the entire
///   value lives in the slot itself.
/// - **Dynamic** (`String`, `Vec<u8>`): the slot stores layout metadata
///   (length) and the payload lives at slots derived from
///   `keccak256(slot)`, matching `solc`'s layout for the corresponding
///   Solidity type.
///
/// Implementors describe both directions. The `Lazy<T>` and `Mapping<K, V>`
/// wrappers route their `get` / `set` / `clear` calls through these methods.
///
/// The trait is sealed at the SDK boundary: external crates implement it via
/// `pvm-contract-types`' encoding impls (`SolEncode + SolDecode + StaticEncodedLen`
/// for static, plus the alloc-gated blanket impls for `String` / `Vec<u8>` in
/// this crate). Future additions (e.g. user-defined wrapper types) will gain
/// blanket impls here rather than ad-hoc unsafe code in consumer crates.
pub trait SlotValue: Sized {
    /// Read the value from a storage slot. Returns the zero/default value if
    /// the slot was never written (matching Solidity semantics).
    fn slot_get(host: &Host, slot: &StorageKey) -> Self;

    /// Read the value, distinguishing "never written" from "has been set."
    /// Returns `None` if the slot was never written or has been cleared back
    /// to the zero value.
    fn slot_try_get(host: &Host, slot: &StorageKey) -> Option<Self>;

    /// Write the value to a storage slot.
    fn slot_set(&self, host: &Host, slot: &StorageKey);

    /// Clear the storage slot (delete the entry).
    fn slot_clear(host: &Host, slot: &StorageKey);
}

/// Internal helper: read a static-encoded value from a single 32-byte slot.
///
/// Used by the per-type `SlotValue` impls produced by [`impl_static_slot_value!`].
/// Generic over any `StaticDecode` type whose canonical encoding is 32 bytes.
/// The slot buffer is always 32 bytes and the caller guarantees the type has a
/// 32-byte encoding via the `const_assert!` inside `impl_static_slot_value!`,
/// so `decode_unchecked` is safe.
fn static_slot_get<T: StaticDecode>(host: &Host, slot: &StorageKey) -> T {
    let buf = storage_get_32(host, slot.as_bytes());
    unsafe { T::decode_unchecked(&buf, 0) }
}

fn static_slot_try_get<T: StaticDecode>(host: &Host, slot: &StorageKey) -> Option<T> {
    storage_try_get_32(host, slot.as_bytes()).map(|buf| unsafe { T::decode_unchecked(&buf, 0) })
}

fn static_slot_set<T: SolEncode>(value: &T, host: &Host, slot: &StorageKey) {
    let mut buf = [0u8; 32];
    SolEncode::encode_body_to(value, &mut buf);
    storage_set_32(host, slot.as_bytes(), &buf);
}

fn static_slot_clear(host: &Host, slot: &StorageKey) {
    storage_set_32(host, slot.as_bytes(), &[0u8; 32]);
}

/// Implement [`SlotValue`] for each 32-byte static type.
///
/// Per-type impl rather than a blanket `impl<T: StaticEncodedLen>` because the
/// blanket form forbids us from later adding a *different* `SlotValue` impl
/// for `String` / `Vec<u8>` (which DO implement `StaticEncodedLen` via the
/// `alloc` feature, but encode as Solidity `bytes`/`string` — a fundamentally
/// different storage layout). The const-assert guards against accidentally
/// listing a non-32-byte type here.
macro_rules! impl_static_slot_value {
    ($($ty:ty),* $(,)?) => {$(
        impl SlotValue for $ty {
            fn slot_get(host: &Host, slot: &StorageKey) -> Self {
                const {
                    assert!(
                        <$ty as StaticEncodedLen>::ENCODED_SIZE == 32,
                        "static SlotValue requires a 32-byte ABI encoding"
                    );
                }
                static_slot_get::<$ty>(host, slot)
            }

            fn slot_try_get(host: &Host, slot: &StorageKey) -> Option<Self> {
                static_slot_try_get::<$ty>(host, slot)
            }

            fn slot_set(&self, host: &Host, slot: &StorageKey) {
                static_slot_set::<$ty>(self, host, slot)
            }

            fn slot_clear(host: &Host, slot: &StorageKey) {
                static_slot_clear(host, slot)
            }
        }
    )*};
}

// Mirrors the list in `impl_scalar_storage_key!` above — all 32-byte scalar
// types from `pvm-contract-types`.
impl_static_slot_value!(
    U256, u128, u64, u32, u16, u8, // Signed integers
    I256, i128, i64, i32, i16, i8, // Other value types
    bool, Address,
);

// Fixed-size byte arrays [u8; N] encode as Solidity `bytesN` (left-aligned, 32 bytes).
impl<const N: usize> SlotValue for [u8; N] {
    fn slot_get(host: &Host, slot: &StorageKey) -> Self {
        const {
            assert!(
                N <= 32,
                "static SlotValue requires a 32-byte ABI encoding ([u8; N] with N <= 32)"
            );
        }
        static_slot_get::<[u8; N]>(host, slot)
    }

    fn slot_try_get(host: &Host, slot: &StorageKey) -> Option<Self> {
        static_slot_try_get::<[u8; N]>(host, slot)
    }

    fn slot_set(&self, host: &Host, slot: &StorageKey) {
        static_slot_set::<[u8; N]>(self, host, slot)
    }

    fn slot_clear(host: &Host, slot: &StorageKey) {
        static_slot_clear(host, slot)
    }
}

// ---------------------------------------------------------------------------
// Dynamic SlotValue impls (alloc-gated): bytes/string Solidity layout.
//
// Layout matches solc for `bytes`/`string` at storage slot S:
//   slot[S]                 = encoded length (bytes as right-aligned big-endian
//                             integer; short-string optimization is intentionally
//                             NOT implemented — see comment below).
//   slot[keccak256(S) + i]  = 32-byte body chunks, big-endian.
//
// We deliberately *skip* solc's "short string optimization" (where bytes <= 31
// pack length+data into the slot itself). The optimization is incompatible
// with how `set_storage_or_clear` decides whether to delete: solc encodes the
// length in the low byte, but our static-value path always treats the slot as
// a 32-byte big-endian integer. Storing the full length in the slot lets
// `try_get` distinguish "never written" from "empty value" reliably.
// `cast storage` and similar tooling still read this correctly; only the
// gas-saving short-string fast path differs from Solidity.
// ---------------------------------------------------------------------------

#[cfg(feature = "alloc")]
fn dynamic_bytes_set(host: &Host, slot: &StorageKey, data: &[u8]) {
    // Write the length to the header slot.
    let mut length_buf = [0u8; 32];
    let len_bytes = (data.len() as u64).to_be_bytes();
    length_buf[24..32].copy_from_slice(&len_bytes);
    storage_set_32(host, slot.as_bytes(), &length_buf);

    if data.is_empty() {
        return;
    }

    // Write the body, 32 bytes per slot, starting at keccak256(slot).
    let mut body_slot = dynamic_data_root(host, slot.as_bytes());
    let mut offset = 0usize;
    while offset < data.len() {
        let mut chunk = [0u8; 32];
        let remaining = data.len() - offset;
        let take = if remaining >= 32 { 32 } else { remaining };
        chunk[..take].copy_from_slice(&data[offset..offset + take]);
        storage_set_32(host, &body_slot, &chunk);
        offset += take;
        inc_slot(&mut body_slot);
    }
}

#[cfg(feature = "alloc")]
fn dynamic_bytes_get(host: &Host, slot: &StorageKey) -> alloc::vec::Vec<u8> {
    let length_buf = storage_get_32(host, slot.as_bytes());
    let length = u64::from_be_bytes([
        length_buf[24],
        length_buf[25],
        length_buf[26],
        length_buf[27],
        length_buf[28],
        length_buf[29],
        length_buf[30],
        length_buf[31],
    ]) as usize;
    if length == 0 {
        return alloc::vec::Vec::new();
    }
    let mut out = alloc::vec::Vec::with_capacity(length);
    let mut body_slot = dynamic_data_root(host, slot.as_bytes());
    let mut remaining = length;
    while remaining > 0 {
        let chunk = storage_get_32(host, &body_slot);
        let take = if remaining >= 32 { 32 } else { remaining };
        out.extend_from_slice(&chunk[..take]);
        remaining -= take;
        inc_slot(&mut body_slot);
    }
    out
}

#[cfg(feature = "alloc")]
fn dynamic_bytes_try_get(host: &Host, slot: &StorageKey) -> Option<alloc::vec::Vec<u8>> {
    let length_buf = storage_try_get_32(host, slot.as_bytes())?;
    let length = u64::from_be_bytes([
        length_buf[24],
        length_buf[25],
        length_buf[26],
        length_buf[27],
        length_buf[28],
        length_buf[29],
        length_buf[30],
        length_buf[31],
    ]) as usize;
    if length == 0 {
        return Some(alloc::vec::Vec::new());
    }
    let mut out = alloc::vec::Vec::with_capacity(length);
    let mut body_slot = dynamic_data_root(host, slot.as_bytes());
    let mut remaining = length;
    while remaining > 0 {
        let chunk = storage_get_32(host, &body_slot);
        let take = if remaining >= 32 { 32 } else { remaining };
        out.extend_from_slice(&chunk[..take]);
        remaining -= take;
        inc_slot(&mut body_slot);
    }
    Some(out)
}

#[cfg(feature = "alloc")]
fn dynamic_bytes_clear(host: &Host, slot: &StorageKey) {
    // Read current length first so we know how many body slots to clear.
    let length_buf = storage_get_32(host, slot.as_bytes());
    let length = u64::from_be_bytes([
        length_buf[24],
        length_buf[25],
        length_buf[26],
        length_buf[27],
        length_buf[28],
        length_buf[29],
        length_buf[30],
        length_buf[31],
    ]) as usize;

    // Clear the header.
    storage_set_32(host, slot.as_bytes(), &[0u8; 32]);

    if length == 0 {
        return;
    }

    let mut body_slot = dynamic_data_root(host, slot.as_bytes());
    let chunks = length.div_ceil(32);
    for _ in 0..chunks {
        storage_set_32(host, &body_slot, &[0u8; 32]);
        inc_slot(&mut body_slot);
    }
}

#[cfg(feature = "alloc")]
impl SlotValue for alloc::vec::Vec<u8> {
    fn slot_get(host: &Host, slot: &StorageKey) -> Self {
        dynamic_bytes_get(host, slot)
    }

    fn slot_try_get(host: &Host, slot: &StorageKey) -> Option<Self> {
        dynamic_bytes_try_get(host, slot)
    }

    fn slot_set(&self, host: &Host, slot: &StorageKey) {
        dynamic_bytes_set(host, slot, self);
    }

    fn slot_clear(host: &Host, slot: &StorageKey) {
        dynamic_bytes_clear(host, slot);
    }
}

#[cfg(feature = "alloc")]
impl SlotValue for alloc::string::String {
    fn slot_get(host: &Host, slot: &StorageKey) -> Self {
        let bytes = dynamic_bytes_get(host, slot);
        // Solidity `string` is UTF-8 by convention; we tolerate invalid bytes
        // by replacing them, matching how alloy decodes string return data.
        alloc::string::String::from_utf8_lossy(&bytes).into_owned()
    }

    fn slot_try_get(host: &Host, slot: &StorageKey) -> Option<Self> {
        let bytes = dynamic_bytes_try_get(host, slot)?;
        Some(alloc::string::String::from_utf8_lossy(&bytes).into_owned())
    }

    fn slot_set(&self, host: &Host, slot: &StorageKey) {
        dynamic_bytes_set(host, slot, self.as_bytes());
    }

    fn slot_clear(host: &Host, slot: &StorageKey) {
        dynamic_bytes_clear(host, slot);
    }
}

// ---------------------------------------------------------------------------
// StorageComponent: how a typed storage object claims root slots.
// ---------------------------------------------------------------------------

/// A typed storage helper that occupies one or more contiguous root slots.
///
/// Implementations:
///
/// - [`Lazy<T>`]      — 1 slot.
/// - [`Mapping<K,V>`] — 1 slot (the root; entries live at derived keys).
/// - user storage structs annotated with `#[storage]` — sum of their fields'
///   `SLOTS`, assigned in declaration order.
///
/// The `#[contract]` macro reads `SLOTS` to assign slot numbers to fields. The
/// macro-generated constructor calls [`StorageComponent::new_at`] with the
/// assigned base slot and a clone of the contract's host handle.
///
/// # Why a separate trait from [`SlotValue`]
///
/// `SlotValue` describes how a *Rust value* lives in storage; `StorageComponent`
/// describes how a *typed storage helper* is constructed. `U256` implements
/// `SlotValue` (it can be stored), but never `StorageComponent` — you never put
/// a `U256` directly on a contract struct; you put a `Lazy<U256>` there.
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

#[cfg(feature = "abi-gen")]
impl<K: SolEncode, V: StorageLayoutType> StorageLayoutType for Mapping<K, V> {
    fn sol_type_name() -> String {
        format!("mapping({},{})", K::SOL_NAME, V::sol_type_name())
    }
}

// ---------------------------------------------------------------------------
// Lazy<T>
// ---------------------------------------------------------------------------

/// A single typed value at a fixed storage slot.
///
/// "Lazy" because there is no caching: every [`get`](Lazy::get) reads from host
/// storage, every [`set`](Lazy::set) writes immediately.
///
/// `T` may be any [`SlotValue`] — static 32-byte types (`U256`, `Address`,
/// `bool`, `[u8; 32]`, …) and, with the `alloc` feature, dynamic types
/// (`String`, `Vec<u8>`).
pub struct Lazy<T> {
    key: StorageKey,
    host: Host,
    _marker: PhantomData<T>,
}

impl<T: SlotValue> Lazy<T> {
    /// Create a new `Lazy` at the given storage key, bound to a host handle.
    pub fn new(key: StorageKey, host: Host) -> Self {
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
        T::slot_get(&self.host, &self.key)
    }

    /// Read the value, distinguishing "never written" from "has been set."
    ///
    /// Returns `None` if the slot was never written or was cleared.
    /// Returns `Some(value)` if a non-zero value was written.
    ///
    /// Note: writing an all-zero value deletes the key (Solidity semantics),
    /// so `try_get()` returns `None` after writing zero.
    pub fn try_get(&self) -> Option<T> {
        T::slot_try_get(&self.host, &self.key)
    }

    /// Write a value to storage.
    ///
    /// Takes `&mut self` so that view methods (which receive `&Storage`)
    /// cannot call this through an immutable borrow.
    pub fn set(&mut self, value: &T) {
        value.slot_set(&self.host, &self.key);
    }

    /// Clear the storage slot.
    ///
    /// Writes all-zero, which the host deletes from storage.
    pub fn clear(&mut self) {
        T::slot_clear(&self.host, &self.key);
    }
}

impl<T: SlotValue> StorageComponent for Lazy<T> {
    const SLOTS: u64 = 1;

    fn new_at(slot: u64, host: Host) -> Self {
        Lazy::new(StorageKey::from_slot(slot), host)
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

impl<K: AsStorageKey, V: SlotValue> Mapping<K, V> {
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
        V::slot_get(&self.host, &self.slot_of(key))
    }

    /// Read the value, returning `None` if the key was never written.
    pub fn try_get(&self, key: &K) -> Option<V> {
        V::slot_try_get(&self.host, &self.slot_of(key))
    }

    /// Write a value at the given key.
    pub fn insert(&mut self, key: &K, value: &V) {
        value.slot_set(&self.host, &self.slot_of(key));
    }

    /// Delete the value at the given key.
    pub fn remove(&mut self, key: &K) {
        V::slot_clear(&self.host, &self.slot_of(key));
    }
}

// ---------------------------------------------------------------------------
// Mapping<K1, Mapping<K2, V>> (nested)
// ---------------------------------------------------------------------------

/// Nested mappings can also be accessed with tuple keys:
/// `Mapping<(Address, Address), U256>` produces the same slots as
/// `Mapping<Address, Mapping<Address, U256>>`. Tuple key support is
/// implemented via `AsStorageKey` for tuples up to arity 5.
impl<K1: AsStorageKey, K2: AsStorageKey, V: SlotValue> Mapping<K1, Mapping<K2, V>> {
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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate alloc;
    extern crate std;

    use super::*;
    use alloc::rc::Rc;
    use alloc::string::String;
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

    // --- Dynamic SlotValue: String / Vec<u8> ---

    #[test]
    fn lazy_roundtrip_string_short() {
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
        lazy.set(&String::from("hello"));
        assert_eq!(lazy.get(), "hello");
    }

    #[test]
    fn lazy_roundtrip_string_long() {
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
        let long = "a".repeat(200);
        lazy.set(&long);
        assert_eq!(lazy.get(), long);
    }

    #[test]
    fn lazy_string_empty_is_default() {
        let lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
        assert_eq!(lazy.get(), "");
        assert_eq!(lazy.try_get(), None);
    }

    #[test]
    fn lazy_string_clear() {
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
        lazy.set(&String::from("payload"));
        assert_eq!(lazy.try_get().as_deref(), Some("payload"));
        lazy.clear();
        assert_eq!(lazy.try_get(), None);
        assert_eq!(lazy.get(), "");
    }

    #[test]
    fn lazy_string_overwrite_smaller() {
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
        lazy.set(&"hello world this is a longer string".into());
        lazy.set(&"short".into());
        assert_eq!(lazy.get(), "short");
    }

    #[test]
    fn lazy_roundtrip_vec_u8() {
        let mut lazy = Lazy::<Vec<u8>>::new(StorageKey::from_slot(0), h());
        lazy.set(&alloc::vec![1, 2, 3, 4, 5]);
        assert_eq!(lazy.get(), alloc::vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn lazy_vec_u8_large() {
        let mut lazy = Lazy::<Vec<u8>>::new(StorageKey::from_slot(0), h());
        let data: Vec<u8> = (0..=255u8).collect();
        lazy.set(&data);
        assert_eq!(lazy.get(), data);
    }

    #[test]
    fn mapping_address_to_string() {
        let mut m = Mapping::<Address, String>::new(StorageKey::from_slot(0), h());
        let a = Address([0x01; 20]);
        let b = Address([0x02; 20]);
        m.insert(&a, &"alice".into());
        m.insert(&b, &"bob".into());
        assert_eq!(m.get(&a), "alice");
        assert_eq!(m.get(&b), "bob");
        m.remove(&a);
        assert_eq!(m.try_get(&a), None);
        assert_eq!(m.get(&b), "bob");
    }

    #[test]
    fn dynamic_data_root_independent_per_slot() {
        // Distinct header slots must hash to distinct data roots so two
        // dynamic values on adjacent slots can't trample each other.
        let mut a = Lazy::<String>::new(StorageKey::from_slot(0), h());
        let host = a.host.clone();
        let mut b = Lazy::<String>::new(StorageKey::from_slot(1), host);
        a.set(&"first".into());
        b.set(&"second".into());
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
        assert_eq!(<Lazy<String> as StorageComponent>::SLOTS, 1);
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
}

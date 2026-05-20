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
//! accessors as static types — `Lazy<String>`, `Lazy<Vec<u8>>`,
//! `Mapping<K, String>`, `Mapping<K, Bytes>` encode inline when `len < 32` and
//! spill to `keccak256(slot) + i` chunks otherwise, matching `solc`'s storage
//! layout.
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
#[cfg(feature = "abi-gen")]
use pvm_contract_types::StaticEncodedLen;
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
/// Increase this if a contract needs larger inline static values.
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

/// A typed storage helper that occupies one or more contiguous root slots.
///
/// Implementations:
///
/// - [`Lazy<T>`]      — 1 slot. `T` may be static (e.g. `U256`) or dynamic
///   (e.g. `String`, `Vec<u8>`) with solc-compatible inline/spilled layout.
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

// Note: `StorageLayoutType` for dynamic value types (`String`, `Bytes`,
// `Vec<u8>`) is intentionally not implemented. Adding explicit impls clashes
// with the static blanket above (`impl<T: SolEncode + StaticEncodedLen> ..`)
// under Rust's coherence rules — the compiler cannot rule out a future
// `StaticEncodedLen` impl for these foreign types. Contracts that declare a
// dynamic-value mapping as a `#[slot]` field on the contract struct will get
// a clear "trait `StorageLayoutType` is not implemented" error at the abi-gen
// site. Resolving this requires sealing the blanket (separate refactor).
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
/// Dynamic `T` (`String`, `Vec<u8>`, `Bytes`, or `#[derive(SolType)]` structs
/// with dynamic fields) uses the same `Lazy<T>` accessor: the header lives
/// inline at `self.key` and any spilled body sits at `keccak256(key) + i`.
pub struct Lazy<T> {
    key: StorageKey,
    host: Host,
    _marker: PhantomData<T>,
}

impl<T: StorageEncode + StorageDecode> Lazy<T> {
    /// Compile-time validation of `T::STORAGE_SLOTS`. Referencing this in
    /// every public method forces the const evaluator to run the check at
    /// each monomorphization, even though the actual check lives in one place.
    const _SIZE_CHECK: () = {
        assert!(T::STORAGE_SLOTS > 0, "Lazy<T>: T::STORAGE_SLOTS must be positive");
        assert!(
            T::STORAGE_SLOTS <= MAX_STATIC_SLOTS,
            "Lazy<T>: T::STORAGE_SLOTS exceeds MAX_STATIC_SLOTS. \
             Use a dynamic value (String, Vec<u8>, Bytes) or raise MAX_STATIC_SLOTS."
        );
    };

    /// Create a new `Lazy` at the given storage key, bound to a host handle.
    pub fn new(key: StorageKey, host: Host) -> Self {
        let () = Self::_SIZE_CHECK;
        Lazy {
            key,
            host,
            _marker: PhantomData,
        }
    }

    /// Read the value from storage. For multi-slot `T`, reads
    /// `T::STORAGE_SLOTS` consecutive slots starting at `self.key`.
    ///
    /// Returns the zero value for `T` if the slot was never written,
    /// matching Solidity's default-to-zero semantics.
    pub fn get(&self) -> T {
        let () = Self::_SIZE_CHECK;
        if T::HAS_DYNAMIC_BODY {
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
            read_slots(&self.host, self.key.as_bytes(), &mut slots[..T::STORAGE_SLOTS]);
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
    pub fn try_get(&self) -> Option<T> {
        let () = Self::_SIZE_CHECK;
        if T::HAS_DYNAMIC_BODY {
            // Multi-slot dynamic V: "set" iff any header slot is non-zero.
            // For a single-slot LazySlot<T>, the header itself is the marker.
            // For a struct with a LazyDynamic field, the dynamic field's
            // header may be the only non-zero slot — checking just slot 0
            // would miss it.
            let mut buf = [[0u8; 32]; MAX_STATIC_SLOTS];
            if try_read_slots(&self.host, self.key.as_bytes(), &mut buf[..T::STORAGE_SLOTS])
                .is_none()
            {
                return None;
            }
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
            try_read_slots(&self.host, self.key.as_bytes(), &mut slots[..T::STORAGE_SLOTS])?;
            Some(T::from_slots(&slots[..T::STORAGE_SLOTS]))
        }
    }

    /// Write a value to storage. Encodes `value` slot-by-slot and writes to
    /// `T::STORAGE_SLOTS` consecutive slots starting at `self.key`.
    ///
    /// Takes `&mut self` so that view methods (which receive `&Storage`)
    /// cannot call this through an immutable borrow.
    pub fn set(&mut self, value: &T) {
        let () = Self::_SIZE_CHECK;
        if T::HAS_DYNAMIC_BODY {
            value.write_to_storage(&self.host, self.key.as_bytes());
        } else if T::STORAGE_SLOTS == 1 {
            let mut buf = [0u8; 32];
            value.encode_slot(0, &mut buf);
            storage_set_32(&self.host, self.key.as_bytes(), &buf);
        } else {
            write_value(&self.host, self.key.as_bytes(), value);
        }
    }

    /// Clear every slot occupied by this value.
    pub fn clear(&mut self) {
        let () = Self::_SIZE_CHECK;
        if T::HAS_DYNAMIC_BODY {
            <T as StorageEncode>::clear_storage(
                &self.host,
                self.key.as_bytes(),
                T::STORAGE_SLOTS,
            );
        } else if T::STORAGE_SLOTS == 1 {
            storage_set_32(&self.host, self.key.as_bytes(), &[0u8; 32]);
        } else {
            clear_n_slots(&self.host, self.key.as_bytes(), T::STORAGE_SLOTS);
        }
    }
}

impl<T: StorageEncode + StorageDecode> StorageComponent for Lazy<T> {
    /// One root slot per slot of `T::STORAGE_SLOTS`. A multi-slot `T` (e.g.
    /// `(U256, U256)`) reserves multiple consecutive slots, mirroring
    /// Solidity's struct-in-storage layout.
    const SLOTS: u64 = T::STORAGE_SLOTS as u64;

    fn new_at(slot: u64, host: Host) -> Self {
        Lazy::new(StorageKey::from_slot(slot), host)
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

impl<K: AsStorageKey, V: StorageEncode + StorageDecode> Mapping<K, V> {
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

    /// Read the value at the given key. For multi-slot `V`, reads
    /// `V::STORAGE_SLOTS` consecutive slots starting at the derived key.
    ///
    /// Returns the zero value if the key was never written.
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
            if try_read_slots(&self.host, slot.as_bytes(), &mut buf[..V::STORAGE_SLOTS]).is_none() {
                return None;
            }
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
            <V as StorageEncode>::clear_storage(
                &self.host,
                slot.as_bytes(),
                V::STORAGE_SLOTS,
            );
        } else if V::STORAGE_SLOTS == 1 {
            storage_set_32(&self.host, slot.as_bytes(), &[0u8; 32]);
        } else {
            clear_n_slots(&self.host, slot.as_bytes(), V::STORAGE_SLOTS);
        }
    }
}

// ---------------------------------------------------------------------------
// Mapping<K1, Mapping<K2, V>> (nested)
// ---------------------------------------------------------------------------

/// Nested mappings can also be accessed with tuple keys:
/// `Mapping<(Address, Address), U256>` produces the same slots as
/// `Mapping<Address, Mapping<Address, U256>>`. Tuple key support is
/// implemented via `AsStorageKey` for tuples up to arity 5.
impl<K1: AsStorageKey, K2: AsStorageKey, V: StorageEncode + StorageDecode>
    Mapping<K1, Mapping<K2, V>>
{
    /// Read path for nested mappings: derives the inner mapping root and
    /// returns a [`Ref`] so the inner mapping inherits the caller's `&self`
    /// borrow. Only `&self` methods on `Mapping<K2, V>` (e.g. `get`,
    /// `try_get`, `slot_of`) are reachable through it; `insert` / `entry`
    /// / `remove` would require `&mut self` and are blocked at compile time.
    pub fn get(&self, key: &K1) -> Ref<'_, Mapping<K2, V>> {
        Ref::new(Mapping::new(
            self.root.derive(&self.host, key),
            self.host.clone(),
        ))
    }

    /// Write path for nested mappings: derives the inner mapping root and
    /// returns a [`RefMut`] tied to the caller's `&mut self` borrow. The
    /// full mutating API on `Mapping<K2, V>` is reachable through the
    /// returned guard.
    pub fn entry(&mut self, key: &K1) -> RefMut<'_, Mapping<K2, V>> {
        RefMut::new(Mapping::new(
            self.root.derive(&self.host, key),
            self.host.clone(),
        ))
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

    // --- Multi-slot Lazy<T> (T spans >1 storage slot) ---

    #[test]
    fn lazy_roundtrip_tuple_two_u256() {
        let mut lazy = Lazy::<(U256, U256)>::new(StorageKey::from_slot(0), h());
        let v = (U256::from(7u64), U256::from(11u64));
        lazy.set(&v);
        assert_eq!(lazy.get(), v);
    }

    #[test]
    fn lazy_multi_slot_writes_consecutive_keys() {
        // (U256, U256) has ENCODED_SIZE == 64, so set() must touch slots
        // `key` and `key + 1`. Confirm the wire format by reading the slots
        // directly: the first U256 lands at `key`, the second at `key + 1`.
        let mut lazy = Lazy::<(U256, U256)>::new(StorageKey::from_slot(0), h());
        let host = lazy.host.clone();
        let base = *lazy.key.as_bytes();

        lazy.set(&(U256::from(0xAAu64), U256::from(0xBBu64)));

        let slot0 = storage_get_32(&host, &base);
        let mut next = base;
        inc_slot(&mut next);
        let slot1 = storage_get_32(&host, &next);

        assert_eq!(slot0[31], 0xAA, "first U256 at base slot: {slot0:?}");
        assert_eq!(slot1[31], 0xBB, "second U256 at base + 1: {slot1:?}");
    }

    #[test]
    fn lazy_multi_slot_try_get_some_when_only_second_word_set() {
        // Direct-write a value where the first 32-byte word is zero but the
        // second is non-zero. try_get must still observe the entry as present.
        let host = h();
        let key = StorageKey::from_slot(0);
        let mut second = [0u8; 32];
        second[31] = 0x42;
        let mut next = *key.as_bytes();
        inc_slot(&mut next);
        storage_set_32(&host, &next, &second);

        let lazy = Lazy::<(U256, U256)>::new(key, host);
        assert_eq!(lazy.try_get(), Some((U256::ZERO, U256::from(0x42u64))));
    }

    #[test]
    fn lazy_multi_slot_try_get_none_when_unwritten() {
        let lazy = Lazy::<(U256, U256)>::new(StorageKey::from_slot(0), h());
        assert_eq!(lazy.try_get(), None);
    }

    #[test]
    fn lazy_multi_slot_clear_removes_all_words() {
        // Set both words non-zero, clear, then verify each underlying slot
        // is truly absent (not just zero in the decoded value).
        let mut lazy = Lazy::<(U256, U256)>::new(StorageKey::from_slot(0), h());
        let host = lazy.host.clone();
        let base = *lazy.key.as_bytes();

        lazy.set(&(U256::from(1u64), U256::from(2u64)));
        lazy.clear();

        let mut next = base;
        assert_eq!(storage_try_get_32(&host, &next), None, "word 0 not cleared");
        inc_slot(&mut next);
        assert_eq!(storage_try_get_32(&host, &next), None, "word 1 not cleared");
    }

    #[test]
    fn lazy_multi_slot_overwrite_zero_clears_stale_slot() {
        // After writing (5, 5), writing (5, 0) must auto-delete slot 1 so
        // try_get observes the zero on subsequent reads.
        let mut lazy = Lazy::<(U256, U256)>::new(StorageKey::from_slot(0), h());
        let host = lazy.host.clone();
        let mut next = *lazy.key.as_bytes();
        inc_slot(&mut next);

        lazy.set(&(U256::from(5u64), U256::from(5u64)));
        lazy.set(&(U256::from(5u64), U256::ZERO));

        assert_eq!(lazy.get(), (U256::from(5u64), U256::ZERO));
        assert_eq!(
            storage_try_get_32(&host, &next),
            None,
            "stale slot for word 1 must be auto-deleted"
        );
    }

    #[test]
    fn lazy_multi_slot_slots_const_matches_word_count() {
        // SLOTS = ENCODED_SIZE / 32. For (U256, U256) that's 2, so an
        // auto-numbered field after this Lazy would be 2 slots later.
        assert_eq!(<Lazy<U256> as StorageComponent>::SLOTS, 1);
        assert_eq!(<Lazy<(U256, U256)> as StorageComponent>::SLOTS, 2);
        assert_eq!(<Lazy<(U256, U256, U256)> as StorageComponent>::SLOTS, 3);
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

    // --- Multi-slot Mapping<K, V> (V spans >1 storage slot) ---

    #[test]
    fn mapping_insert_get_tuple_value() {
        let mut m = Mapping::<Address, (U256, U256)>::new(StorageKey::from_slot(0), h());
        let addr = Address([0xAB; 20]);
        let v = (U256::from(123u64), U256::from(456u64));
        m.insert(&addr, &v);
        assert_eq!(m.get(&addr), v);
    }

    #[test]
    fn mapping_multi_slot_remove_clears_all_words() {
        let mut m = Mapping::<Address, (U256, U256)>::new(StorageKey::from_slot(0), h());
        let host = m.host.clone();
        let addr = Address([0xCD; 20]);
        let derived = *m.slot_of(&addr).as_bytes();

        m.insert(&addr, &(U256::from(1u64), U256::from(2u64)));
        m.remove(&addr);

        let mut k = derived;
        assert_eq!(storage_try_get_32(&host, &k), None, "word 0 not removed");
        inc_slot(&mut k);
        assert_eq!(storage_try_get_32(&host, &k), None, "word 1 not removed");
        assert_eq!(m.try_get(&addr), None);
    }

    #[test]
    fn mapping_multi_slot_overwrite_smaller_clears_stale_word() {
        // insert (1, 2) then insert (1, 0): the second word must be deleted
        // so a follow-up read doesn't return stale 2.
        let mut m = Mapping::<Address, (U256, U256)>::new(StorageKey::from_slot(0), h());
        let host = m.host.clone();
        let addr = Address([0xEF; 20]);
        let mut next = *m.slot_of(&addr).as_bytes();
        inc_slot(&mut next);

        m.insert(&addr, &(U256::from(1u64), U256::from(2u64)));
        m.insert(&addr, &(U256::from(1u64), U256::ZERO));

        assert_eq!(m.get(&addr), (U256::from(1u64), U256::ZERO));
        assert_eq!(storage_try_get_32(&host, &next), None);
    }

    #[test]
    fn mapping_multi_slot_entry_handle_reads_and_writes_full_value() {
        // entry() returns a Lazy<V> at the derived key. With multi-slot V it
        // must still read/write all chunks correctly.
        let mut m = Mapping::<Address, (U256, U256)>::new(StorageKey::from_slot(0), h());
        let addr = Address([0x10; 20]);
        let v = (U256::from(99u64), U256::from(100u64));

        let mut cell = m.entry(&addr);
        cell.set(&v);
        assert_eq!(cell.get(), v);

        // And the parent Mapping reads back the same value through its own
        // derived key, confirming entry() didn't drift off the right key.
        assert_eq!(m.get(&addr), v);
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

    // --- Dynamic accessors: Lazy<String> / Lazy<Vec<u8>> ---

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_roundtrip_string_short() {
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
        lazy.set(&String::from("hello"));
        assert_eq!(lazy.get(), "hello");
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_roundtrip_string_long() {
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
        let long = "a".repeat(200);
        lazy.set(&long);
        assert_eq!(lazy.get(), long);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_empty_is_default() {
        let lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
        assert_eq!(lazy.get(), "");
        assert_eq!(lazy.try_get(), None);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_clear() {
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
        lazy.set(&String::from("payload"));
        assert_eq!(lazy.try_get().as_deref(), Some("payload"));
        lazy.clear();
        assert_eq!(lazy.try_get(), None);
        assert_eq!(lazy.get(), "");
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_overwrite_smaller() {
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
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
        let mut written = Lazy::<String>::new(StorageKey::from_slot(0), h());
        let never = Lazy::<String>::new(StorageKey::from_slot(1), written.host.clone());

        written.set(&String::new());

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
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
        let host = lazy.host.clone();
        let key = lazy.key;

        lazy.set(&String::new());

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
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
        let host = lazy.host.clone();
        let key = lazy.key;

        lazy.set(&String::new());
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
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
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
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
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
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
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

    /// A spilled header (low bit of byte 31 set) with non-zero bytes in the
    /// upper half of the u256 length field cannot be a real stored length —
    /// any plausible value fits in the low 128 bits. Without validation the
    /// decoder would silently use the truncated low bits and `read_dyn_body`
    /// would walk a fabricated number of chunks. The decoder now treats any
    /// such slot as empty.
    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_bytes_spilled_high_bytes_treated_as_malformed() {
        let host = h();
        let key = StorageKey::from_slot(0);
        let mut malformed = [0u8; 32];
        malformed[0] = 0xFF; // non-zero high byte ⇒ malformed
        malformed[31] = 0x01; // low bit set ⇒ spilled
        storage_set_32(&host, key.as_bytes(), &malformed);

        let lazy = Lazy::<Vec<u8>>::new(key, host);
        assert!(lazy.get().is_empty());
    }

    /// A malformed inline header (byte31 > 62, low bit 0) encodes a decoded
    /// length > 31. Without a cap, `dynamic_bytes_get` would slice past the
    /// 32-byte slot buffer and panic. The decoder caps `len` at 31 so reads
    /// of corrupted / foreign-written slots return at most 31 bytes instead.
    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_bytes_inline_len_capped_on_malformed_slot() {
        let host = h();
        let key = StorageKey::from_slot(0);
        // byte31 = 0xFE → decoded len = 127 (way past slot capacity).
        let mut malformed = [0u8; 32];
        for (i, b) in malformed.iter_mut().enumerate().take(31) {
            *b = i as u8 + 1;
        }
        malformed[31] = 0xFE;
        storage_set_32(&host, key.as_bytes(), &malformed);

        let lazy = Lazy::<Vec<u8>>::new(key, host);
        // Must not panic. Cap is 31 bytes — the original 31 prefix bytes.
        let bytes = lazy.get();
        assert_eq!(bytes.len(), 31);
        assert_eq!(&bytes[..], &malformed[..31]);
    }

    /// Long-spill probe: header is `len * 2 + 1` big-endian, body chunks live
    /// at consecutive slots starting from `keccak256(slot)`.
    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_long_spill_layout() {
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
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
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
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
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
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
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
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

    /// `Mapping<Address, String>` with a spill-form value round-trips through
    /// the same layout path.
    #[cfg(feature = "alloc")]
    #[test]
    fn mapping_with_long_string_value() {
        let mut m = Mapping::<Address, String>::new(StorageKey::from_slot(0), h());
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
        let mut lazy = Lazy::<Vec<u8>>::new(StorageKey::from_slot(0), h());
        lazy.set(&alloc::vec![1, 2, 3, 4, 5]);
        assert_eq!(lazy.get(), alloc::vec![1, 2, 3, 4, 5]);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_vec_u8_large() {
        let mut lazy = Lazy::<Vec<u8>>::new(StorageKey::from_slot(0), h());
        let data: Vec<u8> = (0..=255u8).collect();
        lazy.set(&data);
        assert_eq!(lazy.get(), data);
    }

    /// `Vec<u8>` rides the same solc-compatible path as `String`. Cover the
    /// inline / spill boundary explicitly: 31 bytes inline, 32 bytes spills.
    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_vec_u8_boundary() {
        let mut a = Lazy::<Vec<u8>>::new(StorageKey::from_slot(0), h());
        let host = a.host.clone();
        let key_a = a.key;

        let inline: Vec<u8> = (0..31).collect();
        a.set(&inline);
        let slot_bytes = storage_get_32(&host, key_a.as_bytes());
        assert_eq!(slot_bytes[31], 31 * 2, "31B vec inline, byte31 = 62");
        assert_eq!(a.get(), inline);

        let mut b = Lazy::<Vec<u8>>::new(StorageKey::from_slot(1), host);
        let spill: Vec<u8> = (0..32).collect();
        b.set(&spill);
        let slot_b = storage_get_32(&b.host, b.key.as_bytes());
        assert_eq!(slot_b[31], 32 * 2 + 1, "32B vec spills, byte31 = 65");
        assert_eq!(b.get(), spill);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn mapping_address_to_string() {
        let mut m = Mapping::<Address, String>::new(StorageKey::from_slot(0), h());
        let a = Address([0x01; 20]);
        let b = Address([0x02; 20]);
        m.insert(&a, &String::from("alice"));
        m.insert(&b, &String::from("bob"));
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
        let mut a = Lazy::<String>::new(StorageKey::from_slot(0), h());
        let host = a.host.clone();
        let mut b = Lazy::<String>::new(StorageKey::from_slot(1), host);
        a.set(&String::from("first"));
        b.set(&String::from("second"));
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
        assert_eq!(<Lazy<String> as StorageComponent>::SLOTS, 1);
        assert_eq!(<Lazy<Vec<u8>> as StorageComponent>::SLOTS, 1);
        assert_eq!(<Mapping<Address, String> as StorageComponent>::SLOTS, 1);
        assert_eq!(<Mapping<Address, Vec<u8>> as StorageComponent>::SLOTS, 1);
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

    // --- Solidity zero-slot semantics ---

    /// `insert(k, &V::default())` deletes the slot (matching `SSTORE`-clears),
    /// so a subsequent `try_get` returns `None` even though we just wrote.
    /// Pinned here so the conflation between "never written" and "explicit
    /// zero" stays documented behavior, not an accidental regression.
    #[test]
    fn try_get_returns_none_after_inserting_zero() {
        let host = h();
        let mut m = Mapping::<Address, U256>::new(StorageKey::from_slot(0), host);
        let addr = Address([0x77; 20]);

        m.insert(&addr, &U256::from(42));
        assert_eq!(m.try_get(&addr), Some(U256::from(42)));

        m.insert(&addr, &U256::ZERO);
        assert_eq!(m.try_get(&addr), None);
        assert_eq!(m.get(&addr), U256::ZERO);
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

    // ---------------------------------------------------------------------
    // Native String / Vec<u8> in Lazy / Mapping
    // ---------------------------------------------------------------------

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_native_short_round_trip() {
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
        lazy.set(&String::from("hello"));
        assert_eq!(lazy.get(), "hello");
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_native_long_round_trip() {
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
        let long: String = "x".repeat(80); // spills across multiple body chunks
        lazy.set(&long);
        assert_eq!(lazy.get(), long);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_native_try_get_distinguishes_set_empty_from_unset() {
        let mut written = Lazy::<String>::new(StorageKey::from_slot(0), h());
        let never = Lazy::<String>::new(StorageKey::from_slot(1), written.host.clone());

        written.set(&String::new());
        let got = written.try_get();
        assert_eq!(got, Some(String::new()));
        assert!(never.try_get().is_none());
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_native_clear_removes_header_and_body() {
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
        let host = lazy.host.clone();
        let key = lazy.key;

        lazy.set(&"x".repeat(80));
        lazy.clear();

        assert_eq!(
            storage_try_get_32(&host, key.as_bytes()),
            None,
            "header not cleared"
        );
        let mut body = dynamic_data_root(&host, key.as_bytes());
        for _ in 0..3 {
            assert_eq!(storage_try_get_32(&host, &body), None);
            inc_slot(&mut body);
        }
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn mapping_string_native_round_trip() {
        let mut m = Mapping::<u64, String>::new(StorageKey::from_slot(0), h());
        m.insert(&1u64, &String::from("hello"));
        m.insert(&2u64, &"y".repeat(64));

        assert_eq!(m.get(&1u64), "hello");
        assert_eq!(m.get(&2u64), "y".repeat(64));
        assert!(m.try_get(&3u64).is_none());

        m.remove(&1u64);
        assert!(m.try_get(&1u64).is_none());
        assert_eq!(m.get(&2u64), "y".repeat(64));
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_bytes_native_round_trip() {
        let mut lazy = Lazy::<Vec<u8>>::new(StorageKey::from_slot(0), h());
        let payload: Vec<u8> = (0..50).collect();
        lazy.set(&payload);
        assert_eq!(lazy.get(), payload);
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn lazy_string_native_layout_matches_solc_short() {
        let mut lazy = Lazy::<String>::new(StorageKey::from_slot(0), h());
        let host = lazy.host.clone();
        let key = lazy.key;
        lazy.set(&String::from("hello"));

        let header = storage_get_32(&host, key.as_bytes());
        assert_eq!(&header[..5], b"hello");
        assert!(header[5..31].iter().all(|&b| b == 0));
        assert_eq!(header[31], 5 * 2);
    }
}

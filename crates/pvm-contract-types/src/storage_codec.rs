//! Solidity-compatible storage codec.
//!
//! Encodes Rust values into the byte layout that `solc` uses for contract
//! storage — sub-word packing for primitives that fit, multi-slot spread for
//! larger composites, big-endian right-aligned for both integers and `bytesN`.
//! The on-chain bytes produced by [`StorageEncode`] / [`StorageDecode`] match
//! what an equivalent solc-compiled contract would write, so tools like
//! `cast storage` interoperate transparently.
//!
//! This codec is intentionally separate from [`SolEncode`](crate::SolEncode) /
//! [`SolDecode`](crate::SolDecode), which describe the ABI wire format
//! (calldata, return values, events). Solidity itself uses different rules for
//! those two contexts; the SDK mirrors that split.
//!
//! # Roles
//!
//! - [`StorageEncode`] / [`StorageDecode`] describe a top-level (possibly
//!   multi-slot) storage value. They drive `Mapping<K, V>` / `Lazy<T>` writes
//!   and reads.
//! - [`StoragePackable`] is the public sub-word packing API for primitives
//!   that can share a slot with siblings at arbitrary byte offsets. It owns
//!   the [`pack_into`](StoragePackable::pack_into) /
//!   [`unpack_from`](StoragePackable::unpack_from) operations as required
//!   methods (no defaults), so the bound enforces that callers can actually
//!   pack the type — used by tuple `StorageEncode` impls and the
//!   `#[derive(SolStorage)]` macro's struct field encoders.
//!
//! `StorageEncode` / `StorageDecode` also carry hidden polymorphic dispatch
//! hooks (`__pack_into_dispatched` / `__unpack_from_dispatched`) used by
//! `Lazy<T>`'s packed-path. Their defaults are a `const { assert!(PACKED_BYTES
//! == 32) }` that fails the build for a sub-word type lacking a
//! `StoragePackable` override (and is inert for full-slot types, whose packed
//! branch is statically dead); in-tree packable impls override them to
//! delegate to `StoragePackable`. Downstream code
//! should impl `StoragePackable` (which compile-checks the operations) and
//! mirror the delegation pattern in its `StorageEncode` / `StorageDecode`
//! impls so `Lazy<T>` works.
//!
//! Static types additionally implement [`StaticStorageEncode`] /
//! [`StaticStorageDecode`] — the slot-buffer codec refinement. Dynamic types
//! (`String`, `Bytes`, structs containing dynamic fields) do NOT impl the
//! `Static*` refinement; their body lives at `keccak256(slot)+i`, so they
//! cannot be reconstructed from a slot buffer alone. Each type owns its
//! own host access via the required [`write_to_storage`], [`read_from_storage`],
//! [`try_read_from_storage`], [`clear_storage`] methods — `Lazy<T>` and
//! `Mapping<K, V>` never branch on which kind T is.
//!
//! [`write_to_storage`]: StorageEncode::write_to_storage
//! [`read_from_storage`]: StorageDecode::read_from_storage
//! [`try_read_from_storage`]: StorageDecode::try_read_from_storage
//! [`clear_storage`]: StorageEncode::clear_storage

use crate::{Address, Host, HostApi, I256, StorageFlags, U256};
use crate::{LayoutStep, layout_step};

/// `StorageEncode`-family wrapper over [`layout_step`]: reads the field type's
/// `PACKED_BYTES` + `STORAGE_SLOTS` so call sites pass only the type — not the
/// two consts separately, not the `as u64` cast, and with no way to mix two
/// different types' consts. Used by the tuple `StorageEncode` impls and the
/// `#[derive(SolStorage)]` static-field walker. The trait-agnostic
/// [`layout_step`] stays the primitive underneath.
pub const fn layout_step_encode<T: StorageEncode>(prev: LayoutStep) -> LayoutStep {
    layout_step(prev, T::PACKED_BYTES, T::STORAGE_SLOTS as u64)
}

/// Increment a 32-byte big-endian integer in-place. Used to walk consecutive
/// storage slots for multi-slot values.
///
/// Public (and re-exported via `pvm_contract_sdk::__private`) so the
/// `#[derive(SolStorage)]` dynamic-struct codegen calls one shared definition
/// instead of pasting a copy into every generated method body.
#[doc(hidden)]
#[inline]
pub fn inc_be_32(slot: &mut [u8; 32]) {
    for byte in slot.iter_mut().rev() {
        let (next, carry) = byte.overflowing_add(1);
        *byte = next;
        if !carry {
            return;
        }
    }
}

// ---------------------------------------------------------------------------
// Shared static-slot sweep for `#[derive(SolStorage)]` dynamic structs.
//
// A dynamic-bodied struct interleaves static-field slots with dynamic-field
// header slots. `dynamic_mask` bit `i` is set when slot `i` belongs to a
// dynamic field (`String`/`Bytes`), whose body is written/read by that field's
// own `StorageEncode`/`StorageDecode` impl at a derived sub-key. The three
// helpers below own the slot-walk + mask logic so the derive macro emits one
// helper call per method instead of an inline loop in each of write / clear /
// read / try_read. Slot count is bounded by `MAX_STATIC_SLOTS`, and the
// `dynamic_mask: u64` covers up to 64 slots, so the index can never overflow
// the mask. Re-exported via `pvm_contract_sdk::__private`.
// ---------------------------------------------------------------------------

/// SSTORE the non-masked (static) slots from a pre-encoded `slots` buffer,
/// skipping dynamic-field slots (written separately by the field's own impl).
/// `slots.len()` is the struct's `STORAGE_SLOTS`.
#[doc(hidden)]
#[inline]
pub fn write_static_slots(host: &Host, base_key: &[u8; 32], slots: &[[u8; 32]], dynamic_mask: u64) {
    let mut k = *base_key;
    for (i, slot) in slots.iter().enumerate() {
        if dynamic_mask & (1u64 << i) == 0 {
            host.set_storage_or_clear(StorageFlags::empty(), &k, slot);
        }
        inc_be_32(&mut k);
    }
}

/// Zero the non-masked (static) slots, skipping dynamic-field slots (cleared
/// separately by the field's own impl). `n` is the struct's `STORAGE_SLOTS`.
#[doc(hidden)]
#[inline]
pub fn clear_static_slots(host: &Host, base_key: &[u8; 32], n: usize, dynamic_mask: u64) {
    let mut k = *base_key;
    for i in 0..n {
        if dynamic_mask & (1u64 << i) == 0 {
            host.set_storage_or_clear(StorageFlags::empty(), &k, &[0u8; 32]);
        }
        inc_be_32(&mut k);
    }
}

/// SLOAD the non-masked (static) slots into `slots_out`, skipping dynamic-field
/// header slots (those are decoded from their own sub-keys). Used by
/// `read_from_storage`, which has no presence check to do, so dynamic header
/// slots are never loaded here. `slots_out.len()` must be `>= n`.
#[doc(hidden)]
#[inline]
pub fn load_static_slots(
    host: &Host,
    base_key: &[u8; 32],
    n: usize,
    dynamic_mask: u64,
    slots_out: &mut [[u8; 32]],
) {
    let mut k = *base_key;
    for (i, slot) in slots_out[..n].iter_mut().enumerate() {
        if dynamic_mask & (1u64 << i) == 0 {
            host.get_storage_or_zero(StorageFlags::empty(), &k, slot);
        }
        inc_be_32(&mut k);
    }
}

/// SLOAD all `n` slots starting at `base_key`, returning `true` if any slot
/// — **including dynamic header slots** — is non-zero. Non-masked (static)
/// slots are copied into `slots_out` so `try_read_from_storage` can decode
/// them without a second pass; dynamic header slots count toward presence but
/// are not stored (their fields decode from their own sub-keys). This matches
/// the Solidity-compat "all-zero ⇒ absent" rule across the whole struct.
/// `slots_out.len()` must be `>= n`.
#[doc(hidden)]
#[inline]
pub fn try_load_static_slots(
    host: &Host,
    base_key: &[u8; 32],
    n: usize,
    dynamic_mask: u64,
    slots_out: &mut [[u8; 32]],
) -> bool {
    let mut k = *base_key;
    let mut any_nonzero = false;
    for (i, slot) in slots_out[..n].iter_mut().enumerate() {
        let mut buf = [0u8; 32];
        host.get_storage_or_zero(StorageFlags::empty(), &k, &mut buf);
        if buf != [0u8; 32] {
            any_nonzero = true;
        }
        if dynamic_mask & (1u64 << i) == 0 {
            *slot = buf;
        }
        inc_be_32(&mut k);
    }
    any_nonzero
}

/// Top-level storage encoder.
///
/// A type implementing this trait can be the value of a `Mapping<K, V>` or
/// `Lazy<T>`. Required methods are host-aware — each type owns its own
/// access pattern (single-slot primitives do one SLOAD; multi-slot tuples
/// loop; dynamic types like `String` write a header + body chunks).
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be stored in `Lazy<{Self}>` or `Mapping<_, {Self}>`",
    label = "`{Self}` does not implement `StorageEncode`",
    note = "add `#[derive(SolStorage)]` to `{Self}` — only types deriving `SolStorage` can be `Lazy<T>` / `Mapping<_, T>` values",
    note = "if `{Self}` only appears in calldata, returns, or events, keep `#[derive(SolType)]` and don't put it in storage"
)]
pub trait StorageEncode {
    /// Total number of slots this type occupies when stored at the top of a
    /// layout. Always >= 1.
    const STORAGE_SLOTS: usize;

    /// Number of bytes this type consumes within a single slot when packed
    /// alongside sibling fields. Must satisfy `1 <= PACKED_BYTES <= 32`.
    /// Full-slot types use `32`; composites and dynamic-body types
    /// (`String`, `Bytes`) also use `32` — they always claim a fresh slot.
    ///
    /// **Types with `PACKED_BYTES < 32` must also implement
    /// [`StoragePackable`].** `Lazy<T>::set` / `Lazy<T>::get` take a
    /// read-modify-write path for sub-word values and dispatch through
    /// [`__pack_into_dispatched`](Self::__pack_into_dispatched) /
    /// [`StorageDecode::__unpack_from_dispatched`]. Their default impls
    /// `const`-assert `PACKED_BYTES == 32`, so a sub-word type that omits the
    /// `StoragePackable` impl **fails to compile** (rather than panicking at
    /// runtime); the `StoragePackable` impl is what supplies the override.
    const PACKED_BYTES: usize = 32;

    /// `true` for types whose value spills outside their slot range — i.e.
    /// `String` / `Bytes` and any `SolType` struct that contains them. These
    /// types route encode/decode through [`write_to_storage`](Self::write_to_storage)
    /// / [`StorageDecode::read_from_storage`] (header in the slot, body at
    /// `keccak256(slot) + i`) rather than the fixed slot-buffer codec.
    ///
    /// Static types (primitives, fixed arrays, tuples, fully-static structs)
    /// leave this `false`. It gates two compile-time guards:
    /// - `StorageEncode for [T; N]` const-asserts `!T::HAS_DYNAMIC_BODY`,
    ///   rejecting `[String; N]` (use [`StorageVec<T>`] instead).
    /// - `StorageVec<T>::clear_at` uses it to choose between a plain
    ///   slot-zeroing clear and `T::clear_storage` (which also tears down
    ///   spilled body chunks).
    ///
    /// [`StorageVec<T>`]: https://docs.rs/pvm-storage
    const HAS_DYNAMIC_BODY: bool = false;

    /// Write self to storage starting at `base_key`. Required. Each impl
    /// owns its access pattern:
    /// - Single-slot primitives do one SSTORE.
    /// - Multi-slot static types (tuples, multi-field structs) loop SSTOREs.
    /// - Dynamic types (`String`, `Bytes`) write a header slot at `base_key`
    ///   and body chunks at `keccak256(base_key) + i`, clearing any stale
    ///   body chunks left from a previously-longer value.
    ///
    /// Static-type impls typically delegate to
    /// [`StaticStorageEncode::write_to_storage_static`] for one-line bodies.
    /// Dynamic types use the `write_dynamic_bytes` helper.
    fn write_to_storage(&self, host: &Host, base_key: &[u8; 32]);

    /// Clear every storage cell this type occupies at `base_key`. Required.
    /// Static types: zero `STORAGE_SLOTS` consecutive slots. Dynamic types:
    /// zero the header slot AND clear any spilled body chunks.
    ///
    /// Static-type impls typically delegate to
    /// [`StaticStorageEncode::clear_storage_static`].
    fn clear_storage(host: &Host, base_key: &[u8; 32]);

    /// Internal polymorphic dispatch hook for `Lazy<T>`'s packed-path
    /// (`Lazy<T>::set` when `PACKED_BYTES < 32`). The canonical
    /// `pack_into` operation lives on [`StoragePackable`]; this hook lets
    /// `Lazy<T>`, whose `T` is only bound by `StorageEncode + StorageDecode`,
    /// reach a packable impl through a const-folded `T::PACKED_BYTES < 32`
    /// branch. Full-slot types (`PACKED_BYTES == 32`) and dynamic types never
    /// reach this branch at runtime, but the const-folded dead branch still
    /// forces monomorphization of this default for them — so the `const`
    /// assert is gated on `PACKED_BYTES == 32` (passes for full-slot, where it
    /// is dead, and **fails the build** for a sub-word type that forgot to
    /// implement `StoragePackable` and override this hook). An unconditional
    /// `const { panic!() }` would instead break `Lazy<U256>` and every other
    /// full-slot type, since their dead branch monomorphizes this body too.
    #[doc(hidden)]
    fn __pack_into_dispatched(&self, _buf: &mut [u8; 32], _offset: usize) {
        const {
            assert!(
                <Self as StorageEncode>::PACKED_BYTES == 32,
                "StorageEncode type with PACKED_BYTES < 32 must implement StoragePackable AND override `fn __pack_into_dispatched`",
            )
        }
        unreachable!(
            "Lazy<T>::set dispatches sub-word T to StoragePackable::pack_into; this default is reached only by full-slot types whose branch is statically dead",
        )
    }
}

/// Top-level storage decoder. Symmetric with [`StorageEncode`].
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be read from `Lazy<{Self}>` or `Mapping<_, {Self}>`",
    label = "`{Self}` does not implement `StorageDecode`",
    note = "add `#[derive(SolStorage)]` to `{Self}` — only types deriving `SolStorage` can be `Lazy<T>` / `Mapping<_, T>` values",
    note = "if `{Self}` only appears in calldata, returns, or events, keep `#[derive(SolType)]` and don't put it in storage"
)]
pub trait StorageDecode: StorageEncode + Sized {
    /// Read self from storage at `base_key`. Required.
    ///
    /// Static-type impls typically delegate to
    /// [`StaticStorageDecode::read_from_storage_static`]. Dynamic types read
    /// their header + body chunks.
    fn read_from_storage(host: &Host, base_key: &[u8; 32]) -> Self;

    /// Read self if present, else `None`. **Solidity-compat invariant:** a
    /// static type with all-zero slots returns `None` (matches solc's
    /// "writing the zero value deletes the slot" semantics — there is no
    /// way to distinguish "never written" from "explicitly set to zero").
    /// A dynamic type returns `None` iff the header slot is zero.
    ///
    /// Static-type impls typically delegate to
    /// [`StaticStorageDecode::try_read_from_storage_static`].
    fn try_read_from_storage(host: &Host, base_key: &[u8; 32]) -> Option<Self>;

    /// Internal polymorphic dispatch hook for `Lazy<T>`'s packed-path
    /// (`Lazy<T>::get` when `PACKED_BYTES < 32`). Symmetric with
    /// [`StorageEncode::__pack_into_dispatched`]; the canonical
    /// `unpack_from` lives on [`StoragePackable`]. The `const` assert is gated
    /// on `PACKED_BYTES == 32` for the same reason as
    /// [`StorageEncode::__pack_into_dispatched`]: a sub-word type that forgot
    /// to override fails the build, while full-slot types (whose branch is
    /// dead) pass.
    #[doc(hidden)]
    fn __unpack_from_dispatched(_buf: &[u8; 32], _offset: usize) -> Self {
        const {
            assert!(
                <Self as StorageEncode>::PACKED_BYTES == 32,
                "StorageEncode type with PACKED_BYTES < 32 must implement StoragePackable AND override `fn __unpack_from_dispatched`",
            )
        }
        unreachable!(
            "Lazy<T>::get dispatches sub-word T to StoragePackable::unpack_from; this default is reached only by full-slot types whose branch is statically dead",
        )
    }
}

/// Slot-buffer encoder refinement. Implemented only by types that can be
/// reconstructed from a fixed-size slot buffer alone — primitives, fixed
/// arrays, tuples of packable elements, and fully-static SolType-derived
/// structs. Dynamic types (`String`, `Bytes`, structs with dynamic fields)
/// do NOT implement this; the absence is the type-level expression of "this
/// has a body that lives outside its slot range."
///
/// The defaulted [`write_to_storage_static`](Self::write_to_storage_static) /
/// [`clear_storage_static`](Self::clear_storage_static) methods provide the
/// canonical host-aware codepaths for static types. Per-type
/// [`StorageEncode::write_to_storage`] / [`StorageEncode::clear_storage`]
/// impls are one-line delegates to these defaults.
///
/// **Single-slot fast path**: every method on this trait const-folds the
/// `STORAGE_SLOTS == 1` branch at monomorphization, so primitives
/// (`u32`, `U256`, `Address`, `[u8; N]`, ...) skip the 32-byte unaligned
/// key copy and the wasted `inc_be_32` — they produce the same tight
/// SSTORE/SLOAD codegen as direct calls would.
pub trait StaticStorageEncode: StorageEncode {
    /// Encode slot `slot_idx` of this value into `buf`. Caller passes a
    /// to-be-overwritten 32-byte buffer; the implementation fills the bytes
    /// that belong to this slot at their canonical positions. For primitives
    /// the entire slot is overwritten; for composites only the field's
    /// byte window is touched.
    ///
    /// `slot_idx` must satisfy `slot_idx < STORAGE_SLOTS`.
    fn encode_slot(&self, slot_idx: usize, buf: &mut [u8; 32]);

    /// Default host-aware write. Walks `STORAGE_SLOTS` consecutive slots,
    /// encoding each via [`encode_slot`](Self::encode_slot). Per-type
    /// [`StorageEncode::write_to_storage`] impls for static types delegate
    /// here.
    #[inline]
    fn write_to_storage_static(&self, host: &Host, base_key: &[u8; 32]) {
        if Self::STORAGE_SLOTS == 1 {
            // Const-folds for every primitive → tight one-SSTORE body.
            // Skips the 32-byte unaligned `*base_key` copy that the
            // multi-slot loop forces.
            let mut buf = [0u8; 32];
            self.encode_slot(0, &mut buf);
            host.set_storage_or_clear(StorageFlags::empty(), base_key, &buf);
            return;
        }
        let mut k = *base_key;
        for i in 0..Self::STORAGE_SLOTS {
            let mut buf = [0u8; 32];
            self.encode_slot(i, &mut buf);
            host.set_storage_or_clear(StorageFlags::empty(), &k, &buf);
            inc_be_32(&mut k);
        }
    }

    /// Default host-aware clear. Zeroes `STORAGE_SLOTS` consecutive cells.
    /// Per-type [`StorageEncode::clear_storage`] impls for static types
    /// delegate here.
    #[inline]
    fn clear_storage_static(host: &Host, base_key: &[u8; 32]) {
        if Self::STORAGE_SLOTS == 1 {
            host.set_storage_or_clear(StorageFlags::empty(), base_key, &[0u8; 32]);
            return;
        }
        let mut k = *base_key;
        for _ in 0..Self::STORAGE_SLOTS {
            host.set_storage_or_clear(StorageFlags::empty(), &k, &[0u8; 32]);
            inc_be_32(&mut k);
        }
    }
}

/// Slot-buffer decoder refinement. Symmetric with [`StaticStorageEncode`].
pub trait StaticStorageDecode: StorageDecode + StaticStorageEncode {
    /// Decode from `slots`, which must have length `STORAGE_SLOTS`.
    fn from_slots(slots: &[[u8; 32]]) -> Self;

    /// All-zero slots → `None`; otherwise decode via [`from_slots`]. This is
    /// the canonical Solidity-compat presence check for static types: if
    /// every slot reads as zero, the type was never written (or was
    /// explicitly cleared to zero — solc/EVM cannot distinguish those).
    ///
    /// [`from_slots`]: Self::from_slots
    fn try_from_slots(slots: &[[u8; 32]]) -> Option<Self> {
        if slots.iter().all(|s| s == &[0u8; 32]) {
            None
        } else {
            Some(Self::from_slots(slots))
        }
    }

    /// Default host-aware read. Per-type [`StorageDecode::read_from_storage`]
    /// impls for static types delegate here.
    #[inline]
    fn read_from_storage_static(host: &Host, base_key: &[u8; 32]) -> Self {
        if Self::STORAGE_SLOTS == 1 {
            let mut buf = [0u8; 32];
            host.get_storage_or_zero(StorageFlags::empty(), base_key, &mut buf);
            return Self::from_slots(core::slice::from_ref(&buf));
        }
        // `MAX_STATIC_SLOTS` sizes the stack buffer. The `Lazy<T>` /
        // `Mapping<_, T>` entry points also assert this via `_SIZE_CHECK`, but
        // the `const { assert!(..) }` below makes the bound hold for *any*
        // `StaticStorageDecode` impl at monomorphization — including downstream
        // impls that bypass those entry points — and, unlike `debug_assert!`,
        // it is not compiled out in release (the on-chain build profile), so
        // `slots[..used]` can never index OOB.
        const {
            assert!(
                Self::STORAGE_SLOTS <= crate::MAX_STATIC_SLOTS,
                "STORAGE_SLOTS exceeds MAX_STATIC_SLOTS",
            )
        };
        let mut slots = [[0u8; 32]; crate::MAX_STATIC_SLOTS];
        let used = Self::STORAGE_SLOTS;
        let mut k = *base_key;
        for slot in slots[..used].iter_mut() {
            host.get_storage_or_zero(StorageFlags::empty(), &k, slot);
            inc_be_32(&mut k);
        }
        Self::from_slots(&slots[..used])
    }

    /// Default host-aware try-read. Returns `None` for an all-zero static
    /// value (Solidity-compat presence). Per-type
    /// [`StorageDecode::try_read_from_storage`] impls for static types
    /// delegate here.
    #[inline]
    fn try_read_from_storage_static(host: &Host, base_key: &[u8; 32]) -> Option<Self> {
        if Self::STORAGE_SLOTS == 1 {
            let mut buf = [0u8; 32];
            host.get_storage_or_zero(StorageFlags::empty(), base_key, &mut buf);
            if buf == [0u8; 32] {
                return None;
            }
            return Some(Self::from_slots(core::slice::from_ref(&buf)));
        }
        const {
            assert!(
                Self::STORAGE_SLOTS <= crate::MAX_STATIC_SLOTS,
                "STORAGE_SLOTS exceeds MAX_STATIC_SLOTS",
            )
        };
        let mut slots = [[0u8; 32]; crate::MAX_STATIC_SLOTS];
        let used = Self::STORAGE_SLOTS;
        let mut k = *base_key;
        for slot in slots[..used].iter_mut() {
            host.get_storage_or_zero(StorageFlags::empty(), &k, slot);
            inc_be_32(&mut k);
        }
        Self::try_from_slots(&slots[..used])
    }
}

pub trait StoragePackable: StaticStorageEncode + StaticStorageDecode + Sized {
    /// Pack self into `buf[offset..offset + Self::PACKED_BYTES]` WITHOUT
    /// zeroing surrounding bytes. The caller is responsible for any
    /// pre-zeroing of the target byte window (e.g. the contract-field
    /// walker zeros the window before a read-modify-write).
    fn pack_into(&self, buf: &mut [u8; 32], offset: usize);

    /// Unpack self from `buf[offset..offset + Self::PACKED_BYTES]`.
    /// Symmetric with [`pack_into`](Self::pack_into).
    fn unpack_from(buf: &[u8; 32], offset: usize) -> Self;
}

// ---------------------------------------------------------------------------
// Primitive impls
// ---------------------------------------------------------------------------

macro_rules! impl_uint {
    ($ty:ty, $bytes:literal) => {
        impl StorageEncode for $ty {
            const STORAGE_SLOTS: usize = 1;
            const PACKED_BYTES: usize = $bytes;

            #[inline]
            fn write_to_storage(&self, host: &Host, key: &[u8; 32]) {
                <Self as StaticStorageEncode>::write_to_storage_static(self, host, key)
            }
            #[inline]
            fn clear_storage(host: &Host, key: &[u8; 32]) {
                <Self as StaticStorageEncode>::clear_storage_static(host, key)
            }
            #[inline]
            fn __pack_into_dispatched(&self, buf: &mut [u8; 32], offset: usize) {
                <Self as StoragePackable>::pack_into(self, buf, offset)
            }
        }

        impl StorageDecode for $ty {
            #[inline]
            fn read_from_storage(host: &Host, key: &[u8; 32]) -> Self {
                <Self as StaticStorageDecode>::read_from_storage_static(host, key)
            }
            #[inline]
            fn try_read_from_storage(host: &Host, key: &[u8; 32]) -> Option<Self> {
                <Self as StaticStorageDecode>::try_read_from_storage_static(host, key)
            }
            #[inline]
            fn __unpack_from_dispatched(buf: &[u8; 32], offset: usize) -> Self {
                <Self as StoragePackable>::unpack_from(buf, offset)
            }
        }

        impl StaticStorageEncode for $ty {
            #[inline]
            fn encode_slot(&self, _slot_idx: usize, buf: &mut [u8; 32]) {
                debug_assert!(_slot_idx == 0);
                *buf = [0u8; 32];
                <Self as StoragePackable>::pack_into(self, buf, 32 - $bytes);
            }
        }

        impl StaticStorageDecode for $ty {
            #[inline]
            fn from_slots(slots: &[[u8; 32]]) -> Self {
                <Self as StoragePackable>::unpack_from(&slots[0], 32 - $bytes)
            }
        }

        impl StoragePackable for $ty {
            #[inline]
            fn pack_into(&self, buf: &mut [u8; 32], offset: usize) {
                buf[offset..offset + $bytes].copy_from_slice(&self.to_be_bytes());
            }

            #[inline]
            fn unpack_from(buf: &[u8; 32], offset: usize) -> Self {
                let mut bytes = [0u8; $bytes];
                bytes.copy_from_slice(&buf[offset..offset + $bytes]);
                <$ty>::from_be_bytes(bytes)
            }
        }

        #[cfg(feature = "abi-gen")]
        impl crate::StorageTypeName for $ty {
            fn name() -> alloc::string::String {
                alloc::string::String::from(<Self as crate::SolEncode>::SOL_NAME)
            }
        }
    };
}

impl_uint!(u8, 1);
impl_uint!(u16, 2);
impl_uint!(u32, 4);
impl_uint!(u64, 8);
impl_uint!(u128, 16);

impl_uint!(i8, 1);
impl_uint!(i16, 2);
impl_uint!(i32, 4);
impl_uint!(i64, 8);
impl_uint!(i128, 16);

// U256 and I256 are full-slot 32-byte types.
impl StorageEncode for U256 {
    const STORAGE_SLOTS: usize = 1;
    const PACKED_BYTES: usize = 32;

    #[inline]
    fn write_to_storage(&self, host: &Host, key: &[u8; 32]) {
        <Self as StaticStorageEncode>::write_to_storage_static(self, host, key)
    }
    #[inline]
    fn clear_storage(host: &Host, key: &[u8; 32]) {
        <Self as StaticStorageEncode>::clear_storage_static(host, key)
    }
    #[inline]
    fn __pack_into_dispatched(&self, buf: &mut [u8; 32], offset: usize) {
        <Self as StoragePackable>::pack_into(self, buf, offset)
    }
}

impl StorageDecode for U256 {
    #[inline]
    fn read_from_storage(host: &Host, key: &[u8; 32]) -> Self {
        <Self as StaticStorageDecode>::read_from_storage_static(host, key)
    }
    #[inline]
    fn try_read_from_storage(host: &Host, key: &[u8; 32]) -> Option<Self> {
        <Self as StaticStorageDecode>::try_read_from_storage_static(host, key)
    }
    #[inline]
    fn __unpack_from_dispatched(buf: &[u8; 32], offset: usize) -> Self {
        <Self as StoragePackable>::unpack_from(buf, offset)
    }
}

impl StaticStorageEncode for U256 {
    #[inline]
    fn encode_slot(&self, _slot_idx: usize, buf: &mut [u8; 32]) {
        debug_assert!(_slot_idx == 0);
        *buf = self.to_be_bytes::<32>();
    }
}

impl StaticStorageDecode for U256 {
    #[inline]
    fn from_slots(slots: &[[u8; 32]]) -> Self {
        U256::from_be_bytes(slots[0])
    }
}

impl StoragePackable for U256 {
    #[inline]
    fn pack_into(&self, buf: &mut [u8; 32], offset: usize) {
        debug_assert!(offset == 0, "U256 takes a full slot");
        *buf = self.to_be_bytes::<32>();
    }

    #[inline]
    fn unpack_from(buf: &[u8; 32], offset: usize) -> Self {
        debug_assert!(offset == 0, "U256 takes a full slot");
        U256::from_be_bytes(*buf)
    }
}

#[cfg(feature = "abi-gen")]
impl crate::StorageTypeName for U256 {
    fn name() -> alloc::string::String {
        alloc::string::String::from(<Self as crate::SolEncode>::SOL_NAME)
    }
}

impl StorageEncode for I256 {
    const STORAGE_SLOTS: usize = 1;
    const PACKED_BYTES: usize = 32;

    #[inline]
    fn write_to_storage(&self, host: &Host, key: &[u8; 32]) {
        <Self as StaticStorageEncode>::write_to_storage_static(self, host, key)
    }
    #[inline]
    fn clear_storage(host: &Host, key: &[u8; 32]) {
        <Self as StaticStorageEncode>::clear_storage_static(host, key)
    }
    #[inline]
    fn __pack_into_dispatched(&self, buf: &mut [u8; 32], offset: usize) {
        <Self as StoragePackable>::pack_into(self, buf, offset)
    }
}

impl StorageDecode for I256 {
    #[inline]
    fn read_from_storage(host: &Host, key: &[u8; 32]) -> Self {
        <Self as StaticStorageDecode>::read_from_storage_static(host, key)
    }
    #[inline]
    fn try_read_from_storage(host: &Host, key: &[u8; 32]) -> Option<Self> {
        <Self as StaticStorageDecode>::try_read_from_storage_static(host, key)
    }
    #[inline]
    fn __unpack_from_dispatched(buf: &[u8; 32], offset: usize) -> Self {
        <Self as StoragePackable>::unpack_from(buf, offset)
    }
}

impl StaticStorageEncode for I256 {
    #[inline]
    fn encode_slot(&self, _slot_idx: usize, buf: &mut [u8; 32]) {
        debug_assert!(_slot_idx == 0);
        *buf = self.to_be_bytes();
    }
}

impl StaticStorageDecode for I256 {
    #[inline]
    fn from_slots(slots: &[[u8; 32]]) -> Self {
        I256::from_be_slice(&slots[0])
    }
}

impl StoragePackable for I256 {
    #[inline]
    fn pack_into(&self, buf: &mut [u8; 32], offset: usize) {
        debug_assert!(offset == 0, "I256 takes a full slot");
        *buf = self.to_be_bytes();
    }

    #[inline]
    fn unpack_from(buf: &[u8; 32], offset: usize) -> Self {
        debug_assert!(offset == 0, "I256 takes a full slot");
        I256::from_be_slice(buf)
    }
}

#[cfg(feature = "abi-gen")]
impl crate::StorageTypeName for I256 {
    fn name() -> alloc::string::String {
        alloc::string::String::from(<Self as crate::SolEncode>::SOL_NAME)
    }
}

// bool — 1 byte, right-aligned (solc convention).
impl StorageEncode for bool {
    const STORAGE_SLOTS: usize = 1;
    const PACKED_BYTES: usize = 1;

    #[inline]
    fn write_to_storage(&self, host: &Host, key: &[u8; 32]) {
        <Self as StaticStorageEncode>::write_to_storage_static(self, host, key)
    }
    #[inline]
    fn clear_storage(host: &Host, key: &[u8; 32]) {
        <Self as StaticStorageEncode>::clear_storage_static(host, key)
    }
    #[inline]
    fn __pack_into_dispatched(&self, buf: &mut [u8; 32], offset: usize) {
        <Self as StoragePackable>::pack_into(self, buf, offset)
    }
}

impl StorageDecode for bool {
    #[inline]
    fn read_from_storage(host: &Host, key: &[u8; 32]) -> Self {
        <Self as StaticStorageDecode>::read_from_storage_static(host, key)
    }
    #[inline]
    fn try_read_from_storage(host: &Host, key: &[u8; 32]) -> Option<Self> {
        <Self as StaticStorageDecode>::try_read_from_storage_static(host, key)
    }
    #[inline]
    fn __unpack_from_dispatched(buf: &[u8; 32], offset: usize) -> Self {
        <Self as StoragePackable>::unpack_from(buf, offset)
    }
}

impl StaticStorageEncode for bool {
    #[inline]
    fn encode_slot(&self, _slot_idx: usize, buf: &mut [u8; 32]) {
        debug_assert!(_slot_idx == 0);
        *buf = [0u8; 32];
        <Self as StoragePackable>::pack_into(self, buf, 31);
    }
}

impl StaticStorageDecode for bool {
    #[inline]
    fn from_slots(slots: &[[u8; 32]]) -> Self {
        <Self as StoragePackable>::unpack_from(&slots[0], 31)
    }
}

impl StoragePackable for bool {
    #[inline]
    fn pack_into(&self, buf: &mut [u8; 32], offset: usize) {
        buf[offset] = u8::from(*self);
    }

    #[inline]
    fn unpack_from(buf: &[u8; 32], offset: usize) -> Self {
        buf[offset] != 0
    }
}

#[cfg(feature = "abi-gen")]
impl crate::StorageTypeName for bool {
    fn name() -> alloc::string::String {
        alloc::string::String::from(<Self as crate::SolEncode>::SOL_NAME)
    }
}

// Address — 20 bytes, right-aligned (solc convention).
impl StorageEncode for Address {
    const STORAGE_SLOTS: usize = 1;
    const PACKED_BYTES: usize = 20;

    #[inline]
    fn write_to_storage(&self, host: &Host, key: &[u8; 32]) {
        <Self as StaticStorageEncode>::write_to_storage_static(self, host, key)
    }
    #[inline]
    fn clear_storage(host: &Host, key: &[u8; 32]) {
        <Self as StaticStorageEncode>::clear_storage_static(host, key)
    }
    #[inline]
    fn __pack_into_dispatched(&self, buf: &mut [u8; 32], offset: usize) {
        <Self as StoragePackable>::pack_into(self, buf, offset)
    }
}

impl StorageDecode for Address {
    #[inline]
    fn read_from_storage(host: &Host, key: &[u8; 32]) -> Self {
        <Self as StaticStorageDecode>::read_from_storage_static(host, key)
    }
    #[inline]
    fn try_read_from_storage(host: &Host, key: &[u8; 32]) -> Option<Self> {
        <Self as StaticStorageDecode>::try_read_from_storage_static(host, key)
    }
    #[inline]
    fn __unpack_from_dispatched(buf: &[u8; 32], offset: usize) -> Self {
        <Self as StoragePackable>::unpack_from(buf, offset)
    }
}

impl StaticStorageEncode for Address {
    #[inline]
    fn encode_slot(&self, _slot_idx: usize, buf: &mut [u8; 32]) {
        debug_assert!(_slot_idx == 0);
        *buf = [0u8; 32];
        <Self as StoragePackable>::pack_into(self, buf, 12);
    }
}

impl StaticStorageDecode for Address {
    #[inline]
    fn from_slots(slots: &[[u8; 32]]) -> Self {
        <Self as StoragePackable>::unpack_from(&slots[0], 12)
    }
}

impl StoragePackable for Address {
    #[inline]
    fn pack_into(&self, buf: &mut [u8; 32], offset: usize) {
        buf[offset..offset + 20].copy_from_slice(&self.0);
    }

    #[inline]
    fn unpack_from(buf: &[u8; 32], offset: usize) -> Self {
        let mut bytes = [0u8; 20];
        bytes.copy_from_slice(&buf[offset..offset + 20]);
        Address(bytes)
    }
}

#[cfg(feature = "abi-gen")]
impl crate::StorageTypeName for Address {
    fn name() -> alloc::string::String {
        alloc::string::String::from(<Self as crate::SolEncode>::SOL_NAME)
    }
}

// [u8; N] — Solidity `bytesN`, right-aligned in the slot.
// N is bounded at 1..=32 to match solc's `bytesN` types.
impl<const N: usize> StorageEncode for [u8; N] {
    const STORAGE_SLOTS: usize = 1;
    const PACKED_BYTES: usize = N;

    #[inline]
    fn write_to_storage(&self, host: &Host, key: &[u8; 32]) {
        <Self as StaticStorageEncode>::write_to_storage_static(self, host, key)
    }
    #[inline]
    fn clear_storage(host: &Host, key: &[u8; 32]) {
        <Self as StaticStorageEncode>::clear_storage_static(host, key)
    }
    #[inline]
    fn __pack_into_dispatched(&self, buf: &mut [u8; 32], offset: usize) {
        <Self as StoragePackable>::pack_into(self, buf, offset)
    }
}

impl<const N: usize> StorageDecode for [u8; N] {
    #[inline]
    fn read_from_storage(host: &Host, key: &[u8; 32]) -> Self {
        <Self as StaticStorageDecode>::read_from_storage_static(host, key)
    }
    #[inline]
    fn try_read_from_storage(host: &Host, key: &[u8; 32]) -> Option<Self> {
        <Self as StaticStorageDecode>::try_read_from_storage_static(host, key)
    }
    #[inline]
    fn __unpack_from_dispatched(buf: &[u8; 32], offset: usize) -> Self {
        <Self as StoragePackable>::unpack_from(buf, offset)
    }
}

impl<const N: usize> StaticStorageEncode for [u8; N] {
    #[inline]
    fn encode_slot(&self, _slot_idx: usize, buf: &mut [u8; 32]) {
        const {
            assert!(
                N >= 1 && N <= 32,
                "bytesN storage only valid for N in 1..=32"
            )
        };
        debug_assert!(_slot_idx == 0);
        *buf = [0u8; 32];
        <Self as StoragePackable>::pack_into(self, buf, 32 - N);
    }
}

impl<const N: usize> StaticStorageDecode for [u8; N] {
    #[inline]
    fn from_slots(slots: &[[u8; 32]]) -> Self {
        <Self as StoragePackable>::unpack_from(&slots[0], 32 - N)
    }
}

impl<const N: usize> StoragePackable for [u8; N] {
    // `bytesN` is **right-aligned** in solc storage (verified against
    // solc 0.8.30 bytecode for `bytes4 public a; a = 0xdeadbeef;` which
    // emits an SSTORE of `0x000000...deadbeef`). The Solidity docs phrasing
    // "stored from the start of the array" refers to in-memory ABI layout,
    // not on-chain storage. `encode_slot`/`from_slots` pack at offset `32 - N`.
    #[inline]
    fn pack_into(&self, buf: &mut [u8; 32], offset: usize) {
        const {
            assert!(
                N >= 1 && N <= 32,
                "bytesN storage only valid for N in 1..=32"
            )
        };
        buf[offset..offset + N].copy_from_slice(self);
    }

    #[inline]
    fn unpack_from(buf: &[u8; 32], offset: usize) -> Self {
        const {
            assert!(
                N >= 1 && N <= 32,
                "bytesN storage only valid for N in 1..=32"
            )
        };
        let mut out = [0u8; N];
        out.copy_from_slice(&buf[offset..offset + N]);
        out
    }
}

#[cfg(feature = "abi-gen")]
impl<const N: usize> crate::StorageTypeName for [u8; N] {
    fn name() -> alloc::string::String {
        alloc::string::String::from(<Self as crate::SolEncode>::SOL_NAME)
    }
}

#[cfg(feature = "abi-gen")]
impl<T: crate::SolArrayElement, const N: usize> crate::StorageTypeName for [T; N] {
    fn name() -> alloc::string::String {
        alloc::string::String::from(<Self as crate::SolEncode>::SOL_NAME)
    }
}

// ---------------------------------------------------------------------------
// Fixed-size arrays `[T; N]` for T != u8.
//
// solc supports `T[N]` for any static storage type T. This impl mirrors
// solc's layout for the shapes the SDK ships impls for:
//   - sub-word T (`uint16`..`uint128`, `int16`..`int128`, `bool`, `Address`,
//     `[u8; M]` for M < 32): density elements per slot
//     (`density = 32 / PACKED_BYTES`), right-aligned within each slot;
//     total slots = ceil(N / density).
//   - single-slot full-word T (`U256`, `I256`, `[u8; 32]`): one element per
//     slot; total slots = N.
//   - multi-slot static T (e.g. `[U256; M]` if added via marker, derived
//     structs spanning >1 slot): each element strides by `T::STORAGE_SLOTS`;
//     total slots = N * STORAGE_SLOTS.
//
// `[u8; N]` keeps its dedicated `bytesN` impl above (the marker excludes
// `u8`). Tuples are not in the default `StorageArrayElement` list — `[(A,
// B); N]` won't compile out of the box. Downstream code that wants
// `[MyTuple; N]` or `[MyStruct; N]` must `impl StorageArrayElement` for the
// element type manually.
//
// Dynamic-body T (`String`, `Bytes`) is not supported in fixed arrays —
// solc's storage layout for those involves per-element headers and is left
// as a follow-up. Even if a downstream crate implements `StorageArrayElement`
// for a dynamic-body T, `[T; N]` will be rejected at compile time by the
// `!T::HAS_DYNAMIC_BODY` const-assert in `StorageEncode for [T; N]`.
// ---------------------------------------------------------------------------

/// Marker trait gating which element types can appear in `[T; N]` storage.
///
/// Implemented in-tree for every primitive scalar except `u8` (`[u8; N]`
/// keeps its dedicated `bytesN` impl). Downstream code can implement this
/// for custom **static** `SolType`-derived structs (or tuples) to opt them
/// into `[MyStruct; N]` support.
///
/// # Static elements only
///
/// The supertrait bound is [`StaticStorageEncode`] + [`StaticStorageDecode`],
/// so dynamic-body types (`String`, `Bytes`, or any `SolType` struct with
/// `HAS_DYNAMIC_BODY = true`) cannot implement it at all — they don't provide
/// the `encode_slot` / `from_slots` slot-buffer codec the `[T; N]` impl
/// dispatches through. The `!T::HAS_DYNAMIC_BODY` const-assert in
/// `StorageEncode for [T; N]` is a redundant belt-and-braces guard for the
/// same case.
pub trait StorageArrayElement: StaticStorageEncode + StaticStorageDecode {}

macro_rules! impl_storage_array_element {
    ($($T:ty),+ $(,)?) => {
        $(impl StorageArrayElement for $T {})+
    };
}

impl_storage_array_element!(
    u16, u32, u64, u128, U256, i8, i16, i32, i64, i128, I256, bool, Address,
);

impl<T: StorageArrayElement, const N: usize> StorageEncode for [T; N] {
    /// Sub-word: ceil(N / density). Single-slot full-word: N. Multi-slot
    /// static: N * STORAGE_SLOTS.
    ///
    /// The leading `assert!(!T::HAS_DYNAMIC_BODY, ...)` is a compile-time
    /// guard: if a downstream impl opts a dynamic-body type (e.g. `String`,
    /// `Bytes`) into [`StorageArrayElement`], the const-eval of
    /// `STORAGE_SLOTS` (forced as soon as the array is used in a `Lazy`,
    /// `Mapping`, or `StorageVec`) fails with a clear message. (The
    /// supertrait bound already makes such an impl impossible; this keeps a
    /// readable error if that bound ever loosens.)
    const STORAGE_SLOTS: usize = {
        assert!(
            !T::HAS_DYNAMIC_BODY,
            "[T; N]: dynamic-body T (String, Bytes, or any SolType with \
             HAS_DYNAMIC_BODY = true) is not supported in fixed-size arrays. \
             solc's layout for arrays of dynamic-body elements requires \
             per-element header+body routing that this impl does not provide."
        );
        if T::PACKED_BYTES < 32 {
            let density = 32 / T::PACKED_BYTES;
            N.div_ceil(density)
        } else {
            N * T::STORAGE_SLOTS
        }
    };

    /// Fixed arrays always start a fresh slot (`PACKED_BYTES = 32`), matching
    /// solc's layout for `T[N]` fields.
    const PACKED_BYTES: usize = 32;

    #[inline]
    fn write_to_storage(&self, host: &Host, base_key: &[u8; 32]) {
        <Self as StaticStorageEncode>::write_to_storage_static(self, host, base_key)
    }

    #[inline]
    fn clear_storage(host: &Host, base_key: &[u8; 32]) {
        <Self as StaticStorageEncode>::clear_storage_static(host, base_key)
    }
}

impl<T: StorageArrayElement, const N: usize> StaticStorageEncode for [T; N] {
    fn encode_slot(&self, slot_idx: usize, buf: &mut [u8; 32]) {
        *buf = [0u8; 32];
        if T::PACKED_BYTES < 32 {
            // Sub-word: pack `density` elements right-aligned within this slot.
            let density = 32 / T::PACKED_BYTES;
            let start = slot_idx * density;
            let end = ((slot_idx + 1) * density).min(N);
            let mut tmp = [0u8; 32];
            let elem_start = 32 - T::PACKED_BYTES;
            for (within, elem) in self[start..end].iter().enumerate() {
                let offset = 32 - T::PACKED_BYTES * (within + 1);
                tmp.fill(0);
                elem.encode_slot(0, &mut tmp);
                buf[offset..offset + T::PACKED_BYTES]
                    .copy_from_slice(&tmp[elem_start..elem_start + T::PACKED_BYTES]);
            }
        } else if T::STORAGE_SLOTS == 1 {
            // One element per slot.
            self[slot_idx].encode_slot(0, buf);
        } else {
            // Multi-slot static: stride `T::STORAGE_SLOTS` per element.
            let elem_idx = slot_idx / T::STORAGE_SLOTS;
            let within_elem = slot_idx % T::STORAGE_SLOTS;
            self[elem_idx].encode_slot(within_elem, buf);
        }
    }
}

impl<T: StorageArrayElement, const N: usize> StorageDecode for [T; N] {
    #[inline]
    fn read_from_storage(host: &Host, base_key: &[u8; 32]) -> Self {
        <Self as StaticStorageDecode>::read_from_storage_static(host, base_key)
    }

    #[inline]
    fn try_read_from_storage(host: &Host, base_key: &[u8; 32]) -> Option<Self> {
        <Self as StaticStorageDecode>::try_read_from_storage_static(host, base_key)
    }
}

impl<T: StorageArrayElement, const N: usize> StaticStorageDecode for [T; N] {
    fn from_slots(slots: &[[u8; 32]]) -> Self {
        core::array::from_fn(|i| {
            if T::PACKED_BYTES < 32 {
                let density = 32 / T::PACKED_BYTES;
                let slot_idx = i / density;
                let within = i % density;
                let offset = 32 - T::PACKED_BYTES * (within + 1);
                let mut tmp = [0u8; 32];
                let elem_start = 32 - T::PACKED_BYTES;
                tmp[elem_start..elem_start + T::PACKED_BYTES]
                    .copy_from_slice(&slots[slot_idx][offset..offset + T::PACKED_BYTES]);
                T::from_slots(&[tmp])
            } else if T::STORAGE_SLOTS == 1 {
                T::from_slots(&slots[i..i + 1])
            } else {
                let start = i * T::STORAGE_SLOTS;
                T::from_slots(&slots[start..start + T::STORAGE_SLOTS])
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Tuple impls — same packing rules as structs.
//
// Implemented for arities 1..=8 over `StoragePackable` element types. Each
// element occupies its `PACKED_BYTES` at the right-aligned position within
// its assigned slot; multiple small elements share a slot when they fit.
// Composite elements (nested structs, dynamic-bodied types like `String` /
// `Bytes`) are not supported as tuple elements — `StoragePackable` is the
// binding constraint.
// ---------------------------------------------------------------------------

macro_rules! impl_storage_tuple {
    ($( ($($T:ident : $idx:tt),+) ),+ $(,)?) => {
        $(
            impl<$($T: StoragePackable),+> StorageEncode for ($($T,)+) {
                /// Compile-time-evaluated layout walker. Shares the one
                /// `layout_step` const-fn with `#[derive(SolStorage)]` and the
                /// `#[contract]` / `#[storage]` macros, so tuple layout can't
                /// drift from struct layout.
                const STORAGE_SLOTS: usize = {
                    let mut step = crate::LayoutStep::FIRST;
                    $(
                        step = crate::layout_step_encode::<$T>(step);
                    )+
                    step.next_slot as usize + 1
                };

                const PACKED_BYTES: usize = 32;

                #[inline]
                fn write_to_storage(&self, host: &Host, key: &[u8; 32]) {
                    <Self as StaticStorageEncode>::write_to_storage_static(self, host, key)
                }
                #[inline]
                fn clear_storage(host: &Host, key: &[u8; 32]) {
                    <Self as StaticStorageEncode>::clear_storage_static(host, key)
                }
            }

            impl<$($T: StoragePackable),+> StorageDecode for ($($T,)+) {
                #[inline]
                fn read_from_storage(host: &Host, key: &[u8; 32]) -> Self {
                    // N=8 upper bound: tuples have arity <= 8, so STORAGE_SLOTS <= 8.
                    <Self as StaticStorageDecode>::read_from_storage_static(host, key)
                }
                #[inline]
                fn try_read_from_storage(host: &Host, key: &[u8; 32]) -> Option<Self> {
                    <Self as StaticStorageDecode>::try_read_from_storage_static(host, key)
                }
            }

            impl<$($T: StoragePackable),+> StaticStorageEncode for ($($T,)+) {
                fn encode_slot(&self, slot_idx: usize, buf: &mut [u8; 32]) {
                    *buf = [0u8; 32];
                    let mut step = crate::LayoutStep::FIRST;
                    $(
                        step = crate::layout_step_encode::<$T>(step);
                        if step.slot as usize == slot_idx {
                            <$T as StoragePackable>::pack_into(
                                &self.$idx, buf, step.offset as usize,
                            );
                        }
                    )+
                }
            }

            impl<$($T: StoragePackable),+> StaticStorageDecode for ($($T,)+) {
                fn from_slots(slots: &[[u8; 32]]) -> Self {
                    let mut step = crate::LayoutStep::FIRST;
                    (
                        $(
                            {
                                step = crate::layout_step_encode::<$T>(step);
                                <$T as StoragePackable>::unpack_from(
                                    &slots[step.slot as usize], step.offset as usize,
                                )
                            },
                        )+
                    )
                }
            }

            // Tuples have no Rust struct name; the ABI tuple notation
            // (e.g. `"(uint256,address)"`) is the natural representation in
            // storage layout JSON, so forward to `SolEncode::SOL_NAME`.
            #[cfg(feature = "abi-gen")]
            impl<$($T: StoragePackable + crate::SolEncode),+> crate::StorageTypeName for ($($T,)+) {
                fn name() -> alloc::string::String {
                    alloc::string::String::from(<Self as crate::SolEncode>::SOL_NAME)
                }
            }
        )+
    };
}

impl_storage_tuple! {
    (A: 0),
    (A: 0, B: 1),
    (A: 0, B: 1, C: 2),
    (A: 0, B: 1, C: 2, D: 3),
    (A: 0, B: 1, C: 2, D: 3, E: 4),
    (A: 0, B: 1, C: 2, D: 3, E: 4, F: 5),
    (A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6),
    (A: 0, B: 1, C: 2, D: 3, E: 4, F: 5, G: 6, H: 7),
}

// ---------------------------------------------------------------------------
// Dynamic-body machinery (solc `string` / `bytes` layout): header in the
// field's own slot, body chunks at `keccak256(slot) + i`. Used by the
// `StorageEncode` / `StorageDecode` impls for `String` below and `Bytes`
// (in `alloc_types.rs`), which both call into the `pub(crate)` helpers.
// ---------------------------------------------------------------------------

/// Sentinel byte stored at `slot[30]` when a dynamic value is set to empty
/// (`""` / `vec![]`). Keeps the slot from being auto-deleted by
/// `set_storage_or_clear` so `try_get` can distinguish "explicitly empty"
/// from "never written". The decoder ignores `slot[..31]` when len == 0, so
/// the sentinel byte is invisible at read time.
#[cfg(feature = "alloc")]
pub(crate) const EMPTY_INLINE_SENTINEL: u8 = 0x01;

#[cfg(feature = "alloc")]
enum DynHeader {
    Inline { len: usize },
    Spilled { len: usize },
}

#[cfg(feature = "alloc")]
fn decode_dyn_header(slot_bytes: &[u8; 32]) -> DynHeader {
    if slot_bytes[31] & 1 == 0 {
        // Short form: byte 31 encodes `len * 2`, len in [0, 31]. Cap to 31
        // so a malformed read can't trigger a slice panic downstream.
        DynHeader::Inline {
            len: ((slot_bytes[31] >> 1) as usize).min(31),
        }
    } else {
        // Spilled: whole slot encodes `len * 2 + 1` as big-endian u256.
        // A dynamic value has no fixed upper bound — like solc (and Stylus) it
        // stripes across as many 32-byte slots as `len` needs, so the header
        // length is authoritative. Storage is per-contract isolated (the header
        // is always self-written) and the read loop is gas-bounded on-chain, so
        // there's nothing to clamp. We only reject lengths that can't be
        // legitimate — non-zero high 16 bytes (>= 2^128) or one that doesn't fit
        // `usize` — as corruption / a non-Solidity writer, treating them as empty.
        let high_zero = slot_bytes[..16].iter().all(|&b| b == 0);
        if !high_zero {
            return DynHeader::Spilled { len: 0 };
        }
        let mut len_be = [0u8; 16];
        len_be.copy_from_slice(&slot_bytes[16..32]);
        let raw_len = u128::from_be_bytes(len_be) >> 1;
        if raw_len > usize::MAX as u128 {
            return DynHeader::Spilled { len: 0 };
        }
        DynHeader::Spilled {
            len: raw_len as usize,
        }
    }
}

#[cfg(feature = "alloc")]
pub(crate) fn encode_long_header(len: usize) -> [u8; 32] {
    let raw: u128 = (len as u128) * 2 + 1;
    let mut out = [0u8; 32];
    out[16..32].copy_from_slice(&raw.to_be_bytes());
    out
}

#[cfg(feature = "alloc")]
fn dyn_body_root(host: &Host, slot: &[u8; 32]) -> [u8; 32] {
    let mut output = [0u8; 32];
    host.hash_keccak_256(slot, &mut output);
    output
}

#[cfg(feature = "alloc")]
fn read_dyn_body(host: &Host, slot: &[u8; 32], len: usize) -> alloc::vec::Vec<u8> {
    // Grow incrementally: capacity tracks bytes actually read from storage,
    // never the self-reported header length, so a corrupt header can't drive a
    // huge up-front allocation. The read loop itself is gas-bounded on-chain.
    let mut out = alloc::vec::Vec::new();
    let mut body_slot = dyn_body_root(host, slot);
    let mut remaining = len;
    while remaining > 0 {
        let mut chunk = [0u8; 32];
        host.get_storage_or_zero(StorageFlags::empty(), &body_slot, &mut chunk);
        let take = if remaining >= 32 { 32 } else { remaining };
        out.extend_from_slice(&chunk[..take]);
        remaining -= take;
        inc_be_32(&mut body_slot);
    }
    out
}

#[cfg(feature = "alloc")]
fn clear_dyn_body_range(host: &Host, slot: &[u8; 32], start_chunk: usize, count: usize) {
    if count == 0 {
        return;
    }
    let mut body_slot = dyn_body_root(host, slot);
    for _ in 0..start_chunk {
        inc_be_32(&mut body_slot);
    }
    for _ in 0..count {
        host.set_storage_or_clear(StorageFlags::empty(), &body_slot, &[0u8; 32]);
        inc_be_32(&mut body_slot);
    }
}

/// Write `data` at the dynamic field located at `slot` (header + spilled
/// body chunks). Clears stale body chunks left over from a longer previous
/// value so storage doesn't leak.
#[cfg(feature = "alloc")]
pub(crate) fn write_dynamic_bytes(host: &Host, slot: &[u8; 32], data: &[u8]) {
    let new_len = data.len();
    let new_chunks = if new_len < 32 {
        0
    } else {
        new_len.div_ceil(32)
    };

    // Inspect old layout to free body chunks the new value no longer needs.
    let mut old_slot_bytes = [0u8; 32];
    host.get_storage_or_zero(StorageFlags::empty(), slot, &mut old_slot_bytes);
    if let DynHeader::Spilled { len: old_len } = decode_dyn_header(&old_slot_bytes) {
        let old_chunks = old_len.div_ceil(32);
        if old_chunks > new_chunks {
            clear_dyn_body_range(host, slot, new_chunks, old_chunks - new_chunks);
        }
    }

    if new_len < 32 {
        let mut packed = [0u8; 32];
        packed[..new_len].copy_from_slice(data);
        packed[31] = (new_len as u8) << 1;
        if new_len == 0 {
            packed[30] = EMPTY_INLINE_SENTINEL;
        }
        host.set_storage_or_clear(StorageFlags::empty(), slot, &packed);
        return;
    }

    let header = encode_long_header(new_len);
    host.set_storage_or_clear(StorageFlags::empty(), slot, &header);

    let mut body_slot = dyn_body_root(host, slot);
    let mut offset = 0usize;
    while offset < new_len {
        let mut chunk = [0u8; 32];
        let remaining = new_len - offset;
        let take = if remaining >= 32 { 32 } else { remaining };
        chunk[..take].copy_from_slice(&data[offset..offset + take]);
        host.set_storage_or_clear(StorageFlags::empty(), &body_slot, &chunk);
        offset += take;
        inc_be_32(&mut body_slot);
    }
}

/// Read the dynamic value stored at `slot`, materialising any spilled body.
#[cfg(feature = "alloc")]
pub(crate) fn read_dynamic_bytes(host: &Host, slot: &[u8; 32]) -> alloc::vec::Vec<u8> {
    let mut slot_bytes = [0u8; 32];
    host.get_storage_or_zero(StorageFlags::empty(), slot, &mut slot_bytes);
    match decode_dyn_header(&slot_bytes) {
        DynHeader::Inline { len } => alloc::vec::Vec::from(&slot_bytes[..len]),
        DynHeader::Spilled { len } => read_dyn_body(host, slot, len),
    }
}

/// Clear the dynamic value at `slot` (header + any spilled body chunks).
#[cfg(feature = "alloc")]
pub(crate) fn clear_dynamic_bytes(host: &Host, slot: &[u8; 32]) {
    let mut slot_bytes = [0u8; 32];
    host.get_storage_or_zero(StorageFlags::empty(), slot, &mut slot_bytes);
    if let DynHeader::Spilled { len } = decode_dyn_header(&slot_bytes) {
        let chunks = len.div_ceil(32);
        clear_dyn_body_range(host, slot, 0, chunks);
    }
    host.set_storage_or_clear(StorageFlags::empty(), slot, &[0u8; 32]);
}

// ---------------------------------------------------------------------------
// String / Bytes: native dynamic storage impls.
//
// `Vec<u8>` intentionally has no storage impl: its `SolEncode::SOL_NAME` is
// `"uint8[]"` (Solidity dynamic array of bytes, padded per-element), which
// disagrees with the short/long inline layout `solc` uses for storage `bytes`.
// `Lazy<Vec<u8>>` and `Mapping<K, Vec<u8>>` therefore fail to compile —
// contracts must use [`Bytes`](crate::Bytes) (`SolEncode::SOL_NAME = "bytes"`),
// whose `StorageEncode` impl lives in `alloc_types.rs` and uses the same
// dynamic-body machinery as `String` below.
//
// Solidity-compatible `string` / `bytes` storage layout:
//   - Short (`len < 32`):  inline data + `byte 31 = len * 2`; empty values
//     carry the [`EMPTY_INLINE_SENTINEL`] at byte 30 so `try_get` can tell
//     them apart from a never-written slot.
//   - Long (`len >= 32`):  header at the slot encodes `len * 2 + 1` as a
//     big-endian u256; body chunks live at `keccak256(slot) + i`.
// ---------------------------------------------------------------------------

#[cfg(feature = "alloc")]
impl StorageEncode for alloc::string::String {
    const STORAGE_SLOTS: usize = 1;
    const PACKED_BYTES: usize = 32;
    const HAS_DYNAMIC_BODY: bool = true;

    fn write_to_storage(&self, host: &Host, base_key: &[u8; 32]) {
        write_dynamic_bytes(host, base_key, self.as_bytes());
    }

    fn clear_storage(host: &Host, base_key: &[u8; 32]) {
        clear_dynamic_bytes(host, base_key);
    }
}

#[cfg(feature = "alloc")]
impl StorageDecode for alloc::string::String {
    fn read_from_storage(host: &Host, base_key: &[u8; 32]) -> Self {
        let bytes = read_dynamic_bytes(host, base_key);
        // Lossy UTF-8 decode: invalid sequences become U+FFFD. Matches
        // Stylus's `StorageString::get_string`. Trapping on invalid bytes
        // would be a DoS vector when storage is shared with a Solidity
        // contract that doesn't validate. For byte-exact roundtrips use
        // `Lazy<Bytes>` / `Mapping<K, Bytes>` instead.
        alloc::string::String::from_utf8_lossy(&bytes).into_owned()
    }

    fn try_read_from_storage(host: &Host, base_key: &[u8; 32]) -> Option<Self> {
        // Header-only peek: if the slot is all-zero, nothing was ever written.
        // Solidity-compat — empty inline values write the
        // [`EMPTY_INLINE_SENTINEL`] at byte 30, so they're distinguishable
        // from a never-written slot.
        let mut header = [0u8; 32];
        host.get_storage_or_zero(StorageFlags::empty(), base_key, &mut header);
        if header == [0u8; 32] {
            return None;
        }
        // Header is non-zero → some value was written; load body.
        Some(Self::read_from_storage(host, base_key))
    }
}

#[cfg(feature = "abi-gen")]
impl crate::StorageTypeName for alloc::string::String {
    fn name() -> alloc::string::String {
        alloc::string::String::from(<Self as crate::SolEncode>::SOL_NAME)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn slot() -> [u8; 32] {
        [0u8; 32]
    }

    // --- primitives in a freshly-zeroed slot via encode_slot ---

    #[test]
    fn u32_encode_slot_right_aligned() {
        let v: u32 = 0x01020304;
        let mut buf = [0xffu8; 32]; // non-zero starting bytes to prove encode_slot zeros
        v.encode_slot(0, &mut buf);
        // Right-aligned: high 28 bytes zero, low 4 bytes hold value (big-endian).
        let mut expected = [0u8; 32];
        expected[28..32].copy_from_slice(&v.to_be_bytes());
        assert_eq!(buf, expected);
    }

    #[test]
    fn u32_round_trip() {
        let v: u32 = 0xdeadbeef;
        let mut buf = slot();
        v.encode_slot(0, &mut buf);
        let decoded = u32::from_slots(core::slice::from_ref(&buf));
        assert_eq!(decoded, v);
    }

    #[test]
    fn i32_negative_round_trip() {
        let v: i32 = -42;
        let mut buf = slot();
        v.encode_slot(0, &mut buf);
        let decoded = i32::from_slots(core::slice::from_ref(&buf));
        assert_eq!(decoded, v);
    }

    #[test]
    fn u256_round_trip() {
        let v = U256::from_limbs([1, 2, 3, 4]);
        let mut buf = slot();
        v.encode_slot(0, &mut buf);
        let decoded = U256::from_slots(core::slice::from_ref(&buf));
        assert_eq!(decoded, v);
    }

    #[test]
    fn address_round_trip() {
        let v = Address([
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
            0x0f, 0x10, 0x11, 0x12, 0x13, 0x14,
        ]);
        let mut buf = slot();
        v.encode_slot(0, &mut buf);
        // solc: address at bytes 12..32.
        assert_eq!(&buf[..12], &[0u8; 12]);
        assert_eq!(&buf[12..32], &v.0);
        let decoded = Address::from_slots(core::slice::from_ref(&buf));
        assert_eq!(decoded, v);
    }

    #[test]
    fn bool_round_trip() {
        let mut buf = slot();
        true.encode_slot(0, &mut buf);
        assert_eq!(buf[31], 1);
        assert!(buf[..31].iter().all(|&b| b == 0));
        assert!(bool::from_slots(core::slice::from_ref(&buf)));

        let mut buf = slot();
        false.encode_slot(0, &mut buf);
        assert_eq!(buf, [0u8; 32]);
        assert!(!bool::from_slots(core::slice::from_ref(&buf)));
    }

    #[test]
    fn bytes20_round_trip_right_aligned() {
        let v: [u8; 20] = [0xaa; 20];
        let mut buf = slot();
        v.encode_slot(0, &mut buf);
        // solc bytes20: right-aligned (verified vs. solc bytecode), data
        // lives at bytes 12..32 of the slot.
        assert!(buf[..12].iter().all(|&b| b == 0));
        assert_eq!(&buf[12..32], &v);
        let decoded = <[u8; 20]>::from_slots(core::slice::from_ref(&buf));
        assert_eq!(decoded, v);
    }

    #[test]
    fn bytes32_round_trip_full_slot() {
        let mut v = [0u8; 32];
        for (i, b) in v.iter_mut().enumerate() {
            *b = i as u8;
        }
        let mut buf = slot();
        v.encode_slot(0, &mut buf);
        assert_eq!(buf, v);
        let decoded = <[u8; 32]>::from_slots(core::slice::from_ref(&buf));
        assert_eq!(decoded, v);
    }

    // --- packed (sub-word) round-trips ---

    #[test]
    fn pack_two_u128_into_one_slot() {
        // solc layout: struct { uint128 a; uint128 b; }
        //   slot[0..16] = b, slot[16..32] = a   (first field at low-order end)
        let a: u128 = 0x0102030405060708090a0b0c0d0e0f10;
        let b: u128 = 0x1112131415161718191a1b1c1d1e1f20;
        let mut buf = slot();
        a.pack_into(&mut buf, 16); // a right-aligned in low half
        b.pack_into(&mut buf, 0); // b in high half
        assert_eq!(&buf[16..32], &a.to_be_bytes());
        assert_eq!(&buf[0..16], &b.to_be_bytes());

        // Round-trip
        assert_eq!(u128::unpack_from(&buf, 16), a);
        assert_eq!(u128::unpack_from(&buf, 0), b);
    }

    #[test]
    fn pack_address_u32_bool_into_one_slot() {
        // solc layout for { bool x; uint32 y; address z; }:
        //   x at byte 31, y at bytes 27..31, z at bytes 7..27.
        let x = true;
        let y: u32 = 0xabcdef01;
        let z = Address([0x42; 20]);
        let mut buf = slot();
        x.pack_into(&mut buf, 31);
        y.pack_into(&mut buf, 27);
        z.pack_into(&mut buf, 7);

        assert_eq!(buf[31], 1);
        assert_eq!(&buf[27..31], &y.to_be_bytes());
        assert_eq!(&buf[7..27], &z.0);
        assert!(buf[..7].iter().all(|&b| b == 0));

        assert!(bool::unpack_from(&buf, 31));
        assert_eq!(u32::unpack_from(&buf, 27), y);
        assert_eq!(Address::unpack_from(&buf, 7), z);
    }

    #[test]
    fn pack_does_not_disturb_surrounding_bytes() {
        let mut buf = [0xa5u8; 32];
        let v: u32 = 0x11223344;
        v.pack_into(&mut buf, 10);
        // Bytes [10..14] hold v, the rest stays 0xa5.
        assert_eq!(&buf[10..14], &v.to_be_bytes());
        assert!(buf[..10].iter().all(|&b| b == 0xa5));
        assert!(buf[14..].iter().all(|&b| b == 0xa5));
    }

    // --- consts ---

    // --- dynamic-body roundtrip + header corruption handling ---

    #[cfg(feature = "std")]
    fn mock_host() -> Host {
        Host::from_dyn(alloc::rc::Rc::new(crate::MockHostBuilder::new().build()))
    }

    #[cfg(feature = "alloc")]
    #[test]
    fn decode_dyn_header_rejects_corrupt_high_bytes() {
        // A spilled header with non-zero high 16 bytes (>= 2^128) can only be
        // corruption / a non-Solidity writer — decode to empty rather than a
        // bogus huge length.
        let header = [0xffu8; 32]; // byte 31 has bit 0 set => spilled form
        match decode_dyn_header(&header) {
            DynHeader::Spilled { len } => assert_eq!(len, 0),
            DynHeader::Inline { .. } => panic!("expected spilled header"),
        }
    }

    #[cfg(feature = "std")]
    #[test]
    fn dynamic_bytes_long_roundtrip() {
        // A legitimate long value (>= 32 bytes) roundtrips byte-for-byte
        // through the spilled path.
        let host = mock_host();
        let slot = [9u8; 32];
        let data: alloc::vec::Vec<u8> = (0..200u32).map(|i| i as u8).collect();
        write_dynamic_bytes(&host, &slot, &data);
        assert_eq!(read_dynamic_bytes(&host, &slot), data);
    }

    #[cfg(feature = "std")]
    #[test]
    fn dynamic_bytes_above_old_cap_roundtrips() {
        // Regression guard: a value well past the old 416-byte clamp must
        // roundtrip. Dynamic values have no fixed cap — like solc/Stylus they
        // stripe across as many 32-byte slots as the length needs.
        let host = mock_host();
        let slot = [11u8; 32];
        let data: alloc::vec::Vec<u8> = (0..1000u32).map(|i| i as u8).collect();
        write_dynamic_bytes(&host, &slot, &data);
        assert_eq!(read_dynamic_bytes(&host, &slot), data);
    }

    #[test]
    fn const_invariants() {
        assert_eq!(<u32 as StorageEncode>::STORAGE_SLOTS, 1);
        assert_eq!(<u32 as StorageEncode>::PACKED_BYTES, 4);
        assert_eq!(<Address as StorageEncode>::PACKED_BYTES, 20);
        assert_eq!(<U256 as StorageEncode>::PACKED_BYTES, 32);
        assert_eq!(<[u8; 20] as StorageEncode>::PACKED_BYTES, 20);
    }
}

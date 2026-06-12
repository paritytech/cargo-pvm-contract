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
//!   `#[derive(SolType)]` macro's struct field encoders.
//!
//! `StorageEncode` / `StorageDecode` also carry hidden polymorphic dispatch
//! hooks (`__pack_into_dispatched` / `__unpack_from_dispatched`) used by
//! `Lazy<T>`'s packed-path. Those hooks panic by default; in-tree packable
//! impls override them to delegate to `StoragePackable`. Downstream code
//! should impl `StoragePackable` (which compile-checks the operations) and
//! mirror the delegation pattern in its `StorageEncode` / `StorageDecode`
//! impls so `Lazy<T>` works.
//!
//! Composite types (structs, fixed arrays of compound elements) and dynamic
//! types (`String`, `Bytes`) always start a new slot and never pack — they
//! implement only [`StorageEncode`] / [`StorageDecode`]. Dynamic types
//! additionally set [`HAS_DYNAMIC_BODY`](StorageEncode::HAS_DYNAMIC_BODY) so
//! `Mapping` / `Lazy` route reads and writes through the host-aware
//! [`write_to_storage`](StorageEncode::write_to_storage) /
//! [`read_from_storage`](StorageDecode::read_from_storage) path.

use crate::{Address, Host, HostApi, I256, StorageFlags, U256};

/// Increment a 32-byte big-endian integer in-place. Used to walk consecutive
/// storage slots for multi-slot values.
#[inline]
fn inc_be_32(slot: &mut [u8; 32]) {
    for byte in slot.iter_mut().rev() {
        let (next, carry) = byte.overflowing_add(1);
        *byte = next;
        if !carry {
            return;
        }
    }
}

/// Top-level storage encoder.
///
/// A type implementing this trait can be the value of a `Mapping<K, V>` or
/// `Lazy<T>`. The total number of slots is fixed at compile time
/// ([`STORAGE_SLOTS`](Self::STORAGE_SLOTS)).
///
/// For primitives, [`STORAGE_SLOTS`](Self::STORAGE_SLOTS) is 1 and
/// [`encode_slot`](Self::encode_slot) writes the value at the type's canonical
/// position within a freshly-zeroed slot (right-aligned for both integers and
/// `bytesN`).
///
/// For structs, [`encode_slot`](Self::encode_slot) walks the per-slot field
/// placements computed by `#[derive(SolType)]`.
pub trait StorageEncode {
    /// Total number of slots this type occupies when stored at the top of a
    /// layout. Always >= 1.
    const STORAGE_SLOTS: usize;

    /// Number of bytes this type consumes within a single slot when packed
    /// alongside sibling fields. Must satisfy `1 <= PACKED_BYTES <= 32`.
    /// Full-slot types use `32`; composites that always claim a fresh slot
    /// also use `32`.
    ///
    /// **Types with `PACKED_BYTES < 32` must also implement
    /// [`StoragePackable`].** `Lazy<T>::set` / `Lazy<T>::get` take a
    /// read-modify-write path for sub-word values and dispatch through
    /// [`__pack_into_dispatched`](Self::__pack_into_dispatched) /
    /// [`StorageDecode::__unpack_from_dispatched`], whose default impls
    /// panic at runtime; the `StoragePackable` impl is what supplies a
    /// non-panicking override.
    const PACKED_BYTES: usize;

    /// `true` iff this type stores data outside its `STORAGE_SLOTS` (e.g.
    /// `String` / `Bytes` spill their body to `keccak256(slot)+i`). Containers
    /// like `Mapping<K, V>` and `Lazy<T>` use this to route reads/writes
    /// through the host-aware [`write_to_storage`](Self::write_to_storage) /
    /// [`StorageDecode::read_from_storage`] path instead of the
    /// single-slot fast path.
    ///
    /// Default `false` so primitives and pure-static structs aren't affected.
    const HAS_DYNAMIC_BODY: bool = false;

    /// Encode slot `slot_idx` of this value into `buf`. Caller passes a
    /// freshly-zeroed (or to-be-zeroed) `buf`; for top-level primitive types
    /// the implementation overwrites the entire slot, for composite types it
    /// fills the bytes that belong to slot `slot_idx`.
    ///
    /// `slot_idx` must satisfy `slot_idx < STORAGE_SLOTS`.
    ///
    /// Required for every impl. Static types implement this with their actual
    /// slot codec; dynamic-body types ([`HAS_DYNAMIC_BODY`] = `true`) provide
    /// a panicking `unreachable!()` body and override [`write_to_storage`]
    /// instead — callers must dispatch through `write_to_storage` for those
    /// types. Keeping the method required (rather than defaulted) is a
    /// compile-time guarantee that no static impl forgets to provide a real
    /// slot codec.
    ///
    /// [`HAS_DYNAMIC_BODY`]: Self::HAS_DYNAMIC_BODY
    /// [`write_to_storage`]: Self::write_to_storage
    fn encode_slot(&self, slot_idx: usize, buf: &mut [u8; 32]);

    /// Internal polymorphic dispatch hook for `Lazy<T>`'s packed-path
    /// (`Lazy<T>::set` when `PACKED_BYTES < 32`). The canonical
    /// `pack_into` operation lives on [`StoragePackable`]; this hook lets
    /// `Lazy<T>`, whose `T` is only bound by `StorageEncode + StorageDecode`,
    /// reach a packable impl through a const-folded `T::PACKED_BYTES < 32`
    /// branch. Default impl panics — packable types override to delegate
    /// to `<Self as StoragePackable>::pack_into`. Full-slot types never
    /// reach this branch.
    #[doc(hidden)]
    fn __pack_into_dispatched(&self, _buf: &mut [u8; 32], _offset: usize) {
        panic!(
            "__pack_into_dispatched not implemented for full-slot type; Lazy<T>::set should never reach this branch",
        );
    }

    /// Write self to storage starting at `base_key`. The default impl writes
    /// `STORAGE_SLOTS` consecutive 32-byte slots via [`encode_slot`]; types
    /// with [`HAS_DYNAMIC_BODY`] = `true` override this to also write body
    /// chunks at `keccak256(base_key) + i`.
    ///
    /// [`encode_slot`]: Self::encode_slot
    /// [`HAS_DYNAMIC_BODY`]: Self::HAS_DYNAMIC_BODY
    fn write_to_storage(&self, host: &Host, base_key: &[u8; 32]) {
        let mut k = *base_key;
        for i in 0..Self::STORAGE_SLOTS {
            let mut buf = [0u8; 32];
            self.encode_slot(i, &mut buf);
            host.set_storage_or_clear(StorageFlags::empty(), &k, &buf);
            inc_be_32(&mut k);
        }
    }

    /// Clear every slot this type occupies at `base_key`. Default impl writes
    /// `STORAGE_SLOTS` zero-slots (`set_storage_or_clear` auto-deletes).
    /// Dynamic types override to also clear body chunks.
    fn clear_storage(host: &Host, base_key: &[u8; 32], slots: usize) {
        let mut k = *base_key;
        for _ in 0..slots {
            host.set_storage_or_clear(StorageFlags::empty(), &k, &[0u8; 32]);
            inc_be_32(&mut k);
        }
    }
}

/// Top-level storage decoder.
///
/// Symmetric with [`StorageEncode`]: given exactly [`STORAGE_SLOTS`] consecutive
/// 32-byte slots in `slots`, reconstruct the value.
///
/// [`STORAGE_SLOTS`]: StorageEncode::STORAGE_SLOTS
pub trait StorageDecode: StorageEncode + Sized {
    /// Decode from `slots`, which must have length `STORAGE_SLOTS`.
    ///
    /// The name `from_slots` (rather than `decode`) avoids ambiguity with
    /// [`SolDecode::decode`](crate::SolDecode::decode); the two codecs are
    /// distinct and a type implementing both must dispatch through trait
    /// qualification at the call site.
    ///
    /// Required for every impl. Static types implement this with their actual
    /// slot codec; dynamic-body types ([`StorageEncode::HAS_DYNAMIC_BODY`] =
    /// `true`) cannot fully reconstruct from a slot buffer alone — the body
    /// lives outside the passed slots, so they provide a panicking
    /// `unreachable!()` body and override
    /// [`read_from_storage`](Self::read_from_storage) instead. Callers must
    /// dispatch through `read_from_storage` for those types. Keeping the
    /// method required is a compile-time guarantee that no static impl
    /// forgets to provide a real slot codec.
    fn from_slots(slots: &[[u8; 32]]) -> Self;

    /// Internal polymorphic dispatch hook for `Lazy<T>`'s packed-path
    /// (`Lazy<T>::get` when `PACKED_BYTES < 32`). Symmetric with
    /// [`StorageEncode::__pack_into_dispatched`]; the canonical
    /// `unpack_from` lives on [`StoragePackable`]. Default impl panics —
    /// packable types override to delegate.
    #[doc(hidden)]
    fn __unpack_from_dispatched(_buf: &[u8; 32], _offset: usize) -> Self {
        panic!(
            "__unpack_from_dispatched not implemented for full-slot type; Lazy<T>::get should never reach this branch",
        );
    }

    /// Read self from storage starting at `base_key`. Default impl reads
    /// `STORAGE_SLOTS` consecutive slots and decodes via [`from_slots`].
    /// Types with [`StorageEncode::HAS_DYNAMIC_BODY`] = `true` override
    /// this to also read body chunks at `keccak256(base_key) + i`.
    ///
    /// `MAX_INLINE_SLOTS` caps the stack buffer used by the default impl.
    /// Callers (like `Mapping::get`) sized this for the typical record
    /// shape; types occupying more slots must override.
    ///
    /// [`from_slots`]: Self::from_slots
    fn read_from_storage<const MAX_INLINE_SLOTS: usize>(host: &Host, base_key: &[u8; 32]) -> Self {
        debug_assert!(
            Self::STORAGE_SLOTS <= MAX_INLINE_SLOTS,
            "STORAGE_SLOTS exceeds MAX_INLINE_SLOTS",
        );
        let mut slots = [[0u8; 32]; MAX_INLINE_SLOTS];
        let n = Self::STORAGE_SLOTS;
        let mut k = *base_key;
        for slot in slots[..n].iter_mut() {
            host.get_storage_or_zero(StorageFlags::empty(), &k, slot);
            inc_be_32(&mut k);
        }
        Self::from_slots(&slots[..n])
    }
}

/// Sub-word packable primitive.
///
/// Implemented by types that fit in a single 32-byte slot and can share that
/// slot with sibling fields at arbitrary byte offsets. Owns the canonical
/// [`pack_into`](Self::pack_into) / [`unpack_from`](Self::unpack_from)
/// operations as required methods — the bound `T: StoragePackable` enforces
/// at compile time that callers can actually pack/unpack the type. Used by
/// tuple `StorageEncode` impls and the `#[derive(SolType)]` macro's struct
/// field encoders.
///
/// Composite types do not implement this trait — they always start a new
/// slot and never pack.
///
/// Implementing this trait for a downstream type also requires implementing
/// `StorageEncode` and `StorageDecode` (the supertraits) and overriding
/// their `__pack_into_dispatched` / `__unpack_from_dispatched` hooks to
/// delegate to the `StoragePackable` methods. This delegation is what lets
/// `Lazy<T>` reach the packed-path code through its polymorphic
/// `T: StorageEncode + StorageDecode` bound.
pub trait StoragePackable: StorageEncode + StorageDecode + Sized {
    /// Byte offset within a slot where this type lives when it occupies a slot
    /// on its own (solc's right-aligned rule for both integers and `bytesN`).
    /// Always equals `32 - PACKED_BYTES` in practice; kept as an explicit
    /// const for documentation and call-site clarity.
    const CANONICAL_OFFSET: usize;

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
            fn encode_slot(&self, _slot_idx: usize, buf: &mut [u8; 32]) {
                debug_assert!(_slot_idx == 0);
                *buf = [0u8; 32];
                <Self as StoragePackable>::pack_into(self, buf, 32 - $bytes);
            }

            #[inline]
            fn __pack_into_dispatched(&self, buf: &mut [u8; 32], offset: usize) {
                <Self as StoragePackable>::pack_into(self, buf, offset)
            }
        }

        impl StorageDecode for $ty {
            #[inline]
            fn from_slots(slots: &[[u8; 32]]) -> Self {
                <Self as StoragePackable>::unpack_from(&slots[0], 32 - $bytes)
            }

            #[inline]
            fn __unpack_from_dispatched(buf: &[u8; 32], offset: usize) -> Self {
                <Self as StoragePackable>::unpack_from(buf, offset)
            }
        }

        impl StoragePackable for $ty {
            const CANONICAL_OFFSET: usize = 32 - $bytes;

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
    fn encode_slot(&self, _slot_idx: usize, buf: &mut [u8; 32]) {
        debug_assert!(_slot_idx == 0);
        *buf = self.to_be_bytes::<32>();
    }

    #[inline]
    fn __pack_into_dispatched(&self, buf: &mut [u8; 32], offset: usize) {
        <Self as StoragePackable>::pack_into(self, buf, offset)
    }
}

impl StorageDecode for U256 {
    #[inline]
    fn from_slots(slots: &[[u8; 32]]) -> Self {
        U256::from_be_bytes(slots[0])
    }

    #[inline]
    fn __unpack_from_dispatched(buf: &[u8; 32], offset: usize) -> Self {
        <Self as StoragePackable>::unpack_from(buf, offset)
    }
}

impl StoragePackable for U256 {
    const CANONICAL_OFFSET: usize = 0;

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

impl StorageEncode for I256 {
    const STORAGE_SLOTS: usize = 1;
    const PACKED_BYTES: usize = 32;

    #[inline]
    fn encode_slot(&self, _slot_idx: usize, buf: &mut [u8; 32]) {
        debug_assert!(_slot_idx == 0);
        *buf = self.to_be_bytes();
    }

    #[inline]
    fn __pack_into_dispatched(&self, buf: &mut [u8; 32], offset: usize) {
        <Self as StoragePackable>::pack_into(self, buf, offset)
    }
}

impl StorageDecode for I256 {
    #[inline]
    fn from_slots(slots: &[[u8; 32]]) -> Self {
        I256::from_be_slice(&slots[0])
    }

    #[inline]
    fn __unpack_from_dispatched(buf: &[u8; 32], offset: usize) -> Self {
        <Self as StoragePackable>::unpack_from(buf, offset)
    }
}

impl StoragePackable for I256 {
    const CANONICAL_OFFSET: usize = 0;

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

// bool — 1 byte, right-aligned (solc convention).
impl StorageEncode for bool {
    const STORAGE_SLOTS: usize = 1;
    const PACKED_BYTES: usize = 1;

    #[inline]
    fn encode_slot(&self, _slot_idx: usize, buf: &mut [u8; 32]) {
        debug_assert!(_slot_idx == 0);
        *buf = [0u8; 32];
        <Self as StoragePackable>::pack_into(self, buf, 31);
    }

    #[inline]
    fn __pack_into_dispatched(&self, buf: &mut [u8; 32], offset: usize) {
        <Self as StoragePackable>::pack_into(self, buf, offset)
    }
}

impl StorageDecode for bool {
    #[inline]
    fn from_slots(slots: &[[u8; 32]]) -> Self {
        <Self as StoragePackable>::unpack_from(&slots[0], 31)
    }

    #[inline]
    fn __unpack_from_dispatched(buf: &[u8; 32], offset: usize) -> Self {
        <Self as StoragePackable>::unpack_from(buf, offset)
    }
}

impl StoragePackable for bool {
    const CANONICAL_OFFSET: usize = 31;

    #[inline]
    fn pack_into(&self, buf: &mut [u8; 32], offset: usize) {
        buf[offset] = u8::from(*self);
    }

    #[inline]
    fn unpack_from(buf: &[u8; 32], offset: usize) -> Self {
        buf[offset] != 0
    }
}

// Address — 20 bytes, right-aligned (solc convention).
impl StorageEncode for Address {
    const STORAGE_SLOTS: usize = 1;
    const PACKED_BYTES: usize = 20;

    #[inline]
    fn encode_slot(&self, _slot_idx: usize, buf: &mut [u8; 32]) {
        debug_assert!(_slot_idx == 0);
        *buf = [0u8; 32];
        <Self as StoragePackable>::pack_into(self, buf, 12);
    }

    #[inline]
    fn __pack_into_dispatched(&self, buf: &mut [u8; 32], offset: usize) {
        <Self as StoragePackable>::pack_into(self, buf, offset)
    }
}

impl StorageDecode for Address {
    #[inline]
    fn from_slots(slots: &[[u8; 32]]) -> Self {
        <Self as StoragePackable>::unpack_from(&slots[0], 12)
    }

    #[inline]
    fn __unpack_from_dispatched(buf: &[u8; 32], offset: usize) -> Self {
        <Self as StoragePackable>::unpack_from(buf, offset)
    }
}

impl StoragePackable for Address {
    const CANONICAL_OFFSET: usize = 12;

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

// [u8; N] — Solidity `bytesN`, right-aligned in the slot (see the
// `StoragePackable` impl below for the verification against solc).
//
// Note: N is bounded at 1..=32 to match solc's `bytesN` types. A const assert
// in each method enforces the bound at monomorphisation.
impl<const N: usize> StorageEncode for [u8; N] {
    const STORAGE_SLOTS: usize = 1;
    const PACKED_BYTES: usize = N;

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

    #[inline]
    fn __pack_into_dispatched(&self, buf: &mut [u8; 32], offset: usize) {
        <Self as StoragePackable>::pack_into(self, buf, offset)
    }
}

impl<const N: usize> StorageDecode for [u8; N] {
    #[inline]
    fn from_slots(slots: &[[u8; 32]]) -> Self {
        <Self as StoragePackable>::unpack_from(&slots[0], 32 - N)
    }

    #[inline]
    fn __unpack_from_dispatched(buf: &[u8; 32], offset: usize) -> Self {
        <Self as StoragePackable>::unpack_from(buf, offset)
    }
}

impl<const N: usize> StoragePackable for [u8; N] {
    /// `bytesN` is **right-aligned** in solc storage (verified against
    /// solc 0.8.30 bytecode for `bytes4 public a; a = 0xdeadbeef;` which
    /// emits an SSTORE of `0x000000...deadbeef`). The Solidity docs phrasing
    /// "stored from the start of the array" refers to in-memory ABI layout,
    /// not on-chain storage.
    const CANONICAL_OFFSET: usize = 32 - N;

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
/// # Do not implement for dynamic-body types
///
/// `String`, `Bytes`, and any `SolType` struct with `HAS_DYNAMIC_BODY =
/// true` route their encode/decode through `write_to_storage` /
/// `read_from_storage`; their `encode_slot` / `from_slots` are
/// `unreachable!()` stubs. The generic `[T; N]` impl dispatches through
/// `encode_slot` / `from_slots`, so a dynamic-body T will be rejected at
/// compile time by the `[T; N]` const-assert. Stick to static element types.
pub trait StorageArrayElement: StorageEncode + StorageDecode {}

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
    /// `Mapping`, or `StorageVec`) fails with a clear message — preventing
    /// the runtime `unreachable!()` panic from `encode_slot` / `from_slots`
    /// that would otherwise occur.
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
                /// Compile-time-evaluated layout walker. Mirrors the
                /// algorithm `#[derive(SolType)]` emits for static structs.
                const STORAGE_SLOTS: usize = {
                    let mut slot: usize = 0;
                    let mut space: usize = 32;
                    let mut placed: usize = 0;
                    $(
                        {
                            let bytes = <$T as StorageEncode>::PACKED_BYTES;
                            if space < bytes {
                                if placed != 0 { slot += 1; }
                                space = 32;
                            }
                            space -= bytes;
                            placed += 1;
                        }
                    )+
                    let _ = (space, placed);
                    slot + 1
                };

                const PACKED_BYTES: usize = 32;

                fn encode_slot(&self, slot_idx: usize, buf: &mut [u8; 32]) {
                    *buf = [0u8; 32];
                    let mut slot: usize = 0;
                    let mut space: usize = 32;
                    let mut placed: usize = 0;
                    $(
                        let bytes = <$T as StorageEncode>::PACKED_BYTES;
                        if space < bytes {
                            if placed != 0 { slot += 1; }
                            space = 32;
                        }
                        space -= bytes;
                        if slot == slot_idx {
                            <$T as StoragePackable>::pack_into(&self.$idx, buf, space);
                        }
                        placed += 1;
                    )+
                    let _ = (slot, space, placed);
                }
            }

            impl<$($T: StoragePackable),+> StorageDecode for ($($T,)+) {
                fn from_slots(slots: &[[u8; 32]]) -> Self {
                    let mut slot: usize = 0;
                    let mut space: usize = 32;
                    let mut placed: usize = 0;
                    let result = (
                        $(
                            {
                                let bytes = <$T as StorageEncode>::PACKED_BYTES;
                                if space < bytes {
                                    if placed != 0 { slot += 1; }
                                    space = 32;
                                }
                                space -= bytes;
                                let v = <$T as StoragePackable>::unpack_from(
                                    &slots[slot], space,
                                );
                                placed += 1;
                                v
                            },
                        )+
                    );
                    let _ = (slot, space, placed);
                    result
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
        // Real-world lengths fit in usize; non-zero high bytes indicate
        // corruption / non-Solidity writer — treat as empty.
        let high_zero = slot_bytes[..16].iter().all(|&b| b == 0);
        let mut len_be = [0u8; 16];
        len_be.copy_from_slice(&slot_bytes[16..32]);
        let raw = u128::from_be_bytes(len_be);
        let raw_len = raw >> 1;
        if !high_zero || raw_len > usize::MAX as u128 {
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
    let mut out = alloc::vec::Vec::with_capacity(len);
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

    // Dynamic-body type: the live path goes through `write_to_storage` below,
    // which writes header + body in one operation. `encode_slot` exists only
    // to satisfy the trait's required-method contract — making the method
    // required (rather than defaulted) is what compile-checks that every
    // static impl provides a real slot codec.
    fn encode_slot(&self, _slot_idx: usize, _buf: &mut [u8; 32]) {
        unreachable!("String::encode_slot: dispatch through write_to_storage")
    }

    fn write_to_storage(&self, host: &Host, base_key: &[u8; 32]) {
        write_dynamic_bytes(host, base_key, self.as_bytes());
    }

    fn clear_storage(host: &Host, base_key: &[u8; 32], _slots: usize) {
        clear_dynamic_bytes(host, base_key);
    }
}

#[cfg(feature = "alloc")]
impl StorageDecode for alloc::string::String {
    // Dynamic-body type: see `encode_slot` above for the rationale. Reads
    // dispatch through `read_from_storage`.
    fn from_slots(_slots: &[[u8; 32]]) -> Self {
        unreachable!("String::from_slots: dispatch through read_from_storage")
    }

    fn read_from_storage<const MAX_INLINE_SLOTS: usize>(host: &Host, base_key: &[u8; 32]) -> Self {
        let bytes = read_dynamic_bytes(host, base_key);
        // Rust's `String` invariant requires valid UTF-8. We use lossy decoding
        // to keep `get()` infallible: invalid sequences are replaced with
        // U+FFFD instead of trapping.
        //
        // Also lossy, also infallible — use `Bytes` for byte-exact reads.
        // This diverges from a Solidity contract reading the same slot,
        // which sees the raw bytes verbatim because solc never decodes
        // `string` (it's just `bytes` with a UTF-8 hint). Trapping on
        // invalid bytes (ink!'s choice via SCALE decode) would be a DoS
        // vector when storage is shared with a Solidity contract that
        // doesn't validate.
        //
        // Contracts needing byte-exact roundtrips (e.g. computing a keccak256
        // that matches what an off-chain client hashed) must use
        // `Lazy<Bytes>` / `Mapping<K, Bytes>` instead — `Bytes` round-trips
        // every byte verbatim, no substitution.
        alloc::string::String::from_utf8_lossy(&bytes).into_owned()
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

    #[test]
    fn const_invariants() {
        assert_eq!(<u32 as StorageEncode>::STORAGE_SLOTS, 1);
        assert_eq!(<u32 as StorageEncode>::PACKED_BYTES, 4);
        assert_eq!(<u32 as StoragePackable>::CANONICAL_OFFSET, 28);

        assert_eq!(<Address as StorageEncode>::PACKED_BYTES, 20);
        assert_eq!(<Address as StoragePackable>::CANONICAL_OFFSET, 12);

        assert_eq!(<U256 as StorageEncode>::PACKED_BYTES, 32);
        assert_eq!(<U256 as StoragePackable>::CANONICAL_OFFSET, 0);

        assert_eq!(<[u8; 20] as StorageEncode>::PACKED_BYTES, 20);
        assert_eq!(<[u8; 20] as StoragePackable>::CANONICAL_OFFSET, 12);
    }
}

//! Solidity-compatible storage codec.
//!
//! Encodes Rust values into the byte layout that `solc` uses for contract
//! storage — sub-word packing for primitives that fit, multi-slot spread for
//! larger composites, big-endian right-aligned for integers, left-aligned for
//! `bytesN`. The on-chain bytes produced by [`StorageEncode`] / [`StorageDecode`]
//! match what an equivalent solc-compiled contract would write, so tools like
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
//! - [`StoragePackable`] is implemented by primitives that can be packed at a
//!   sub-word byte offset within a slot. The `#[derive(SolType)]` macro calls
//!   `pack_into` / `unpack_from` when emitting struct field encoders.
//!
//! Composite types (structs, fixed arrays of compound elements, dynamic
//! markers like `LazySlot<T>`) always start a new slot and never pack — they
//! implement only [`StorageEncode`] / [`StorageDecode`].

use crate::{Address, I256, U256};

/// Top-level storage encoder.
///
/// A type implementing this trait can be the value of a `Mapping<K, V>` or
/// `Lazy<T>`. The total number of slots is fixed at compile time
/// ([`STORAGE_SLOTS`](Self::STORAGE_SLOTS)).
///
/// For primitives, [`STORAGE_SLOTS`](Self::STORAGE_SLOTS) is 1 and
/// [`encode_slot`](Self::encode_slot) writes the value at the type's canonical
/// position within a freshly-zeroed slot (right-aligned for integers,
/// left-aligned for `bytesN`).
///
/// For structs, [`STARTS_NEW_SLOT`](Self::STARTS_NEW_SLOT) is `true` and
/// [`encode_slot`](Self::encode_slot) walks the per-slot field placements
/// computed by `#[derive(SolType)]`.
pub trait StorageEncode {
    /// Total number of slots this type occupies when stored at the top of a
    /// layout. Always >= 1.
    const STORAGE_SLOTS: usize;

    /// Number of bytes this type consumes within a single slot when packed
    /// alongside sibling fields. Must satisfy `1 <= PACKED_BYTES <= 32`.
    ///
    /// Only meaningful when [`STARTS_NEW_SLOT`](Self::STARTS_NEW_SLOT) is
    /// `false`; composite types ignore it (they always take whole slots).
    const PACKED_BYTES: usize;

    /// `true` iff this type forces the layout walker to advance to a new slot
    /// regardless of remaining space. Composite types (structs, arrays of
    /// compound elements, dynamic markers) set this to `true`; primitives set
    /// it to `false` so they can pack with neighbours.
    const STARTS_NEW_SLOT: bool;

    /// Encode slot `slot_idx` of this value into `buf`. Caller passes a
    /// freshly-zeroed (or to-be-zeroed) `buf`; for top-level primitive types
    /// the implementation overwrites the entire slot, for composite types it
    /// fills the bytes that belong to slot `slot_idx`.
    ///
    /// `slot_idx` must satisfy `slot_idx < STORAGE_SLOTS`.
    fn encode_slot(&self, slot_idx: usize, buf: &mut [u8; 32]);
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
    fn from_slots(slots: &[[u8; 32]]) -> Self;
}

/// Sub-word packable primitive.
///
/// Implemented by types that fit in a single 32-byte slot and can share that
/// slot with sibling fields at arbitrary byte offsets. The `#[derive(SolType)]`
/// macro emits calls to [`pack_into`](Self::pack_into) /
/// [`unpack_from`](Self::unpack_from) when laying out struct fields.
///
/// Composite types do not implement this trait — they always start a new
/// slot and never pack.
pub trait StoragePackable: StorageEncode + Sized {
    /// Byte offset within a slot where this type lives when it occupies a slot
    /// on its own (solc's "right-aligned for integers, left-aligned for
    /// `bytesN`" rule).
    const CANONICAL_OFFSET: usize;

    /// Write self into `buf[offset..offset + PACKED_BYTES]`. Does **not** zero
    /// surrounding bytes; the caller is responsible for slot initialisation
    /// when packing multiple fields into the same slot.
    ///
    /// `offset + PACKED_BYTES` must be `<= 32`.
    fn pack_into(&self, buf: &mut [u8; 32], offset: usize);

    /// Read self from `buf[offset..offset + PACKED_BYTES]`.
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
            const STARTS_NEW_SLOT: bool = false;

            #[inline]
            fn encode_slot(&self, _slot_idx: usize, buf: &mut [u8; 32]) {
                debug_assert_eq!(_slot_idx, 0);
                *buf = [0u8; 32];
                self.pack_into(buf, Self::CANONICAL_OFFSET);
            }
        }

        impl StorageDecode for $ty {
            #[inline]
            fn from_slots(slots: &[[u8; 32]]) -> Self {
                Self::unpack_from(&slots[0], <Self as StoragePackable>::CANONICAL_OFFSET)
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
    const STARTS_NEW_SLOT: bool = false;

    #[inline]
    fn encode_slot(&self, _slot_idx: usize, buf: &mut [u8; 32]) {
        debug_assert_eq!(_slot_idx, 0);
        *buf = self.to_be_bytes::<32>();
    }
}

impl StorageDecode for U256 {
    #[inline]
    fn from_slots(slots: &[[u8; 32]]) -> Self {
        U256::from_be_bytes(slots[0])
    }
}

impl StoragePackable for U256 {
    const CANONICAL_OFFSET: usize = 0;

    #[inline]
    fn pack_into(&self, buf: &mut [u8; 32], offset: usize) {
        debug_assert_eq!(offset, 0, "U256 takes a full slot");
        *buf = self.to_be_bytes::<32>();
    }

    #[inline]
    fn unpack_from(buf: &[u8; 32], offset: usize) -> Self {
        debug_assert_eq!(offset, 0, "U256 takes a full slot");
        U256::from_be_bytes(*buf)
    }
}

impl StorageEncode for I256 {
    const STORAGE_SLOTS: usize = 1;
    const PACKED_BYTES: usize = 32;
    const STARTS_NEW_SLOT: bool = false;

    #[inline]
    fn encode_slot(&self, _slot_idx: usize, buf: &mut [u8; 32]) {
        debug_assert_eq!(_slot_idx, 0);
        *buf = self.to_be_bytes();
    }
}

impl StorageDecode for I256 {
    #[inline]
    fn from_slots(slots: &[[u8; 32]]) -> Self {
        I256::from_be_slice(&slots[0])
    }
}

impl StoragePackable for I256 {
    const CANONICAL_OFFSET: usize = 0;

    #[inline]
    fn pack_into(&self, buf: &mut [u8; 32], offset: usize) {
        debug_assert_eq!(offset, 0, "I256 takes a full slot");
        *buf = self.to_be_bytes();
    }

    #[inline]
    fn unpack_from(buf: &[u8; 32], offset: usize) -> Self {
        debug_assert_eq!(offset, 0, "I256 takes a full slot");
        I256::from_be_slice(buf)
    }
}

// bool — 1 byte, right-aligned (solc convention).
impl StorageEncode for bool {
    const STORAGE_SLOTS: usize = 1;
    const PACKED_BYTES: usize = 1;
    const STARTS_NEW_SLOT: bool = false;

    #[inline]
    fn encode_slot(&self, _slot_idx: usize, buf: &mut [u8; 32]) {
        debug_assert_eq!(_slot_idx, 0);
        *buf = [0u8; 32];
        self.pack_into(buf, Self::CANONICAL_OFFSET);
    }
}

impl StorageDecode for bool {
    #[inline]
    fn from_slots(slots: &[[u8; 32]]) -> Self {
        Self::unpack_from(&slots[0], <Self as StoragePackable>::CANONICAL_OFFSET)
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
    const STARTS_NEW_SLOT: bool = false;

    #[inline]
    fn encode_slot(&self, _slot_idx: usize, buf: &mut [u8; 32]) {
        debug_assert_eq!(_slot_idx, 0);
        *buf = [0u8; 32];
        self.pack_into(buf, Self::CANONICAL_OFFSET);
    }
}

impl StorageDecode for Address {
    #[inline]
    fn from_slots(slots: &[[u8; 32]]) -> Self {
        Self::unpack_from(&slots[0], <Self as StoragePackable>::CANONICAL_OFFSET)
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

// [u8; N] — Solidity `bytesN`, left-aligned in the slot.
//
// Note: N is bounded at 1..=32 to match solc's `bytesN` types. A const assert
// in each method enforces the bound at monomorphisation.
impl<const N: usize> StorageEncode for [u8; N] {
    const STORAGE_SLOTS: usize = 1;
    const PACKED_BYTES: usize = N;
    const STARTS_NEW_SLOT: bool = false;

    #[inline]
    fn encode_slot(&self, _slot_idx: usize, buf: &mut [u8; 32]) {
        const { assert!(N >= 1 && N <= 32, "bytesN storage only valid for N in 1..=32") };
        debug_assert_eq!(_slot_idx, 0);
        *buf = [0u8; 32];
        self.pack_into(buf, <Self as StoragePackable>::CANONICAL_OFFSET);
    }
}

impl<const N: usize> StorageDecode for [u8; N] {
    #[inline]
    fn from_slots(slots: &[[u8; 32]]) -> Self {
        Self::unpack_from(&slots[0], <Self as StoragePackable>::CANONICAL_OFFSET)
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
        const { assert!(N >= 1 && N <= 32, "bytesN storage only valid for N in 1..=32") };
        buf[offset..offset + N].copy_from_slice(self);
    }

    #[inline]
    fn unpack_from(buf: &[u8; 32], offset: usize) -> Self {
        const { assert!(N >= 1 && N <= 32, "bytesN storage only valid for N in 1..=32") };
        let mut out = [0u8; N];
        out.copy_from_slice(&buf[offset..offset + N]);
        out
    }
}

// ---------------------------------------------------------------------------
// Tuple impls — same packing rules as structs.
//
// Implemented for arities 1..=8 over `StoragePackable` element types. Each
// element occupies its `PACKED_BYTES` at the right-aligned position within
// its assigned slot; multiple small elements share a slot when they fit.
// Composite elements (nested structs, `LazySlot<T>`) are not supported as
// tuple elements — `StoragePackable` is the binding constraint.
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
                const STARTS_NEW_SLOT: bool = true;

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
        assert!(!<u32 as StorageEncode>::STARTS_NEW_SLOT);
        assert_eq!(<u32 as StoragePackable>::CANONICAL_OFFSET, 28);

        assert_eq!(<Address as StorageEncode>::PACKED_BYTES, 20);
        assert_eq!(<Address as StoragePackable>::CANONICAL_OFFSET, 12);

        assert_eq!(<U256 as StorageEncode>::PACKED_BYTES, 32);
        assert_eq!(<U256 as StoragePackable>::CANONICAL_OFFSET, 0);

        assert_eq!(<[u8; 20] as StorageEncode>::PACKED_BYTES, 20);
        assert_eq!(<[u8; 20] as StoragePackable>::CANONICAL_OFFSET, 12);
    }
}

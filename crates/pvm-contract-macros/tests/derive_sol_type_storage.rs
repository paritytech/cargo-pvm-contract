//! Tests for the `StorageEncode` / `StorageDecode` impls emitted by
//! `#[derive(SolType)]` for static structs.
//!
//! These verify the solc-compatible storage layout: sub-word packing rules
//! for primitives, right-alignment for integers and Address, left-alignment
//! for bytesN, and consecutive slots for composite (struct-in-struct) fields.

extern crate alloc;

use pvm_contract_sdk::SolType;
use pvm_contract_sdk::{
    Address, Bytes, Lazy, Mapping, StorageDecode, StorageEncode, StorageKey, StoragePackable, U256,
};
use pvm_contract_types::MockHostBuilder;

fn fresh_host() -> pvm_contract_sdk::Host {
    pvm_contract_sdk::Host::from_dyn(alloc::rc::Rc::new(MockHostBuilder::new().build()))
}

// Helper to encode all slots of a value via the streaming encoder.
fn encode_all<T: StorageEncode>(value: &T) -> alloc::vec::Vec<[u8; 32]> {
    let mut slots = alloc::vec::Vec::with_capacity(T::STORAGE_SLOTS);
    for i in 0..T::STORAGE_SLOTS {
        let mut buf = [0u8; 32];
        value.encode_slot(i, &mut buf);
        slots.push(buf);
    }
    slots
}

// ========================================================================
// One-slot packed: (address, uint32) — solc packs into a single 32-byte slot.
// ========================================================================

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
struct AddrAndCounter {
    addr: Address,
    counter: u32,
}

#[test]
fn addr_and_counter_packs_into_one_slot() {
    assert_eq!(<AddrAndCounter as StorageEncode>::STORAGE_SLOTS, 1);
    assert_eq!(<AddrAndCounter as StorageEncode>::PACKED_BYTES, 32);
}

#[test]
fn addr_and_counter_slot_bytes_match_solc_layout() {
    // solc layout for `struct { address addr; uint32 counter; }`:
    //   field 0 (addr) at low-order end:    bytes 12..32 = addr
    //   field 1 (counter) above it:         bytes 8..12  = counter (BE)
    let v = AddrAndCounter {
        addr: Address([
            0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xaa, 0xbb, 0xcc, 0xdd, 0xee,
            0xff, 0x00, 0x12, 0x34, 0x56, 0x78,
        ]),
        counter: 0xdeadbeef,
    };
    let slots = encode_all(&v);
    assert_eq!(slots.len(), 1);

    let s = slots[0];
    assert_eq!(&s[12..32], &v.addr.0, "address at bytes 12..32");
    assert_eq!(
        &s[8..12],
        &v.counter.to_be_bytes(),
        "counter at bytes 8..12"
    );
    assert!(s[..8].iter().all(|&b| b == 0), "high bytes zero");
}

#[test]
fn addr_and_counter_round_trip() {
    let v = AddrAndCounter {
        addr: Address([0xab; 20]),
        counter: 12345,
    };
    let slots = encode_all(&v);
    let decoded = AddrAndCounter::from_slots(&slots);
    assert_eq!(decoded, v);
}

// ========================================================================
// Two-slot packed: a packed slot 0 + a full U256 slot 1.
// ========================================================================

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
struct UserInfo {
    active: bool,
    joined_at: u32,
    addr: Address,
    balance: U256,
}

#[test]
fn user_info_takes_two_slots() {
    assert_eq!(<UserInfo as StorageEncode>::STORAGE_SLOTS, 2);
}

#[test]
fn user_info_layout_matches_solc() {
    // solc packing for { bool active; uint32 joined_at; address addr; uint256 balance; }
    //   slot 0: active at 31, joined_at at 27..31, addr at 7..27
    //   slot 1: balance (full slot)
    let v = UserInfo {
        active: true,
        joined_at: 0x01020304,
        addr: Address([0x42; 20]),
        balance: U256::from(0xfeedu32),
    };
    let slots = encode_all(&v);
    assert_eq!(slots.len(), 2);

    let s0 = slots[0];
    assert_eq!(s0[31], 1, "bool at byte 31 = 0x01");
    assert_eq!(&s0[27..31], &v.joined_at.to_be_bytes(), "uint32 at 27..31");
    assert_eq!(&s0[7..27], &v.addr.0, "address at 7..27");
    assert!(s0[..7].iter().all(|&b| b == 0), "padding zero");

    let s1 = slots[1];
    assert_eq!(s1, v.balance.to_be_bytes::<32>(), "balance fills slot 1");
}

#[test]
fn user_info_round_trip() {
    let v = UserInfo {
        active: false,
        joined_at: 999,
        addr: Address([0xaa; 20]),
        balance: U256::from_limbs([1, 2, 3, 4]),
    };
    let slots = encode_all(&v);
    let decoded = UserInfo::from_slots(&slots);
    assert_eq!(decoded, v);
}

// Nested struct fields are deferred to a future phase — see the
// `classify_storage_field` rationale in `pvm-contract-macros`. For now,
// `Inner` / `Outer` examples are out of scope.

// ========================================================================
// bytesN — right-aligned in solc storage (verified vs. solc 0.8.30 bytecode).
// ========================================================================

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
struct WithBytes {
    tag: [u8; 4],
    payload: U256,
}

#[test]
fn bytes4_right_aligned_in_slot() {
    let v = WithBytes {
        tag: [0xde, 0xad, 0xbe, 0xef],
        payload: U256::from(42u32),
    };
    let slots = encode_all(&v);
    assert_eq!(slots.len(), 2);

    // bytes4 at the LSB end of its packed window: bytes 28..32 of slot 0.
    // (Solc emits `SSTORE 0x000000...deadbeef` for top-level `bytes4 a;`.)
    assert!(slots[0][..28].iter().all(|&b| b == 0), "high bytes zero");
    assert_eq!(&slots[0][28..32], &v.tag);

    assert_eq!(slots[1], U256::from(42u32).to_be_bytes::<32>());

    let decoded = WithBytes::from_slots(&slots);
    assert_eq!(decoded, v);
}

// ========================================================================
// Single-field struct — same slot count as the field's type.
// ========================================================================

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
struct OneField {
    x: u32,
}

#[test]
fn single_field_struct_one_slot() {
    assert_eq!(<OneField as StorageEncode>::STORAGE_SLOTS, 1);
    let v = OneField { x: 0xabcdef };
    let slots = encode_all(&v);
    assert_eq!(&slots[0][28..32], &v.x.to_be_bytes());
    assert_eq!(OneField::from_slots(&slots), v);
}

// ========================================================================
// Spill across a slot boundary: small + small + big that doesn't fit.
// ========================================================================

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
struct Spill {
    a: u128,
    b: u128,
    c: u128, // doesn't fit with a+b in one slot, so c spills to slot 1
}

#[test]
fn spill_layout() {
    // slot 0: a in low half (16..32), b in high half (0..16). Full.
    // slot 1: c in low half (16..32).
    assert_eq!(<Spill as StorageEncode>::STORAGE_SLOTS, 2);

    let v = Spill { a: 1, b: 2, c: 3 };
    let slots = encode_all(&v);
    assert_eq!(slots.len(), 2);

    assert_eq!(&slots[0][16..32], &v.a.to_be_bytes());
    assert_eq!(&slots[0][..16], &v.b.to_be_bytes());

    assert_eq!(&slots[1][16..32], &v.c.to_be_bytes());
    assert!(slots[1][..16].iter().all(|&b| b == 0));

    assert_eq!(Spill::from_slots(&slots), v);
}

// ========================================================================
// Ensure the per-field packing helpers (StoragePackable) are emitted for
// primitives via the macro and accessible from user code.
// ========================================================================

#[test]
fn primitives_implement_storage_packable() {
    fn assert_packable<T: StoragePackable>() {}
    assert_packable::<u8>();
    assert_packable::<u16>();
    assert_packable::<u32>();
    assert_packable::<u64>();
    assert_packable::<u128>();
    assert_packable::<bool>();
    assert_packable::<Address>();
    assert_packable::<U256>();
    assert_packable::<[u8; 20]>();
}

// ========================================================================
// End-to-end through `Lazy<T>` / `Mapping<K, V>`: a `#[derive(SolType)]`
// struct must round-trip through the typed-storage helpers for every shape —
// single-slot packed, multi-slot static, and dynamic-bodied.
// ========================================================================

// --- Two `u64`s pack into a single slot (sub-word static path) -------------

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
struct RunningAverage {
    sum: u64,
    total: u64,
}

#[test]
fn packed_struct_single_slot_via_mapping_round_trip() {
    assert_eq!(<RunningAverage as StorageEncode>::STORAGE_SLOTS, 1);
    let host = fresh_host();
    let mut m = unsafe { Mapping::<u64, RunningAverage>::new(StorageKey::from_slot(0), host) };
    let v = RunningAverage { sum: 10, total: 3 };
    m.insert(&1u64, &v);
    assert_eq!(m.get(&1u64), v);
}

#[test]
fn packed_struct_single_slot_via_lazy_round_trip() {
    let host = fresh_host();
    let mut lazy = unsafe { Lazy::<RunningAverage>::new(StorageKey::from_slot(0), 0, host) };
    let v = RunningAverage { sum: 7, total: 11 };
    lazy.set(&v);
    assert_eq!(lazy.get(), v);
}

// --- Three `U256`s — genuinely multi-slot static (3 slots) -----------------

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
struct ThreeWords {
    a: U256,
    b: U256,
    c: U256,
}

#[test]
fn multi_slot_static_struct_takes_three_slots() {
    assert_eq!(<ThreeWords as StorageEncode>::STORAGE_SLOTS, 3);
}

#[test]
fn multi_slot_static_struct_via_mapping_round_trip() {
    let host = fresh_host();
    let mut m = unsafe { Mapping::<u64, ThreeWords>::new(StorageKey::from_slot(0), host) };
    let v = ThreeWords {
        a: U256::from(1u64),
        b: U256::from(2u64),
        c: U256::from(3u64),
    };
    m.insert(&5u64, &v);
    assert_eq!(m.get(&5u64), v);
}

// --- Struct with a dynamic `String` field: solc's header + spilled body ----

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
struct DynamicReview {
    reviewer: Address,
    comment_uri: alloc::string::String,
}

#[test]
fn dynamic_field_struct_takes_two_slots_and_marks_dynamic_body() {
    // `reviewer` (Address, 20 bytes) packs into slot 0; `comment_uri`
    // (`String`, PACKED_BYTES=32) starts a new slot at slot 1.
    assert_eq!(<DynamicReview as StorageEncode>::STORAGE_SLOTS, 2);
    const { assert!(<DynamicReview as StorageEncode>::HAS_DYNAMIC_BODY) };
}

#[test]
fn dynamic_field_struct_via_mapping_round_trip_inline() {
    let host = fresh_host();
    let mut m = unsafe { Mapping::<u64, DynamicReview>::new(StorageKey::from_slot(0), host) };
    let v = DynamicReview {
        reviewer: Address([0x42; 20]),
        comment_uri: alloc::string::String::from("ipfs://short"),
    };
    m.insert(&1u64, &v);
    assert_eq!(m.get(&1u64), v);
}

#[test]
fn dynamic_field_struct_via_mapping_round_trip_spilled() {
    let host = fresh_host();
    let mut m = unsafe { Mapping::<u64, DynamicReview>::new(StorageKey::from_slot(0), host) };
    let long_uri = alloc::string::String::from(
        "ipfs://this-is-a-much-longer-uri-that-will-spill-into-the-keccak-derived-body-slots",
    );
    let v = DynamicReview {
        reviewer: Address([0xab; 20]),
        comment_uri: long_uri,
    };
    m.insert(&5u64, &v);
    assert_eq!(m.get(&5u64), v);
}

#[test]
#[should_panic(expected = "encode_slot called on a dynamic-body slot")]
fn dynamic_field_struct_encode_slot_panics_on_dynamic_slot() {
    // Calling `encode_slot` directly on the dynamic slot of a struct with a
    // dynamic-body field must panic — same as standalone `String::encode_slot`,
    // which inherits the trait's `unreachable!()` default. Without this guard
    // the dynamic arm would silently no-op and return zeros, making the
    // behavior asymmetric between standalone `String` and a struct wrapping it.
    let v = DynamicReview {
        reviewer: Address([0; 20]),
        comment_uri: alloc::string::String::from("hi"),
    };
    let mut buf = [0u8; 32];
    v.encode_slot(1, &mut buf);
}

#[test]
fn dynamic_field_struct_encode_slot_static_slot_still_writes() {
    // Slot 0 (the static `Address` slot) must still encode normally — the
    // panic only fires on the dynamic slot.
    let v = DynamicReview {
        reviewer: Address([0x42; 20]),
        comment_uri: alloc::string::String::from("hi"),
    };
    let mut buf = [0u8; 32];
    v.encode_slot(0, &mut buf);
    let mut expected = [0u8; 32];
    expected[12..].copy_from_slice(&[0x42; 20]);
    assert_eq!(buf, expected);
}

// --- Struct mixing packable statics (Address + u8) with a dynamic String --

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
struct Review {
    reviewer: Address,
    rating: u8,
    comment_uri: alloc::string::String,
}

#[test]
fn review_takes_two_slots_and_marks_dynamic_body() {
    // Slot 0: Address (20B at offset 12..32) + u8 (1B at offset 11) packed.
    // Slot 1: header for `comment_uri`. Two slots total.
    assert_eq!(<Review as StorageEncode>::STORAGE_SLOTS, 2);
    const { assert!(<Review as StorageEncode>::HAS_DYNAMIC_BODY) };
}

#[test]
fn review_via_mapping_round_trip_inline() {
    let host = fresh_host();
    let mut m = unsafe { Mapping::<u64, Review>::new(StorageKey::from_slot(0), host) };
    let v = Review {
        reviewer: Address([0xCD; 20]),
        rating: 5,
        comment_uri: alloc::string::String::from("nice"),
    };
    m.insert(&42u64, &v);
    assert_eq!(m.get(&42u64), v);
}

#[test]
fn review_via_mapping_round_trip_spilled() {
    let host = fresh_host();
    let mut m = unsafe { Mapping::<u64, Review>::new(StorageKey::from_slot(0), host) };
    let v = Review {
        reviewer: Address([0xCD; 20]),
        rating: 5,
        comment_uri: alloc::string::String::from(
            "long enough to force solc's spill encoding so the body lives at \
             keccak256(slot1) and the header carries the length only",
        ),
    };
    m.insert(&42u64, &v);
    assert_eq!(m.get(&42u64), v);
}

#[test]
fn review_via_mapping_remove_clears_storage() {
    let host = fresh_host();
    let mut m = unsafe { Mapping::<u64, Review>::new(StorageKey::from_slot(0), host) };
    let v = Review {
        reviewer: Address([0xCD; 20]),
        rating: 9,
        comment_uri: alloc::string::String::from(
            "long enough comment to spill into keccak-derived body slots that must be cleared",
        ),
    };
    m.insert(&7u64, &v);
    assert_eq!(m.try_get(&7u64), Some(v));
    m.remove(&7u64);
    assert_eq!(m.try_get(&7u64), None);
}

// --- Struct with a dynamic `Bytes` field: mirrors `DynamicReview` (String) ---
//
// `classify_storage_field` puts `SolType::String` and `SolType::DynBytes` on
// the same `Dynamic` arm, so a `Bytes` field exercises the identical codegen
// path. Mirroring the `DynamicReview` / `Review` tests with `Bytes` would
// otherwise let divergent codegen between the two go silent.

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
struct DynamicBlob {
    owner: Address,
    payload: Bytes,
}

#[test]
fn dynamic_blob_takes_two_slots_and_marks_dynamic_body() {
    // `owner` (Address, 20 bytes) packs into slot 0; `payload` (`Bytes`,
    // PACKED_BYTES=32) starts a new slot at slot 1. Same shape as
    // DynamicReview but with Bytes instead of String.
    assert_eq!(<DynamicBlob as StorageEncode>::STORAGE_SLOTS, 2);
    const { assert!(<DynamicBlob as StorageEncode>::HAS_DYNAMIC_BODY) };
}

#[test]
fn dynamic_blob_via_mapping_round_trip_inline() {
    let host = fresh_host();
    let mut m = unsafe { Mapping::<u64, DynamicBlob>::new(StorageKey::from_slot(0), host) };
    let v = DynamicBlob {
        owner: Address([0x42; 20]),
        payload: Bytes(alloc::vec![0xde, 0xad, 0xbe, 0xef]),
    };
    m.insert(&1u64, &v);
    assert_eq!(m.get(&1u64), v);
}

#[test]
fn dynamic_blob_via_mapping_round_trip_spilled() {
    let host = fresh_host();
    let mut m = unsafe { Mapping::<u64, DynamicBlob>::new(StorageKey::from_slot(0), host) };
    let v = DynamicBlob {
        owner: Address([0xab; 20]),
        // > 32 bytes forces solc's spill encoding: header in slot 1, body at
        // keccak256(slot1) + i.
        payload: Bytes(alloc::vec![0x77u8; 80]),
    };
    m.insert(&5u64, &v);
    assert_eq!(m.get(&5u64), v);
}

#[test]
#[should_panic(expected = "from_slots called on dynamic-body struct")]
fn dynamic_blob_from_slots_panics() {
    // Mirrors the trait contract: a dynamic-body struct's `from_slots` is a
    // stub — reads must dispatch through `read_from_storage`.
    let slots = [[0u8; 32]; 2];
    let _ = <DynamicBlob as StorageDecode>::from_slots(&slots);
}

#[test]
#[should_panic(expected = "encode_slot called on a dynamic-body slot")]
fn dynamic_blob_encode_slot_panics_on_dynamic_slot() {
    let v = DynamicBlob {
        owner: Address([0; 20]),
        payload: Bytes(alloc::vec![1, 2, 3]),
    };
    let mut buf = [0u8; 32];
    v.encode_slot(1, &mut buf);
}

// --- Struct mixing packable statics (Address + u8) with a dynamic Bytes ---
// Mirrors `Review` (String + Address + u8) with Bytes in place of String.

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
struct BlobMetadata {
    owner: Address,
    version: u8,
    payload: Bytes,
}

#[test]
fn blob_metadata_takes_two_slots_and_marks_dynamic_body() {
    // Slot 0: Address (20B at offset 12..32) + u8 (1B at offset 11) packed.
    // Slot 1: header for `payload`. Two slots total.
    assert_eq!(<BlobMetadata as StorageEncode>::STORAGE_SLOTS, 2);
    const { assert!(<BlobMetadata as StorageEncode>::HAS_DYNAMIC_BODY) };
}

#[test]
fn blob_metadata_via_mapping_round_trip_inline() {
    let host = fresh_host();
    let mut m = unsafe { Mapping::<u64, BlobMetadata>::new(StorageKey::from_slot(0), host) };
    let v = BlobMetadata {
        owner: Address([0xCD; 20]),
        version: 5,
        payload: Bytes(alloc::vec![0xaa, 0xbb, 0xcc, 0xdd]),
    };
    m.insert(&42u64, &v);
    assert_eq!(m.get(&42u64), v);
}

#[test]
fn blob_metadata_via_mapping_round_trip_spilled() {
    let host = fresh_host();
    let mut m = unsafe { Mapping::<u64, BlobMetadata>::new(StorageKey::from_slot(0), host) };
    let v = BlobMetadata {
        owner: Address([0xCD; 20]),
        version: 9,
        // > 32 bytes forces solc's spill encoding.
        payload: Bytes(alloc::vec![0xee; 96]),
    };
    m.insert(&42u64, &v);
    assert_eq!(m.get(&42u64), v);
}

#[test]
fn blob_metadata_via_mapping_remove_clears_storage() {
    let host = fresh_host();
    let mut m = unsafe { Mapping::<u64, BlobMetadata>::new(StorageKey::from_slot(0), host) };
    let v = BlobMetadata {
        owner: Address([0xCD; 20]),
        version: 9,
        // Spilled-length value so remove must also clear the keccak-derived
        // body chunks, not just the header slot.
        payload: Bytes(alloc::vec![0x55; 96]),
    };
    m.insert(&7u64, &v);
    assert_eq!(m.try_get(&7u64), Some(v));
    m.remove(&7u64);
    assert_eq!(m.try_get(&7u64), None);
}

// ========================================================================
// Sub-word packing inside a `#[derive(SolType)]` struct stored under one
// `Lazy<S>`.
//
// The derive's `__STORAGE_LAYOUT` walker uses `PACKED_BYTES` and
// `STORAGE_SLOTS` to lay out fields with solc-compatible sub-word
// packing — two `u128` fields share a single 32-byte slot. The
// contract-field walker now does the same for adjacent `Lazy<u128>`
// fields, so this is no longer a workaround; it's the same packing rule
// applied one level down, exercised here on the derive path.
// ========================================================================

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
struct U128Pair {
    a: u128,
    b: u128,
}

#[test]
fn u128_pair_packs_into_one_slot_via_soltype() {
    // The SolType derive's layout walker packs two u128s (16 bytes each)
    // into a single 32-byte slot.
    assert_eq!(<U128Pair as StorageEncode>::STORAGE_SLOTS, 1);
    assert_eq!(<U128Pair as StorageEncode>::PACKED_BYTES, 32);
}

#[test]
fn u128_pair_layout_matches_solc_packing() {
    // solc layout for `struct { uint128 a; uint128 b; }`:
    //   field 0 (a) lower-order aligned:  bytes 16..32 = a (BE)
    //   field 1 (b) packed above:         bytes 0..16  = b (BE)
    let v = U128Pair {
        a: 0x1111_1111_1111_1111u128,
        b: 0x2222_2222_2222_2222u128,
    };
    let slots = encode_all(&v);
    assert_eq!(slots.len(), 1, "packs into a single 32-byte slot");

    let s = slots[0];
    assert_eq!(&s[16..32], &v.a.to_be_bytes(), "a at bytes 16..31");
    assert_eq!(&s[0..16], &v.b.to_be_bytes(), "b at bytes 0..15");
}

#[test]
fn lazy_of_u128_pair_advances_chain_by_one_slot() {
    // `Lazy<U128Pair>` claims a single root slot. Each `Lazy<u128>` also
    // reports `SLOTS = 1` — the contract-field walker packs adjacent
    // sub-word `Lazy` fields into the same slot via `PACKED_BYTES`, so
    // `Lazy<u128>; Lazy<u128>;` lands at (slot=0, offset=16) and
    // (slot=0, offset=0) rather than consuming two slots.
    assert_eq!(
        <Lazy<U128Pair> as pvm_contract_sdk::StorageComponent>::SLOTS,
        1
    );
    assert_eq!(<Lazy<u128> as pvm_contract_sdk::StorageComponent>::SLOTS, 1);
}

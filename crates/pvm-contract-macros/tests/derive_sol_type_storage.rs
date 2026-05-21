//! Tests for the `StorageEncode` / `StorageDecode` impls emitted by
//! `#[derive(SolType)]` for static structs.
//!
//! These verify the solc-compatible storage layout: sub-word packing rules
//! for primitives, right-alignment for integers and Address, left-alignment
//! for bytesN, and consecutive slots for composite (struct-in-struct) fields.

extern crate alloc;

use pvm_contract_sdk::SolType;
use pvm_contract_sdk::{
    Address, Lazy, Mapping, StorageDecode, StorageEncode, StorageKey, StoragePackable, U256,
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
    const { assert!(<AddrAndCounter as StorageEncode>::STARTS_NEW_SLOT) };
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
    assert_eq!(
        &s0[27..31],
        &v.joined_at.to_be_bytes(),
        "uint32 at 27..31"
    );
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

    let v = Spill {
        a: 1,
        b: 2,
        c: 3,
    };
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
    let mut m = Mapping::<u64, RunningAverage>::new(StorageKey::from_slot(0), host);
    let v = RunningAverage { sum: 10, total: 3 };
    m.insert(&1u64, &v);
    assert_eq!(m.get(&1u64), v);
}

#[test]
fn packed_struct_single_slot_via_lazy_round_trip() {
    let host = fresh_host();
    let mut lazy = Lazy::<RunningAverage>::new(StorageKey::from_slot(0), host);
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
    let mut m = Mapping::<u64, ThreeWords>::new(StorageKey::from_slot(0), host);
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
    // (`String`, STARTS_NEW_SLOT=true) starts a new slot at slot 1.
    assert_eq!(<DynamicReview as StorageEncode>::STORAGE_SLOTS, 2);
    const { assert!(<DynamicReview as StorageEncode>::HAS_DYNAMIC_BODY) };
}

#[test]
fn dynamic_field_struct_via_mapping_round_trip_inline() {
    let host = fresh_host();
    let mut m = Mapping::<u64, DynamicReview>::new(StorageKey::from_slot(0), host);
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
    let mut m = Mapping::<u64, DynamicReview>::new(StorageKey::from_slot(0), host);
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

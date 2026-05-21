//! Verify that the cases from issue #80 now work or fail as expected.
//!
//! Three categories from the issue:
//!   1. Multi-word static structs as `Mapping<K, V>` values (e.g. RunningAverage).
//!   2. Structs with dynamic fields (e.g. DynamicReview with String).
//!   3. Tuples that ABI-encode to >32 bytes (e.g. (U256, U256)).
//!
//! Expected after Phase 1-3:
//!   - (1) and (3) work — solc-packed storage layout, multi-slot if needed.
//!   - (2) fails to compile at the use site (trait not implemented).

extern crate alloc;

use pvm_contract_sdk::{Address, Lazy, Mapping, SolType, StorageEncode, U256};
use pvm_contract_types::MockHostBuilder;

fn h() -> pvm_contract_sdk::Host {
    pvm_contract_sdk::Host::from_dyn(alloc::rc::Rc::new(MockHostBuilder::new().build()))
}

// --- Issue example: RunningAverage (two u64 — solc packs into one slot) -----

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
pub struct RunningAverage {
    pub sum: u64,
    pub total: u64,
}

#[test]
fn issue_80_running_average_packs_into_one_slot() {
    assert_eq!(<RunningAverage as StorageEncode>::STORAGE_SLOTS, 1);
}

#[test]
fn issue_80_mapping_to_running_average() {
    let host = h();
    let mut m = Mapping::<u64, RunningAverage>::new(
        pvm_contract_sdk::StorageKey::from_slot(0),
        host,
    );
    let v = RunningAverage { sum: 10, total: 3 };
    m.insert(&1u64, &v);
    assert_eq!(m.get(&1u64), v);
}

// --- TwoWords: same shape (issue's compile-test target) --------------------

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
pub struct TwoWords {
    pub a: u64,
    pub b: u64,
}

#[test]
fn issue_80_lazy_two_words_round_trip() {
    let host = h();
    let mut lazy = Lazy::<TwoWords>::new(pvm_contract_sdk::StorageKey::from_slot(0), host);
    lazy.set(&TwoWords { a: 7, b: 11 });
    assert_eq!(lazy.get(), TwoWords { a: 7, b: 11 });
}

// --- Genuinely multi-slot static struct (≥ 2 slots in solc layout) ---------

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
pub struct BigStatic {
    pub a: U256,
    pub b: U256,
    pub c: U256,
}

#[test]
fn issue_80_big_static_takes_three_slots() {
    assert_eq!(<BigStatic as StorageEncode>::STORAGE_SLOTS, 3);
}

#[test]
fn issue_80_mapping_to_big_static() {
    let host = h();
    let mut m = Mapping::<u64, BigStatic>::new(
        pvm_contract_sdk::StorageKey::from_slot(0),
        host,
    );
    let v = BigStatic {
        a: U256::from(1u64),
        b: U256::from(2u64),
        c: U256::from(3u64),
    };
    m.insert(&5u64, &v);
    assert_eq!(m.get(&5u64), v);
}

// --- Issue #80 expectation #1: struct with bare `String` field --------------

#[derive(Clone, Debug, PartialEq, Eq, SolType)]
pub struct DynamicReview {
    pub reviewer: Address,
    pub comment_uri: alloc::string::String,
}

#[test]
fn issue_80_dynamic_review_takes_two_slots() {
    // reviewer (Address, 20 bytes) packs into slot 0; comment_uri (`String`,
    // STARTS_NEW_SLOT=true) starts a new slot at slot 1.
    assert_eq!(<DynamicReview as StorageEncode>::STORAGE_SLOTS, 2);
    const { assert!(<DynamicReview as StorageEncode>::HAS_DYNAMIC_BODY) };
}

#[test]
fn issue_80_dynamic_review_round_trip_short_uri() {
    let host = h();
    let mut m = Mapping::<u64, DynamicReview>::new(
        pvm_contract_sdk::StorageKey::from_slot(0),
        host,
    );
    let v = DynamicReview {
        reviewer: Address([0x42; 20]),
        comment_uri: alloc::string::String::from("ipfs://short"),
    };
    m.insert(&1u64, &v);
    let got = m.get(&1u64);
    assert_eq!(got, v);
}

#[test]
fn issue_80_dynamic_review_round_trip_long_uri() {
    let host = h();
    let mut m = Mapping::<u64, DynamicReview>::new(
        pvm_contract_sdk::StorageKey::from_slot(0),
        host,
    );
    let long_uri = alloc::string::String::from(
        "ipfs://this-is-a-much-longer-uri-that-will-spill-into-the-keccak-derived-body-slots",
    );
    let v = DynamicReview {
        reviewer: Address([0xab; 20]),
        comment_uri: long_uri,
    };
    m.insert(&5u64, &v);
    let got = m.get(&5u64);
    assert_eq!(got, v);
}

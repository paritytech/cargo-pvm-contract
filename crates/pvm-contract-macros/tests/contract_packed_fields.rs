//! End-to-end `#[contract]`-level tests for packed-field reads/writes.
//!
//! The unit tests in `pvm-storage/src/lib.rs` drive `Lazy::new` directly with
//! hand-picked `(slot, offset)` values. These tests close the gap by going
//! through a real `#[contract]` struct: the macro emits
//! `StorageComponent::new_at(slot, offset, host.clone())` calls in its
//! generated `deploy()`/`call()` glue, and adjacent sub-32-byte fields must
//! pack into one slot byte-for-byte matching solc's `storageLayout`.
//!
//! Approach: each contract is declared with `#[contract(no_main)]` so the
//! integration-test harness keeps its own `main`. We construct each storage
//! field by hand via `<T as StorageComponent>::new_at(slot, offset, host)` at
//! the placement the macro's walker would have picked, build the contract
//! struct, and exercise the typed methods. Raw slot bytes are inspected via
//! `MockHost::get_raw_storage` to assert solc-compatible packing.
//!
//! The macro's own walker is exercised at compile time by declaring the
//! contract: a mismatch between the `layout_step` chain and the literal
//! `(slot, offset)` we pass to `new_at` from the test would either fail the
//! macro's runtime asserts or be caught by Test 9's `storageLayout` JSON.

extern crate alloc;

use pvm_contract_sdk::{Address, Lazy, Mapping, U256};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
//
// Under `--features abi-gen`, the `#[contract]` macro cfg-gates user `impl`
// blocks out, so contract methods (`set_a`, etc.) are unavailable. The
// runtime helpers and tests are gated to `not(feature = "abi-gen")`; the
// layout-JSON assertions further down use the `#[cfg(feature = "abi-gen")]`
// gate.

#[cfg(not(feature = "abi-gen"))]
use alloc::rc::Rc;
#[cfg(not(feature = "abi-gen"))]
use pvm_contract_sdk::{Host, MockHost, MockHostBuilder, StorageComponent, StorageKey};

#[cfg(not(feature = "abi-gen"))]
fn fresh() -> (Host, MockHost) {
    let mock = MockHostBuilder::new().build();
    let host = Host::from_dyn(Rc::new(mock.clone()));
    (host, mock)
}

/// Raw read of slot `n`, returns 32 zero bytes if the slot was never written
/// (matching Solidity's default-to-zero semantics).
#[cfg(not(feature = "abi-gen"))]
fn raw_slot(mock: &MockHost, n: u64) -> [u8; 32] {
    let key = StorageKey::from_slot(n);
    match mock.get_raw_storage(key.as_bytes()) {
        Some(v) => {
            assert_eq!(v.len(), 32, "raw slot must be 32 bytes; got {}", v.len());
            let mut out = [0u8; 32];
            out.copy_from_slice(&v);
            out
        }
        None => [0u8; 32],
    }
}

// ===========================================================================
// Test 1 — Two adjacent `Lazy<u128>` pack into slot 0 via `#[contract]`
// ===========================================================================
//
// solc layout for `contract C { uint128 a; uint128 b; }`:
//   slot 0 bytes 16..32 = a (right-aligned u128)
//   slot 0 bytes  0..16 = b (packed above a)
// One slot, not two. The macro's walker must propagate u128's
// `PACKED_BYTES = 16` to `new_at(0, 16, host)` for `a` and `new_at(0, 0, host)`
// for `b`.

#[allow(dead_code)] // route/deploy/call are riscv64-gated
#[pvm_contract_macros::contract(no_main)]
mod packed_pair {
    use super::*;

    pub struct PackedPair {
        pub a: Lazy<u128>, // expected: (slot=0, offset=16)
        pub b: Lazy<u128>, // expected: (slot=0, offset=0)
    }

    impl PackedPair {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn set_a(&mut self, v: u128) {
            self.a.set(&v);
        }

        #[pvm_contract_macros::method]
        pub fn set_b(&mut self, v: u128) {
            self.b.set(&v);
        }

        #[pvm_contract_macros::method]
        pub fn get_a(&self) -> u128 {
            self.a.get()
        }

        #[pvm_contract_macros::method]
        pub fn get_b(&self) -> u128 {
            self.b.get()
        }
    }
}

#[cfg(not(feature = "abi-gen"))]
fn build_packed_pair(host: &Host) -> packed_pair::PackedPair {
    // Mirror what the macro emits for adjacent `Lazy<u128>` fields:
    //   __pvm_storage_slot_a = layout_step(FIRST,  16, 1) -> (slot=0, offset=16)
    //   __pvm_storage_slot_b = layout_step(prev_a, 16, 1) -> (slot=0, offset=0)
    let a = <Lazy<u128> as StorageComponent>::new_at(0, 16, host.clone());
    let b = <Lazy<u128> as StorageComponent>::new_at(0, 0, host.clone());
    packed_pair::PackedPair {
        a,
        b,
        host: host.clone(),
    }
}

#[cfg(not(feature = "abi-gen"))]
#[test]
fn two_lazy_u128_fields_share_slot_0_with_solc_layout() {
    let (host, mock) = fresh();
    let mut c = build_packed_pair(&host);

    c.set_a(0x1111_1111_1111_1111u128);
    c.set_b(0x2222_2222_2222_2222u128);

    let s0 = raw_slot(&mock, 0);
    assert_eq!(
        &s0[16..32],
        &0x1111_1111_1111_1111u128.to_be_bytes(),
        "slot 0 bytes 16..31 hold `a` (solc: uint128 a at offset 16)",
    );
    assert_eq!(
        &s0[0..16],
        &0x2222_2222_2222_2222u128.to_be_bytes(),
        "slot 0 bytes 0..15 hold `b` (solc: uint128 b at offset 0)",
    );
    // Crucial: only ONE slot was used. Slot 1 must stay completely empty.
    assert_eq!(
        raw_slot(&mock, 1),
        [0u8; 32],
        "slot 1 untouched — packing saved a slot",
    );

    // Round-trip via typed methods.
    assert_eq!(c.get_a(), 0x1111_1111_1111_1111u128);
    assert_eq!(c.get_b(), 0x2222_2222_2222_2222u128);
}

// ===========================================================================
// Test 2 — RMW correctness via `&mut self` methods, both write orders
// ===========================================================================
//
// Writing one packed field must not clobber its neighbour, regardless of
// which is written first. Exercises the `Lazy::set` RMW path via the
// macro-emitted `&mut self` dispatch (Test 1 only covered one order).

#[cfg(not(feature = "abi-gen"))]
#[test]
fn packed_lazy_u128_rmw_preserves_neighbour_via_methods_both_orders() {
    for (a_first, label) in [(true, "a then b"), (false, "b then a")] {
        let (host, _mock) = fresh();
        let mut c = build_packed_pair(&host);

        let av = 0xAAAA_AAAA_AAAA_AAAAu128;
        let bv = 0xBBBB_BBBB_BBBB_BBBBu128;
        if a_first {
            c.set_a(av);
            c.set_b(bv);
        } else {
            c.set_b(bv);
            c.set_a(av);
        }
        assert_eq!(c.get_a(), av, "{label}: a survived after both writes");
        assert_eq!(c.get_b(), bv, "{label}: b survived after both writes");
    }
}

// ===========================================================================
// Test 3 — Classic solc layout: bool + u32 + Address + U256
// ===========================================================================
//
// solc layout for
//   contract C { bool flag; uint32 counter; address owner; uint256 balance; }
//   slot 0: flag (offset 31), counter (offset 27..31), owner (offset 7..27)
//   slot 1: balance (full slot)

#[allow(dead_code)]
#[pvm_contract_macros::contract(no_main)]
mod classic_layout {
    use super::*;

    pub struct ClassicLayout {
        pub flag: Lazy<bool>,     // (slot=0, offset=31)
        pub counter: Lazy<u32>,   // (slot=0, offset=27)
        pub owner: Lazy<Address>, // (slot=0, offset=7)
        pub balance: Lazy<U256>,  // (slot=1, offset=0)
    }

    impl ClassicLayout {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn set_flag(&mut self, v: bool) {
            self.flag.set(&v);
        }

        #[pvm_contract_macros::method]
        pub fn set_counter(&mut self, v: u32) {
            self.counter.set(&v);
        }

        #[pvm_contract_macros::method]
        pub fn set_owner(&mut self, v: Address) {
            self.owner.set(&v);
        }

        #[pvm_contract_macros::method]
        pub fn set_balance(&mut self, v: U256) {
            self.balance.set(&v);
        }

        #[pvm_contract_macros::method]
        pub fn get_flag(&self) -> bool {
            self.flag.get()
        }

        #[pvm_contract_macros::method]
        pub fn get_counter(&self) -> u32 {
            self.counter.get()
        }

        #[pvm_contract_macros::method]
        pub fn get_owner(&self) -> Address {
            self.owner.get()
        }

        #[pvm_contract_macros::method]
        pub fn get_balance(&self) -> U256 {
            self.balance.get()
        }
    }
}

#[cfg(not(feature = "abi-gen"))]
fn build_classic(host: &Host) -> classic_layout::ClassicLayout {
    classic_layout::ClassicLayout {
        flag: <Lazy<bool> as StorageComponent>::new_at(0, 31, host.clone()),
        counter: <Lazy<u32> as StorageComponent>::new_at(0, 27, host.clone()),
        owner: <Lazy<Address> as StorageComponent>::new_at(0, 7, host.clone()),
        balance: <Lazy<U256> as StorageComponent>::new_at(1, 0, host.clone()),
        host: host.clone(),
    }
}

#[cfg(not(feature = "abi-gen"))]
#[test]
fn classic_solc_layout_packs_bool_u32_address_into_slot_0_and_u256_into_slot_1() {
    let (host, mock) = fresh();
    let mut c = build_classic(&host);

    let owner_addr = Address([0x42; 20]);
    c.set_flag(true);
    c.set_counter(0x01020304);
    c.set_owner(owner_addr);
    c.set_balance(U256::from(0xfeedu32));

    let s0 = raw_slot(&mock, 0);
    assert_eq!(s0[31], 1, "bool at byte 31 = 0x01");
    assert_eq!(&s0[27..31], &0x01020304u32.to_be_bytes(), "u32 at 27..31");
    assert_eq!(&s0[7..27], &owner_addr.0, "address at 7..27");
    assert!(s0[..7].iter().all(|&b| b == 0), "high padding zero");

    let s1 = raw_slot(&mock, 1);
    assert_eq!(
        s1,
        U256::from(0xfeedu32).to_be_bytes::<32>(),
        "U256 fills slot 1"
    );

    // Round-trip through view methods.
    assert!(c.get_flag());
    assert_eq!(c.get_counter(), 0x01020304);
    assert_eq!(c.get_owner(), owner_addr);
    assert_eq!(c.get_balance(), U256::from(0xfeedu32));
}

// ===========================================================================
// Test 4 — Spill across a slot boundary: u128 a; u128 b; u128 c;
// ===========================================================================
//
// solc layout: a+b share slot 0; c does not fit, spills to slot 1 at offset 16.

#[allow(dead_code)]
#[pvm_contract_macros::contract(no_main)]
mod spill {
    use super::*;

    pub struct Spill {
        pub a: Lazy<u128>, // (slot=0, offset=16)
        pub b: Lazy<u128>, // (slot=0, offset=0)
        pub c: Lazy<u128>, // (slot=1, offset=16) — spilled
    }

    impl Spill {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn set_all(&mut self, av: u128, bv: u128, cv: u128) {
            self.a.set(&av);
            self.b.set(&bv);
            self.c.set(&cv);
        }

        #[pvm_contract_macros::method]
        pub fn get_a(&self) -> u128 {
            self.a.get()
        }

        #[pvm_contract_macros::method]
        pub fn get_b(&self) -> u128 {
            self.b.get()
        }

        #[pvm_contract_macros::method]
        pub fn get_c(&self) -> u128 {
            self.c.get()
        }
    }
}

#[cfg(not(feature = "abi-gen"))]
#[test]
fn three_u128_fields_spill_third_to_a_fresh_slot() {
    let (host, mock) = fresh();
    let mut c = spill::Spill {
        a: <Lazy<u128> as StorageComponent>::new_at(0, 16, host.clone()),
        b: <Lazy<u128> as StorageComponent>::new_at(0, 0, host.clone()),
        c: <Lazy<u128> as StorageComponent>::new_at(1, 16, host.clone()),
        host: host.clone(),
    };

    c.set_all(1, 2, 3);

    let s0 = raw_slot(&mock, 0);
    assert_eq!(
        &s0[16..32],
        &1u128.to_be_bytes(),
        "a at slot 0 bytes 16..31"
    );
    assert_eq!(&s0[0..16], &2u128.to_be_bytes(), "b at slot 0 bytes 0..15");

    let s1 = raw_slot(&mock, 1);
    assert_eq!(
        &s1[16..32],
        &3u128.to_be_bytes(),
        "c spilled to slot 1 bytes 16..31"
    );
    assert!(
        s1[..16].iter().all(|&b| b == 0),
        "slot 1 high bytes zero (c alone)"
    );

    // Slot 2 must be untouched: c does not consume slot 2.
    assert_eq!(raw_slot(&mock, 2), [0u8; 32]);

    assert_eq!(c.get_a(), 1);
    assert_eq!(c.get_b(), 2);
    assert_eq!(c.get_c(), 3);
}

// ===========================================================================
// Test 5 — Mapping forces a fresh slot mid-contract
// ===========================================================================
//
// `Mapping` reports `PACKED_BYTES = 32`, so it always starts a fresh slot
// and the next packable field cannot share its slot either. Layout:
//   before: (slot=0, offset=31)   bool
//   m:     (slot=1)               Mapping root
//   after:  (slot=2, offset=31)   bool
// `after` must NOT land at slot 1 offset 31.

#[allow(dead_code)]
#[pvm_contract_macros::contract(no_main)]
mod with_mapping {
    use super::*;

    pub struct WithMapping {
        pub before: Lazy<bool>,               // (slot=0, offset=31)
        pub balances: Mapping<Address, U256>, // (slot=1)
        pub after: Lazy<bool>,                // (slot=2, offset=31)
    }

    impl WithMapping {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn set_before(&mut self, v: bool) {
            self.before.set(&v);
        }

        #[pvm_contract_macros::method]
        pub fn set_after(&mut self, v: bool) {
            self.after.set(&v);
        }

        #[pvm_contract_macros::method]
        pub fn credit(&mut self, who: Address, amount: U256) {
            self.balances.insert(&who, &amount);
        }

        #[pvm_contract_macros::method]
        pub fn balance_of(&self, who: Address) -> U256 {
            self.balances.get(&who)
        }
    }
}

#[cfg(not(feature = "abi-gen"))]
#[test]
fn mapping_forces_fresh_slot_and_following_packable_lands_on_a_new_slot() {
    let (host, mock) = fresh();
    let mut c = with_mapping::WithMapping {
        before: <Lazy<bool> as StorageComponent>::new_at(0, 31, host.clone()),
        balances: unsafe { Mapping::<Address, U256>::new(StorageKey::from_slot(1), host.clone()) },
        after: <Lazy<bool> as StorageComponent>::new_at(2, 31, host.clone()),
        host: host.clone(),
    };

    let alice = Address([0xAA; 20]);
    c.set_before(true);
    c.credit(alice, U256::from(42));
    c.set_after(true);

    // `before` sits at slot 0 offset 31.
    let s0 = raw_slot(&mock, 0);
    assert_eq!(s0[31], 1, "before=true at slot 0 byte 31");
    assert!(s0[..31].iter().all(|&b| b == 0), "rest of slot 0 zero");

    // The mapping ROOT slot (slot 1) stays empty — entries live at derived keys.
    // Critically, `after` must NOT have packed into slot 1 byte 31.
    assert_eq!(
        raw_slot(&mock, 1),
        [0u8; 32],
        "mapping root slot 1 stays empty; `after` did NOT pack here",
    );

    // `after` lives on its own fresh slot 2 at offset 31.
    let s2 = raw_slot(&mock, 2);
    assert_eq!(s2[31], 1, "after=true at slot 2 byte 31");
    assert!(s2[..31].iter().all(|&b| b == 0), "rest of slot 2 zero");

    // Cross-check: an independent Mapping rooted at slot 1 sees the same entry.
    let independent =
        unsafe { Mapping::<Address, U256>::new(StorageKey::from_slot(1), host.clone()) };
    assert_eq!(independent.get(&alice), U256::from(42));
    assert_eq!(c.balance_of(alice), U256::from(42));
}

// ===========================================================================
// Test 6 — Multi-slot composite forces a fresh slot
// ===========================================================================
//
// A `(U256, U256)` tuple field reports `SLOTS = 2, PACKED_BYTES = 32`. It must
// claim slots 1..=2 to their full extent, so a following sub-word field can NOT
// reuse slot 2's high bytes — it must land on slot 3.
//
// Layout:
//   flag: (slot=0, offset=31)   bool
//   pair: (slot=1..=2)          (U256, U256)
//   tail: (slot=3, offset=28)   u32

#[allow(dead_code)]
#[pvm_contract_macros::contract(no_main)]
mod with_multi_slot {
    use super::*;

    pub struct WithMultiSlot {
        pub flag: Lazy<bool>,         // (slot=0, offset=31)
        pub pair: Lazy<(U256, U256)>, // (slot=1, claims slots 1..=2)
        pub tail: Lazy<u32>,          // (slot=3, offset=28)
    }

    impl WithMultiSlot {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn set_flag(&mut self, v: bool) {
            self.flag.set(&v);
        }

        #[pvm_contract_macros::method]
        pub fn set_pair(&mut self, x: U256, y: U256) {
            self.pair.set(&(x, y));
        }

        #[pvm_contract_macros::method]
        pub fn set_tail(&mut self, v: u32) {
            self.tail.set(&v);
        }

        #[pvm_contract_macros::method]
        pub fn get_pair(&self) -> (U256, U256) {
            self.pair.get()
        }

        #[pvm_contract_macros::method]
        pub fn get_tail(&self) -> u32 {
            self.tail.get()
        }
    }
}

#[cfg(not(feature = "abi-gen"))]
#[test]
fn multi_slot_composite_forces_fresh_slot_for_following_field() {
    let (host, mock) = fresh();
    let mut c = with_multi_slot::WithMultiSlot {
        flag: <Lazy<bool> as StorageComponent>::new_at(0, 31, host.clone()),
        pair: <Lazy<(U256, U256)> as StorageComponent>::new_at(1, 0, host.clone()),
        tail: <Lazy<u32> as StorageComponent>::new_at(3, 28, host.clone()),
        host: host.clone(),
    };

    let x = U256::from(0x1111u32);
    let y = U256::from(0x2222u32);
    c.set_flag(true);
    c.set_pair(x, y);
    c.set_tail(0xDEADBEEF);

    // flag at slot 0 byte 31.
    assert_eq!(raw_slot(&mock, 0)[31], 1);

    // pair spans slots 1 and 2.
    assert_eq!(
        raw_slot(&mock, 1),
        x.to_be_bytes::<32>(),
        "pair.0 fills slot 1"
    );
    assert_eq!(
        raw_slot(&mock, 2),
        y.to_be_bytes::<32>(),
        "pair.1 fills slot 2"
    );

    // tail lands on a FRESH slot 3 — must not have tried to pack into slot 2.
    let s3 = raw_slot(&mock, 3);
    assert_eq!(
        &s3[28..32],
        &0xDEADBEEFu32.to_be_bytes(),
        "tail at slot 3 offset 28"
    );
    assert!(s3[..28].iter().all(|&b| b == 0), "rest of slot 3 zero");

    assert_eq!(c.get_pair(), (x, y));
    assert_eq!(c.get_tail(), 0xDEADBEEF);
}

// ===========================================================================
// Test 7 — `clear()` preserves the packed neighbour through `&mut self`
// ===========================================================================
//
// Clearing one half of a packed slot must perform an RMW that zeros only its
// byte window, not the whole slot. The unit test in `pvm-storage` covers raw
// `Lazy::clear`; this covers the macro-dispatched method path.

#[allow(dead_code)]
#[pvm_contract_macros::contract(no_main)]
mod packed_clear {
    use super::*;

    pub struct PackedClear {
        pub a: Lazy<u128>, // (slot=0, offset=16)
        pub b: Lazy<u128>, // (slot=0, offset=0)
    }

    impl PackedClear {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn set_a(&mut self, v: u128) {
            self.a.set(&v);
        }

        #[pvm_contract_macros::method]
        pub fn set_b(&mut self, v: u128) {
            self.b.set(&v);
        }

        #[pvm_contract_macros::method]
        pub fn clear_b(&mut self) {
            self.b.clear();
        }

        #[pvm_contract_macros::method]
        pub fn get_a(&self) -> u128 {
            self.a.get()
        }

        #[pvm_contract_macros::method]
        pub fn get_b(&self) -> u128 {
            self.b.get()
        }
    }
}

#[cfg(not(feature = "abi-gen"))]
#[test]
fn clear_packed_field_preserves_neighbour_via_method() {
    let (host, mock) = fresh();
    let mut c = packed_clear::PackedClear {
        a: <Lazy<u128> as StorageComponent>::new_at(0, 16, host.clone()),
        b: <Lazy<u128> as StorageComponent>::new_at(0, 0, host.clone()),
        host: host.clone(),
    };

    let av = 0xAAAA_AAAA_AAAA_AAAAu128;
    let bv = 0xBBBB_BBBB_BBBB_BBBBu128;
    c.set_a(av);
    c.set_b(bv);
    c.clear_b();

    assert_eq!(c.get_a(), av, "a untouched after clear_b()");
    assert_eq!(c.get_b(), 0, "b is zero after clear");

    // Slot stays non-zero overall (a's bytes are still there) — the host's
    // auto-delete on all-zero would lose a's data otherwise.
    let s0 = raw_slot(&mock, 0);
    assert_eq!(
        &s0[16..32],
        &av.to_be_bytes(),
        "a still at slot 0 bytes 16..31"
    );
    assert!(s0[..16].iter().all(|&b| b == 0), "b's window zeroed");
}

// ===========================================================================
// Test 9 — `__storage_layout_json()` matches solc for packed layout (abi-gen)
// ===========================================================================
//
// Locks the public ABI surface to the wire format Tests 1, 3, 4, 5, 6 verified.
// A consumer reading `cast storage <addr> <slot>` would see the same offsets.
//
// Test 8 (compile-fail for `try_get` on packed `Lazy<u128>`) lives in
// `crates/pvm-storage/tests/ui/lazy_try_get_packed_rejected.rs`.

#[cfg(feature = "abi-gen")]
#[test]
fn packed_pair_emits_solc_compatible_storage_layout() {
    let actual: pvm_contract_sdk::serde_json::Value =
        pvm_contract_sdk::serde_json::from_str(&packed_pair::__storage_layout_json()).unwrap();
    let expected: pvm_contract_sdk::serde_json::Value = pvm_contract_sdk::serde_json::json!({
        "storage": [
            // solc `offset` counts from the least-significant byte: the first
            // packed field is lower-order aligned (offset 0), the next sits
            // above it (offset 16).
            { "label": "a", "offset": 0,  "slot": "0", "type": "uint128" },
            { "label": "b", "offset": 16, "slot": "0", "type": "uint128" },
        ]
    });
    assert_eq!(actual, expected);
}

#[cfg(feature = "abi-gen")]
#[test]
fn classic_layout_emits_solc_compatible_storage_layout() {
    let actual: pvm_contract_sdk::serde_json::Value =
        pvm_contract_sdk::serde_json::from_str(&classic_layout::__storage_layout_json()).unwrap();
    let expected: pvm_contract_sdk::serde_json::Value = pvm_contract_sdk::serde_json::json!({
        "storage": [
            // bool(1) + uint32(4) + address(20) packed low-order first:
            // offsets 0, 1, 5 — matching solc's classic packing example.
            { "label": "flag",    "offset": 0, "slot": "0", "type": "bool" },
            { "label": "counter", "offset": 1, "slot": "0", "type": "uint32" },
            { "label": "owner",   "offset": 5, "slot": "0", "type": "address" },
            { "label": "balance", "offset": 0, "slot": "1", "type": "uint256" },
        ]
    });
    assert_eq!(actual, expected);
}

#[cfg(feature = "abi-gen")]
#[test]
fn spill_emits_solc_compatible_storage_layout() {
    let actual: pvm_contract_sdk::serde_json::Value =
        pvm_contract_sdk::serde_json::from_str(&spill::__storage_layout_json()).unwrap();
    let expected: pvm_contract_sdk::serde_json::Value = pvm_contract_sdk::serde_json::json!({
        "storage": [
            { "label": "a", "offset": 0,  "slot": "0", "type": "uint128" },
            { "label": "b", "offset": 16, "slot": "0", "type": "uint128" },
            // `c` spills to a fresh slot as the first (lower-order) field there.
            { "label": "c", "offset": 0,  "slot": "1", "type": "uint128" },
        ]
    });
    assert_eq!(actual, expected);
}

#[cfg(feature = "abi-gen")]
#[test]
fn with_mapping_emits_solc_compatible_storage_layout() {
    let actual: pvm_contract_sdk::serde_json::Value =
        pvm_contract_sdk::serde_json::from_str(&with_mapping::__storage_layout_json()).unwrap();
    let expected: pvm_contract_sdk::serde_json::Value = pvm_contract_sdk::serde_json::json!({
        "storage": [
            { "label": "before",   "offset": 0, "slot": "0", "type": "bool" },
            { "label": "balances", "offset": 0, "slot": "1", "type": "mapping(address => uint256)" },
            { "label": "after",    "offset": 0, "slot": "2", "type": "bool" },
        ]
    });
    assert_eq!(actual, expected);
}

#[cfg(feature = "abi-gen")]
#[test]
fn with_multi_slot_emits_solc_compatible_storage_layout() {
    let actual: pvm_contract_sdk::serde_json::Value =
        pvm_contract_sdk::serde_json::from_str(&with_multi_slot::__storage_layout_json()).unwrap();
    let expected: pvm_contract_sdk::serde_json::Value = pvm_contract_sdk::serde_json::json!({
        "storage": [
            { "label": "flag", "offset": 0, "slot": "0", "type": "bool" },
            { "label": "pair", "offset": 0, "slot": "1", "type": "(uint256,uint256)" },
            { "label": "tail", "offset": 0, "slot": "3", "type": "uint32" },
        ]
    });
    assert_eq!(actual, expected);
}

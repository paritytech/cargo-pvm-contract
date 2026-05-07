extern crate std;

use super::*;
use alloc::rc::Rc;
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
        0x8f, 0x22, 0x84, 0x85, 0x72, 0xde, 0xaf, 0x32, 0x1e, 0xcb, 0x41, 0x09, 0x5a, 0x0a, 0x57,
        0xd3, 0xf1, 0x9e, 0xda, 0x24, 0xb9, 0x2a, 0x3f, 0x4a, 0x8e, 0x55, 0x4a, 0x2e, 0x56, 0xf4,
        0x5b, 0xc4,
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
    let allowances = Mapping::<Address, Mapping<Address, U256>>::new(StorageKey::from_slot(2), h());
    let owner = Address([0xAA; 20]);
    let spender = Address([0xBB; 20]);

    // Derive via chaining: get(&owner) returns inner Mapping, then slot_of(&spender)
    let inner = allowances.get(&owner);
    let slot = inner.slot_of(&spender);

    let expected = [
        0x35, 0x81, 0x5c, 0x85, 0x0a, 0xc7, 0xd4, 0xd0, 0xaf, 0x32, 0x28, 0x24, 0x69, 0x97, 0x87,
        0xb1, 0x46, 0xe3, 0x3c, 0x6c, 0xac, 0x5d, 0x0a, 0x52, 0xab, 0x32, 0x25, 0xd6, 0x98, 0x5a,
        0x27, 0xa7,
    ];
    assert_eq!(
        slot.as_bytes(),
        &expected,
        "must match chained `cast index` output"
    );
}

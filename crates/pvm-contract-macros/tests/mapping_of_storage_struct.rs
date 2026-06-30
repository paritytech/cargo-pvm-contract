//! `Mapping<K, V>` where `V` is itself a storage component (`#[storage]`
//! sub-struct, `Lazy<T>`, nested `Mapping<K2, V'>`, …).
//!
//! Covers the new storage-typed `view` / `view_mut` API introduced by the
//! `StorageComponent::new_at(StorageKey, …)` generalization. The on-chain
//! layout matches solc's `mapping(K => struct)` pattern: the value lives at
//! the derived key, sub-fields at `derived + N`, and inner mappings derive
//! sub-keys from the per-key derived slot.
//!
//! Three angles per pattern:
//!   - **Round-trip** — write through `view_mut(k).field.set(v)`, read
//!     through `view(k).field.get()`.
//!   - **Independent keys** — different outer K values don't interfere.
//!   - **Layout** — the inner field's slot matches the
//!     `keccak256(pad32(k)+slot).add(N)` derivation solc would emit.

use pvm_contract_sdk::{
    Address, HostApi, Lazy, Mapping, StorageComponent, StorageFlags, StorageKey,
};
use pvm_contract_types::{Host, MockHostBuilder};
use ruint::aliases::U256;
use std::rc::Rc;

extern crate alloc;

fn fresh_host() -> Host {
    Host::from_dyn(Rc::new(MockHostBuilder::new().build()))
}

fn raw_slot(host: &Host, key: &StorageKey) -> [u8; 32] {
    let mut buf = [0u8; 32];
    host.get_storage_or_zero(StorageFlags::empty(), key.as_bytes(), &mut buf);
    buf
}

// ---------------------------------------------------------------------------
// 1. Mapping<K, MyStorageStruct> with mixed Lazy + Mapping fields
// ---------------------------------------------------------------------------

#[pvm_contract_sdk::storage]
pub struct VaultData {
    pub total_shares: Lazy<U256>,       // field offset 0 of derived key
    pub shares: Mapping<Address, U256>, // field offset 1 of derived key
    pub last_deposit: Mapping<Address, U256>, // field offset 2 of derived key
}

#[test]
fn mapping_of_storage_struct_round_trip() {
    let host = fresh_host();
    let mut vaults =
        unsafe { Mapping::<Address, VaultData>::new(StorageKey::from_slot(0), host.clone()) };
    let vault = Address([0x11; 20]);
    let alice = Address([0xAA; 20]);
    let bob = Address([0xBB; 20]);

    // Write through the storage-typed mutable view.
    {
        let mut v = vaults.view_mut(&vault);
        v.total_shares.set(&U256::from(1_000_000));
        v.shares.insert(&alice, &U256::from(700_000));
        v.shares.insert(&bob, &U256::from(300_000));
        v.last_deposit.insert(&alice, &U256::from(42));
    }

    // Read through the storage-typed immutable view.
    {
        let v = vaults.view(&vault);
        assert_eq!(v.total_shares.get(), U256::from(1_000_000));
        assert_eq!(v.shares.get(&alice), U256::from(700_000));
        assert_eq!(v.shares.get(&bob), U256::from(300_000));
        assert_eq!(v.last_deposit.get(&alice), U256::from(42));
    }
}

#[test]
fn mapping_of_storage_struct_independent_outer_keys() {
    let host = fresh_host();
    let mut vaults =
        unsafe { Mapping::<Address, VaultData>::new(StorageKey::from_slot(0), host.clone()) };
    let vault_a = Address([0x11; 20]);
    let vault_b = Address([0x22; 20]);
    let user = Address([0xCC; 20]);

    vaults
        .view_mut(&vault_a)
        .shares
        .insert(&user, &U256::from(111));
    vaults
        .view_mut(&vault_b)
        .shares
        .insert(&user, &U256::from(222));

    assert_eq!(vaults.view(&vault_a).shares.get(&user), U256::from(111));
    assert_eq!(vaults.view(&vault_b).shares.get(&user), U256::from(222));
}

#[test]
fn mapping_of_storage_struct_layout_matches_solc_derivation() {
    let host = fresh_host();
    let mut vaults =
        unsafe { Mapping::<Address, VaultData>::new(StorageKey::from_slot(0), host.clone()) };
    let vault = Address([0x11; 20]);

    vaults
        .view_mut(&vault)
        .total_shares
        .set(&U256::from(99_999));

    // solc-equivalent derivation: the struct lives at derived =
    // keccak256(pad32(vault) ++ pad32(0)), and total_shares (field 0)
    // occupies `derived + 0` (i.e. derived itself). Independently
    // constructing a Lazy<U256> at that key should read the same value.
    let derived = vaults.slot_of(&vault);
    let total_shares_via_raw = unsafe { Lazy::<U256>::new(derived, 0, host.clone()) };
    assert_eq!(total_shares_via_raw.get(), U256::from(99_999));
}

// ---------------------------------------------------------------------------
// 2. Mapping<K, Lazy<U256>> — the simplest storage-typed value
// ---------------------------------------------------------------------------

#[test]
fn mapping_of_lazy_round_trip() {
    let host = fresh_host();
    let mut m = unsafe { Mapping::<u64, Lazy<U256>>::new(StorageKey::from_slot(0), host.clone()) };

    // Storage-typed map: write/read via view_mut/view returning Ref<Lazy<U256>>
    m.view_mut(&1u64).set(&U256::from(42));
    m.view_mut(&2u64).set(&U256::from(7));
    assert_eq!(m.view(&1u64).get(), U256::from(42));
    assert_eq!(m.view(&2u64).get(), U256::from(7));
}

#[test]
fn mapping_of_lazy_subword_matches_canonical_offset() {
    // `Mapping<u64, Lazy<u128>>` — sub-word V with the canonical-offset fix
    // applied at the inner level. The Lazy<u128>::new_at constructor places
    // the u128 at byte 16 of the derived slot, matching solc's
    // `mapping(uint64 => uint128)`.
    let host = fresh_host();
    let mut m = unsafe { Mapping::<u64, Lazy<u128>>::new(StorageKey::from_slot(0), host.clone()) };
    let v: u128 = 0xCAFE_BABE_DEAD_BEEFu128;
    m.view_mut(&1u64).set(&v);

    assert_eq!(m.view(&1u64).get(), v);

    // Wire-level check: the value lives at canonical offset 16 of derived slot.
    let derived = m.slot_of(&1u64);
    let bytes = raw_slot(&host, &derived);
    assert_eq!(
        &bytes[16..32],
        &v.to_be_bytes(),
        "u128 right-aligned at solc canonical offset",
    );
    assert!(
        bytes[..16].iter().all(|&b| b == 0),
        "bytes above canonical are zero (no neighbour in a Mapping entry)",
    );
}

// ---------------------------------------------------------------------------
// 3. Nested storage-typed mappings via the generalized impl
//    (subsumes the previously-hand-rolled Mapping<K1, Mapping<K2, V>> special case)
// ---------------------------------------------------------------------------

#[test]
fn nested_mapping_via_generalized_storage_typed_impl() {
    let host = fresh_host();
    let mut allowances = unsafe {
        Mapping::<Address, Mapping<Address, U256>>::new(StorageKey::from_slot(2), host.clone())
    };
    let owner = Address([0xAA; 20]);
    let spender = Address([0xBB; 20]);

    // `view_mut(owner)` → RefMut<Mapping<Address, U256>>, then .insert via DerefMut.
    allowances
        .view_mut(&owner)
        .insert(&spender, &U256::from(500));

    // Read via `view(owner)` → Ref<Mapping<Address, U256>>, then .get via Deref.
    assert_eq!(allowances.view(&owner).get(&spender), U256::from(500));
}

#[test]
fn nested_storage_typed_mapping_independent_outer_keys() {
    let host = fresh_host();
    let mut m = unsafe {
        Mapping::<Address, Mapping<Address, U256>>::new(StorageKey::from_slot(0), host.clone())
    };
    let owner_a = Address([0x11; 20]);
    let owner_b = Address([0x22; 20]);
    let spender = Address([0xCC; 20]);

    m.view_mut(&owner_a).insert(&spender, &U256::from(100));
    m.view_mut(&owner_b).insert(&spender, &U256::from(200));

    assert_eq!(m.view(&owner_a).get(&spender), U256::from(100));
    assert_eq!(m.view(&owner_b).get(&spender), U256::from(200));
}

// ---------------------------------------------------------------------------
// 4. Two layers of storage-typed indirection via a containing #[storage]
//    struct. Exercises BOTH the runtime composition AND the abi-gen layout
//    emit path (which needs StorageTypeName to resolve VaultData's name).
// ---------------------------------------------------------------------------

#[pvm_contract_sdk::storage]
pub struct OuterRegistry {
    pub by_owner: Mapping<Address, VaultData>,
}

#[test]
fn outer_storage_struct_holding_mapping_of_storage_struct_runtime() {
    let host = fresh_host();
    let mut outer = <OuterRegistry as StorageComponent>::new_at(
        StorageKey::from_slot(0),
        0,
        true,
        host.clone(),
    );

    let vault = Address([0x11; 20]);
    let alice = Address([0xAA; 20]);

    // Two layers of indirection: outer.by_owner is a Mapping; its value is
    // VaultData which itself contains a `shares` Mapping. End-to-end write
    // and read demonstrate the disjoint-impl storage-typed path composes
    // cleanly at any depth.
    outer
        .by_owner
        .view_mut(&vault)
        .shares
        .insert(&alice, &U256::from(31_415));

    assert_eq!(
        outer.by_owner.view(&vault).shares.get(&alice),
        U256::from(31_415),
    );
}

/// Layout JSON for `Mapping<K, MyStorageStruct>` resolves V's type-name
/// via `<MyStorageStruct as StorageTypeName>::name()` (emitted by the
/// `#[storage]` derive). Without this, the layout-emit codegen falls
/// through to `<MyStorageStruct as SolEncode>::SOL_NAME` and fails to
/// compile (the struct doesn't implement `SolEncode`).
///
/// `#[contract]`-level test contracts emit a `__storage_layout_json()`
/// accessor; we declare a tiny one here and assert the expected JSON
/// shape for `Mapping<Address, VaultData>`.
#[cfg(feature = "abi-gen")]
#[allow(dead_code)]
#[pvm_contract_macros::contract(no_main)]
mod layout_contract {
    use super::*;

    pub struct Registry {
        pub by_owner: Mapping<Address, VaultData>,
    }

    impl Registry {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }
}

#[cfg(feature = "abi-gen")]
#[test]
fn mapping_of_storage_struct_layout_json_names_value_struct() {
    let layout = layout_contract::__storage_layout_json();
    // The single field `by_owner` is a Mapping<Address, VaultData>. The
    // layout entry must record:
    //   - label  = "by_owner"
    //   - slot   = "0"
    //   - offset = 0
    //   - type   = "mapping(address => VaultData)"
    // The value-type name "VaultData" comes from the `#[storage]`-derived
    // `impl StorageTypeName for VaultData { const NAME = "VaultData"; }`.
    let parsed: serde_json::Value = serde_json::from_str(&layout).unwrap();
    let entries = parsed["storage"].as_array().unwrap();
    assert_eq!(entries.len(), 1, "single field expected: {layout}");
    let e = &entries[0];
    assert_eq!(e["label"], "by_owner");
    assert_eq!(e["slot"], "0");
    assert_eq!(e["offset"], 0);
    assert_eq!(
        e["type"], "mapping(address => VaultData)",
        "storage-typed mapping value resolved via StorageTypeName: {layout}",
    );
}

// ---------------------------------------------------------------------------
// `Mapping<K, V: StorageComponent>::delete(&K)` — Solidity-style
//     `delete mapping[key];` for struct-valued maps. Calls V::clear(),
//     which recurses through every field of a #[storage] sub-struct.
// ---------------------------------------------------------------------------

#[test]
fn delete_struct_entry_clears_all_lazy_fields() {
    let host = fresh_host();
    let mut vaults =
        unsafe { Mapping::<Address, VaultData>::new(StorageKey::from_slot(0), host.clone()) };
    let vault = Address([0x11; 20]);
    let alice = Address([0xAA; 20]);

    // Set up: one Lazy field + two Mapping entries.
    {
        let mut v = vaults.view_mut(&vault);
        v.total_shares.set(&U256::from(1_000_000));
        v.shares.insert(&alice, &U256::from(700_000));
        v.last_deposit.insert(&alice, &U256::from(42));
    }

    // Delete the entry. Matches solc's `delete vaults[vault]`:
    //  - total_shares (a Lazy<U256> = value-shaped) → cleared
    //  - shares (a Mapping) → no-op (entries remain at their derived keys)
    //  - last_deposit (a Mapping) → no-op
    vaults.delete(&vault);

    // Lazy field is cleared.
    assert_eq!(
        vaults.view(&vault).total_shares.get(),
        U256::ZERO,
        "Lazy field cleared by delete",
    );
    // Sub-mapping entries remain — solc behaviour, no enumeration available.
    assert_eq!(
        vaults.view(&vault).shares.get(&alice),
        U256::from(700_000),
        "sub-mapping entries survive (solc-compatible)",
    );
    assert_eq!(
        vaults.view(&vault).last_deposit.get(&alice),
        U256::from(42),
        "sub-mapping entries survive (solc-compatible)",
    );
}

#[test]
fn delete_lazy_entry_clears_the_slot() {
    let host = fresh_host();
    let mut m = unsafe { Mapping::<u64, Lazy<U256>>::new(StorageKey::from_slot(0), host.clone()) };
    m.view_mut(&1u64).set(&U256::from(42));
    assert_eq!(m.view(&1u64).get(), U256::from(42));

    m.delete(&1u64);
    // Slot is auto-deleted by `set_storage_or_clear` on all-zero write.
    let raw = raw_slot(&host, &m.slot_of(&1u64));
    assert_eq!(raw, [0u8; 32], "Lazy<U256> entry's slot is zeroed");
    assert_eq!(m.view(&1u64).get(), U256::ZERO);
}

#[test]
fn delete_lazy_subword_entry_clears_the_canonical_window() {
    let host = fresh_host();
    let mut m = unsafe { Mapping::<u64, Lazy<u128>>::new(StorageKey::from_slot(0), host.clone()) };
    let v: u128 = 0xCAFE_BABE_DEAD_BEEFu128;
    m.view_mut(&1u64).set(&v);
    assert_eq!(m.view(&1u64).get(), v);

    m.delete(&1u64);
    let raw = raw_slot(&host, &m.slot_of(&1u64));
    assert_eq!(
        raw, [0u8; 32],
        "no-neighbour sub-word entry auto-deletes the slot on clear",
    );
    assert_eq!(m.view(&1u64).get(), 0u128);
}

#[test]
fn delete_nested_mapping_entry_is_noop_matching_solc() {
    let host = fresh_host();
    let mut m = unsafe {
        Mapping::<Address, Mapping<Address, U256>>::new(StorageKey::from_slot(0), host.clone())
    };
    let owner = Address([0xAA; 20]);
    let spender = Address([0xBB; 20]);

    m.view_mut(&owner).insert(&spender, &U256::from(500));

    // `delete m[owner]` for a nested mapping: solc cannot enumerate inner
    // keys, so the entry persists. Our Mapping::clear is a no-op,
    // delete propagates that.
    m.delete(&owner);

    assert_eq!(
        m.view(&owner).get(&spender),
        U256::from(500),
        "nested-mapping entries are NOT cleared by delete on the outer key (matches solc)",
    );
}

#[test]
fn delete_then_overwrite_storage_struct_entry() {
    let host = fresh_host();
    let mut vaults =
        unsafe { Mapping::<Address, VaultData>::new(StorageKey::from_slot(0), host.clone()) };
    let vault = Address([0x11; 20]);

    vaults.view_mut(&vault).total_shares.set(&U256::from(999));
    vaults.delete(&vault);
    vaults.view_mut(&vault).total_shares.set(&U256::from(7));

    assert_eq!(vaults.view(&vault).total_shares.get(), U256::from(7));
}

// ---------------------------------------------------------------------------
// Type-alias resolution through `StorageTypeName`
//
// The macro names every field uniformly via `<#ty as StorageTypeName>::name()`.
// For a type *alias* the syntactic ident is the alias name (e.g. "Balances",
// not "Mapping"), so resolution relies entirely on the explicit `StorageTypeName`
// impls on `Lazy<T>` / `Mapping<K, V>` in `pvm-storage`. Since there is no
// blanket `StorageTypeName` impl, those explicit impls are what make this
// resolve; without them codegen would fail.
//
// These tests pin the alias-resolution path so that the impls live forever.
// ---------------------------------------------------------------------------

#[cfg(feature = "abi-gen")]
type Balances = Mapping<Address, U256>;
#[cfg(feature = "abi-gen")]
type Counter = Lazy<U256>;
#[cfg(feature = "abi-gen")]
type Nested = Mapping<Address, Mapping<Address, U256>>;

#[cfg(feature = "abi-gen")]
#[allow(dead_code)]
#[pvm_contract_macros::contract(no_main)]
mod aliased_layout_contract {
    use super::*;
    // Pin the alias inside the macro's view so codegen sees only the
    // `Balances` / `Counter` / `Nested` idents at field positions.
    pub struct Registry {
        pub counter: Counter,
        pub balances: Balances,
        pub allowances: Nested,
    }

    impl Registry {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }
}

// Aliased *sub-word* fields. These exercise the regression that motivated
// unifying layout emission onto `StorageLayoutEmit`: because the field types
// are aliases (`PackedU32` / `PackedFlag`, not the literal `Lazy<…>` idents),
// they take the trait dispatch path rather than any syntactic leaf shortcut.
// The trait must therefore carry the packed byte `offset` — a previous version
// hardcoded `offset: 0` on the `Lazy<T>` impl, which silently flattened
// aliased packed fields. solc layout for `{ uint32 a; bool b; }` (`offset`
// counted from the least-significant byte):
//   a (uint32) → slot 0, offset 0   (right-aligned, low 4 bytes)
//   b (bool)   → slot 0, offset 4   (packed directly above a)
#[cfg(feature = "abi-gen")]
type PackedU32 = Lazy<u32>;
#[cfg(feature = "abi-gen")]
type PackedFlag = Lazy<bool>;

#[cfg(feature = "abi-gen")]
#[allow(dead_code)]
#[pvm_contract_macros::contract(no_main)]
mod aliased_packed_contract {
    use super::*;
    pub struct Packed {
        pub a: PackedU32,
        pub b: PackedFlag,
    }

    impl Packed {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }
}

#[cfg(feature = "abi-gen")]
#[test]
fn aliased_sub_word_fields_carry_packed_offset_through_trait() {
    let actual: serde_json::Value =
        serde_json::from_str(&aliased_packed_contract::__storage_layout_json()).unwrap();
    let expected: serde_json::Value = serde_json::json!({
        "storage": [
            { "label": "a", "offset": 0, "slot": "0", "type": "uint32" },
            { "label": "b", "offset": 4, "slot": "0", "type": "bool" },
        ]
    });
    assert_eq!(actual, expected);
}

#[cfg(feature = "abi-gen")]
#[test]
fn type_alias_resolution_for_lazy_and_mapping_in_layout_json() {
    let layout = aliased_layout_contract::__storage_layout_json();
    let parsed: serde_json::Value = serde_json::from_str(&layout).unwrap();
    let entries = parsed["storage"].as_array().unwrap();
    assert_eq!(entries.len(), 3, "three aliased fields: {layout}");

    // `Counter = Lazy<U256>` → `Lazy<T>`'s `StorageTypeName::name()`
    // unwraps to `T::name()` = "uint256".
    assert_eq!(entries[0]["label"], "counter");
    assert_eq!(entries[0]["type"], "uint256");

    // `Balances = Mapping<Address, U256>` → `Mapping<K, V>::name()`
    // returns `mapping(K_name => V_name)`.
    assert_eq!(entries[1]["label"], "balances");
    assert_eq!(entries[1]["type"], "mapping(address => uint256)");

    // `Nested = Mapping<Address, Mapping<Address, U256>>` → inner Mapping
    // resolves recursively via the same impl.
    assert_eq!(entries[2]["label"], "allowances");
    assert_eq!(
        entries[2]["type"],
        "mapping(address => mapping(address => uint256))",
    );
}

// ---------------------------------------------------------------------------
// Layout JSON for value-shaped `#[derive(SolType, SolStorage)]` structs.
//
// A struct deriving `SolType + SolStorage` is a value-shaped storage element:
// it lives at a fixed slot range (like a primitive) but carries multiple
// fields. When used as `Lazy<S>` or `Mapping<_, S>`, the storage layout JSON
// must name it the same way solc does — by the Rust ident (≈ Solidity
// struct name), not by the ABI tuple notation that `SolEncode::SOL_NAME`
// produces (`"(uint64,uint64)"`).
//
// This is a parity test against the existing `#[storage]` attribute path
// (one section above) which correctly emits `"VaultData"` for the value
// type. The same shape must hold for `#[derive(SolStorage)]` — anything
// else would mean two storage-eligible structs in the same JSON come out
// with inconsistent type-name conventions, breaking downstream tooling
// that reads the `"type"` field.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq, pvm_contract_sdk::SolType, pvm_contract_sdk::SolStorage)]
pub struct PackedPoint {
    pub x: u64,
    pub y: u64,
}

#[cfg(feature = "abi-gen")]
#[allow(dead_code)]
#[pvm_contract_macros::contract(no_main)]
mod sol_storage_layout_contract {
    use super::*;

    pub struct PointRegistry {
        pub origin: Lazy<PackedPoint>,
        pub by_id: Mapping<u64, PackedPoint>,
    }

    impl PointRegistry {
        #[pvm_contract_macros::constructor]
        pub fn constructor(&mut self) {}
    }
}

#[cfg(feature = "abi-gen")]
#[test]
fn sol_storage_value_struct_uses_struct_name_in_layout_json() {
    let layout = sol_storage_layout_contract::__storage_layout_json();
    let parsed: serde_json::Value = serde_json::from_str(&layout).unwrap();
    let entries = parsed["storage"].as_array().unwrap();

    // Find each field's entry (order isn't guaranteed by the JSON spec).
    let origin = entries
        .iter()
        .find(|e| e["label"] == "origin")
        .expect("origin entry");
    let by_id = entries
        .iter()
        .find(|e| e["label"] == "by_id")
        .expect("by_id entry");

    // `Lazy<PackedPoint>` — value-type name must be the Rust ident
    // (matches how `#[storage]` sub-structs render via StorageTypeName).
    // The `SolStorage` derive emits a `StorageTypeName` impl returning the
    // ident; without it the type would render as the ABI tuple
    // `"(uint64,uint64)"`.
    assert_eq!(
        origin["type"], "PackedPoint",
        "Lazy<PackedPoint> should report struct ident, not ABI tuple notation. Got layout: {layout}",
    );

    // `Mapping<u64, PackedPoint>` — value-type name embedded in mapping
    // notation, again as the struct ident.
    assert_eq!(
        by_id["type"], "mapping(uint64 => PackedPoint)",
        "Mapping<_, PackedPoint> should embed struct ident, not ABI tuple. Got layout: {layout}",
    );
}

//! End-to-end test for the new storage composition surface:
//!
//! - Auto-numbered slots on the `#[contract]` struct (no `#[slot(N)]` needed).
//! - `#[storage]`-derived embedded sub-storage struct claiming a contiguous
//!   slot range.
//! - `StorageComponent::SLOTS` chained through both levels so the outer
//!   contract gets the correct overall layout without manual offset math.
//! - Dynamic `String` value stored via native `Lazy<String>` (Gap 1).
//!
//! The contract module uses `#[contract(no_main)]` so cargo's integration-test
//! harness keeps its own `main`. The abi-gen `__abi_json()` /
//! `__storage_layout_json()` accessors are still emitted under
//! `#[cfg(feature = "abi-gen")]` and exercised by
//! `composed_contract_emits_storage_layout_under_abi_gen` below.

use pvm_contract_sdk::{Lazy, Mapping, StorageComponent, StorageKey};
use pvm_contract_types::{Address, Host, MockHostBuilder};
use ruint::aliases::U256;

/// A composable sub-storage struct. After `#[storage]` expansion, this type
/// implements `StorageComponent` with `SLOTS = 3` (one for each `Lazy` /
/// `Mapping`).
#[pvm_contract_sdk::storage]
pub struct Erc20State {
    pub total_supply: Lazy<U256>,
    pub balances: Mapping<Address, U256>,
    pub allowances: Mapping<Address, Mapping<Address, U256>>,
}

/// A second sub-storage struct using native dynamic `Lazy<String>`. Still
/// 1 slot per field because the header slot stores length and the body
/// lives at `keccak256(slot)`.
#[pvm_contract_sdk::storage]
pub struct MetadataState {
    pub name: Lazy<alloc::string::String>,
    pub symbol: Lazy<alloc::string::String>,
}

// Pull `alloc` into the test crate for the `String` type.
extern crate alloc;

/// SLOTS counts at the type level should match what we expect.
#[test]
fn storage_component_slots_are_field_sums() {
    assert_eq!(<Erc20State as StorageComponent>::SLOTS, 3);
    assert_eq!(<MetadataState as StorageComponent>::SLOTS, 2);
    assert_eq!(<Lazy<U256> as StorageComponent>::SLOTS, 1);
    assert_eq!(<Mapping<Address, U256> as StorageComponent>::SLOTS, 1);
}

fn fresh_host() -> Host {
    Host::from_dyn(alloc::rc::Rc::new(MockHostBuilder::new().build()))
}

/// Constructing an `Erc20State` via `StorageComponent::new_at(base, host)`
/// produces fields rooted at `base, base+1, base+2` — exactly the layout
/// `cast storage` will expect.
#[test]
fn erc20_state_new_at_assigns_contiguous_slots() {
    let host = fresh_host();
    let state = <Erc20State as StorageComponent>::new_at(5, 0, host.clone());

    // Mint to alice via balance map insert; that should write at the slot
    // derived from `keccak256(pad32(alice) ++ pad32(6))` because balances is
    // the *second* field of Erc20State and we passed base=5 so it claims slot 6.
    let alice = Address([0xAA; 20]);
    let mut state = state;
    state.balances.insert(&alice, &U256::from(1_000));

    // Cross-check: a standalone Mapping rooted at slot 6 should see the same
    // entry.
    let independent = unsafe { Mapping::<Address, U256>::new(StorageKey::from_slot(6), host) };
    assert_eq!(independent.get(&alice), U256::from(1_000));
}

/// Auto-numbered storage on a `#[contract]` struct that embeds an
/// `#[storage]`-derived sub-struct.
///
/// Layout produced by the macro:
///
///   slot 0   = erc20.total_supply
///   slot 1   = erc20.balances     (mapping root)
///   slot 2   = erc20.allowances   (mapping root)
///   slot 3   = metadata.name
///   slot 4   = metadata.symbol
///   slot 5   = paused
///
/// The `#[contract]` macro never sees `Erc20State::SLOTS = 3`; it just
/// references the const at codegen time so the chain `0 + 3 + 2 = 5` is
/// resolved at compile time.
#[allow(dead_code)] // route(), deploy(), call() are riscv64-gated
#[pvm_contract_macros::contract(no_main)]
mod composed_contract {
    use super::*;

    pub struct ComposedContract {
        pub erc20: Erc20State,
        pub metadata: MetadataState,
        pub paused: Lazy<bool>,
    }

    impl ComposedContract {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {
            self.erc20.total_supply.set(&U256::from(1_000_000));
            self.metadata.name.set(&"Composed Token".to_string());
            self.metadata.symbol.set(&"CMP".to_string());
            self.paused.set(&false);
        }

        #[pvm_contract_macros::method]
        pub fn balance_of(&self, who: Address) -> U256 {
            self.erc20.balances.get(&who)
        }

        #[pvm_contract_macros::method]
        pub fn total_supply(&self) -> U256 {
            self.erc20.total_supply.get()
        }
    }
}

/// Cross-check the composed layout by deriving the expected slot for each
/// embedded field via two independent paths:
///
/// 1. Construct the embedded type directly with the offset that the macro
///    would have assigned (3 for `metadata`, 5 for `paused`).
/// 2. Construct the outer contract's storage layout *by hand* using the
///    same `StorageComponent::new_at` calls the macro generates, write a
///    distinctive value through one field, and read it back through the
///    standalone path.
///
/// If they agree, the macro's auto-numbering matches the trait's slot
/// arithmetic at the type level.
#[test]
fn composed_contract_layout_matches_hand_constructed() {
    let host = fresh_host();
    let mut erc20 = <Erc20State as StorageComponent>::new_at(0, 0, host.clone());
    let mut metadata = <MetadataState as StorageComponent>::new_at(3, 0, host.clone());
    // Phase 3 walker: a standalone `Lazy<bool>` field at the start of a
    // fresh slot lands at canonical offset 31 (right-aligned, 1 byte wide).
    // The macro would have called `new_at(5, 31, host)` for this field.
    let mut paused = <Lazy<bool> as StorageComponent>::new_at(5, 31, host.clone());

    // Write through each sub-component.
    let alice = Address([0xAA; 20]);
    erc20.total_supply.set(&U256::from(1_000));
    erc20.balances.insert(&alice, &U256::from(42));
    metadata.name.set(&"Composed".to_string());
    metadata.symbol.set(&"CMP".to_string());
    paused.set(&true);

    // Read back via slot-pinned standalone helpers and confirm the slot
    // arithmetic from `#[storage]` matches our manual `new_at` calls above.
    let supply_slot = unsafe { Lazy::<U256>::new(StorageKey::from_slot(0), 0, host.clone()) };
    assert_eq!(supply_slot.get(), U256::from(1_000));

    let balances_slot =
        unsafe { Mapping::<Address, U256>::new(StorageKey::from_slot(1), host.clone()) };
    assert_eq!(balances_slot.get(&alice), U256::from(42));

    // `allowances` claims slot 2 — write/read via two paths to confirm.
    let mut allowances_via_outer = unsafe {
        <Mapping<Address, Mapping<Address, U256>>>::new(StorageKey::from_slot(2), host.clone())
    };
    let bob = Address([0xBB; 20]);
    allowances_via_outer
        .entry(&alice)
        .insert(&bob, &U256::from(7));
    assert_eq!(erc20.allowances.get(&alice).get(&bob), U256::from(7));

    let name_slot =
        unsafe { Lazy::<alloc::string::String>::new(StorageKey::from_slot(3), 0, host.clone()) };
    assert_eq!(name_slot.get(), "Composed");

    let symbol_slot =
        unsafe { Lazy::<alloc::string::String>::new(StorageKey::from_slot(4), 0, host.clone()) };
    assert_eq!(symbol_slot.get(), "CMP");

    // `paused` was written via the macro-built `Lazy<bool>` which gets
    // canonical offset 31; mirror that here.
    let paused_slot = unsafe { Lazy::<bool>::new(StorageKey::from_slot(5), 31, host) };
    assert!(paused_slot.get());
}

/// Nesting: a `#[storage]` struct that itself contains another `#[storage]`
/// struct sums correctly.
#[pvm_contract_sdk::storage]
pub struct OuterState {
    pub flag: Lazy<bool>,
    pub erc20: Erc20State, // 3 slots
}

#[test]
fn nested_storage_struct_slot_sum() {
    assert_eq!(<OuterState as StorageComponent>::SLOTS, 4);
}

#[test]
fn nested_storage_struct_uses_offset() {
    let host = fresh_host();
    let mut outer = <OuterState as StorageComponent>::new_at(10, 0, host.clone());

    // OuterState at base 10 places `flag` at slot 10 and `erc20` starting at
    // slot 11 (because flag claims 1 slot).
    outer.flag.set(&true);
    outer.erc20.total_supply.set(&U256::from(999));

    // Phase 3: the walker emits canonical offsets for sub-32-byte primitives
    // (right-aligned, matching solc). `bool` has PACKED_BYTES = 1 so its
    // canonical offset is 31. Manual `Lazy::new` constructions must mirror
    // that — otherwise we'd read byte 0 instead of byte 31.
    let flag_check = unsafe { Lazy::<bool>::new(StorageKey::from_slot(10), 31, host.clone()) };
    assert!(flag_check.get());

    // `U256` is full-slot, canonical offset = 0.
    let supply_check = unsafe { Lazy::<U256>::new(StorageKey::from_slot(11), 0, host) };
    assert_eq!(supply_check.get(), U256::from(999));
}

/// Under `--features abi-gen`, the `#[contract]` macro emits
/// `__abi_json()` / `__storage_layout_json()` accessors on the module. With
/// `no_main`, the integration-test harness's own `main` is preserved and
/// these accessors are reachable from a `#[test]` fn. Verifies that
/// embedded `#[storage]` sub-structs flatten into the layout JSON with
/// dotted labels (`erc20.total_supply`, `metadata.name`, …) via
/// `StorageLayoutEmit::emit_entries`.
#[cfg(feature = "abi-gen")]
#[test]
fn composed_contract_emits_storage_layout_under_abi_gen() {
    let actual: serde_json::Value =
        serde_json::from_str(&composed_contract::__storage_layout_json()).unwrap();
    let golden: serde_json::Value = serde_json::from_str(include_str!(
        "test_abi_contract/abi_storage_layout_composed.json"
    ))
    .unwrap();
    assert_eq!(actual, golden["storageLayout"]);
}

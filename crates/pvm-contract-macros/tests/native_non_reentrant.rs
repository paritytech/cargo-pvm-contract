#![cfg(not(feature = "abi-gen"))]
//! Native unit tests for the `#[non_reentrant]` modifier against `MockHost`.
//!
//! These drive the generated `route()` directly. The reentrancy lock lives at a
//! fixed namespaced storage key; we simulate "a guarded section is in progress"
//! by pre-setting that slot in the mock, and assert the guarded method reverts
//! with the OZ-compatible `ReentrancyGuardReentrantCall` selector. We also check
//! the happy path leaves the lock cleared, across every `&mut self` body shape.

use pvm_contract_types::{
    HostApi, MockHost, MockHostBuilder, ReturnFlags, StorageFlags, const_keccak256, const_selector,
};

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract]
mod guarded {
    pub struct Guarded;

    impl Guarded {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        // Full guard, `Result<T>` body shape.
        #[pvm_contract_macros::method]
        #[pvm_contract_macros::non_reentrant]
        pub fn guarded_result(&mut self) -> Result<u64, pvm_contract_sdk::EmptyError> {
            Ok(7)
        }

        // Full guard, `Result<()>` body shape.
        #[pvm_contract_macros::method]
        #[pvm_contract_macros::non_reentrant]
        pub fn guarded_unit(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        // Full guard, plain (non-Result) value body shape.
        #[pvm_contract_macros::method]
        #[pvm_contract_macros::non_reentrant]
        pub fn guarded_plain(&mut self) -> u64 {
            9
        }

        // Read-only check (`nonReentrantView`).
        #[pvm_contract_macros::method]
        #[pvm_contract_macros::non_reentrant]
        pub fn guarded_view(&self) -> u64 {
            5
        }

        // Unguarded control.
        #[pvm_contract_macros::method]
        pub fn plain(&mut self) -> u64 {
            1
        }
    }
}

const REENTRANCY_KEY: [u8; 32] = const_keccak256(b"pvm.guards.reentrancy");

fn new_contract() -> (guarded::Guarded, MockHost) {
    let mock = MockHostBuilder::new().build();
    let contract = guarded::Guarded::with_host(mock.clone());
    (contract, mock)
}

fn set_lock(mock: &MockHost) {
    mock.set_storage_or_clear(StorageFlags::empty(), &REENTRANCY_KEY, &[1u8; 32]);
}

fn lock_is_set(mock: &MockHost) -> bool {
    let mut buf = [0u8; 32];
    mock.get_storage_or_zero(StorageFlags::empty(), &REENTRANCY_KEY, &mut buf);
    buf != [0u8; 32]
}

fn reentrancy_selector() -> [u8; 4] {
    const_selector("ReentrancyGuardReentrantCall()")
}

fn assert_reverted_with_reentrancy(mock: &MockHost, ctx: &str) {
    let rv = mock.take_return_value().expect("return_value called");
    assert_eq!(rv.flags, ReturnFlags::REVERT, "{ctx}: expected a revert");
    assert_eq!(
        &rv.data[..4],
        &reentrancy_selector(),
        "{ctx}: expected ReentrancyGuardReentrantCall selector"
    );
}

/// Every guarded method: the full-guard `&mut self` body shapes (`Result<()>`,
/// `Result<T>`, plain) and the `&self` view check.
const GUARDED: &[&str] = &[
    "guardedResult()",
    "guardedUnit()",
    "guardedPlain()",
    "guardedView()",
];

#[test]
fn guarded_methods_revert_when_lock_held() {
    for &sig in GUARDED {
        let (mut contract, mock) = new_contract();
        set_lock(&mock);
        guarded::route(&mut contract, const_selector(sig), &[]);
        assert_reverted_with_reentrancy(&mock, sig);
    }
}

#[test]
fn guarded_methods_succeed_and_leave_lock_clear_when_unlocked() {
    for &sig in GUARDED {
        let (mut contract, mock) = new_contract();
        guarded::route(&mut contract, const_selector(sig), &[]);
        let rv = mock.take_return_value().expect("return_value called");
        assert_eq!(rv.flags, ReturnFlags::empty(), "{sig} should succeed");
        // Full guard sets-then-clears; the view check never writes — either way
        // the lock must be absent afterwards.
        assert!(!lock_is_set(&mock), "{sig} must leave the lock clear");
    }
}

#[test]
fn unguarded_method_ignores_lock() {
    let (mut contract, mock) = new_contract();
    set_lock(&mock);
    guarded::route(&mut contract, const_selector("plain()"), &[]);
    let rv = mock.take_return_value().expect("return_value called");
    assert_eq!(
        rv.flags,
        ReturnFlags::empty(),
        "unguarded method should run"
    );
}

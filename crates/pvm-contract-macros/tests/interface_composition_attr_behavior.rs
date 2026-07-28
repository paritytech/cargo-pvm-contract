#![cfg(not(feature = "abi-gen"))]
//! Per-method attributes on a *folded* interface method behave at runtime exactly
//! as on an inherent `#[method]`. The parse-level tests in `contract.rs` prove the
//! attributes are read into the folded `MethodInfo`; these drive the generated
//! `route()` against a `MockHost` to prove the emitted guards actually fire.
//!
//! - `#[payable]` on the impl fn: the folded method accepts value, while a
//!   non-payable folded sibling reverts on a value transfer.
//! - `#[non_reentrant]` on the impl fn: the folded method reverts with the
//!   OZ-compatible `ReentrancyGuardReentrantCall` when the lock is held, and
//!   otherwise runs and leaves the lock clear.

use pvm_contract_types::{
    HostApi, MockHost, MockHostBuilder, OutSink, Outcome, ReturnFlags, SolDecode, StorageFlags,
    const_keccak256, const_selector,
};

fn selector(sig: &str) -> [u8; 4] {
    const_selector(sig)
}

// ----------------------------------------------------------------------------
// #[payable] on a folded method
// ----------------------------------------------------------------------------

pub trait IVault {
    // Payability isn't part of the Rust signature, so the trait can't carry it;
    // `#[payable]` goes on the impl fn (mirroring the inherent path).
    fn deposit(&mut self) -> u64;
    fn poke(&mut self) -> u64;
}

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract(implements(IVault))]
mod vault {
    use super::IVault;

    pub struct V;

    impl V {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl IVault for V {
        #[pvm_contract_macros::payable]
        fn deposit(&mut self) -> u64 {
            1
        }
        fn poke(&mut self) -> u64 {
            2
        }
    }
}

#[test]
fn folded_payable_accepts_value_and_non_payable_sibling_rejects_it() {
    // A non-zero value transfer against a contract with mixed folded payability.
    let mock = MockHostBuilder::new().value_transferred([0x11; 32]).build();
    let mut contract = vault::V::with_host(mock.clone());
    let mut buf = [0u8; vault::MAX_RETURN_LEN];

    // The `#[payable]` folded method accepts the value and returns normally.
    let mut out: &mut [u8] = &mut buf;
    let outcome = vault::route(&mut contract, selector("deposit()"), &[], &mut out);
    let Outcome::Return(n) = outcome else {
        panic!("payable folded method should accept value, got {outcome:?}");
    };
    assert_eq!(u64::decode(out.view(n)).unwrap(), 1);

    // The non-payable folded sibling reverts with the framework's
    // `NonPayableValueReceived` selector via the per-arm value guard.
    let mut out: &mut [u8] = &mut buf;
    let rv = mock.expect_revert(|| {
        vault::route(&mut contract, selector("poke()"), &[], &mut out);
    });
    assert_eq!(rv.flags, ReturnFlags::REVERT);
    assert_eq!(
        rv.data.as_slice(),
        &pvm_contract_types::framework_errors::NON_PAYABLE_VALUE_RECEIVED[..],
    );
}

#[test]
fn folded_non_payable_accepts_zero_value() {
    // With no value transfer the non-payable folded method runs as usual.
    let mut contract = vault::V::with_host(MockHostBuilder::new().build());
    let mut buf = [0u8; vault::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;
    let outcome = vault::route(&mut contract, selector("poke()"), &[], &mut out);
    let Outcome::Return(n) = outcome else {
        panic!("expected Return, got {outcome:?}");
    };
    assert_eq!(u64::decode(out.view(n)).unwrap(), 2);
}

// ----------------------------------------------------------------------------
// #[non_reentrant] on a folded method
// ----------------------------------------------------------------------------

pub trait IGuarded {
    fn guarded(&mut self) -> u64;
}

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract(implements(IGuarded))]
mod guarded {
    use super::IGuarded;

    pub struct G;

    impl G {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl IGuarded for G {
        #[pvm_contract_macros::non_reentrant]
        fn guarded(&mut self) -> u64 {
            7
        }
    }
}

const REENTRANCY_KEY: [u8; 32] = const_keccak256(b"pvm.guards.reentrancy");

fn lock_is_set(mock: &MockHost) -> bool {
    let mut buf = [0u8; 32];
    mock.get_storage_or_zero(StorageFlags::empty(), &REENTRANCY_KEY, &mut buf);
    buf != [0u8; 32]
}

#[test]
fn folded_non_reentrant_reverts_when_lock_held() {
    let mock = MockHostBuilder::new().build();
    let mut contract = guarded::G::with_host(mock.clone());
    // Simulate "a guarded section is already in progress".
    mock.set_storage_or_clear(StorageFlags::empty(), &REENTRANCY_KEY, &[1u8; 32]);

    let mut buf = [0u8; guarded::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;
    let rv = mock.expect_revert(|| {
        guarded::route(&mut contract, selector("guarded()"), &[], &mut out);
    });
    assert_eq!(rv.flags, ReturnFlags::REVERT);
    assert_eq!(
        &rv.data[..4],
        &const_selector("ReentrancyGuardReentrantCall()"),
    );
}

#[test]
fn folded_non_reentrant_succeeds_and_clears_lock_when_unlocked() {
    let mock = MockHostBuilder::new().build();
    let mut contract = guarded::G::with_host(mock.clone());

    let mut buf = [0u8; guarded::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;
    let outcome = guarded::route(&mut contract, selector("guarded()"), &[], &mut out);
    let Outcome::Return(n) = outcome else {
        panic!("expected Return, got {outcome:?}");
    };
    assert_eq!(u64::decode(out.view(n)).unwrap(), 7);
    // The full guard sets-then-clears the lock across the call.
    assert!(!lock_is_set(&mock), "guard must leave the lock clear");
}

#![cfg(not(feature = "abi-gen"))]
//! Worked example: a minimal EIP-1967 upgradeable proxy.
//!
//! The implementation address lives at the standard pseudo-random slot
//! `keccak256("eip1967.proxy.implementation") - 1`, deliberately outside the
//! compiler-assigned sequential slot range so it never collides with regular
//! fields. It's bound to a typed `Lazy<Address>` field via `#[slot(raw = …)]`,
//! so reads/writes go through the borrow-checker's view-vs-mutating gate just
//! like any other storage field — no raw `host.get_storage`, no unsafe.
//!
//! The EIP-1967 constant lives here in the contract, not the SDK: the SDK only
//! provides the generic `#[slot(raw = KEY)]` mechanism.

use pvm_contract_sdk::{
    Address, Host, Lazy, MockHost, MockHostBuilder, StorageComponent, StorageKey,
};

/// `bytes32(uint256(keccak256("eip1967.proxy.implementation")) - 1)`.
const IMPLEMENTATION_SLOT: [u8; 32] = [
    0x36, 0x08, 0x94, 0xa1, 0x3b, 0xa1, 0xa3, 0x21, 0x06, 0x67, 0xc8, 0x28, 0x49, 0x2d, 0xb9, 0x8d,
    0xca, 0x3e, 0x20, 0x76, 0xcc, 0x37, 0x35, 0xa9, 0x20, 0xa3, 0xca, 0x50, 0x5d, 0x38, 0x2b, 0xbc,
];

#[allow(dead_code)]
#[pvm_contract_sdk::contract]
mod upgradeable_proxy {
    use super::*;

    pub struct UpgradeableProxy {
        #[slot(raw = IMPLEMENTATION_SLOT)]
        pub impl_addr: Lazy<Address>,
    }

    impl UpgradeableProxy {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}

        /// Current implementation address, read from the EIP-1967 slot.
        #[pvm_contract_sdk::method]
        pub fn implementation(&self) -> Address {
            self.impl_addr.get()
        }

        /// Point the proxy at a new implementation contract.
        #[pvm_contract_sdk::method]
        pub fn upgrade_to(&mut self, new_implementation: Address) {
            self.impl_addr.set(&new_implementation);
        }
    }
}

use upgradeable_proxy::UpgradeableProxy;

/// Build an instance the same way the macro's on-chain path would: bind the
/// `Lazy<Address>` field to the raw EIP-1967 slot, right-aligned like solc.
fn proxy() -> (UpgradeableProxy, MockHost) {
    let mock = MockHostBuilder::new().build();
    let host = Host::from_dyn(::std::rc::Rc::new(mock.clone()));
    let contract = UpgradeableProxy {
        impl_addr: <Lazy<Address> as StorageComponent>::new_at(
            StorageKey::from_raw(IMPLEMENTATION_SLOT),
            (32 - <Lazy<Address> as StorageComponent>::PACKED_BYTES) as u8,
            true,
            host.clone(),
        ),
        host,
    };
    (contract, mock)
}

#[test]
fn implementation_defaults_to_zero() {
    let (p, _) = proxy();
    assert_eq!(p.implementation(), Address::from([0u8; 20]));
}

#[test]
fn upgrade_to_then_implementation_roundtrips() {
    let (mut p, _) = proxy();
    let impl_addr = Address::from([0x42; 20]);
    p.upgrade_to(impl_addr);
    assert_eq!(p.implementation(), impl_addr);
}

#[test]
fn upgrade_to_writes_the_eip1967_slot_right_aligned() {
    let (mut p, mock) = proxy();
    let impl_addr = Address::from([0xCD; 20]);
    p.upgrade_to(impl_addr);

    // Confirm the write landed at the EIP-1967 slot, right-aligned like solc
    // (low 20 bytes), so a slot written by a real Solidity proxy would decode.
    let raw = mock
        .get_raw_storage(&IMPLEMENTATION_SLOT)
        .expect("implementation slot was written");
    let mut expected = [0u8; 32];
    expected[12..].copy_from_slice(impl_addr.as_ref() as &[u8; 20]);
    assert_eq!(raw.as_slice(), &expected);
}

#![cfg(not(feature = "abi-gen"))]
//! Method-level unit tests — the simplest and fastest way to exercise
//! contract logic with `MockHost`.
//!
//! Unlike the dispatch-level tests (`native_e2e_token.rs`) that go through
//! `route()` with encoded calldata, these tests **call contract methods
//! directly** on a `Contract<MockHost>` instance. No selector matching, no
//! ABI encode/decode, no revert-data parsing — just typed Rust:
//!
//! ```ignore
//! let mut contract = MiniToken { host };
//! let result = contract.transfer(bob, U256::from(200));
//! assert!(result.is_ok());
//! assert_eq!(contract.balance_of(alice), U256::from(300));
//! ```
//!
//! When to use which layer:
//! - **Method-level** (this file): business logic, revert variants, state
//!   transitions — 99% of unit tests should look like this.
//! - **Dispatch-level** (`native_e2e_token.rs`): selector routing, calldata
//!   parsing, ABI revert encoding — end-to-end verification that the
//!   deployed contract will decode the same bytes Foundry/ethers send.

use pvm_contract_sdk::{Address, Host, MockHost, MockHostBuilder};
use ruint::aliases::U256;

const OWNER_SLOT: [u8; 32] = [0u8; 32];

fn balance_key(addr: Address) -> [u8; 32] {
    let mut key = [0u8; 32];
    key[0] = 0x01;
    key[12..].copy_from_slice(addr.as_ref() as &[u8; 20]);
    key
}

#[allow(dead_code)]
#[pvm_contract_sdk::contract]
mod mini_token {
    use super::*;
    use pvm_contract_sdk::StorageFlags;

    #[derive(Debug, PartialEq, Eq, pvm_contract_sdk::SolError)]
    pub struct Unauthorized;

    #[derive(Debug, PartialEq, Eq, pvm_contract_sdk::SolError)]
    pub struct InsufficientBalance {
        pub available: U256,
        pub required: U256,
    }

    pub struct MiniToken;

    impl MiniToken {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {
            let mut caller = [0u8; 20];
            self.host().caller(&mut caller);
            let mut slot = [0u8; 32];
            slot[12..].copy_from_slice(&caller);
            self.host()
                .set_storage(StorageFlags::empty(), &OWNER_SLOT, &slot);
        }

        #[pvm_contract_sdk::method]
        pub fn owner(&self) -> Address {
            let mut buf = [0u8; 32];
            self.host()
                .get_storage_or_zero(StorageFlags::empty(), &OWNER_SLOT, &mut buf);
            let mut addr = [0u8; 20];
            addr.copy_from_slice(&buf[12..]);
            Address::from(addr)
        }

        #[pvm_contract_sdk::method]
        pub fn balance_of(&self, account: Address) -> U256 {
            let mut buf = [0u8; 32];
            self.host()
                .get_storage_or_zero(StorageFlags::empty(), &balance_key(account), &mut buf);
            U256::from_be_bytes::<32>(buf)
        }

        #[pvm_contract_sdk::method]
        pub fn mint(&mut self, to: Address, amount: U256) -> Result<(), Unauthorized> {
            let mut caller = [0u8; 20];
            self.host().caller(&mut caller);
            if caller != *<Address as AsRef<[u8; 20]>>::as_ref(&self.owner()) {
                return Err(Unauthorized);
            }
            let new = self.balance_of(to) + amount;
            self.host().set_storage(
                StorageFlags::empty(),
                &balance_key(to),
                &new.to_be_bytes::<32>(),
            );
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn transfer(&mut self, to: Address, amount: U256) -> Result<(), InsufficientBalance> {
            let mut caller = [0u8; 20];
            self.host().caller(&mut caller);
            let from = Address::from(caller);
            let available = self.balance_of(from);
            if available < amount {
                return Err(InsufficientBalance {
                    available,
                    required: amount,
                });
            }
            let new_from = available - amount;
            let new_to = self.balance_of(to) + amount;
            self.host().set_storage(
                StorageFlags::empty(),
                &balance_key(from),
                &new_from.to_be_bytes::<32>(),
            );
            self.host().set_storage(
                StorageFlags::empty(),
                &balance_key(to),
                &new_to.to_be_bytes::<32>(),
            );
            Ok(())
        }
    }
}

use mini_token::{InsufficientBalance, MiniToken, Unauthorized};

// --- Test helpers ---

const OWNER: [u8; 20] = [0xA0; 20];
const ALICE: [u8; 20] = [0xA1; 20];
const BOB: [u8; 20] = [0xB0; 20];

fn contract_with_caller(caller: [u8; 20]) -> (MiniToken, MockHost) {
    let mock = MockHostBuilder::new().caller(caller).build();
    let contract = MiniToken {
        host: Host::from_dyn(::std::rc::Rc::new(mock.clone())),
    };
    (contract, mock)
}

fn seed_owner(host: &MockHost, owner: [u8; 20]) {
    let mut slot = [0u8; 32];
    slot[12..].copy_from_slice(&owner);
    host.set_raw_storage(OWNER_SLOT.to_vec(), slot.to_vec());
}

fn seed_balance(host: &MockHost, addr: [u8; 20], amount: U256) {
    host.set_raw_storage(
        balance_key(Address::from(addr)).to_vec(),
        amount.to_be_bytes::<32>().to_vec(),
    );
}

// --- Tests: direct method calls, no dispatch ---

#[test]
fn owner_returns_stored_address() {
    let (contract, mock) = contract_with_caller(ALICE);
    seed_owner(&mock, OWNER);

    // Direct method call — no selector, no encoding.
    assert_eq!(contract.owner(), Address::from(OWNER));
}

#[test]
fn balance_of_returns_zero_by_default() {
    let (contract, mock) = contract_with_caller(ALICE);

    assert_eq!(contract.balance_of(Address::from(ALICE)), U256::ZERO);
}

#[test]
fn transfer_happy_path_returns_ok_and_moves_balance() {
    let (mut contract, mock) = contract_with_caller(ALICE);
    seed_balance(&mock, ALICE, U256::from(1000u64));

    // Direct call — the result is a typed `Result<(), InsufficientBalance>`.
    let result = contract.transfer(Address::from(BOB), U256::from(300u64));

    assert!(result.is_ok(), "expected Ok, got {:?}", result);
    assert_eq!(
        contract.balance_of(Address::from(ALICE)),
        U256::from(700u64)
    );
    assert_eq!(contract.balance_of(Address::from(BOB)), U256::from(300u64));
}

#[test]
fn transfer_insufficient_balance_returns_err_with_exact_fields() {
    let (mut contract, mock) = contract_with_caller(ALICE);
    seed_balance(&mock, ALICE, U256::from(50u64));

    let result = contract.transfer(Address::from(BOB), U256::from(100u64));

    // Typed error — no ABI decode, no selector parsing. Just pattern match.
    assert_eq!(
        result,
        Err(InsufficientBalance {
            available: U256::from(50u64),
            required: U256::from(100u64),
        })
    );

    // State unchanged.
    assert_eq!(contract.balance_of(Address::from(ALICE)), U256::from(50u64));
    assert_eq!(contract.balance_of(Address::from(BOB)), U256::ZERO);
}

#[test]
fn mint_by_non_owner_returns_unauthorized() {
    // caller = BOB, but owner = OWNER
    let (mut contract, mock) = contract_with_caller(BOB);
    seed_owner(&mock, OWNER);

    assert_eq!(
        contract.mint(Address::from(BOB), U256::from(100u64)),
        Err(Unauthorized),
    );

    // Nothing credited.
    assert_eq!(contract.balance_of(Address::from(BOB)), U256::ZERO);
}

#[test]
fn mint_then_transfer_chain_updates_state_correctly() {
    // Multi-step stateful flow on a single contract instance.
    let (mut contract, mock) = contract_with_caller(OWNER);
    seed_owner(&mock, OWNER);

    // Owner mints to ALICE.
    contract
        .mint(Address::from(ALICE), U256::from(1000u64))
        .unwrap();
    assert_eq!(
        contract.balance_of(Address::from(ALICE)),
        U256::from(1000u64)
    );

    // Simulate next transaction: rebuild host with a different caller, carry
    // storage over. (MockHost is per-instance; caller is set at build time.)
    let next_host = MockHostBuilder::new().caller(ALICE).build();
    for key in [
        OWNER_SLOT,
        balance_key(Address::from(ALICE)),
        balance_key(Address::from(BOB)),
    ] {
        if let Some(v) = mock.get_raw_storage(&key) {
            next_host.set_raw_storage(key.to_vec(), v);
        }
    }
    let mut contract = MiniToken {
        host: Host::from_dyn(::std::rc::Rc::new(next_host.clone())),
    };

    contract
        .transfer(Address::from(BOB), U256::from(400u64))
        .unwrap();

    assert_eq!(
        contract.balance_of(Address::from(ALICE)),
        U256::from(600u64)
    );
    assert_eq!(contract.balance_of(Address::from(BOB)), U256::from(400u64));
}

#[test]
fn property_like_test_with_arbitrary_inputs() {
    // Because methods are plain Rust, you can wrap them in `proptest!` /
    // `quickcheck` / loops without any framework boilerplate.
    for amount in [0u64, 1, 42, 999, u64::MAX / 2] {
        let (mut contract, mock) = contract_with_caller(ALICE);
        seed_balance(&mock, ALICE, U256::from(amount));

        // Transferring exactly the balance must always succeed.
        let result = contract.transfer(Address::from(BOB), U256::from(amount));
        assert!(
            result.is_ok(),
            "transferring full balance {} failed: {:?}",
            amount,
            result
        );
        assert_eq!(contract.balance_of(Address::from(ALICE)), U256::ZERO);
        assert_eq!(contract.balance_of(Address::from(BOB)), U256::from(amount));
    }
}

#![cfg(not(feature = "abi-gen"))]
//! End-to-end test for **trait-based dispatch**: methods declared in a
//! `#[interface_id]`-annotated trait and implemented via
//! `impl ITrait for Contract { ... }` are dispatched alongside inherent
//! methods when the trait is listed in `#[contract(implements(...))]`.
//!
//! This is the load-bearing piece for OpenZeppelin-style inheritance: each
//! extension is a `#[storage]` struct, the outer contract embeds it as a
//! field, and the contract's `IErc20` impl forwards to extension helpers
//! while the dispatch layer routes each selector to the appropriate trait
//! method through UFCS.
//!
//! What's exercised:
//!
//! - `#[pvm_contract_sdk::interface_id]` adds `interface_id() -> [u8; 4]` to
//!   a trait and produces an ERC-165-compatible XOR of method selectors.
//! - `#[contract(implements(IErc20<Error = …>))]` declares the trait set.
//! - Methods provided by `impl IErc20 for MyToken { ... }` are dispatched
//!   without `#[method]` attributes — the macro picks them up implicitly.
//! - Dispatch flows through UFCS (`<MyToken as IErc20>::transfer(&mut this, …)`),
//!   so inherent methods of the same name would not silently shadow trait
//!   methods.
//! - Inherent methods still work alongside trait-impl methods.

use pvm_contract_sdk::{Address, U256};
use pvm_contract_types::{
    Host, MockHost, MockHostBuilder, ReturnFlags, SolEncode, StaticEncodedLen,
};

extern crate alloc;

const BALANCE_PREFIX: u8 = 0xB1;

fn balance_key(addr: Address) -> [u8; 32] {
    let mut key = [0u8; 32];
    key[0] = BALANCE_PREFIX;
    key[12..].copy_from_slice(addr.as_ref() as &[u8; 20]);
    key
}

// ---------------------------------------------------------------------------
// Trait declaration. The `#[interface_id]` macro adds `interface_id()` and is
// also the trait signature the contract impl must match.
// ---------------------------------------------------------------------------

#[pvm_contract_sdk::interface_id]
pub trait IMiniErc20 {
    /// Associated error type.
    type Error;

    fn balance_of(&self, account: Address) -> U256;
    fn transfer(&mut self, to: Address, value: U256) -> Result<(), Self::Error>;
}

// ---------------------------------------------------------------------------
// Custom error so we exercise the Result-returning dispatch path.
// ---------------------------------------------------------------------------

#[derive(Debug, pvm_contract_sdk::SolError)]
pub struct InsufficientBalance {
    pub available: U256,
    pub required: U256,
}

// ---------------------------------------------------------------------------
// The contract itself, with the trait listed under `implements(...)`.
//
// The contract has:
//  - One inherent `#[constructor]` (deploy entry).
//  - One inherent `#[method]` (`mint`) — exercises that inherent methods still
//    dispatch alongside trait methods.
//  - One `impl IMiniErc20 for MiniErc20 { ... }` block — exercises trait
//    dispatch.
// ---------------------------------------------------------------------------

#[allow(dead_code)] // deploy()/call() are riscv64-gated; tests poke route() directly.
#[pvm_contract_sdk::contract(implements(IMiniErc20<Error = InsufficientBalance>))]
mod mini_erc20 {
    use super::*;
    use pvm_contract_types::StorageFlags;

    pub struct MiniErc20;

    impl MiniErc20 {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}

        /// Inherent (non-trait) method — verifies the dispatch table mixes
        /// inherent and trait methods.
        #[pvm_contract_sdk::method]
        pub fn mint(&mut self, to: Address, value: U256) {
            let key = balance_key(to);
            let mut buf = [0u8; 32];
            self.host()
                .get_storage_or_zero(StorageFlags::empty(), &key, &mut buf);
            let current = U256::from_be_bytes::<32>(buf);
            let new = current + value;
            self.host()
                .set_storage(StorageFlags::empty(), &key, &new.to_be_bytes::<32>());
        }
    }

    /// Trait impl — every fn here is dispatched implicitly because
    /// `IMiniErc20` is in `implements(...)` on the `#[contract]` attribute.
    impl super::IMiniErc20 for MiniErc20 {
        type Error = super::InsufficientBalance;

        fn balance_of(&self, account: super::Address) -> super::U256 {
            let key = super::balance_key(account);
            let mut buf = [0u8; 32];
            self.host()
                .get_storage_or_zero(StorageFlags::empty(), &key, &mut buf);
            super::U256::from_be_bytes::<32>(buf)
        }

        fn transfer(
            &mut self,
            to: super::Address,
            value: super::U256,
        ) -> Result<(), Self::Error> {
            let mut caller_bytes = [0u8; 20];
            self.host().caller(&mut caller_bytes);
            let from = super::Address::from(caller_bytes);

            let from_key = super::balance_key(from);
            let mut buf = [0u8; 32];
            self.host()
                .get_storage_or_zero(StorageFlags::empty(), &from_key, &mut buf);
            let available = super::U256::from_be_bytes::<32>(buf);

            if available < value {
                return Err(super::InsufficientBalance {
                    available,
                    required: value,
                });
            }

            let new_from = available - value;
            self.host().set_storage(
                StorageFlags::empty(),
                &from_key,
                &new_from.to_be_bytes::<32>(),
            );

            let to_key = super::balance_key(to);
            let mut buf = [0u8; 32];
            self.host()
                .get_storage_or_zero(StorageFlags::empty(), &to_key, &mut buf);
            let new_to = super::U256::from_be_bytes::<32>(buf) + value;
            self.host()
                .set_storage(StorageFlags::empty(), &to_key, &new_to.to_be_bytes::<32>());

            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Test harness — share storage between contract and assertion site.
// ---------------------------------------------------------------------------

fn host_with_caller(caller: [u8; 20]) -> MockHost {
    MockHostBuilder::new().caller(caller).build()
}

fn make_contract(mock: &MockHost) -> mini_erc20::MiniErc20 {
    mini_erc20::MiniErc20 {
        host: Host::from_dyn(alloc::rc::Rc::new(mock.clone())),
    }
}

fn seed_balance(host: &MockHost, addr: [u8; 20], amount: U256) {
    let key = balance_key(Address::from(addr));
    host.set_raw_storage(key.to_vec(), amount.to_be_bytes::<32>().to_vec());
}

fn selector(sig: &str) -> [u8; 4] {
    pvm_contract_types::const_selector(sig)
}

fn route_ok(contract: &mut mini_erc20::MiniErc20, mock: &MockHost, sel: [u8; 4], input: &[u8]) -> Vec<u8> {
    let outcome = mini_erc20::route(contract, sel, input);
    assert_eq!(outcome, Some(()), "expected matched selector");
    let rv = mock.take_return_value().expect("return_value was called");
    assert_eq!(rv.flags, ReturnFlags::empty(), "expected success flags");
    rv.data
}

fn route_revert(contract: &mut mini_erc20::MiniErc20, mock: &MockHost, sel: [u8; 4], input: &[u8]) -> Vec<u8> {
    let outcome = mini_erc20::route(contract, sel, input);
    assert_eq!(outcome, Some(()), "expected matched selector");
    let rv = mock.take_return_value().expect("return_value was called");
    assert_eq!(rv.flags, ReturnFlags::REVERT, "expected REVERT flags");
    rv.data
}

fn encode_addr(addr: Address) -> Vec<u8> {
    let mut buf = vec![0u8; <Address as StaticEncodedLen>::ENCODED_SIZE];
    addr.encode_to(&mut buf);
    buf
}

fn encode_addr_u256(addr: Address, value: U256) -> Vec<u8> {
    const LEN: usize =
        <Address as StaticEncodedLen>::ENCODED_SIZE + <U256 as StaticEncodedLen>::ENCODED_SIZE;
    let mut buf = vec![0u8; LEN];
    (addr, value).encode_to(&mut buf);
    buf
}

// ---------------------------------------------------------------------------
// Tests.
// ---------------------------------------------------------------------------

const ALICE: [u8; 20] = [0xA1; 20];
const BOB: [u8; 20] = [0xB0; 20];

#[test]
fn interface_id_method_is_available_on_the_trait() {
    // The macro adds `interface_id() -> [u8; 4]` as a provided method.
    // Cross-check by independently XOR'ing the two method selectors.
    let id = <mini_erc20::MiniErc20 as IMiniErc20>::interface_id();
    let sel_balance = selector("balanceOf(address)");
    let sel_transfer = selector("transfer(address,uint256)");
    let mut expected = [0u8; 4];
    for j in 0..4 {
        expected[j] = sel_balance[j] ^ sel_transfer[j];
    }
    assert_eq!(id, expected);
}

#[test]
fn trait_balance_of_dispatches_through_ufcs() {
    let mock = host_with_caller(ALICE);
    seed_balance(&mock, ALICE, U256::from(1000));
    let mut contract = make_contract(&mock);

    // `balance_of` came from `impl IMiniErc20 for MiniErc20`. It's dispatched
    // implicitly because `IMiniErc20` is in `implements(...)`.
    let data = route_ok(
        &mut contract,
        &mock,
        selector("balanceOf(address)"),
        &encode_addr(Address::from(ALICE)),
    );
    assert_eq!(data, U256::from(1000).to_be_bytes::<32>().to_vec());
}

#[test]
fn trait_transfer_moves_balance() {
    let mock = host_with_caller(ALICE);
    seed_balance(&mock, ALICE, U256::from(1000));
    let mut contract = make_contract(&mock);

    route_ok(
        &mut contract,
        &mock,
        selector("transfer(address,uint256)"),
        &encode_addr_u256(Address::from(BOB), U256::from(300)),
    );

    // Cross-check via `balanceOf` after the transfer.
    let data = route_ok(
        &mut contract,
        &mock,
        selector("balanceOf(address)"),
        &encode_addr(Address::from(ALICE)),
    );
    assert_eq!(data, U256::from(700).to_be_bytes::<32>().to_vec());

    let data = route_ok(
        &mut contract,
        &mock,
        selector("balanceOf(address)"),
        &encode_addr(Address::from(BOB)),
    );
    assert_eq!(data, U256::from(300).to_be_bytes::<32>().to_vec());
}

#[test]
fn trait_transfer_reverts_with_custom_error_when_insufficient_balance() {
    let mock = host_with_caller(ALICE);
    seed_balance(&mock, ALICE, U256::from(50));
    let mut contract = make_contract(&mock);

    let data = route_revert(
        &mut contract,
        &mock,
        selector("transfer(address,uint256)"),
        &encode_addr_u256(Address::from(BOB), U256::from(100)),
    );

    // Revert payload starts with the InsufficientBalance selector (per
    // SolError::SELECTOR), then encodes (available, required).
    let expected_sel = selector("InsufficientBalance(uint256,uint256)");
    assert_eq!(&data[..4], &expected_sel[..], "revert selector mismatch");

    let available = U256::from_be_bytes::<32>({
        let mut b = [0u8; 32];
        b.copy_from_slice(&data[4..36]);
        b
    });
    let required = U256::from_be_bytes::<32>({
        let mut b = [0u8; 32];
        b.copy_from_slice(&data[36..68]);
        b
    });
    assert_eq!(available, U256::from(50));
    assert_eq!(required, U256::from(100));
}

#[test]
fn inherent_mint_method_still_dispatches_alongside_trait_methods() {
    let mock = host_with_caller(ALICE);
    let mut contract = make_contract(&mock);

    // `mint` is an inherent #[method] on MiniErc20 — separate from the trait
    // impl, dispatched through method-call syntax.
    let _ = route_ok(
        &mut contract,
        &mock,
        selector("mint(address,uint256)"),
        &encode_addr_u256(Address::from(BOB), U256::from(777)),
    );

    let data = route_ok(
        &mut contract,
        &mock,
        selector("balanceOf(address)"),
        &encode_addr(Address::from(BOB)),
    );
    assert_eq!(data, U256::from(777).to_be_bytes::<32>().to_vec());
}

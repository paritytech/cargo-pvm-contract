#![cfg(not(feature = "abi-gen"))]
//! `#[contract(implements(ITrait, ...))]` folds the methods of each in-module
//! `impl ITrait for Contract` block into the dispatch table as real entry points
//!, so an author writes forwarders once as a trait impl instead of a pile
//! of inherent `#[method]`s. Overrides are just a different impl body.

use pvm_contract_sdk::{
    Address, Lazy, Mapping, MockHostBuilder, OutSink, Outcome, SolDecode, U256,
};

pub trait IErc20 {
    fn total_supply(&self) -> U256;
    fn balance_of(&self, account: Address) -> U256;
    fn transfer(&mut self, to: Address, amount: U256) -> bool;
}

pub trait IOwnable {
    fn owner(&self) -> Address;
}

// Shares the Rust name `value` with the inherent method below, but a different
// signature — so a distinct selector `value(uint256)` vs `value()`.
pub trait IValued {
    fn value(&self, key: U256) -> U256;
}

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract(implements(IErc20, IOwnable, IValued))]
mod token {
    use super::{Address, IErc20, IOwnable, IValued, Lazy, Mapping, U256};

    pub struct Token {
        total: Lazy<U256>,
        balances: Mapping<Address, U256>,
    }

    impl Token {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        // Inherent method `value()` — shares its Rust name with the folded
        // `IValued::value(uint256)` below but has a different signature, so a
        // distinct selector. Exercises the fully-qualified trait call: neither
        // shadows the other, both dispatch.
        #[pvm_contract_macros::method]
        pub fn value(&self) -> U256 {
            U256::from(42)
        }
    }

    impl IValued for Token {
        fn value(&self, key: U256) -> U256 {
            key + U256::ONE
        }
    }

    impl IErc20 for Token {
        fn total_supply(&self) -> U256 {
            self.total.get()
        }
        fn balance_of(&self, account: Address) -> U256 {
            self.balances.get(&account)
        }
        // An "override": the impl body adds logic (rejects zero-amount
        // transfers) beyond a plain forward.
        fn transfer(&mut self, to: Address, amount: U256) -> bool {
            if amount == U256::ZERO {
                return false;
            }
            self.balances.insert(&to, &amount);
            true
        }
    }

    impl IOwnable for Token {
        fn owner(&self) -> Address {
            Address([9u8; 20])
        }
    }
}

fn selector(sig: &str) -> [u8; 4] {
    pvm_contract_types::const_selector(sig)
}

/// Route a matched method and return the encoded output. A folded method's
/// success surfaces as `Outcome::Return(n)`, with the ABI-encoded return in the
/// output buffer (a revert would diverge instead).
fn route_ok(contract: &mut token::Token, sig: &str, input: &[u8]) -> Vec<u8> {
    let mut buf = [0u8; token::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;
    let outcome = token::route(contract, selector(sig), input, &mut out);
    let Outcome::Return(n) = outcome else {
        panic!("expected Return for `{sig}`, got {outcome:?}");
    };
    out.view(n).to_vec()
}

fn encode_transfer(to: Address, amount: U256) -> Vec<u8> {
    let mut input = vec![0u8; 32];
    input[12..].copy_from_slice(&to.0);
    input.extend_from_slice(&amount.to_be_bytes::<32>());
    input
}

#[test]
fn folded_and_inherent_methods_dispatch() {
    let mut contract = token::Token::with_host(MockHostBuilder::new().build());
    let mut buf = [0u8; token::MAX_RETURN_LEN];

    // Two interfaces folded + the inherent `value()` all dispatch; a selector
    // not in the table misses (`Unhandled`).
    for sig in ["totalSupply()", "owner()", "value()"] {
        let mut out: &mut [u8] = &mut buf;
        assert!(
            matches!(
                token::route(&mut contract, selector(sig), &[], &mut out),
                Outcome::Return(_)
            ),
            "`{sig}` should dispatch"
        );
    }
    let mut out: &mut [u8] = &mut buf;
    assert_eq!(
        token::route(&mut contract, selector("nope()"), &[], &mut out),
        Outcome::Unhandled
    );
}

#[test]
fn override_body_runs() {
    let mut contract = token::Token::with_host(MockHostBuilder::new().build());

    // amount == 0 hits the override's early-return `false`.
    let to = Address([7u8; 20]);
    let data = route_ok(
        &mut contract,
        "transfer(address,uint256)",
        &encode_transfer(to, U256::ZERO),
    );
    assert!(
        !bool::decode(&data).unwrap(),
        "zero transfer must return false"
    );

    // amount > 0 writes state and returns true; balance_of reads it back.
    let data = route_ok(
        &mut contract,
        "transfer(address,uint256)",
        &encode_transfer(to, U256::from(500u64)),
    );
    assert!(bool::decode(&data).unwrap());

    let mut acct = vec![0u8; 32];
    acct[12..].copy_from_slice(&to.0);
    let data = route_ok(&mut contract, "balanceOf(address)", &acct);
    assert_eq!(U256::decode(&data).unwrap(), U256::from(500u64));
}

#[test]
fn inherent_and_folded_same_name_distinct_selectors() {
    let mut contract = token::Token::with_host(MockHostBuilder::new().build());

    // Inherent `value()` (selector `value()`) returns 42.
    let data = route_ok(&mut contract, "value()", &[]);
    assert_eq!(U256::decode(&data).unwrap(), U256::from(42));

    // Folded `IValued::value(uint256)` (selector `value(uint256)`) returns
    // key + 1 — same Rust name, different selector, neither shadows the other.
    let data = route_ok(
        &mut contract,
        "value(uint256)",
        &U256::from(100u64).to_be_bytes::<32>(),
    );
    assert_eq!(U256::decode(&data).unwrap(), U256::from(101u64));
}

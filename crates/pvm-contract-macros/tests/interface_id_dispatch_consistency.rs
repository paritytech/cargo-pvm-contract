#![cfg(not(feature = "abi-gen"))]
//! Cross-consistency: the selector `#[interface_id]` computes for a method must
//! equal the selector the `#[contract]` macro actually dispatches that method
//! on. The two go through independent signature-building code paths
//! (`interface_id.rs` vs `dispatch.rs`), so this test pins them together — a
//! divergence would make a contract advertise an interface ID that doesn't match
//! the selectors it really answers.
//!
//! A single-method interface is used so `INTERFACE_ID` is that one selector,
//! which can then be fed straight to the contract's `route()`.

use pvm_contract_types::StaticEncodedLen;
use ruint::aliases::U256;

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract]
mod token {
    use super::*;

    pub struct Token;

    impl Token {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn balance_of(&self, _account: pvm_contract_sdk::Address) -> U256 {
            U256::from(7u64)
        }
    }
}

#[pvm_contract_macros::interface_id]
pub trait IBalance {
    fn balance_of(&self, account: pvm_contract_sdk::Address) -> U256;
}

struct Probe;
impl IBalance for Probe {
    fn balance_of(&self, _account: pvm_contract_sdk::Address) -> U256 {
        unimplemented!()
    }
}

#[test]
fn interface_id_selector_matches_contract_dispatch() {
    let selector = <Probe as IBalance>::INTERFACE_ID;

    let mock = pvm_contract_sdk::MockHostBuilder::new().build();
    let mut contract = token::Token::with_host(mock);

    // Value is irrelevant; just enough bytes to clear the decode size check.
    let input = [0u8; <pvm_contract_sdk::Address as StaticEncodedLen>::ENCODED_SIZE];

    assert_eq!(token::route(&mut contract, selector, &input), Some(()));

    // Negative control: a different selector must not route, so the assert above
    // is proving the match, not that route() returns Some for anything.
    let mut bogus = selector;
    bogus[0] ^= 0xff;
    assert_eq!(token::route(&mut contract, bogus, &input), None);
}

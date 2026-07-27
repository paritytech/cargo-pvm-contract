#![cfg(not(feature = "abi-gen"))]
//! `implements(...)` combined with a `.sol` interface. Folded methods are
//! resolved against the `.sol` (coverage + parameter + mutability cross-checks)
//! and dispatch under the `.sol`-derived selectors, exactly like inherent
//! `#[method]`s.

use pvm_contract_sdk::{Address, Outcome, U256};

pub trait IComposed {
    fn total_supply(&self) -> U256;
    fn balance_of(&self, account: Address) -> U256;
}

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract("tests/fixtures/IComposed.sol", implements(IComposed))]
mod token {
    use super::{Address, IComposed, U256};

    pub struct Token {
        total: pvm_contract_sdk::Lazy<U256>,
    }

    impl Token {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl IComposed for Token {
        fn total_supply(&self) -> U256 {
            self.total.get()
        }
        fn balance_of(&self, account: Address) -> U256 {
            let _ = account;
            U256::ZERO
        }
    }
}

fn selector(sig: &str) -> [u8; 4] {
    pvm_contract_types::const_selector(sig)
}

#[test]
fn folded_methods_dispatch_under_sol_selectors() {
    let mut contract = token::Token::with_host(pvm_contract_types::MockHostBuilder::new().build());
    let mut buf = [0u8; token::MAX_RETURN_LEN];

    // Both `.sol` functions are satisfied by the folded trait impl and dispatch
    // under the interface's canonical selectors.
    let mut out: &mut [u8] = &mut buf;
    assert!(matches!(
        token::route(&mut contract, selector("totalSupply()"), &[], &mut out),
        Outcome::Return(_)
    ));
    let mut out: &mut [u8] = &mut buf;
    assert!(matches!(
        token::route(
            &mut contract,
            selector("balanceOf(address)"),
            &[0u8; 32],
            &mut out
        ),
        Outcome::Return(_)
    ));
}

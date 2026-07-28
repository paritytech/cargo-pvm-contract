#![cfg(not(feature = "abi-gen"))]
//! A same-trait `impl ITrait for Helper` on some *other* struct in the module
//! must not preempt the contract's own `impl ITrait for Contract`. The fold skips
//! non-contract impls and picks the contract's, regardless of declaration order —
//! so item ordering can't change which impl is folded.

use pvm_contract_sdk::{MockHostBuilder, OutSink, Outcome, SolDecode, U256};

pub trait IThing {
    fn thing(&self) -> U256;
}

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract(implements(IThing))]
mod token {
    use super::{IThing, U256};

    pub struct Token;
    // A sibling struct that also implements the interface. It is *declared first*
    // and is not the contract struct, so the fold must skip it.
    pub struct Helper;

    impl IThing for Helper {
        fn thing(&self) -> U256 {
            U256::from(111u64)
        }
    }

    impl Token {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl IThing for Token {
        fn thing(&self) -> U256 {
            U256::from(42u64)
        }
    }
}

fn selector(sig: &str) -> [u8; 4] {
    pvm_contract_types::const_selector(sig)
}

#[test]
fn contract_impl_is_folded_not_the_sibling() {
    let mut contract = token::Token::with_host(MockHostBuilder::new().build());
    let mut buf = [0u8; token::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;

    let outcome = token::route(&mut contract, selector("thing()"), &[], &mut out);
    let Outcome::Return(n) = outcome else {
        panic!("expected Return, got {outcome:?}");
    };
    // 42 (Token's impl), not 111 (Helper's) — the sibling impl was skipped.
    assert_eq!(U256::decode(out.view(n)).unwrap(), U256::from(42u64));
}

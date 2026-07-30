#![cfg(not(feature = "abi-gen"))]
//! The fold's impl-matching precision: which `impl` blocks are folded for a given
//! `implements(...)` entry, and how the folded/collected methods are dispatched.
//!
//! - **Qualified path**: an `impl outer::IThing for C` is folded even when the
//!   bare name isn't in scope, dispatching through the impl's own path.
//! - **Sibling skip**: a same-trait impl for another struct is skipped, so the
//!   contract's own impl is folded regardless of declaration order.
//! - **Non-folded `#[method]`**: a `#[method]` on a same-last-segment but
//!   different trait is still collected and dispatched via a fully-qualified
//!   trait call.

use pvm_contract_sdk::{MockHostBuilder, OutSink, Outcome, SolDecode, U256};

fn selector(sig: &str) -> [u8; 4] {
    pvm_contract_types::const_selector(sig)
}

// --- Qualified path ----------------------------------------------------------

// The trait lives in a submodule and is deliberately NOT re-imported under its
// bare name `IThing` where the contract module can see it.
pub mod outer {
    use super::U256;
    pub trait IThing {
        fn thing(&self) -> U256;
    }
}

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract(implements(IThing))]
mod qualified {
    use super::{U256, outer};

    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    // Implemented via the qualified path; bare `IThing` is not imported here.
    impl outer::IThing for C {
        fn thing(&self) -> U256 {
            U256::from(7u64)
        }
    }
}

#[test]
fn qualified_path_impl_dispatches() {
    let mut contract = qualified::C::with_host(MockHostBuilder::new().build());
    let mut buf = [0u8; qualified::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;
    // Would fail to compile (bare `IThing` not in scope for the router) if the
    // fold used the `implements(...)` path instead of the `impl`'s path.
    assert!(matches!(
        qualified::route(&mut contract, selector("thing()"), &[], &mut out),
        Outcome::Return(_)
    ));
}

// --- Sibling-struct skip (order independence) --------------------------------

pub trait ISibling {
    fn thing(&self) -> U256;
}

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract(implements(ISibling))]
mod sibling {
    use super::{ISibling, U256};

    pub struct Token;
    // A sibling struct that also implements the interface. Declared first and not
    // the contract struct, so the fold must skip it.
    pub struct Helper;

    impl ISibling for Helper {
        fn thing(&self) -> U256 {
            U256::from(111u64)
        }
    }

    impl Token {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl ISibling for Token {
        fn thing(&self) -> U256 {
            U256::from(42u64)
        }
    }
}

#[test]
fn contract_impl_is_folded_not_the_sibling() {
    let mut contract = sibling::Token::with_host(MockHostBuilder::new().build());
    let mut buf = [0u8; sibling::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;

    let outcome = sibling::route(&mut contract, selector("thing()"), &[], &mut out);
    let Outcome::Return(n) = outcome else {
        panic!("expected Return, got {outcome:?}");
    };
    // 42 (Token's impl), not 111 (Helper's) — the sibling impl was skipped.
    assert_eq!(U256::decode(out.view(n)).unwrap(), U256::from(42u64));
}

// --- Non-folded `#[method]` on a same-last-segment trait ---------------------

pub mod a {
    pub trait IThing {
        fn folded(&self) -> u64;
    }
}
pub mod b {
    pub trait IThing {
        fn extra(&self) -> u64;
    }
}

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract(implements(a::IThing))]
mod nonfolded {
    use super::a;
    use super::b;
    // `b::IThing` is intentionally NOT imported into method-call scope; the
    // collected `#[method]` dispatches via `<C as b::IThing>::extra`, which
    // resolves through the impl's own trait path regardless of imports.

    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl a::IThing for C {
        fn folded(&self) -> u64 {
            1
        }
    }

    // Same last segment `IThing`, different trait path -> NOT folded. Its
    // `#[method]` must still be collected as an ordinary entry point.
    impl b::IThing for C {
        #[pvm_contract_macros::method]
        fn extra(&self) -> u64 {
            2
        }
    }
}

fn route_u64(sig: &str) -> u64 {
    let mut contract = nonfolded::C::with_host(MockHostBuilder::new().build());
    let mut buf = [0u8; nonfolded::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;
    let outcome = nonfolded::route(&mut contract, selector(sig), &[], &mut out);
    let Outcome::Return(n) = outcome else {
        panic!("expected Return for `{sig}`, got {outcome:?}");
    };
    u64::decode(out.view(n)).unwrap()
}

#[test]
fn folded_and_nonfolded_method_both_dispatch() {
    assert_eq!(route_u64("folded()"), 1); // from the folded a::IThing
    assert_eq!(route_u64("extra()"), 2); // from the non-folded b::IThing #[method]
}

#![cfg(not(feature = "abi-gen"))]
//! A `#[method]` on a trait impl that is *not* the folded interface must still be
//! collected as an ordinary dispatch method. The fold's impl-matching and the
//! inherent-collection skip use the same `trait_path_matches` predicate, so an
//! `impl b::IThing for C` under `implements(a::IThing)` (same last segment, but a
//! different trait) is not treated as folded and its `#[method]`s are not lost.
//!
//! It also dispatches through UFCS `<C as b::IThing>::extra(this)` (not
//! `this.extra()`), so it resolves *without* `b::IThing` in method-call scope and
//! can't be silently shadowed by a same-named inherent method. This test
//! deliberately does not import `b::IThing`'s methods, to prove that.

use pvm_contract_types::{MockHostBuilder, OutSink, Outcome, SolDecode, const_selector};

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
mod c {
    use super::a;
    use super::b;
    // Note: `b::IThing` is intentionally NOT imported into method-call scope.
    // The collected `#[method]` dispatches via UFCS `<C as b::IThing>::extra`,
    // which resolves through the impl's own trait path regardless of imports.

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
    let mut contract = c::C::with_host(MockHostBuilder::new().build());
    let mut buf = [0u8; c::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;
    let outcome = c::route(&mut contract, const_selector(sig), &[], &mut out);
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

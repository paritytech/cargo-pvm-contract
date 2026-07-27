#![cfg(not(feature = "abi-gen"))]
//! A folded interface can be implemented through a *qualified* path
//! (`impl outer::IThing for Contract`) whose bare last segment is not in scope
//! inside the contract module. The generated dispatch must use the `impl`'s own
//! path, not the (possibly unqualified) name written in `implements(...)`.

use pvm_contract_sdk::{Outcome, U256};

// The trait lives in a submodule and is deliberately NOT re-imported under its
// bare name `IThing` anywhere the contract module can see.
pub mod outer {
    use super::U256;
    pub trait IThing {
        fn thing(&self) -> U256;
    }
}

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract(implements(IThing))]
mod c {
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

fn selector(sig: &str) -> [u8; 4] {
    pvm_contract_types::const_selector(sig)
}

#[test]
fn qualified_path_impl_dispatches() {
    let mut contract = c::C::with_host(pvm_contract_types::MockHostBuilder::new().build());
    let mut buf = [0u8; c::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;
    // Would fail to compile (bare `IThing` not in scope for the router) if the
    // fold used the `implements(...)` path instead of the `impl`'s path.
    assert!(matches!(
        c::route(&mut contract, selector("thing()"), &[], &mut out),
        Outcome::Return(_)
    ));
}

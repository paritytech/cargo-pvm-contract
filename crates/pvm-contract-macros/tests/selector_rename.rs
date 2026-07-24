#![cfg(not(feature = "abi-gen"))]
//! `#[selector(name = "...")]` is the canonical Solidity-name override on the
//! inherent `#[method]` path, and `#[method(rename = "...")]` remains a working
//! alias. Both rename the selector the method dispatches under.

use pvm_contract_types::Outcome;

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract]
mod renamed {
    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        #[pvm_contract_macros::selector(name = "transfer")]
        pub fn transfer_tokens(&self) {}

        #[pvm_contract_macros::method(rename = "approve")]
        pub fn approve_tokens(&self) {}
    }
}

fn selector(sig: &str) -> [u8; 4] {
    pvm_contract_types::const_selector(sig)
}

#[test]
fn selector_name_renames_dispatch() {
    let mock = pvm_contract_types::MockHostBuilder::new().build();
    let mut contract = renamed::C::with_host(mock);
    let mut buf = [0u8; renamed::MAX_RETURN_LEN];

    // Void method: a match returns `Return(0)`, a miss returns `Unhandled`.
    let mut out: &mut [u8] = &mut buf;
    assert_eq!(
        renamed::route(&mut contract, selector("transfer()"), &[], &mut out),
        Outcome::Return(0)
    );
    let mut out: &mut [u8] = &mut buf;
    assert_eq!(
        renamed::route(&mut contract, selector("transferTokens()"), &[], &mut out),
        Outcome::Unhandled
    );
}

#[test]
fn method_rename_alias_still_works() {
    let mock = pvm_contract_types::MockHostBuilder::new().build();
    let mut contract = renamed::C::with_host(mock);
    let mut buf = [0u8; renamed::MAX_RETURN_LEN];

    let mut out: &mut [u8] = &mut buf;
    assert_eq!(
        renamed::route(&mut contract, selector("approve()"), &[], &mut out),
        Outcome::Return(0)
    );
    let mut out: &mut [u8] = &mut buf;
    assert_eq!(
        renamed::route(&mut contract, selector("approveTokens()"), &[], &mut out),
        Outcome::Unhandled
    );
}

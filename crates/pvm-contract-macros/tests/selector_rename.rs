#![cfg(not(feature = "abi-gen"))]
//! `#[selector(name = "...")]` is the canonical Solidity-name override on the
//! inherent `#[method]` path, and `#[method(rename = "...")]` remains a working
//! alias. Both rename the selector the method dispatches under (§D).

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

    assert_eq!(
        renamed::route(&mut contract, selector("transfer()"), &[]),
        Some(())
    );
    assert_eq!(
        renamed::route(&mut contract, selector("transferTokens()"), &[]),
        None
    );
}

#[test]
fn method_rename_alias_still_works() {
    let mock = pvm_contract_types::MockHostBuilder::new().build();
    let mut contract = renamed::C::with_host(mock);

    assert_eq!(
        renamed::route(&mut contract, selector("approve()"), &[]),
        Some(())
    );
    assert_eq!(
        renamed::route(&mut contract, selector("approveTokens()"), &[]),
        None
    );
}

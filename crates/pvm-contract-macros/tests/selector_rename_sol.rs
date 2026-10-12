#![cfg(not(feature = "abi-gen"))]
//! `#[selector(name = "...")]` on the `.sol`-interface path binds the Rust method
//! to the interface function of exactly that name. This must work even when the
//! Rust name does not snake-case to the interface name (the auto heuristic), and
//! an explicit rename that matches no interface function must be a hard error
//! (see `tests/ui/sol_selector_rename_no_match.rs`) rather than a silent fallback.

use pvm_contract_sdk::U256;
use pvm_contract_types::{Outcome, const_selector};

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract("tests/fixtures/SelectorRenameOk.sol")]
mod c {
    use super::U256;

    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        // Rust name `xfer` does NOT snake-case to `transferTokens`, so this only
        // resolves through the explicit `#[selector(name)]` match.
        #[pvm_contract_macros::method]
        #[pvm_contract_macros::selector(name = "transferTokens")]
        pub fn xfer(&mut self) {}

        // No rename: `balance_of` matches `balanceOf` via the snake_case heuristic.
        #[pvm_contract_macros::method]
        pub fn balance_of(&self) -> U256 {
            U256::ZERO
        }
    }
}

#[test]
fn explicit_rename_binds_to_named_sol_function() {
    let mock = pvm_contract_types::MockHostBuilder::new().build();
    let mut contract = c::C::with_host(mock);
    let mut buf = [0u8; c::MAX_RETURN_LEN];

    // Dispatches under the interface name, not the Rust name. `xfer` is a void
    // method (`Return(0)`); the Rust-name selector must not match.
    let mut out: &mut [u8] = &mut buf;
    assert_eq!(
        c::route(
            &mut contract,
            const_selector("transferTokens()"),
            &[],
            &mut out
        ),
        Outcome::Return(0)
    );
    let mut out: &mut [u8] = &mut buf;
    assert_eq!(
        c::route(&mut contract, const_selector("xfer()"), &[], &mut out),
        Outcome::Unhandled
    );
}

#[test]
fn auto_snake_case_match_still_works() {
    let mock = pvm_contract_types::MockHostBuilder::new().build();
    let mut contract = c::C::with_host(mock);
    let mut buf = [0u8; c::MAX_RETURN_LEN];

    // `balanceOf` returns a U256 (32 bytes).
    let mut out: &mut [u8] = &mut buf;
    assert_eq!(
        c::route(&mut contract, const_selector("balanceOf()"), &[], &mut out),
        Outcome::Return(32)
    );
}

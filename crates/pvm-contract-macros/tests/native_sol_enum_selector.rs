#![cfg(not(feature = "abi-gen"))]
//! The enum mirror of `native_sol_struct_selector`. A `.sol` interface supplies
//! the dispatch selector directly, so a method taking an `enum` has to hash that
//! type's canonical ABI form — `choose(uint8)`, the same signature solc and the
//! generated `.abi.json` use — rather than its declared name, `choose(Choice)`.
//!
//! This is the only end-to-end cover for the enum branch of
//! `CustomTypes::canonical_name`. The `abi_import!` tests do *not* reach it:
//! there the selector is built at const-eval from the generated Rust type's
//! `SOL_NAME` (`build_method_signature_expr`), so an enum canonicalization
//! regression in `CustomTypes` leaves their calldata untouched. Only the
//! `#[contract("Foo.sol")]` path hashes the signature from the `.sol` tokens.

use pvm_contract_macros::SolType;
use pvm_contract_types::{MockHostBuilder, OutSink, Outcome, SolDecode};

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract("tests/fixtures/EnumParam.sol")]
mod enum_param {
    use super::*;

    #[derive(Clone, Copy, SolType)]
    #[repr(u8)]
    pub enum Choice {
        Yes,
        No,
    }

    pub struct EnumParam;

    impl EnumParam {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn choose(&self, c: Choice) -> u64 {
            match c {
                Choice::Yes => 1,
                Choice::No => 2,
            }
        }
    }
}

fn new_contract() -> enum_param::EnumParam {
    enum_param::EnumParam::with_host(MockHostBuilder::new().build())
}

#[test]
fn enum_param_selector_matches_the_canonical_uint8_signature() {
    let mut contract = new_contract();

    // `cast sig "choose(uint8)"`.
    let sel = [0xf9, 0x4e, 0x34, 0x9d];
    assert_eq!(sel, pvm_contract_types::const_selector("choose(uint8)"));

    // An enum argument occupies a full right-aligned word, like `uint8`.
    let mut input = [0u8; 32];
    input[31] = 1; // Choice::No

    let mut buf = [0u8; enum_param::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;
    let returned = match enum_param::route(&mut contract, sel, &input, &mut out) {
        Outcome::Return(n) => out.view(n).to_vec(),
        other => panic!("expected Return, got {other:?}"),
    };
    assert_eq!(u64::decode_at(&returned, 0).unwrap(), 2);
}

#[test]
fn enum_param_selector_is_not_hashed_from_the_declared_name() {
    let mut contract = new_contract();

    let sel = pvm_contract_types::const_selector("choose(Choice)");
    let mut buf = [0u8; enum_param::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;

    assert_eq!(
        enum_param::route(&mut contract, sel, &[0u8; 32], &mut out),
        Outcome::Unhandled
    );
}

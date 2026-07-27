#![cfg(not(feature = "abi-gen"))]
//! ERC-165 falls out of `#[interface_id]` + `implements(...)` with
//! no generated code: define an `IErc165` interface, list it in
//! `implements(...)`, and hand-write the 3-liner using the `INTERFACE_ID` consts.

use pvm_contract_sdk::{MockHostBuilder, OutSink, Outcome, SolDecode, U256};

#[pvm_contract_macros::interface_id]
pub trait IErc20 {
    fn total_supply(&self) -> U256;
    fn transfer(&mut self, to: pvm_contract_sdk::Address, amount: U256) -> bool;
}

pub trait IErc165 {
    fn supports_interface(&self, id: [u8; 4]) -> bool;
}

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract(implements(IErc20, IErc165))]
mod token {
    use super::{IErc20, IErc165, U256};
    use pvm_contract_sdk::Address;

    pub struct Token;

    impl Token {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl IErc20 for Token {
        fn total_supply(&self) -> U256 {
            U256::ZERO
        }
        fn transfer(&mut self, _to: Address, _amount: U256) -> bool {
            true
        }
    }

    impl IErc165 for Token {
        fn supports_interface(&self, id: [u8; 4]) -> bool {
            id == [0x01, 0xff, 0xc9, 0xa7] // ERC-165 itself
                || id == <Token as IErc20>::INTERFACE_ID
        }
    }
}

fn selector(sig: &str) -> [u8; 4] {
    pvm_contract_types::const_selector(sig)
}

fn supports(contract: &mut token::Token, id: [u8; 4]) -> bool {
    let mut input = vec![0u8; 32];
    input[..4].copy_from_slice(&id);
    let mut buf = [0u8; token::MAX_RETURN_LEN];
    let mut out: &mut [u8] = &mut buf;
    let outcome = token::route(
        contract,
        selector("supportsInterface(bytes4)"),
        &input,
        &mut out,
    );
    let Outcome::Return(n) = outcome else {
        panic!("expected Return, got {outcome:?}");
    };
    bool::decode(out.view(n)).unwrap()
}

#[test]
fn supports_interface_answers_for_known_ids() {
    let mut contract = token::Token::with_host(MockHostBuilder::new().build());

    assert!(supports(&mut contract, [0x01, 0xff, 0xc9, 0xa7]));
    assert!(supports(
        &mut contract,
        <token::Token as IErc20>::INTERFACE_ID
    ));
    assert!(!supports(&mut contract, [0xde, 0xad, 0xbe, 0xef]));
}

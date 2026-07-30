#![cfg(not(feature = "abi-gen"))]
//! A `.sol` interface supplies the dispatch selector directly, so a method
//! taking a user-defined type has to hash that type's canonical ABI form —
//! `sum((uint64,uint64))`, the same signature solc and the generated
//! `.abi.json` use — rather than its declared name, `sum(Point)`.

use pvm_contract_macros::SolType;
use pvm_contract_types::{MockHost, MockHostBuilder, ReturnFlags, SolDecode, SolEncode};

#[allow(dead_code)] // `new()` runs only through deploy() (riscv64-gated)
#[pvm_contract_macros::contract("tests/fixtures/StructParam.sol")]
mod struct_param {
    use super::*;

    #[derive(SolType)]
    pub struct Point {
        pub x: u64,
        pub y: u64,
    }

    pub struct StructParam;

    impl StructParam {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn sum(&self, p: Point) -> u64 {
            p.x + p.y
        }
    }
}

fn new_contract() -> (struct_param::StructParam, MockHost) {
    let mock = MockHostBuilder::new().build();
    let contract = struct_param::StructParam::with_host(mock.clone());
    (contract, mock)
}

#[test]
fn struct_param_selector_matches_the_canonical_tuple_signature() {
    let (mut contract, mock) = new_contract();
    let point = struct_param::Point { x: 20, y: 22 };
    let mut input = vec![0u8; point.encode_len()];
    point.encode_to(&mut input);

    // `solc --hashes` for `function sum(Point calldata) external view returns (uint64)`.
    let sel = [0x96, 0x38, 0x2b, 0x79];
    assert_eq!(
        sel,
        pvm_contract_types::const_selector("sum((uint64,uint64))")
    );
    let outcome = struct_param::route(&mut contract, sel, &input);
    assert_eq!(outcome, Some(()));

    let rv = mock
        .take_return_value()
        .expect("contract called return_value");
    assert_eq!(rv.flags, ReturnFlags::empty());
    assert_eq!(u64::decode_at(&rv.data, 0).unwrap(), 42);
}

#[test]
fn struct_param_selector_is_not_hashed_from_the_declared_name() {
    let (mut contract, mock) = new_contract();

    let sel = pvm_contract_types::const_selector("sum(Point)");
    let outcome = struct_param::route(&mut contract, sel, &[0u8; 64]);

    assert_eq!(outcome, None);
    assert!(mock.take_return_value().is_none());
}

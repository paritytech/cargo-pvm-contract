//! A no-alloc `abi_import!` accepts static user-defined types: an enum is a
//! `uint8` word and a struct of static fields is a static tuple. The gate in
//! `to_rust_type` used to delegate to syn-solidity's `is_abi_dynamic`, which
//! hardcodes every custom type as dynamic.
//!
//! Selectors: `keccak("pick(uint8)")[..4]` = `0x92feb99a`,
//! `keccak("add((uint64,uint64))")[..4]` = `0xd5e60374`.
#![allow(clippy::too_many_arguments)]

extern crate alloc;
pub use pvm_contract_sdk::*;

pvm_contract_sdk::abi_import! {
    pragma solidity ^0.8.0;
    enum Color { Red, Green, Blue }
    struct Point { uint64 x; uint64 y; }
    interface Picker {
        function pick(Color c) external returns (Color);
        function add(Point p) external returns (Point);
    }
}

#[test]
fn calldata_for_pick() {
    let (mut input, mut out) = (vec![0u8; 256], vec![0u8; 256]);
    let mock = MockHostBuilder::new().build();
    let host = Host::from_dyn(alloc::rc::Rc::new(mock.clone()));
    let _ = picker::Picker::from_address(Address([0u8; 20]))
        .pick(Color::Blue)
        .call_raw(&mut Context::new(host), &mut input, &mut out);
    assert_eq!(&input[..4], &const_hex::decode("92feb99a").unwrap()[..]);
    // The enum encodes as a full `uint8` word.
    let mut word = [0u8; 32];
    word[31] = 2;
    assert_eq!(&input[4..36], &word);
}

#[test]
fn calldata_for_add() {
    let (mut input, mut out) = (vec![0u8; 256], vec![0u8; 256]);
    let mock = MockHostBuilder::new().build();
    let host = Host::from_dyn(alloc::rc::Rc::new(mock.clone()));
    let _ = picker::Picker::from_address(Address([0u8; 20]))
        .add(Point { x: 1, y: 2 })
        .call_raw(&mut Context::new(host), &mut input, &mut out);
    assert_eq!(&input[..4], &const_hex::decode("d5e60374").unwrap()[..]);
}

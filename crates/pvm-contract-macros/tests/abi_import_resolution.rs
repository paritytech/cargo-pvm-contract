//! Struct-name resolution across multiple `abi_import!` invocations.
//!
//! Two invocations live in the same module: the first defines a *file-level*
//! `Ballot { uint256 id; }`, the second nests its own `Ballot { address; bool; }`
//! inside `interface VoteG`. The nested definition must win when expanding
//! `cast(Ballot)` to its canonical ABI form, so the selector is
//! `keccak("cast((address,bool))")[..4]` = `0x7003a557`. A resolution
//! regression that picks up the file-level `Ballot` from the first invocation
//! would instead hash `cast((uint256))` and produce different bytes.
//!
//! A third invocation covers the reverse direction: a *file-level* struct whose
//! field is a qualified reference to an interface-nested type
//! (`Wrapper { VoteH.Ballot b; }`). The file-level struct is spliced directly
//! at the invocation site, where `mod vote_h` is a sibling item, so the field's
//! Rust path is the bare `vote_h::Ballot` — an unconditional `super::` prefix
//! would escape one module too high (E0433 at crate root, as here). Selector:
//! `keccak("wrap(((address,bool)))")[..4]` = `0xfb9c1acc`.
//!
//! A fourth invocation covers a qualified reference *between* interfaces:
//! `Sums.sum(Kinds.Pair)` must reach the sibling interface module via
//! `super::kinds::Pair` (the reference lives inside `mod sums`, one level
//! below the invocation site). Selector:
//! `keccak("sum((uint64,uint64))")[..4]` = `0x96382b79`.
#![allow(clippy::too_many_arguments)]

extern crate alloc;
pub use pvm_contract_sdk::*;

pvm_contract_sdk::abi_import! {          // invocation 1: file-level Ballot
    #![abi_import(alloc = true)]
    pragma solidity ^0.8.0;
    struct Ballot { uint256 id; }
    interface Registry { function store(Ballot memory b) external; }
}
pvm_contract_sdk::abi_import! {          // invocation 2: nests its own Ballot
    #![abi_import(alloc = true)]
    pragma solidity ^0.8.0;
    interface VoteG {
        enum Choice { Yes, No }
        struct Ballot { address voter; bool support; }
        function cast(Ballot memory b) external;
        function choose(Choice c) external;
    }
}

pvm_contract_sdk::abi_import! {          // invocation 3: file-level struct referencing a nested type
    #![abi_import(alloc = true)]
    pragma solidity ^0.8.0;
    interface VoteH {
        struct Ballot { address voter; bool support; }
        function wrap(Wrapper memory w) external;
    }
    struct Wrapper { VoteH.Ballot b; }
}

pvm_contract_sdk::abi_import! {          // invocation 4: cross-interface qualified reference
    #![abi_import(alloc = true)]
    pragma solidity ^0.8.0;
    interface Kinds {
        struct Pair { uint64 a; uint64 b; }
    }
    interface Sums {
        function sum(Kinds.Pair p) external returns (uint64);
    }
}

#[test]
fn calldata_for_cast() {
    let (mut input, mut out) = (vec![0u8; 256], vec![0u8; 256]);
    let mock = MockHostBuilder::new().build();
    let host = Host::from_dyn(alloc::rc::Rc::new(mock.clone()));
    let _ = vote_g::VoteG::from_address(Address([0u8; 20]))
        .cast(vote_g::Ballot {
            voter: Address([0; 20]),
            support: false,
        })
        .call_raw(&mut Context::new(host), &mut input, &mut out);
    assert_eq!(&input[..4], &const_hex::decode("7003a557").unwrap()[..]);
}

/// An `enum` nested inside an interface, alongside the nested `Ballot`.
/// Interface-nested *structs* are covered four ways above, but an enum reaches
/// the resolver through `Resolution::Local` on a different `CustomDef` branch
/// and is emitted as a different Rust item — the nested-type path is exactly
/// where the original E0412 lived, and nothing else in the suite declares a
/// Solidity enum anywhere but at file level.
/// Selector: `keccak("choose(uint8)")[..4]` = `0xf94e349d`.
#[test]
fn calldata_for_choose() {
    let (mut input, mut out) = (vec![0u8; 256], vec![0u8; 256]);
    let mock = MockHostBuilder::new().build();
    let host = Host::from_dyn(alloc::rc::Rc::new(mock.clone()));
    let _ = vote_g::VoteG::from_address(Address([0u8; 20]))
        .choose(vote_g::Choice::No)
        .call_raw(&mut Context::new(host), &mut input, &mut out);
    assert_eq!(&input[..4], &const_hex::decode("f94e349d").unwrap()[..]);
    // The enum encodes as a full `uint8` word.
    let mut word = [0u8; 32];
    word[31] = 1;
    assert_eq!(&input[4..36], &word);
}

#[test]
fn calldata_for_wrap() {
    let (mut input, mut out) = (vec![0u8; 256], vec![0u8; 256]);
    let mock = MockHostBuilder::new().build();
    let host = Host::from_dyn(alloc::rc::Rc::new(mock.clone()));
    let _ = vote_h::VoteH::from_address(Address([0u8; 20]))
        .wrap(Wrapper {
            b: vote_h::Ballot {
                voter: Address([0; 20]),
                support: false,
            },
        })
        .call_raw(&mut Context::new(host), &mut input, &mut out);
    assert_eq!(&input[..4], &const_hex::decode("fb9c1acc").unwrap()[..]);
}

#[test]
fn calldata_for_sum() {
    let (mut input, mut out) = (vec![0u8; 256], vec![0u8; 256]);
    let mock = MockHostBuilder::new().build();
    let host = Host::from_dyn(alloc::rc::Rc::new(mock.clone()));
    let _ = sums::Sums::from_address(Address([0u8; 20]))
        .sum(kinds::Pair { a: 1, b: 2 })
        .call_raw(&mut Context::new(host), &mut input, &mut out);
    assert_eq!(&input[..4], &const_hex::decode("96382b79").unwrap()[..]);
}

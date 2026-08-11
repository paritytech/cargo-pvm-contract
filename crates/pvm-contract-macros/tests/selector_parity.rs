extern crate alloc;
pub use pvm_contract_sdk::*;

#[test]
#[allow(clippy::too_many_arguments)]
fn selector_parity() {
    mod t {
        use super::*;
        abi_import! {
            #![abi_import(alloc = true)]
            // SPDX-License-Identifier: MIT
            pragma solidity ^0.8.0;

            struct Point {
                uint a;
                uint b;
            }

            interface PointAdder {
                function add(Point a, Point b) external returns (Point);
            }
        }
    }
    mod alloy {
        use alloy_core::sol;

        sol! {
            pragma solidity ^0.8.0;

            struct Point2 {
                uint a;
                uint b;
            }

            contract PointAdderr {
                function add(Point2 a, Point2 b) external returns (Point2);
            }
        }
    }
    let mut input1 = vec![0u8; 5000];

    let mut out = vec![0u8; 5000];
    let mock_host = MockHostBuilder::new().build();
    let host = Host::from_dyn(alloc::rc::Rc::new(mock_host.clone()));
    let _ = t::point_adder::PointAdder::from_address(Address([0u8; 20]))
        .add(
            t::Point {
                a: U256::from(1),
                b: U256::from(1),
            },
            t::Point {
                a: U256::from(1),
                b: U256::from(1),
            },
        )
        .call_raw(&mut Context::new(host), &mut input1, &mut out);
    use alloy_core::sol_types::SolCall;

    let alloy = alloy::PointAdderr::addCall {
        a: alloy::Point2 {
            a: U256::from(1),
            b: U256::from(1),
        },
        b: alloy::Point2 {
            a: U256::from(1),
            b: U256::from(1),
        },
    }
    .abi_encode();
    input1.truncate(alloy.len());
    assert_eq!(input1, alloy)
}

#[allow(clippy::too_many_arguments)]
mod t {
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
            struct Ballot { address voter; bool support; }
            function cast(Ballot memory b) external;
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
}

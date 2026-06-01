#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

pvm_contract_sdk::abi_import! {
#![abi_import(alloc = true)]
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.0;

error Example(string s);
error Example2(string, uint);

type P is uint;

struct Point {
    uint a;
    uint b;
}

interface PointAdder {
    function add(Point a, Point b) external returns (Point);
}
}

#[pvm_contract_sdk::contract("PointAdderCall.sol", allocator = "pico")]
mod point_adder_call {
    use pvm_contract_sdk::*;

    use super::*;
    use point_adder::*;

    pub struct PointAdderCall;
    #[derive(SolError, Debug)]
    pub enum Error {
        CallError(CallError),
    }

    impl PointAdderCall {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), Error> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn call_point_adder(&mut self, addr: Address) -> Result<(), Error> {
            let adder = PointAdder::from_address(addr);
            let call = adder
                .add(
                    Point {
                        a: U256::from(2),
                        b: U256::from(2),
                    },
                    Point {
                        a: U256::from(2),
                        b: U256::from(2),
                    },
                )
                .call(self)?;

            assert_eq!(
                call,
                Point {
                    a: U256::from(4),
                    b: U256::from(4),
                }
            );
            Ok(())
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), Error> {
            Ok(())
        }
    }
}

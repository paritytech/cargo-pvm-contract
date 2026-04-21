#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

extern crate alloc;

#[derive(pvm_contract_macros::SolType)]
pub struct Point {
    pub x: u64,
    pub y: u64,
}

#[derive(pvm_contract_macros::SolType)]
pub struct Line {
    pub a: Point,
    pub b: Point,
}

#[pvm_contract_macros::contract]
mod my_contract {
    use super::{Line, Point};
    use pvm_contract_types::{HostApi, PolkaVmHost};

    pub struct MyContract<H: HostApi = PolkaVmHost> {
        pub host: H,
    }

    impl<H: HostApi> MyContract<H> {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn reflect(&self, line: Line) -> Line {
            line
        }

        #[pvm_contract_macros::method]
        pub fn origin(&self) -> Point {
            Point { x: 0, y: 0 }
        }
    }
}

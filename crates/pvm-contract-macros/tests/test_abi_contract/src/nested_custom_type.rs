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

    #[pvm_contract_macros::constructor]
    pub fn new() {}

    /// Input and output are nested custom types
    #[pvm_contract_macros::method]
    pub fn reflect(line: Line) -> Line {
        line
    }

    /// Input is a flat custom type
    #[pvm_contract_macros::method]
    pub fn origin() -> Point {
        Point { x: 0, y: 0 }
    }
}

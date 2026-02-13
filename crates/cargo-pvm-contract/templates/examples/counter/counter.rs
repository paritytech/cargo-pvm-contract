#![no_main]
#![no_std]

use pvm_contract as pvm;

#[derive(pvm::SolAbi)]
struct Point {
    x: u32,
    y: u32,
}

#[derive(pvm::SolAbi)]
struct Line {
    point1: Point,
    point2: Point,
}

#[pvm::storage]
struct Storage {
    line: Line,
    count: u32,
}

#[pvm::contract]
mod counter {
    use super::*;

    #[pvm::constructor]
    pub fn new() -> Result<(), Error> {
        Storage::count().set(&0);
        Ok(())
    }

    #[pvm::method]
    pub fn increment() {
        let current = Storage::count().get().unwrap_or(0);
        Storage::count().set(&(current + 1));
    }

    #[pvm::method]
    pub fn get_count() -> u32 {
        Storage::count().get().unwrap_or(0)
    }
}

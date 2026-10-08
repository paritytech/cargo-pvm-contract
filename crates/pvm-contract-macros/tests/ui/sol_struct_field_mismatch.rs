// The `.sol` hashes the selector from `Point`'s field layout, but decoding uses
// the Rust struct's fields. Here the Rust `Point` declares `x: u32` where the
// interface says `uint64`, so the two disagree. The generated signature
// assertions catch it at compile time (once on the param, once on the return)
// instead of silently building an uncallable contract.
use pvm_contract_macros::SolType;

#[pvm_contract_macros::contract("tests/ui/fixtures/StructMismatch.sol")]
mod c {
    use super::SolType;

    #[derive(SolType)]
    pub struct Point {
        pub x: u32,
        pub y: u64,
    }

    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn echo(&self, p: Point) -> Point {
            p
        }
    }
}

fn main() {}

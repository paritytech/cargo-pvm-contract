// The signature assertions compare a Rust return against the `.sol` one, but an
// assertion can only be emitted when both sides have a type. A method that
// returns nothing where the interface declares a return has to be rejected
// outright, otherwise the shipped ABI disagrees with what dispatch encodes and
// the mismatch only surfaces on-chain.
#[pvm_contract_macros::contract("tests/ui/fixtures/ReturnArity.sol")]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn a(&self) {}
    }
}

fn main() {}

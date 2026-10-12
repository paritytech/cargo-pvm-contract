// The mirror of `sol_return_arity_mismatch`: the Rust method returns a value the
// `.sol` interface does not declare, so callers decoding per the ABI would read
// return data that is not supposed to exist.
#[pvm_contract_macros::contract("tests/ui/fixtures/ReturnUnexpected.sol")]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        pub fn b(&self) -> u64 {
            7
        }
    }
}

fn main() {}

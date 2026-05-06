#[pvm_contract_macros::contract("tests/ui/fixtures/NonPayableMismatch.sol")]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::method]
        #[pvm_contract_macros::payable]
        pub fn transfer(&mut self, to: pvm_contract_types::Address) {
            let _ = to;
        }
    }
}

fn main() {}

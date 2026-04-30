#[pvm_contract_macros::contract("tests/ui/fixtures/PayableMismatch.sol")]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::method]
        pub fn deposit(&mut self, to: pvm_contract_types::Address) {
            let _ = to;
        }
    }
}

fn main() {}

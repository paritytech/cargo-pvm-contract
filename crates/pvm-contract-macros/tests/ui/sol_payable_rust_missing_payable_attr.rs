#[pvm_contract_macros::contract("tests/ui/fixtures/PayableMismatch.sol")]
mod c {
    #[pvm_contract_macros::method]
    pub fn deposit(to: pvm_contract_types::Address) {
        let _ = to;
    }
}

fn main() {}

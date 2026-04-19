#[pvm_contract_macros::contract("tests/ui/fixtures/NonPayableMismatch.sol")]
mod c {
    #[pvm_contract_macros::method]
    #[pvm_contract_macros::payable]
    pub fn transfer(to: pvm_contract_types::Address) {
        let _ = to;
    }
}

fn main() {}

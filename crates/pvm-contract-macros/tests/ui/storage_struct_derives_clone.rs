// The contract storage struct must not derive `Clone`. The mutation gate
// (`&self` vs `&mut self`) relies on `Storage: !Clone` so that a view method
// cannot smuggle out a `&mut Storage` via cloning.

#[pvm_contract_macros::contract]
mod c {
    #[derive(Clone)]
    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        #[pvm_contract_macros::method]
        pub fn balance(&self) -> pvm_contract_sdk::U256 {
            pvm_contract_sdk::U256::ZERO
        }
    }
}

fn main() {}

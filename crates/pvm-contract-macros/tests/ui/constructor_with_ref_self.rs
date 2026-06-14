// Constructors must take `&mut self` — they always initialize storage. A
// `&self` constructor cannot mutate, so it would be a useless entry point.

#[pvm_contract_macros::contract]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }
    }
}

fn main() {}

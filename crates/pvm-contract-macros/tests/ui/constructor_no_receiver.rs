// Constructors must take `&mut self`. A no-receiver constructor is incoherent
// for the same reason as a `&self` constructor: it cannot write storage.

#[pvm_contract_macros::contract]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new() -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }
    }
}

fn main() {}

// `#[non_reentrant]` is only valid on `#[method]`s — not on a constructor.

#[pvm_contract_macros::contract]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        #[pvm_contract_macros::non_reentrant]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }
    }
}

fn main() {}

// Two `Lazy<(U256, U256)>` fields each consume two consecutive slots.
// foo @ slot 0 occupies 0..2; bar @ slot 1 occupies 1..3 — overlap on slot 1.
// Pre-fix this compiled silently; post-fix it's rejected at const-eval.

extern crate alloc;

#[pvm_contract_macros::contract]
mod c {
    use pvm_contract_sdk::{Lazy, U256};

    pub struct C {
        #[slot(0)]
        foo: Lazy<(U256, U256)>,
        #[slot(1)]
        bar: Lazy<(U256, U256)>,
    }

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }
    }
}

fn main() {}

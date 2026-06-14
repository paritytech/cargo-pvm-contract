// `#[slot(0)] foo: Lazy<(U256, U256)>` occupies slots 0 and 1.
// `#[slot(1)] bar: Lazy<U256>` collides with foo's second word.
// Pre-fix, the proc-macro pairwise check only compared literal slot numbers
// (0 != 1) and silently accepted this layout, producing on-chain corruption.
// Post-fix, a const-eval overlap check fires at compile time.

extern crate alloc;

#[pvm_contract_macros::contract]
mod c {
    use pvm_contract_sdk::{Lazy, U256};

    pub struct C {
        #[slot(0)]
        foo: Lazy<(U256, U256)>,
        #[slot(1)]
        bar: Lazy<U256>,
    }

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }
    }
}

fn main() {}

// `#[non_reentrant]` needs a receiver: it must read (and, for the full guard,
// write) the lock. A pure method has no `self`/host, so there is nothing to
// guard — this must be a compile error.

#[pvm_contract_macros::contract]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        #[pvm_contract_macros::method]
        #[pvm_contract_macros::non_reentrant]
        pub fn pure_guarded(a: u64) -> u64 {
            a
        }
    }
}

fn main() {}

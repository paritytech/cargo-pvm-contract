// Two `#[selector(name = "...")]` on one method would silently keep only the
// first; the macro rejects the duplicate.
#[pvm_contract_macros::contract]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_macros::method]
        #[pvm_contract_macros::selector(name = "a")]
        #[pvm_contract_macros::selector(name = "b")]
        pub fn foo(&self) -> u64 {
            0
        }
    }
}

fn main() {}

#[pvm_contract_macros::contract]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::receive]
        pub fn receive(&mut self) -> u64 {
            42
        }
    }
}

fn main() {}

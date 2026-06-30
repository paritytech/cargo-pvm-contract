
#[pvm_contract_macros::contract]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::receive]
        pub fn receive(&self) {}
    }
}

fn main() {}

#[pvm_contract_macros::contract]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::receive]
        #[pvm_contract_macros::payable]
        pub fn receive(&mut self) {}
    }
}

fn main() {}

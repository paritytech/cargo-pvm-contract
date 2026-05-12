// `#[payable]` requires `&mut self`. A `&self` receiver implies `view`,
// and view methods cannot mutate storage to record the received value.

#[pvm_contract_macros::contract]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::method]
        #[pvm_contract_macros::payable]
        pub fn deposit(&self) {}
    }
}

fn main() {}

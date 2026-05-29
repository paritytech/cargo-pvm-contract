// `#[payable]` requires `&mut self`. An associated function (no receiver)
// is `pure` — it has no host access, so accepting value is incoherent.

#[pvm_contract_macros::contract]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::method]
        #[pvm_contract_macros::payable]
        pub fn deposit() {}
    }
}

fn main() {}

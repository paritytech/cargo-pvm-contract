// A folded interface method parameter must be a concrete type. `Self::Amount`
// (an associated type) has no macro-time ABI selector name.
pub trait IThing {
    type Amount;
    fn deposit(&mut self, amount: Self::Amount);
}

#[pvm_contract_macros::contract(implements(IThing))]
mod c {
    use super::IThing;
    use pvm_contract_sdk::U256;

    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl IThing for C {
        type Amount = U256;
        fn deposit(&mut self, _amount: Self::Amount) {}
    }
}

fn main() {}

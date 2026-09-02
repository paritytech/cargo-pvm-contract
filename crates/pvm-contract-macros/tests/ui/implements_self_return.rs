// A folded interface method whose return type mentions `Self` (`Self::Value`)
// has no ABI name — the router encodes it at module scope where `Self` is
// meaningless. Reject with a clear diagnostic rather than emit invalid code.
pub trait IThing {
    type Value;
    fn value(&self) -> Self::Value;
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
        type Value = U256;
        fn value(&self) -> Self::Value {
            U256::ZERO
        }
    }
}

fn main() {}

// A folded interface method parameter that mentions `Self` *nested* inside a
// generic (`Vec<Self::Item>`) must be rejected with the concrete-type error,
// not produce an opaque downstream failure.
pub trait IThing {
    type Item;
    fn ingest(&mut self, items: Vec<Self::Item>);
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
        type Item = U256;
        fn ingest(&mut self, _items: Vec<Self::Item>) {}
    }
}

fn main() {}

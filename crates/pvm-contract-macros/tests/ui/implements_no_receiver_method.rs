// A folded interface method with no receiver (associated fn) has no host access
// and isn't dispatchable — reject it.
pub trait IThing {
    fn stateless() -> u64;
}

#[pvm_contract_macros::contract(implements(IThing))]
mod thing {
    use super::IThing;

    pub struct Thing;

    impl Thing {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl IThing for Thing {
        fn stateless() -> u64 {
            0
        }
    }
}

fn main() {}

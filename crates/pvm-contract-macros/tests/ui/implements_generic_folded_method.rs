// A folded interface method must not be generic — its selector is undefined.
pub trait IThing {
    fn thing<T>(&self, x: T) -> u64;
}

#[pvm_contract_macros::contract(implements(IThing))]
mod c {
    use super::IThing;

    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl IThing for C {
        fn thing<T>(&self, _x: T) -> u64 {
            0
        }
    }
}

fn main() {}

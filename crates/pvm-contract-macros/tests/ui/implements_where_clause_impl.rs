// A folded interface `impl` must not carry a `where` clause — contracts are
// concrete, non-generic.
pub trait IThing {
    fn thing(&self) -> u64;
}

#[pvm_contract_macros::contract(implements(IThing))]
mod c {
    use super::IThing;

    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl IThing for C
    where
        C: Sized,
    {
        fn thing(&self) -> u64 {
            0
        }
    }
}

fn main() {}

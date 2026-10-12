// Listing the same interface twice in `implements(...)` would fold its methods
// twice (a guaranteed selector collision). Rejected at parse time. Distinct
// same-named traits (`a::IThing`, `b::IThing`) are allowed via qualified paths;
// this is a literal repeat, so it's caught.
pub trait IThing {
    fn thing(&self) -> u64;
}

#[pvm_contract_macros::contract(implements(IThing, IThing))]
mod c {
    use super::IThing;

    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl IThing for C {
        fn thing(&self) -> u64 {
            0
        }
    }
}

fn main() {}

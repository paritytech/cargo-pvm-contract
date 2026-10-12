// `implements(IThing)` matches by last segment. If the contract implements two
// distinct traits that both end in `IThing`, folding both would silently expose
// the unintended trait's methods — so this is a hard error.
pub mod a {
    pub trait IThing {
        fn a_thing(&self) -> u64;
    }
}
pub mod b {
    pub trait IThing {
        fn b_thing(&self) -> u64;
    }
}

#[pvm_contract_macros::contract(implements(IThing))]
mod c {
    use super::{a, b};

    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl a::IThing for C {
        fn a_thing(&self) -> u64 {
            0
        }
    }

    impl b::IThing for C {
        fn b_thing(&self) -> u64 {
            0
        }
    }
}

fn main() {}

// A qualified `implements(a::IThing)` matches only an impl whose path ends in
// `a::IThing`, never a different `b::IThing`. Here only `impl b::IThing for C`
// exists, so `a::IThing` has no match — the qualified path is respected rather
// than silently folding `b::IThing` (which last-segment-only matching would do,
// exposing the wrong interface).
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

#[pvm_contract_macros::contract(implements(a::IThing))]
mod c {
    use super::{a, b};

    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl b::IThing for C {
        fn b_thing(&self) -> u64 {
            0
        }
    }
}

fn main() {}

// `#[cfg]` on the whole folded `impl` block is rejected for the same reason as
// on a single folded method: the fold runs pre-cfg-eval, so a gated-out impl
// would leave dispatch arms referencing a missing method.
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

    #[cfg(feature = "extra")]
    impl IThing for C {
        fn thing(&self) -> u64 {
            0
        }
    }
}

fn main() {}

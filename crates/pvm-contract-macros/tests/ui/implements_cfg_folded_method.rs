// `#[cfg]` on a folded interface method is rejected for now (honoring it —
// a feature-varying dispatch table — is deferred). Use an
// inherent `#[method]` for feature-gated entry points.
pub trait IThing {
    fn always(&self) -> u64;
    fn maybe(&self) -> u64;
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
        fn always(&self) -> u64 {
            0
        }
        #[cfg(feature = "extra")]
        fn maybe(&self) -> u64 {
            1
        }
    }
}

fn main() {}

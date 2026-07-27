// A folded interface `impl` must target the contract struct. Here `impl IThing`
// targets `Other`, not the contract struct `C`.
pub trait IThing {
    fn thing(&self) -> u64;
}

#[pvm_contract_macros::contract(implements(IThing))]
mod c {
    use super::IThing;

    pub struct C;
    pub struct Other;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl IThing for Other {
        fn thing(&self) -> u64 {
            0
        }
    }
}

fn main() {}

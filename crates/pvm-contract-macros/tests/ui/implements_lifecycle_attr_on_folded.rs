// Lifecycle attributes (`#[constructor]`, `#[fallback]`, `#[receive]`) have no
// meaning on a folded interface method — it always dispatches as an ordinary
// method. Rather than silently drop the attribute (giving the author a plain
// method where they expected receive/fallback semantics), reject it and point at
// the inherent path.
pub trait IThing {
    fn ping(&self) -> u64;
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
        #[pvm_contract_macros::receive]
        fn ping(&self) -> u64 {
            42
        }
    }
}

fn main() {}

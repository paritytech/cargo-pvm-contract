// An `implements(...)`-only module (no inherent `#[constructor]`/`#[method]`
// block to name the contract struct) where the interface is implemented for two
// different structs. The contract type is undetermined; without a check, item
// order would silently decide which struct gets routed. Rejected as ambiguous.
pub trait IThing {
    fn thing(&self) -> u64;
}

#[pvm_contract_macros::contract(implements(IThing))]
mod c {
    use super::IThing;

    pub struct A;
    pub struct B;

    impl IThing for A {
        fn thing(&self) -> u64 {
            1
        }
    }

    impl IThing for B {
        fn thing(&self) -> u64 {
            2
        }
    }
}

fn main() {}

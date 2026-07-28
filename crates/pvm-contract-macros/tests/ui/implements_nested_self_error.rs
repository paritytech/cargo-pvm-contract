// A folded method whose error type *nests* `Self` (`Wrapper<Self::Error>`) can't
// be resolved for the ABI: the `<Error = Ty>` binding names only the associated
// type, so substituting the whole type would record `MyErr` in the ABI while the
// runtime encodes `Wrapper<MyErr>`. Rejected — write the error type concretely.
pub struct Wrapper<E>(pub E);

pub trait IFaulty {
    type Error;
    fn maybe(&self) -> Result<u64, Wrapper<Self::Error>>;
}

#[pvm_contract_macros::contract(implements(IFaulty<Error = MyErr>))]
mod c {
    use super::{IFaulty, Wrapper};

    #[derive(Debug, pvm_contract_macros::SolError)]
    pub struct MyErr;

    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl IFaulty for C {
        type Error = MyErr;
        fn maybe(&self) -> Result<u64, Wrapper<Self::Error>> {
            Ok(1)
        }
    }
}

fn main() {}

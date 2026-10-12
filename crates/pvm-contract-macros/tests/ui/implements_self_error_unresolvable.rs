// A folded method's error type may reference `Self` only as exactly `Self::Error`
// (resolved via the `<Error = Ty>` binding). Two shapes are unresolvable and
// rejected: a nested `Self` (`Wrapper<Self::Error>`) and a different associated
// type (`Self::Other`). Substituting the binding for either would record the
// wrong error type in the ABI.
pub struct Wrapper<E>(pub E);

pub trait INested {
    type Error;
    fn maybe(&self) -> Result<u64, Wrapper<Self::Error>>;
}

pub trait IOther {
    type Error;
    type Other;
    fn maybe(&self) -> Result<u64, Self::Other>;
}

#[pvm_contract_macros::contract(implements(INested<Error = MyErr>))]
mod nested {
    use super::{INested, Wrapper};

    #[derive(Debug, pvm_contract_macros::SolError)]
    pub struct MyErr;

    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl INested for C {
        type Error = MyErr;
        fn maybe(&self) -> Result<u64, Wrapper<Self::Error>> {
            Ok(1)
        }
    }
}

#[pvm_contract_macros::contract(implements(IOther<Error = MyErr>))]
mod other {
    use super::IOther;

    #[derive(Debug, pvm_contract_macros::SolError)]
    pub struct MyErr;

    #[derive(Debug, pvm_contract_macros::SolError)]
    pub struct OtherErr;

    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl IOther for C {
        type Error = MyErr;
        type Other = OtherErr;
        fn maybe(&self) -> Result<u64, Self::Other> {
            Ok(1)
        }
    }
}

fn main() {}

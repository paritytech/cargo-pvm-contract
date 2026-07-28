// The `<Error = Ty>` binding names *only* the associated type `Error`. A folded
// method returning a *different* associated type (`Self::Other`) can't be
// resolved from it — binding `MyErr` there would advertise `MyErr` in the ABI
// while the runtime encodes `OtherErr`. Rejected rather than mis-bound.
pub trait IFaulty {
    type Error;
    type Other;
    fn maybe(&self) -> Result<u64, Self::Other>;
}

#[pvm_contract_macros::contract(implements(IFaulty<Error = MyErr>))]
mod c {
    use super::IFaulty;

    #[derive(Debug, pvm_contract_macros::SolError)]
    pub struct MyErr;

    #[derive(Debug, pvm_contract_macros::SolError)]
    pub struct OtherErr;

    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl IFaulty for C {
        type Error = MyErr;
        type Other = OtherErr;
        fn maybe(&self) -> Result<u64, Self::Other> {
            Ok(1)
        }
    }
}

fn main() {}

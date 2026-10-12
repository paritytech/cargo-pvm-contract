// A folded method returns `Result<_, Self::Error>`, so its error type for the
// ABI comes from the `implements(IFaulty<Error = Declared>)` binding. Here the
// binding names `Declared` while the impl's `type Error` is `Actual` — the ABI
// would advertise `Declared` while the runtime encodes `Actual`. The macro emits
// a const-eval identity check that rejects the mismatch at compile time.
pub trait IFaulty {
    type Error;
    fn maybe(&self, ok: bool) -> Result<u64, Self::Error>;
}

#[pvm_contract_macros::contract(implements(IFaulty<Error = Declared>))]
mod c {
    use super::IFaulty;

    #[derive(Debug, pvm_contract_macros::SolError)]
    pub struct Declared;

    #[derive(Debug, pvm_contract_macros::SolError)]
    pub struct Actual;

    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }

    impl IFaulty for C {
        type Error = Actual;
        fn maybe(&self, ok: bool) -> Result<u64, Self::Error> {
            if ok { Ok(1) } else { Err(Actual) }
        }
    }
}

fn main() {}

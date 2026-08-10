// The macro doesn't follow `.sol` imports, so an imported type would resolve to
// its bare name and hash a wrong selector. Reject the interface at parse time
// instead of building a silently-uncallable contract.
#[pvm_contract_macros::contract("tests/ui/fixtures/WithImport.sol")]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }
}

fn main() {}

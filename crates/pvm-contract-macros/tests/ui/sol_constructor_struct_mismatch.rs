// The arity matches, so `check_signature_compatibility` cannot see the drift: it
// skips any parameter involving a custom type, because a proc macro cannot read
// another struct's field layout from tokens. The `.sol` declares a three-field
// `Point` and the Rust one has two, so the published ABI tells deployers to
// encode three words while `deploy()` decodes two — and the deploy size guard is
// a minimum, so the extra word is silently ignored rather than reverting. The
// signature assertion is what closes this.
use pvm_contract_macros::SolType;

#[pvm_contract_macros::contract("tests/ui/fixtures/CtorStructMismatch.sol")]
mod c {
    use super::SolType;

    #[derive(SolType)]
    pub struct Point {
        pub x: u64,
        pub y: u64,
    }

    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self, origin: Point) {
            let _ = origin;
        }
    }
}

fn main() {}

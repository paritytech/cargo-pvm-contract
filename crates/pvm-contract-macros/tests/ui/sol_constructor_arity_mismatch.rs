// A constructor has no selector, so a drifted signature does not produce a dead
// entry point the way a method would — it produces silent mis-initialization.
// The builder emits the `.abi.json` constructor entry from this same `.sol`, so
// deployers would encode one `uint64` that the contract never decodes. Hold the
// constructor to the same parameter check as a `#[method]`.
#[pvm_contract_macros::contract("tests/ui/fixtures/CtorArity.sol")]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {}
    }
}

fn main() {}

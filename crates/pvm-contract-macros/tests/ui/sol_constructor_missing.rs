// The mirror of "Missing implementations" for the constructor. The builder emits
// the `.abi.json` constructor entry from this `.sol`, so leaving it unimplemented
// ships an ABI telling deployers to encode a `uint64` that the default `deploy()`
// never decodes — storage silently stays zero, with no revert to signal it.
#[pvm_contract_macros::contract("tests/ui/fixtures/CtorMissing.sol")]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::method]
        pub fn ping(&self) {}
    }
}

fn main() {}

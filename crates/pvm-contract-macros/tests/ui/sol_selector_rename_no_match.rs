// The `.sol` interface declares `transferTokens`, but `#[selector(name = "transfer")]`
// asks for a function named `transfer`, which the interface does not declare. This
// must be a hard error — not a silent fallback to the Rust-name heuristic that would
// dispatch under `transferTokens` and ignore the rename.
#[pvm_contract_macros::contract("tests/ui/fixtures/SelectorRename.sol")]
mod c {
    pub struct C;

    impl C {
        #[pvm_contract_macros::method]
        #[pvm_contract_macros::selector(name = "transfer")]
        pub fn transfer_tokens(&mut self) {}
    }
}

fn main() {}

// Two offending struct fields produce two diagnostics, each pointing at its
// own field type — pins the per-field error combining in `expand_fields`.
pvm_contract_macros::abi_import! {
    struct Pair {
        string a;
        bytes b;
    }
    interface UsesPair {
        function id(uint256 x) external returns (uint256);
    }
}

fn main() {}

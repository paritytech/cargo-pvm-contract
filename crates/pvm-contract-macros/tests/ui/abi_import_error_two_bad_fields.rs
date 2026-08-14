// Two offending error parameters produce two diagnostics, each pointing at
// its own parameter type — pins the `expand_error` path through
// `expand_fields`.
pvm_contract_macros::abi_import! {
    error E(string a, bytes b);
    interface UsesE {
        function id(uint256 x) external returns (uint256);
    }
}

fn main() {}

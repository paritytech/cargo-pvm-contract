// Two file-level declarations with the same simple name abort the whole
// invocation — pins the `visit_file` boundary conversion in
// `expand_to_module` (spanned at the invocation; a whole-file error has no
// single offending item).
pvm_contract_macros::abi_import! {
    struct S {
        uint256 a;
    }
    struct S {
        uint256 b;
    }
    interface UsesS {
        function id(uint256 x) external returns (uint256);
    }
}

fn main() {}

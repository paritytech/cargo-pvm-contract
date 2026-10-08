// A `.sol` `import` statement aborts the whole invocation — pins the
// `reject_sol_imports` boundary in `expand_to_module` (the sibling
// `sol_import_rejected` test covers the `#[contract]` path only). The
// interface proves the abort kills real output, not just the import line.
pvm_contract_macros::abi_import! {
    import "./other.sol";
    interface Uses {
        function id(uint256 x) external returns (uint256);
    }
}

fn main() {}

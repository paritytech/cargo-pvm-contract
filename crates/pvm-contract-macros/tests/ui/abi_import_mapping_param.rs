// A `mapping` parameter is rejected with a diagnostic. The unsupported-type
// arms used to quote `compile_error!(..);` with a trailing semicolon, which is
// not a parseable `syn::Type`, so this input aborted the proc macro with
// `error: proc macro panicked` instead of reporting the real problem.
extern crate alloc;

pvm_contract_macros::abi_import! {
    #![abi_import(alloc = true)]
    interface M {
        function f(mapping(uint256 => uint256) x) external;
    }
}

fn main() {}

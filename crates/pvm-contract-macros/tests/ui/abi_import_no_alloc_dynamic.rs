// A dynamic parameter without `alloc = true` is reported once. Each resolved
// type is spliced into several positions of the expansion, so an inline
// `compile_error!` used to surface twice — once directly and once more through
// the `concatcp!` selector expression, which gave it a distinct expansion
// backtrace and defeated rustc's duplicate-diagnostic suppression.
pvm_contract_macros::abi_import! {
    interface NoAlloc {
        function greet(string memory who) external;
    }
}

fn main() {}

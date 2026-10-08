// `abi_import!` input has no imports, so a type path deeper than
// `Interface.Type` can never name anything declarable. `parse_path` used to
// keep only the first and last segments, silently resolving `A.X.Point` as
// `A.Point` — a reference solc rejects must be a compile error, not a wrong
// ABI.
extern crate alloc;

pvm_contract_macros::abi_import! {
    #![abi_import(alloc = true)]
    interface A {
        struct Point { uint64 x; uint64 y; }
    }
    interface B {
        function f(A.X.Point p) external;
    }
}

fn main() {}

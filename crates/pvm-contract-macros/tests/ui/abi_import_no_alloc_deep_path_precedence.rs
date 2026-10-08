// A malformed path reports its real problem even in no-alloc mode: the
// resolve-before-gate ordering in `to_rust_type` makes the path-shape error
// win over the "Enable alloc" hint, which would otherwise fire because the
// path's last segment names a dynamic type. (The struct's own field error
// still reports independently.)
pvm_contract_macros::abi_import! {
    struct Named {
        string s;
    }
    interface A {
        function nop(uint256 x) external;
    }
    interface B {
        function f(A.X.Named p) external;
    }
}

fn main() {}

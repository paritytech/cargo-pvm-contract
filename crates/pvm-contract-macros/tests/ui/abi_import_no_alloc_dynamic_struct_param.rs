// A struct whose expanded fields are dynamic is still rejected without
// alloc — the resolving dynamism check sees through the custom type to its
// `string` field, at both the declaration and every use. (Static custom
// types pass the gate; see tests/abi_import_no_alloc_static_custom.rs.)
pvm_contract_macros::abi_import! {
    struct S {
        string a;
    }
    interface Uses {
        function f(S s) external;
    }
}

fn main() {}

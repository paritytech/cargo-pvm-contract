// A user-defined value type whose underlying type needs alloc errors once at
// the underlying type's span — pins the `expand_udt` error path. (solc itself
// rejects non-value underlying types; syn-solidity's grammar accepts any
// elementary type, so the diagnostic comes from `to_rust_type`.)
pvm_contract_macros::abi_import! {
    type MyStr is string;
    interface UsesMyStr {
        function id(uint256 x) external returns (uint256);
    }
}

fn main() {}

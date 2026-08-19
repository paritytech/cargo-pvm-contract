// An undeclared custom type in an `error` declaration is reported as an
// undeclared type (whole-invocation abort from `check_resolvable`), not as
// the misleading "Enable alloc" hint the no-alloc dynamism gate used to
// produce for any unknown custom.
pvm_contract_macros::abi_import! {
    error Bad(Missing m);
    interface I {
        function f(uint256 x) external;
    }
}

fn main() {}

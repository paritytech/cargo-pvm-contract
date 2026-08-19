// Solidity's pre-0.5 `constant` mutability is unsupported. The rejection used
// to be an inline `compile_error!` standing in for the state-mutability type,
// which was spliced into four positions of the expansion — including
// `#state_mutability::default()`, where it was barely well-formed.
extern crate alloc;

pvm_contract_macros::abi_import! {
    #![abi_import(alloc = true)]
    interface C {
        function f(uint256 x) external constant returns (uint256);
    }
}

fn main() {}

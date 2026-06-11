// A `#[derive(SolStorage)]` value struct cannot have a field that is itself a
// struct (a packed value field). The derive rejects it with a compile error
// that points the user at the working, solc-identical approach: make both
// structs `#[storage]` and access via `.view()` / `.view_mut()`.
//
// This pins the tailored hint for the nested-struct (`SolType::Custom`) case,
// distinct from the generic "drop SolStorage for ABI" hint used for other
// unsupported field kinds.

use pvm_contract_macros::{SolStorage, SolType};
use pvm_contract_types::U256;

#[derive(SolType, SolStorage)]
pub struct Reserves {
    pub r0: u128,
    pub r1: u128,
}

// `inner: Reserves` is a nested struct — not supported as a packed value field.
#[derive(SolType, SolStorage)]
pub struct Account {
    pub inner: Reserves,
    pub bal: U256,
}

fn main() {}

// A `#[derive(SolStorage)]` value struct cannot have a dynamic-element array
// field (`Vec<String>`, `Vec<Bytes>`, `Vec<Vec<_>>`): only static-element
// `Vec<T>` (`T: StorageArrayElement`) is supported as a storage value. The
// derive rejects it at expansion time with a tailored hint.

use pvm_contract_macros::{SolStorage, SolType};

#[derive(SolType, SolStorage)]
pub struct Posts {
    pub bodies: Vec<String>,
}

fn main() {}

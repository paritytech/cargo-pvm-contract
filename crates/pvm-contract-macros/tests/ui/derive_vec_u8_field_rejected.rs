// A `#[derive(SolStorage)]` value struct cannot have a `Vec<u8>` field:
// `Vec<u8>` is Solidity `uint8[]`, a different on-chain layout from `bytes`.
// The derive rejects it at expansion time with a tailored hint pointing at
// `Bytes` — instead of surfacing later as an opaque `StorageEncode`
// trait-bound error inside the generated code.

use pvm_contract_macros::{SolStorage, SolType};

#[derive(SolType, SolStorage)]
pub struct Blob {
    pub data: Vec<u8>,
}

fn main() {}

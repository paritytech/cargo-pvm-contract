#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

extern crate alloc;

use pvm_contract_sdk::{Address, Lazy, Mapping, StorageVec, U256};

/// Composable sub-storage struct embedding a `StorageVec`. Verifies that a
/// `StorageVec` inside a `#[storage]` sub-struct flattens into the top-level
/// storage-layout JSON via `StorageLayoutEmit::emit_entries` with a dotted
/// label (`pool.entries`) — the embedded case, distinct from a flat
/// contract-level `StorageVec` field.
#[pvm_contract_sdk::storage]
pub struct PoolState {
    pub owner: Lazy<Address>,
    pub entries: StorageVec<U256>,
}

/// Contract exercising `StorageVec` storage fields. Verifies that the
/// storage-layout JSON emitted under `--features abi-gen` resolves
/// `StorageVec<T>` to Solidity's `T[]` type name across every shape:
/// flat (`uint256[]`), nested (`uint256[][]`), mapping-valued
/// (`mapping(address,uint256[])`), and embedded in a `#[storage]` sub-struct
/// (`pool.entries: uint256[]`) — via the macro's syntactic leaf path
/// (mirroring how `Lazy` / `Mapping` resolve).
#[pvm_contract_sdk::contract]
mod storage_vec {
    use super::*;

    pub struct StorageVecContract {
        pub numbers: StorageVec<U256>,
        pub accounts: StorageVec<Address>,
        pub matrix: StorageVec<StorageVec<U256>>,
        pub buckets: Mapping<Address, StorageVec<U256>>,
        pub pool: PoolState,
    }

    impl StorageVecContract {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

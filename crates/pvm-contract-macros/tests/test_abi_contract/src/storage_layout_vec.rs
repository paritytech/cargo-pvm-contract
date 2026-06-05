#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

extern crate alloc;

use pvm_contract_sdk::{Address, Mapping, StorageVec, U256};

/// Contract exercising `StorageVec` storage fields. Verifies that the
/// storage-layout JSON emitted under `--features abi-gen` resolves
/// `StorageVec<T>` to Solidity's `T[]` type name — including nested
/// (`T[][]`) and mapping-valued (`mapping(K, T[])`) shapes — via the
/// macro's syntactic leaf path (mirroring how `Lazy` / `Mapping` resolve).
#[pvm_contract_sdk::contract]
mod storage_vec {
    use super::*;

    pub struct StorageVecContract {
        pub numbers: StorageVec<U256>,
        pub accounts: StorageVec<Address>,
        pub matrix: StorageVec<StorageVec<U256>>,
        pub buckets: Mapping<Address, StorageVec<U256>>,
    }

    impl StorageVecContract {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }
}

// Regression pin (see CLAUDE.md "Mutability Enforcement"): contract-author code
// with `#![forbid(unsafe_code)]` must still compile even though the storage
// macros expand to `unsafe` internally (`StorageType`/`StorageComponent` impls
// with `unsafe fn` methods and `unsafe { Lazy::new(...) }` bodies). rustc's
// `unsafe_code` lint has `report_in_external_macro = false`, so proc-macro-
// emitted `unsafe` carrying the macro's span does NOT trip `forbid`. Defining a
// `#[derive(SolStorage)]` value struct and a `#[storage]` struct (whose fields
// force `StorageType` on a value struct and a container) is enough to fire the
// lint if it were going to. This is a compile-PASS test.
#![forbid(unsafe_code)]

use pvm_contract_sdk::{Address, Lazy, Mapping, SolStorage, SolType, StorageVec};
use ruint::aliases::U256;

#[derive(SolType, SolStorage)]
pub struct Point {
    pub x: U256,
    pub y: U256,
}

#[pvm_contract_sdk::storage]
pub struct Store {
    pub total: Lazy<U256>,
    pub points: Mapping<Address, Point>,             // value-struct as Mapping value
    pub nums: StorageVec<U256>,                       // leaf vec
    pub buckets: Mapping<Address, StorageVec<U256>>,  // container value
}

fn main() {}

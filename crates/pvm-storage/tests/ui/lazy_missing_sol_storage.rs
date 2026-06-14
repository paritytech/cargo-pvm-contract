//! A struct used as a `Lazy<T>` / `Mapping<_, T>` value must implement
//! `StorageEncode` + `StorageDecode` (in practice: `#[derive(SolStorage)]`).
//! A type that lacks those impls is rejected. It is modelled here as a plain
//! struct (the derive macros aren't a dependency of this UI test crate).
//!
//! This pins the `#[diagnostic::on_unimplemented]` message so the error stays
//! actionable ("add `#[derive(SolStorage)]`") instead of a bare "trait bound
//! not satisfied" pointing at an unfamiliar trait name. The bound is exercised
//! through a generic mirroring how `Lazy<T>` / `Mapping<_, T>` constrain their
//! value type — this is the same `StorageEncode`/`StorageDecode` obligation the
//! `#[contract]` macro emits for a storage field, which is the real footgun
//! path (a direct `Lazy::new` call surfaces an E0599 instead and is unaffected).
use pvm_contract_types::{StorageDecode, StorageEncode};

// A plain struct with no storage impls — it never got `#[derive(SolStorage)]`.
struct MissingSolStorage {
    _a: u128,
    _b: u128,
}

fn store_as_lazy_value<T: StorageEncode + StorageDecode>() {}

fn main() {
    store_as_lazy_value::<MissingSolStorage>();
}

//! `Vec<T>` for `T != u8` has no `StorageEncode` impl, so `Lazy<Vec<U256>>`
//! cannot be constructed. `Vec<u8>` IS supported (as `bytes`), but a generic
//! dynamic array of values is not yet modelled in storage. The workaround
//! is `Mapping<u64, V>`, which treats the index as a key.
use pvm_contract_types::{Host, MockHostBuilder};
use pvm_storage::{Lazy, StorageKey};
use ruint::aliases::U256;
use std::rc::Rc;

fn main() {
    let host = Host::from_dyn(Rc::new(MockHostBuilder::new().build()));
    let _bad = unsafe { Lazy::<Vec<U256>>::new(StorageKey::from_slot(0), 0, host) };
}

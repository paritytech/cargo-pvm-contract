//! `[T; N]` for `T != u8` has no `StorageEncode` impl, so `Lazy<[U256; 3]>`
//! cannot be constructed. Byte-arrays (`[u8; N]`) are the special case that
//! IS supported (mapping to Solidity `bytesN`). Anything else — a fixed
//! array of integers, addresses, structs — has to be modelled via
//! `Mapping<u64, V>` until a generic array accessor lands.
use pvm_contract_types::{Host, MockHostBuilder};
use pvm_storage::{Lazy, StorageKey};
use ruint::aliases::U256;
use std::rc::Rc;

fn main() {
    let host = Host::from_dyn(Rc::new(MockHostBuilder::new().build()));
    let _bad = unsafe { Lazy::<[U256; 3]>::new(StorageKey::from_slot(0), 0, host) };
}

//! `Vec<u8>` has no storage `StorageEncode` impl (its ABI name is `uint8[]`, a
//! different on-chain layout from Solidity `bytes`), so it cannot be a storage
//! value — use `Bytes` for `bytes`-shaped storage. `Vec<T>` for `T:
//! StorageArrayElement` (e.g. `Vec<U256>`) IS supported as of issue #93; only
//! `Vec<u8>` stays rejected here.
use pvm_contract_types::{Bytes, Host, MockHostBuilder};
use pvm_storage::{Lazy, StorageKey};
use std::rc::Rc;

fn main() {
    let host = Host::from_dyn(Rc::new(MockHostBuilder::new().build()));
    // `Vec<U256>` now works (issue #93) — kept here to document the contrast.
    let _ok = unsafe { Lazy::<Vec<ruint::aliases::U256>>::new(StorageKey::from_slot(0), 0, host.clone()) };
    // `Bytes` is the supported `bytes`-shaped value.
    let _bytes = unsafe { Lazy::<Bytes>::new(StorageKey::from_slot(1), 0, host.clone()) };
    // `Vec<u8>` remains rejected: no `StorageEncode` impl.
    let _bad = unsafe { Lazy::<Vec<u8>>::new(StorageKey::from_slot(2), 0, host) };
}

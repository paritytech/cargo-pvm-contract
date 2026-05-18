use pvm_contract_types::{Host, MockHostBuilder};
use pvm_storage::{Lazy, StorageKey};
use std::rc::Rc;

fn main() {
    let host = Host::from_dyn(Rc::new(MockHostBuilder::new().build()));
    // `Vec<u8>` is dynamic — no `StaticEncodedLen` impl — so it can't go
    // through `Lazy<T>`. Users must reach for `LazyBytes` instead.
    let _lazy = Lazy::<Vec<u8>>::new(StorageKey::from_slot(0), host);
}

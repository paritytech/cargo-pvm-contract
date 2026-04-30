use pvm_contract_types::{Host, MockHostBuilder};
use pvm_storage::{Lazy, StorageKey};
use ruint::aliases::U256;
use std::rc::Rc;

fn main() {
    let host = Host::from_dyn(Rc::new(MockHostBuilder::new().build()));
    let lazy = Lazy::<U256>::new(StorageKey::from_slot(0), host);
    lazy.set(&U256::from(42));
}

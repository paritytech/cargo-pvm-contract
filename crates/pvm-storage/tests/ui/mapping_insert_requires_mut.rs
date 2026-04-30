use pvm_contract_types::{Address, Host, MockHostBuilder};
use pvm_storage::{Mapping, StorageKey};
use ruint::aliases::U256;
use std::rc::Rc;

fn main() {
    let host = Host::from_dyn(Rc::new(MockHostBuilder::new().build()));
    let m = Mapping::<Address, U256>::new(StorageKey::from_slot(0), host);
    m.insert(&Address([0xAA; 20]), &U256::from(42));
}

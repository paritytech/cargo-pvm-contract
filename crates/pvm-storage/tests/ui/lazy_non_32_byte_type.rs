use pvm_contract_types::{Host, MockHostBuilder};
use pvm_storage::{Lazy, StorageKey};
use std::rc::Rc;

fn main() {
    let host = Host::from_dyn(Rc::new(MockHostBuilder::new().build()));
    // (U256, U256) is 64 bytes, not 32.  Lazy's const assertion should reject it.
    let _lazy = Lazy::<(ruint::aliases::U256, ruint::aliases::U256)>::new(
        StorageKey::from_slot(0),
        host,
    );
}

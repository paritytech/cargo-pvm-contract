use pvm_contract_types::Address;
use pvm_storage::{Mapping, StorageKey};
use ruint::aliases::U256;

fn main() {
    let m = Mapping::<Address, U256>::new(StorageKey::from_slot(0));
    m.insert(&Address([0xAA; 20]), &U256::from(42));
}

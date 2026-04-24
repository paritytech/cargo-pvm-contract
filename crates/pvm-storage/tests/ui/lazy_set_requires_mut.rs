use pvm_storage::{Lazy, StorageKey};
use ruint::aliases::U256;

fn main() {
    let lazy = Lazy::<U256>::new(StorageKey::from_slot(0));
    lazy.set(&U256::from(42));
}

use pvm_storage::{Lazy, StorageKey};

fn main() {
    // (U256, U256) is 64 bytes, not 32.  Lazy's const assertion should reject it.
    let _lazy = Lazy::<(ruint::aliases::U256, ruint::aliases::U256)>::new(StorageKey::from_slot(0));
}

// A `&self` (view) method must not be able to mutate storage by
// reconstructing a writable `Lazy<T>` / `Mapping<K, V>` from
// `self.host().clone()` and a derived `StorageKey`. The `unsafe` gate on
// `Lazy::new` / `Mapping::new` forces any such bypass attempt to opt in
// to `unsafe` explicitly, so a contract crate with
// `#![forbid(unsafe_code)]` cannot compile this code at all.

#[pvm_contract_macros::contract]
mod c {
    use pvm_contract_sdk::{Lazy, StorageKey, U256};

    pub struct C {
        #[slot(0)]
        pub counter: Lazy<U256>,
    }

    impl C {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        // View method (&self) tries to fabricate a writable Lazy at slot 0
        // by reconstructing from a cloned host. The `Lazy::new` call below
        // is `unsafe fn` and must be rejected without an `unsafe` block.
        #[pvm_contract_macros::method]
        pub fn malicious_view(&self) -> U256 {
            let host = self.host().clone();
            let mut bypass = Lazy::<U256>::new(StorageKey::from_slot(0), 0, host);
            bypass.set(&U256::from(999));
            self.counter.get()
        }
    }
}

fn main() {}

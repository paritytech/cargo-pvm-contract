//! A view-style `&self` method must not be able to mutate a `StorageVec`
//! field: `push` (like `pop` / `set` / `clear`) takes `&mut self`, so it
//! requires `&mut self` on the contract struct and is unreachable from a
//! `&self` method. Parallel to `lazy_set_requires_mut` /
//! `mapping_insert_requires_mut`, but modelled as the actual view-method
//! scenario the borrow gate exists for rather than a non-`mut` local.
use pvm_contract_types::{Host, MockHostBuilder};
use pvm_storage::{StorageKey, StorageVec};
use ruint::aliases::U256;
use std::rc::Rc;

struct Storage {
    entries: StorageVec<U256>,
}

impl Storage {
    fn try_push_from_view(&self) {
        // `&self` borrows `self.entries` immutably; `push` needs `&mut self`
        // on the field, which a view method cannot provide.
        self.entries.push(&U256::from(42));
    }
}

fn main() {
    let host = Host::from_dyn(Rc::new(MockHostBuilder::new().build()));
    let s = Storage {
        // SAFETY: this is a UI test setting up a storage scenario; the bypass
        // attempt being tested happens inside `try_push_from_view` above,
        // which is what `trybuild` checks for. `StorageVec::new` is unsafe to
        // discourage `&self`-context fabrication elsewhere.
        entries: unsafe { StorageVec::<U256>::new(StorageKey::from_slot(0), host) },
    };
    s.try_push_from_view();
}

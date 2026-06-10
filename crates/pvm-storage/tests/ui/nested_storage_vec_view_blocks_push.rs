//! Regression test: a view-style `&self` method must not be able to mutate
//! storage through a nested `StorageVec<StorageVec<T>>::get()` chain. `get`
//! takes `&self` and returns `Ref<'_, StorageVec<T>>`, which only forwards
//! `&self` methods (no `DerefMut`) — so the inner vec's `&mut self` mutators
//! (`push` / `set` / `pop` / `grow`) are unreachable through it. Parallel to
//! `nested_mapping_view_blocks_insert`.
use pvm_contract_types::{Host, MockHostBuilder};
use pvm_storage::{StorageKey, StorageVec};
use ruint::aliases::U256;
use std::rc::Rc;

struct Storage {
    matrix: StorageVec<StorageVec<U256>>,
}

impl Storage {
    fn try_bypass_view(&self) {
        // `self.matrix.get(0)` returns `Ref<'_, StorageVec<U256>>`, which has
        // no `DerefMut` impl — `push` requires `&mut self` on the inner vec
        // and is therefore unreachable through the read-only guard.
        let mut inner = self.matrix.get(0);
        inner.push(&U256::from(9999));
    }
}

fn main() {
    let host = Host::from_dyn(Rc::new(MockHostBuilder::new().build()));
    let s = Storage {
        // SAFETY: this is a UI test setting up a storage scenario; the bypass
        // attempt being tested happens inside `try_bypass_view` above, which
        // is what `trybuild` checks for. `StorageVec::new` is unsafe to
        // discourage `&self`-context fabrication elsewhere.
        matrix: unsafe { StorageVec::<StorageVec<U256>>::new(StorageKey::from_slot(0), host) },
    };
    s.try_bypass_view();
}

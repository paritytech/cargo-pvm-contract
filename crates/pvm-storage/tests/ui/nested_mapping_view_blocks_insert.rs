//! Regression test: a view-style `&self` method must not be able to mutate
//! storage through a nested-mapping `.get()` chain. Before the `Ref` /
//! `RefMut` split, `Mapping<K1, Mapping<K2, V>>::get(&self)` returned an
//! owned `Mapping<K2, V>` that the caller could bind as `mut` and call
//! `.insert()` on — bypassing the borrow checker's view enforcement. After
//! the fix, `.get()` returns `Ref<'_, Mapping<K2, V>>` which only forwards
//! `&self` methods.
use pvm_contract_types::{Address, Host, MockHostBuilder};
use pvm_storage::{Mapping, StorageKey};
use ruint::aliases::U256;
use std::rc::Rc;

struct Storage {
    allowances: Mapping<Address, Mapping<Address, U256>>,
}

impl Storage {
    fn try_bypass_view(&self, owner: Address, spender: Address) {
        // `self.allowances.get(&owner)` now returns `Ref<'_, Mapping<...>>`,
        // which has no `DerefMut` impl — `insert` requires `&mut self` on
        // the inner mapping and is therefore unreachable.
        let mut inner = self.allowances.get(&owner);
        inner.insert(&spender, &U256::from(9999));
    }
}

fn main() {
    let host = Host::from_dyn(Rc::new(MockHostBuilder::new().build()));
    let s = Storage {
        // SAFETY: this is a UI test setting up a storage scenario; the
        // bypass attempt being tested happens inside `try_bypass_view`
        // above, which is what `trybuild` checks for. `Mapping::new` is
        // unsafe to discourage `&self`-context fabrication elsewhere.
        allowances: unsafe { Mapping::new(StorageKey::from_slot(0), host) },
    };
    s.try_bypass_view(Address([0x11; 20]), Address([0x22; 20]));
}

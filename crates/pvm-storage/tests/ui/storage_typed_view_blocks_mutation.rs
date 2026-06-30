//! Regression test: the storage-typed `Mapping<K, V: StorageComponent>::view`
//! returns a `Ref<'_, V>` that only forwards `&self` methods on the inner
//! component. A view (`&self`) caller cannot reach `&mut self` methods —
//! `Lazy::set`, `Mapping::insert`, etc. — through it.
//!
//! Mirrors the value-typed `Mapping::entry` view-enforcement test but for
//! the new storage-typed path that subsumes the special-case nested-Mapping
//! impl.
use pvm_contract_types::{Address, Host, MockHostBuilder};
use pvm_storage::{Lazy, Mapping, StorageKey};
use ruint::aliases::U256;
use std::rc::Rc;

struct Registry {
    by_addr: Mapping<Address, Lazy<U256>>,
}

impl Registry {
    fn try_bypass_view(&self, addr: Address) {
        // `self.by_addr.view(&addr)` returns `Ref<'_, Lazy<U256>>`, which
        // has no `DerefMut` impl. `Lazy::set` requires `&mut self` and is
        // therefore unreachable.
        let mut inner = self.by_addr.view(&addr);
        inner.set(&U256::from(9999));
    }
}

fn main() {
    let host = Host::from_dyn(Rc::new(MockHostBuilder::new().build()));
    let r = Registry {
        // SAFETY: bypass attempt happens in `try_bypass_view`, which is what
        // trybuild checks.
        by_addr: unsafe { Mapping::new(StorageKey::from_slot(0), host) },
    };
    r.try_bypass_view(Address([0x11; 20]));
}

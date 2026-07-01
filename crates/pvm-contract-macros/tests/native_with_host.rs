#![cfg(not(feature = "abi-gen"))]
//! Verifies the macro-generated `Contract::with_host(backend)` helper.
//!
//! Matches the std-lib `Vec::with_capacity` / `HashMap::with_capacity`
//! idiom for "constructor with a non-default dependency." Wraps any
//! `HostApi` backend in `Rc<dyn HostApi>` and initialises `#[slot(N)]`
//! fields; the user's `#[constructor]` is NOT invoked.

use pvm_contract_sdk::MockHostBuilder;
use ruint::aliases::U256;

#[allow(dead_code)]
#[pvm_contract_sdk::contract]
mod counter {
    use pvm_contract_sdk::StorageFlags;
    use ruint::aliases::U256;

    const KEY: [u8; 32] = [0u8; 32];

    pub struct Counter;

    impl Counter {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_sdk::method]
        pub fn increment(&mut self) {
            let next = self.read() + U256::from(1u64);
            self.write(next);
        }

        #[pvm_contract_sdk::method]
        pub fn get(&self) -> U256 {
            self.read()
        }

        fn read(&self) -> U256 {
            let mut buf = [0u8; 32];
            self.host()
                .get_storage_or_zero(StorageFlags::empty(), &KEY, &mut buf);
            U256::from_be_bytes::<32>(buf)
        }

        fn write(&mut self, value: U256) {
            self.host()
                .set_storage(StorageFlags::empty(), &KEY, &value.to_be_bytes::<32>());
        }
    }
}

use counter::Counter;

#[test]
fn with_host_zero_state() {
    // One line — `with_host` does all the wrapping in Rc + Host::from_dyn.
    let counter = Counter::with_host(MockHostBuilder::new().build());
    assert_eq!(counter.get(), U256::ZERO);
}

#[test]
fn with_host_can_seed_storage_via_mock() {
    let mock = MockHostBuilder::new().build();
    let mut seeded = [0u8; 32];
    seeded[31] = 42;
    mock.set_raw_storage([0u8; 32].to_vec(), seeded.to_vec());

    let counter = Counter::with_host(mock);
    assert_eq!(counter.get(), U256::from(42u64));
}

#[test]
fn with_host_mutating_methods_persist() {
    let mut counter = Counter::with_host(MockHostBuilder::new().build());
    counter.increment();
    counter.increment();
    counter.increment();
    assert_eq!(counter.get(), U256::from(3u64));
}

// --- Verify #[slot(N)] field initialisation via with_host ---

#[allow(dead_code)]
#[pvm_contract_sdk::contract]
mod slot_contract {
    use pvm_contract_sdk::{Address, Lazy, Mapping};
    use ruint::aliases::U256;

    pub struct SlotContract {
        #[slot(0)]
        total: Lazy<U256>,
        #[slot(1)]
        balances: Mapping<Address, U256>,
    }

    impl SlotContract {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}

        #[pvm_contract_sdk::method]
        pub fn total(&self) -> U256 {
            self.total.get()
        }

        #[pvm_contract_sdk::method]
        pub fn set_total(&mut self, value: U256) {
            self.total.set(&value);
        }

        #[pvm_contract_sdk::method]
        pub fn balance_of(&self, who: Address) -> U256 {
            self.balances.get(&who)
        }

        #[pvm_contract_sdk::method]
        pub fn credit(&mut self, who: Address, amount: U256) {
            let current = self.balances.get(&who);
            self.balances.insert(&who, &(current + amount));
        }
    }
}

#[test]
fn with_host_initialises_slot_fields() {
    use pvm_contract_sdk::Address;
    use slot_contract::SlotContract;

    let mut contract = SlotContract::with_host(MockHostBuilder::new().build());

    // Slot fields are wired up — reads return defaults, writes persist.
    assert_eq!(contract.total(), U256::ZERO);
    contract.set_total(U256::from(100u64));
    assert_eq!(contract.total(), U256::from(100u64));

    let alice = Address::from([0xA1; 20]);
    assert_eq!(contract.balance_of(alice), U256::ZERO);
    contract.credit(alice, U256::from(42u64));
    assert_eq!(contract.balance_of(alice), U256::from(42u64));
}

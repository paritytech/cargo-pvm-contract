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

// --- Verify the macro-injected env() accessor against a MockHost backend ---
//
// The macro emits `pub fn env(&self) -> Env { self.host.env() }` on the storage
// struct. That injection is otherwise only checked as a token string in the
// macro's own unit tests, which cannot catch a wrong field, a wrong host
// function behind an accessor, or a `&mut self`-only receiver. These tests run
// the real thing: seed context on a MockHost, read it back through `self.env()`
// from inside method bodies.

#[allow(dead_code)]
#[pvm_contract_sdk::contract]
mod env_reader {
    use pvm_contract_sdk::{Address, Lazy};
    use ruint::aliases::U256;

    pub struct EnvReader {
        // Auto-numbered: `Lazy<Address>` is sub-word (20 bytes), and explicit
        // `#[slot(N)]` is restricted to full-slot types.
        last_caller: Lazy<Address>,
    }

    impl EnvReader {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}

        // Every accessor read from a `&self` (view) method — the receiver that
        // would fail to compile if `env()` were injected as `&mut self`.
        #[pvm_contract_sdk::method]
        pub fn who_called(&self) -> Address {
            self.env().caller()
        }

        #[pvm_contract_sdk::method]
        pub fn who_signed(&self) -> Address {
            self.env().origin()
        }

        #[pvm_contract_sdk::method]
        pub fn me(&self) -> Address {
            self.env().address()
        }

        #[pvm_contract_sdk::method]
        pub fn block_info(&self) -> (u64, u64, u64) {
            let env = self.env();
            (env.block_number(), env.timestamp(), env.chain_id())
        }

        #[pvm_contract_sdk::method]
        #[pvm_contract_sdk::payable]
        pub fn deposited(&mut self) -> U256 {
            self.env().value()
        }

        // Storage write keyed off context: the mixed `self.env()` +
        // `self.<field>` use that a wrongly-injected accessor would break.
        #[pvm_contract_sdk::method]
        pub fn record_caller(&mut self) {
            let caller = self.env().caller();
            self.last_caller.set(&caller);
        }

        #[pvm_contract_sdk::method]
        pub fn last_caller(&self) -> Address {
            self.last_caller.get()
        }
    }
}

#[test]
fn with_host_env_reads_seeded_context() {
    use env_reader::EnvReader;
    use pvm_contract_sdk::Address;

    let mut contract = EnvReader::with_host(
        MockHostBuilder::new()
            .caller([0xAA; 20])
            .origin([0xBB; 20])
            .address([0xCC; 20])
            .value_transferred(U256::from(7u64))
            .block_number(258)
            .block_timestamp(1_700_000_000)
            .chain_id(420)
            .build(),
    );

    assert_eq!(contract.who_called(), Address::from([0xAA; 20]));
    assert_eq!(contract.who_signed(), Address::from([0xBB; 20]));
    assert_eq!(contract.me(), Address::from([0xCC; 20]));
    assert_eq!(contract.block_info(), (258, 1_700_000_000, 420));
    assert_eq!(contract.deposited(), U256::from(7u64));
}

#[test]
fn with_host_env_defaults_to_zero() {
    use env_reader::EnvReader;
    use pvm_contract_sdk::Address;

    // An unseeded MockHost reports zeroes rather than panicking, so a test that
    // does not care about context need not seed any.
    let contract = EnvReader::with_host(MockHostBuilder::new().build());

    assert_eq!(contract.who_called(), Address::from([0u8; 20]));
    assert_eq!(contract.block_info(), (0, 0, 0));
}

#[test]
fn with_host_env_drives_a_storage_write() {
    use env_reader::EnvReader;
    use pvm_contract_sdk::Address;

    let mut contract = EnvReader::with_host(MockHostBuilder::new().caller([0xAA; 20]).build());

    assert_eq!(contract.last_caller(), Address::from([0u8; 20]));
    contract.record_caller();
    assert_eq!(contract.last_caller(), Address::from([0xAA; 20]));
}

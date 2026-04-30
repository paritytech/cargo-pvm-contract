#![cfg(not(feature = "abi-gen"))]
//! End-to-end native unit tests for a realistic contract.
//!
//! Exercises the full dispatch pipeline against `MockHost`:
//! calldata → selector routing → decode → state mutation (storage + events)
//! → encoded return bytes / ABI-encoded revert payload. This is the same
//! path a deployed contract takes on riscv64 minus the `return_value`
//! syscall at the very end.
//!
//! The contract (`mini_token`) models a tiny ERC20-like token:
//! - An `owner` stored in slot 0 (set at construction via `caller()`)
//! - A per-address balance mapping (key = `0x01 || 12 zero bytes || address`)
//! - `mint(to, amount)` — owner-gated, credits and emits `Transfer(0x0, to, amount)`
//! - `transfer(to, amount)` — debits caller, credits recipient, emits `Transfer`
//! - `balance_of(addr)` — pure read
//! - `owner()` — pure read
//!
//! Each method uses `self.host().get_storage` / `set_storage` /
//! `deposit_event` / `caller`. None of it runs on `HostFnImpl` in these
//! tests — all host calls route through `MockHost`.

use pvm_contract_types::{
    Address, Host, MockHost, MockHostBuilder, ReturnFlags, Router, SolDecode, SolEncode,
    StaticEncodedLen,
};
use ruint::aliases::U256;

const OWNER_SLOT: [u8; 32] = [0u8; 32];

fn balance_key(addr: Address) -> [u8; 32] {
    let mut key = [0u8; 32];
    key[0] = 0x01; // prefix to avoid colliding with owner slot
    key[12..].copy_from_slice(addr.as_ref() as &[u8; 20]);
    key
}

fn transfer_topic_0() -> [u8; 32] {
    // keccak256("Transfer(address,address,uint256)")
    [
        0xdd, 0xf2, 0x52, 0xad, 0x1b, 0xe2, 0xc8, 0x9b, 0x69, 0xc2, 0xb0, 0x68, 0xfc, 0x37, 0x8d,
        0xaa, 0x95, 0x2b, 0xa7, 0xf1, 0x63, 0xc4, 0xa1, 0x16, 0x28, 0xf5, 0x5a, 0x4d, 0xf5, 0x23,
        0xb3, 0xef,
    ]
}

fn addr_topic(addr: Address) -> [u8; 32] {
    let mut t = [0u8; 32];
    t[12..].copy_from_slice(addr.as_ref() as &[u8; 20]);
    t
}

#[allow(dead_code)] // `new()` runs through deploy() (riscv64-gated)
#[pvm_contract_macros::contract]
mod mini_token {
    use super::*;
    use pvm_contract_types::StorageFlags;

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub struct Unauthorized;

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub struct InsufficientBalance {
        pub available: U256,
        pub required: U256,
    }

    pub struct MiniToken;

    impl MiniToken {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) {
            let mut caller_bytes = [0u8; 20];
            self.host().caller(&mut caller_bytes);
            let mut slot = [0u8; 32];
            slot[12..].copy_from_slice(&caller_bytes);
            self.host()
                .set_storage(StorageFlags::empty(), &OWNER_SLOT, &slot);
        }

        #[pvm_contract_macros::method]
        pub fn owner(&self) -> Address {
            let mut buf = [0u8; 32];
            self.host()
                .get_storage_or_zero(StorageFlags::empty(), &OWNER_SLOT, &mut buf);
            let mut addr = [0u8; 20];
            addr.copy_from_slice(&buf[12..]);
            Address::from(addr)
        }

        #[pvm_contract_macros::method]
        pub fn balance_of(&self, account: Address) -> U256 {
            let key = balance_key(account);
            let mut buf = [0u8; 32];
            self.host()
                .get_storage_or_zero(StorageFlags::empty(), &key, &mut buf);
            U256::from_be_bytes::<32>(buf)
        }

        #[pvm_contract_macros::method]
        pub fn mint(&mut self, to: Address, amount: U256) -> Result<(), Unauthorized> {
            let mut caller_bytes = [0u8; 20];
            self.host().caller(&mut caller_bytes);
            let owner = self.owner();
            if caller_bytes != *<Address as AsRef<[u8; 20]>>::as_ref(&owner) {
                return Err(Unauthorized);
            }

            let current = self.balance_of(to);
            let new_balance = current + amount;
            let key = balance_key(to);
            self.host().set_storage(
                StorageFlags::empty(),
                &key,
                &new_balance.to_be_bytes::<32>(),
            );

            let topics = [
                transfer_topic_0(),
                addr_topic(Address::ZERO),
                addr_topic(to),
            ];
            self.host()
                .deposit_event(&topics, &amount.to_be_bytes::<32>());
            Ok(())
        }

        #[pvm_contract_macros::method]
        pub fn transfer(&mut self, to: Address, amount: U256) -> Result<(), InsufficientBalance> {
            let mut caller_bytes = [0u8; 20];
            self.host().caller(&mut caller_bytes);
            let from = Address::from(caller_bytes);

            let available = self.balance_of(from);
            if available < amount {
                return Err(InsufficientBalance {
                    available,
                    required: amount,
                });
            }

            let from_key = balance_key(from);
            let to_key = balance_key(to);
            let new_from = available - amount;
            let current_to = self.balance_of(to);
            let new_to = current_to + amount;

            self.host().set_storage(
                StorageFlags::empty(),
                &from_key,
                &new_from.to_be_bytes::<32>(),
            );
            self.host()
                .set_storage(StorageFlags::empty(), &to_key, &new_to.to_be_bytes::<32>());

            let topics = [transfer_topic_0(), addr_topic(from), addr_topic(to)];
            self.host()
                .deposit_event(&topics, &amount.to_be_bytes::<32>());
            Ok(())
        }
    }
}

// --- Test helpers ---

const OWNER: [u8; 20] = [0xA0; 20];
const ALICE: [u8; 20] = [0xA1; 20];
const BOB: [u8; 20] = [0xB0; 20];
const CHARLIE: [u8; 20] = [0xC0; 20];

/// Build a fresh host with `caller` set. Storage is empty.
fn host_with_caller(caller: [u8; 20]) -> MockHost {
    MockHostBuilder::new().caller(caller).build()
}

/// Wire a `MockHost` into a fresh `MiniToken`. The contract and the returned
/// `MockHost` share state (`Rc<RefCell<_>>` internally), so assertions can
/// read back storage/events through the returned handle.
fn make_contract(mock: &MockHost) -> mini_token::MiniToken {
    mini_token::MiniToken {
        host: Host::from_dyn(::std::rc::Rc::new(mock.clone())),
    }
}

/// Seed the owner slot directly (simulates a prior `new()` invocation).
fn seed_owner(host: &MockHost, owner: [u8; 20]) {
    let mut slot = [0u8; 32];
    slot[12..].copy_from_slice(&owner);
    host.set_raw_storage(OWNER_SLOT.to_vec(), slot.to_vec());
}

/// Seed a balance mapping slot directly.
fn seed_balance(host: &MockHost, addr: [u8; 20], amount: U256) {
    let key = balance_key(Address::from(addr));
    host.set_raw_storage(key.to_vec(), amount.to_be_bytes::<32>().to_vec());
}

fn selector(sig: &str) -> [u8; 4] {
    pvm_contract_types::const_selector(sig)
}

fn encode_transfer_calldata(to: Address, amount: U256) -> Vec<u8> {
    // (address, uint256) encodes as two 32-byte words (ABI head).
    const LEN: usize =
        <Address as StaticEncodedLen>::ENCODED_SIZE + <U256 as StaticEncodedLen>::ENCODED_SIZE;
    let mut buf = vec![0u8; LEN];
    (to, amount).encode_to(&mut buf);
    buf
}

fn encode_mint_calldata(to: Address, amount: U256) -> Vec<u8> {
    encode_transfer_calldata(to, amount) // same shape (address, uint256)
}

fn encode_balance_of_calldata(addr: Address) -> Vec<u8> {
    let mut buf = vec![0u8; <Address as StaticEncodedLen>::ENCODED_SIZE];
    addr.encode_to(&mut buf);
    buf
}

fn read_balance(host: &MockHost, addr: [u8; 20]) -> U256 {
    let key = balance_key(Address::from(addr));
    let raw = host.get_raw_storage(&key).unwrap_or_default();
    if raw.is_empty() {
        return U256::ZERO;
    }
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(&raw);
    U256::from_be_bytes::<32>(bytes)
}

/// Call `route()`, expect a successful match (selector handled and
/// `return_value` called with `flags == empty`), and return the captured
/// encoded data.
fn route_ok(
    contract: &mut mini_token::MiniToken,
    mock: &MockHost,
    sel: [u8; 4],
    input: &[u8],
) -> Vec<u8> {
    let outcome = mini_token::route(contract, sel, input);
    assert_eq!(outcome, Some(()), "expected matched selector");
    let rv = mock
        .take_return_value()
        .expect("contract called return_value");
    assert_eq!(rv.flags, ReturnFlags::empty(), "expected success flags");
    rv.data
}

/// Call `route()`, expect a revert (selector handled and `return_value`
/// called with `flags == REVERT`), and return the captured revert payload.
fn route_revert(
    contract: &mut mini_token::MiniToken,
    mock: &MockHost,
    sel: [u8; 4],
    input: &[u8],
) -> Vec<u8> {
    let outcome = mini_token::route(contract, sel, input);
    assert_eq!(outcome, Some(()), "expected matched selector");
    let rv = mock
        .take_return_value()
        .expect("contract called return_value");
    assert_eq!(rv.flags, ReturnFlags::REVERT, "expected REVERT flags");
    rv.data
}

// --- Tests ---

#[test]
fn owner_returns_stored_address() {
    let mock = host_with_caller(OWNER);
    seed_owner(&mock, OWNER);
    let mut contract = make_contract(&mock);

    let data = route_ok(&mut contract, &mock, selector("owner()"), &[]);

    let returned = Address::decode_at(&data, 0);
    assert_eq!(returned, Address::from(OWNER));
}

#[test]
fn balance_of_returns_zero_for_untouched_address() {
    let mock = host_with_caller(ALICE);
    let mut contract = make_contract(&mock);

    let input = encode_balance_of_calldata(Address::from(ALICE));
    let data = route_ok(&mut contract, &mock, selector("balanceOf(address)"), &input);

    assert_eq!(U256::decode_at(&data, 0), U256::ZERO);
}

#[test]
fn mint_by_owner_credits_balance_and_emits_transfer_event() {
    let mock = host_with_caller(OWNER);
    seed_owner(&mock, OWNER);
    let mut contract = make_contract(&mock);

    let input = encode_mint_calldata(Address::from(ALICE), U256::from(1000u64));

    let data = route_ok(
        &mut contract,
        &mock,
        selector("mint(address,uint256)"),
        &input,
    );
    assert_eq!(data, &[] as &[u8], "void success returns empty data");

    // Storage side-effect
    assert_eq!(read_balance(&mock, ALICE), U256::from(1000u64));

    // Event side-effect: one Transfer(0x0, ALICE, 1000)
    let events = mock.events();
    assert_eq!(events.len(), 1);
    let (topics, payload) = &events[0];
    assert_eq!(topics.len(), 3);
    assert_eq!(topics[0], transfer_topic_0());
    assert_eq!(topics[1], addr_topic(Address::ZERO));
    assert_eq!(topics[2], addr_topic(Address::from(ALICE)));
    assert_eq!(
        U256::from_be_bytes::<32>(payload.as_slice().try_into().unwrap()),
        U256::from(1000u64)
    );
}

#[test]
fn mint_by_non_owner_reverts_with_unauthorized() {
    // Owner is OWNER; caller is BOB — mint must revert.
    let mock = host_with_caller(BOB);
    seed_owner(&mock, OWNER);
    let mut contract = make_contract(&mock);

    let input = encode_mint_calldata(Address::from(BOB), U256::from(100u64));

    let data = route_revert(
        &mut contract,
        &mock,
        selector("mint(address,uint256)"),
        &input,
    );

    // Revert payload is exactly the 4-byte `Unauthorized()` selector — no fields.
    assert_eq!(data, selector("Unauthorized()"));

    // State untouched: no balance change, no events.
    assert_eq!(read_balance(&mock, BOB), U256::ZERO);
    assert!(mock.events().is_empty());
}

#[test]
fn transfer_happy_path_moves_balance_and_emits_event() {
    let mock = host_with_caller(ALICE);
    seed_balance(&mock, ALICE, U256::from(500u64));
    let mut contract = make_contract(&mock);

    let input = encode_transfer_calldata(Address::from(BOB), U256::from(200u64));

    let data = route_ok(
        &mut contract,
        &mock,
        selector("transfer(address,uint256)"),
        &input,
    );
    assert_eq!(data, &[] as &[u8]);

    assert_eq!(read_balance(&mock, ALICE), U256::from(300u64));
    assert_eq!(read_balance(&mock, BOB), U256::from(200u64));

    let events = mock.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0[1], addr_topic(Address::from(ALICE)));
    assert_eq!(events[0].0[2], addr_topic(Address::from(BOB)));
}

#[test]
fn transfer_insufficient_balance_reverts_with_encoded_fields() {
    let mock = host_with_caller(ALICE);
    seed_balance(&mock, ALICE, U256::from(50u64));
    let mut contract = make_contract(&mock);

    let input = encode_transfer_calldata(Address::from(BOB), U256::from(100u64));

    let data = route_revert(
        &mut contract,
        &mock,
        selector("transfer(address,uint256)"),
        &input,
    );

    // Expected revert: selector + ABI-encoded (available: U256, required: U256)
    let expected_selector = selector("InsufficientBalance(uint256,uint256)");
    assert_eq!(&data[..4], &expected_selector, "revert selector");
    let available = U256::decode_at(&data[4..], 0);
    let required = U256::decode_at(&data[4..], 32);
    assert_eq!(available, U256::from(50u64));
    assert_eq!(required, U256::from(100u64));
    assert_eq!(data.len(), 4 + 64, "no trailing bytes");

    // State unchanged, no events emitted.
    assert_eq!(read_balance(&mock, ALICE), U256::from(50u64));
    assert_eq!(read_balance(&mock, BOB), U256::ZERO);
    assert!(mock.events().is_empty());
}

#[test]
fn short_input_reverts_with_framework_invalid_calldata() {
    let mock = host_with_caller(ALICE);
    let mut contract = make_contract(&mock);

    let short = [0u8; 10]; // need 64 bytes for (Address, U256)

    let data = route_revert(
        &mut contract,
        &mock,
        selector("transfer(address,uint256)"),
        &short,
    );
    assert_eq!(
        data,
        pvm_contract_types::framework_errors::INVALID_CALLDATA.as_slice()
    );
}

#[test]
fn unknown_selector_returns_unhandled() {
    let mock = host_with_caller(ALICE);
    let mut contract = make_contract(&mock);

    let outcome = mini_token::route(&mut contract, [0xDE, 0xAD, 0xBE, 0xEF], &[]);
    assert_eq!(outcome, None);
    assert!(mock.take_return_value().is_none());
}

#[test]
fn full_lifecycle_mint_transfer_transfer() {
    // Step 1 — OWNER mints 1000 to ALICE.
    let mock = host_with_caller(OWNER);
    seed_owner(&mock, OWNER);
    let mut contract = make_contract(&mock);
    route_ok(
        &mut contract,
        &mock,
        selector("mint(address,uint256)"),
        &encode_mint_calldata(Address::from(ALICE), U256::from(1000u64)),
    );
    assert_eq!(read_balance(&mock, ALICE), U256::from(1000u64));

    // Step 2 — rebuild host with caller = ALICE, migrate storage. Then transfer 300 to BOB.
    let step1_mock = mock;
    let mock = host_with_caller(ALICE);
    seed_owner(&mock, OWNER);
    seed_balance(&mock, ALICE, read_balance(&step1_mock, ALICE));
    let mut contract = make_contract(&mock);
    route_ok(
        &mut contract,
        &mock,
        selector("transfer(address,uint256)"),
        &encode_transfer_calldata(Address::from(BOB), U256::from(300u64)),
    );

    // Step 3 — caller = BOB, transfer 100 to CHARLIE.
    let step2_mock = mock;
    let mock = host_with_caller(BOB);
    seed_owner(&mock, OWNER);
    seed_balance(&mock, ALICE, read_balance(&step2_mock, ALICE));
    seed_balance(&mock, BOB, read_balance(&step2_mock, BOB));
    seed_balance(&mock, CHARLIE, read_balance(&step2_mock, CHARLIE));
    let mut contract = make_contract(&mock);
    route_ok(
        &mut contract,
        &mock,
        selector("transfer(address,uint256)"),
        &encode_transfer_calldata(Address::from(CHARLIE), U256::from(100u64)),
    );

    // Verify final state.
    assert_eq!(read_balance(&mock, ALICE), U256::from(700u64));
    assert_eq!(read_balance(&mock, BOB), U256::from(200u64));
    assert_eq!(read_balance(&mock, CHARLIE), U256::from(100u64));

    // Step 3's MockHost only captured its own Transfer event.
    let events = mock.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].0[1], addr_topic(Address::from(BOB)));
    assert_eq!(events[0].0[2], addr_topic(Address::from(CHARLIE)));
}

#[test]
fn router_trait_path_produces_identical_result_to_free_fn() {
    let mock = host_with_caller(ALICE);
    seed_balance(&mock, ALICE, U256::from(42u64));
    let mut contract = make_contract(&mock);

    let input = encode_balance_of_calldata(Address::from(ALICE));
    let sel = selector("balanceOf(address)");

    // Drive via the free function, take the captured return.
    let outcome = mini_token::route(&mut contract, sel, &input);
    assert_eq!(outcome, Some(()));
    let free = mock
        .take_return_value()
        .expect("free fn called return_value");

    // Drive via the Router trait, take the freshly captured return.
    let outcome = <mini_token::MiniToken as Router<Host>>::route(&mut contract, sel, &input);
    assert_eq!(outcome, Some(()));
    let via_trait = mock
        .take_return_value()
        .expect("trait route called return_value");

    assert_eq!(free, via_trait);
}

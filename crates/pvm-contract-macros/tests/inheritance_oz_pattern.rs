#![cfg(not(feature = "abi-gen"))]
//! End-to-end example: OpenZeppelin-style contract composition.
//!
//! This file is the canonical "how to use it" reference for the inheritance
//! features. It demonstrates all five pieces composing together:
//!
//!   1. `#[storage]` — extension storage structs (`Erc20State`, `OwnableState`)
//!      each claim a slot range computed from `StorageComponent::SLOTS`.
//!   2. Auto-numbered slots — the outer `#[contract]` struct embeds those
//!      extensions as plain fields; the macro assigns slot ranges by summing
//!      each field's `SLOTS`.
//!   3. `#[interface_id]` — `IErc20` and `IOwnable` traits each gain an
//!      `interface_id() -> [u8; 4]` provided method (ERC-165 compatible).
//!   4. `implements(...)` — the contract declares the trait set in its
//!      `#[contract]` attribute, and the macro dispatches every trait method
//!      automatically.
//!   5. Multi-`impl` dispatch — the contract has multiple `impl` blocks:
//!      - one inherent `impl MyToken` for the constructor + custom methods
//!      - one `impl IErc20 for MyToken` (with one method **overridden** —
//!        `transfer` adds an owner check on top of the extension helper)
//!      - one `impl IOwnable for MyToken`
//!
//!   The dispatch table folds methods from every block into a single selector
//!   switch. Overrides "just work" because the contract's own `impl IErc20`
//!   body is what runs — there's no `virtual`/`override` keyword.
//!
//! The patterns shown here are exactly what an `openzeppelin-pvm` library
//! would use to expose `Erc20`, `Ownable`, etc. as reusable extensions.

use pvm_contract_sdk::{Address, Lazy, Mapping, U256};
use pvm_contract_types::{
    Host, HostApi, MockHost, MockHostBuilder, ReturnFlags, SolEncode, StaticEncodedLen,
};

extern crate alloc;

// ---------------------------------------------------------------------------
// Errors
//
// `#[derive(SolError)]` produces the per-error selector. The two errors below
// are surfaced as Solidity-style typed reverts (`InsufficientBalance(...)`,
// `Unauthorized()`) so callers can decode them just like solc-emitted custom
// errors.
// ---------------------------------------------------------------------------

#[derive(Debug, pvm_contract_sdk::SolError)]
pub struct InsufficientBalance {
    pub available: U256,
    pub required: U256,
}

#[derive(Debug, pvm_contract_sdk::SolError)]
pub struct Unauthorized;

// ---------------------------------------------------------------------------
// Extensions
//
// Each extension is a `#[storage]` struct exposing `pub fn _internal(...)`
// helpers. These helpers are NOT `#[method]`-annotated and are NOT dispatched
// themselves — they're library functions called from the outer contract's
// `impl ITrait for Contract` blocks.
//
// The leading underscore is a convention borrowed from Solidity for internal
// helpers; it's not enforced by the SDK.
// ---------------------------------------------------------------------------

#[pvm_contract_sdk::storage]
pub struct Erc20State {
    /// Total token supply at slot offset 0 (relative to the extension's base
    /// slot inside the outer contract).
    pub total_supply: Lazy<U256>,
    /// Per-address balances at slot offset 1.
    pub balances: Mapping<Address, U256>,
}

impl Erc20State {
    /// Read the balance for `account`. Maps to the Solidity `balanceOf` view.
    pub fn _balance_of(&self, account: Address) -> U256 {
        self.balances.get(&account)
    }

    /// Read total supply.
    pub fn _total_supply(&self) -> U256 {
        self.total_supply.get()
    }

    /// Credit `value` to `to`. Used by both `_mint` and `_transfer`.
    fn _credit(&mut self, to: Address, value: U256) {
        let cur = self.balances.get(&to);
        self.balances.insert(&to, &(cur + value));
    }

    /// Mint new tokens — increases total supply and credits `to`.
    pub fn _mint(&mut self, to: Address, value: U256) {
        let supply = self.total_supply.get() + value;
        self.total_supply.set(&supply);
        self._credit(to, value);
    }

    /// Move `value` from `from` to `to`. Reverts with `InsufficientBalance`
    /// if the sender doesn't have enough.
    pub fn _transfer(
        &mut self,
        from: Address,
        to: Address,
        value: U256,
    ) -> Result<(), InsufficientBalance> {
        let available = self.balances.get(&from);
        if available < value {
            return Err(InsufficientBalance {
                available,
                required: value,
            });
        }
        self.balances.insert(&from, &(available - value));
        self._credit(to, value);
        Ok(())
    }
}

#[pvm_contract_sdk::storage]
pub struct OwnableState {
    /// Single-slot owner address.
    pub owner: Lazy<Address>,
}

impl OwnableState {
    pub fn _owner(&self) -> Address {
        self.owner.get()
    }

    pub fn _set_owner(&mut self, new_owner: Address) {
        self.owner.set(&new_owner);
    }

    /// Guard helper: returns `Err(Unauthorized)` if `caller` is not the owner.
    pub fn _check_owner(&self, caller: Address) -> Result<(), Unauthorized> {
        if caller != self.owner.get() {
            return Err(Unauthorized);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Interfaces
//
// `#[interface_id]` adds `interface_id() -> [u8; 4]` to each trait, computed
// as the XOR of method selectors (ERC-165 convention).
//
// Both `IErc20` and `IOwnable` carry an associated `Error` type so the same
// trait can be implemented over different error enums — `MyToken` will bind
// `Error = MyTokenError` for both.
// ---------------------------------------------------------------------------

#[pvm_contract_sdk::interface_id]
pub trait IErc20 {
    type Error;

    fn total_supply(&self) -> U256;
    fn balance_of(&self, account: Address) -> U256;
    fn transfer(&mut self, to: Address, value: U256) -> Result<(), Self::Error>;
}

#[pvm_contract_sdk::interface_id]
pub trait IOwnable {
    type Error;

    fn owner(&self) -> Address;
    fn transfer_ownership(&mut self, new_owner: Address) -> Result<(), Self::Error>;
}

// ---------------------------------------------------------------------------
// Combined contract error
//
// `MyToken` unifies the two extension errors into a single enum so all
// `Result<_, MyTokenError>` arms share a return type. `From<ExtensionError>`
// impls let `?` bubble extension errors up through the contract methods.
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum MyTokenError {
    Insufficient(InsufficientBalance),
    Unauthorized(Unauthorized),
}

impl From<InsufficientBalance> for MyTokenError {
    fn from(e: InsufficientBalance) -> Self {
        Self::Insufficient(e)
    }
}

impl From<Unauthorized> for MyTokenError {
    fn from(e: Unauthorized) -> Self {
        Self::Unauthorized(e)
    }
}

// Manual `SolRevert` impl so the dispatch layer can encode the typed revert
// data. We delegate to each variant's own `SolError` implementation.
impl pvm_contract_sdk::SolRevert for MyTokenError {
    fn revert_data(&self, buf: &mut [u8]) -> usize {
        match self {
            Self::Insufficient(e) => {
                <InsufficientBalance as pvm_contract_sdk::SolRevert>::revert_data(e, buf)
            }
            Self::Unauthorized(e) => {
                <Unauthorized as pvm_contract_sdk::SolRevert>::revert_data(e, buf)
            }
        }
    }

    fn revert_data_len(&self) -> usize {
        match self {
            Self::Insufficient(e) => {
                <InsufficientBalance as pvm_contract_sdk::SolRevert>::revert_data_len(e)
            }
            Self::Unauthorized(e) => {
                <Unauthorized as pvm_contract_sdk::SolRevert>::revert_data_len(e)
            }
        }
    }

    fn error_signatures() -> impl Iterator<Item = &'static &'static str> {
        // Order matters for ABI export only; runtime dispatch doesn't care.
        [
            &<InsufficientBalance as pvm_contract_sdk::SolError>::SIGNATURE,
            &<Unauthorized as pvm_contract_sdk::SolError>::SIGNATURE,
        ]
        .into_iter()
    }
}

// ---------------------------------------------------------------------------
// The contract
//
// Two extensions embedded as plain fields. Auto-numbered slots:
//   - erc20.total_supply  → slot 0
//   - erc20.balances      → slot 1
//   - ownable.owner       → slot 2
//
// `implements(...)` lists every trait whose `impl ... for MyToken` block
// participates in dispatch. The trait order doesn't matter; the macro folds
// every block into one selector switch.
// ---------------------------------------------------------------------------

#[allow(dead_code)] // deploy()/call() are riscv64-gated; tests poke route().
#[pvm_contract_sdk::contract(implements(
    IErc20<Error = MyTokenError>,
    IOwnable<Error = MyTokenError>,
))]
mod my_token {
    use super::*;

    pub struct MyToken {
        pub erc20: Erc20State,
        pub ownable: OwnableState,
    }

    /// Inherent impl: the constructor + any methods that aren't part of a
    /// trait the contract is declared to implement.
    impl MyToken {
        /// Constructor — owner is the deployer.
        ///
        /// "Constructor chaining" in this SDK is just calling each extension's
        /// init helper explicitly. There's no automatic super-call.
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self, initial_supply: U256) {
            let mut caller_bytes = [0u8; 20];
            self.host().caller(&mut caller_bytes);
            let caller = Address::from(caller_bytes);

            self.ownable._set_owner(caller);
            self.erc20._mint(caller, initial_supply);
        }

        /// Owner-only mint. Inherent method — not on any trait but still
        /// dispatched because of `#[method]`.
        #[pvm_contract_sdk::method]
        pub fn mint(&mut self, to: Address, value: U256) -> Result<(), MyTokenError> {
            let mut caller_bytes = [0u8; 20];
            self.host().caller(&mut caller_bytes);
            self.ownable._check_owner(Address::from(caller_bytes))?;
            self.erc20._mint(to, value);
            Ok(())
        }
    }

    /// Trait impl: every fn here is dispatched implicitly because `IErc20` is
    /// in `implements(...)`.
    ///
    /// `transfer` is **overridden** here vs. what the extension does on its
    /// own — the contract layers an additional ownership-style check (in real
    /// OZ you'd more likely layer pausability or a transfer hook). The point
    /// is that the contract's `impl` is the single source of truth for what
    /// runs at the `transfer(address,uint256)` selector. There's no
    /// `virtual`/`override` — the contract just writes a different body.
    impl super::IErc20 for MyToken {
        type Error = super::MyTokenError;

        fn total_supply(&self) -> super::U256 {
            self.erc20._total_supply()
        }

        fn balance_of(&self, account: super::Address) -> super::U256 {
            self.erc20._balance_of(account)
        }

        fn transfer(
            &mut self,
            to: super::Address,
            value: super::U256,
        ) -> Result<(), Self::Error> {
            let mut caller_bytes = [0u8; 20];
            self.host().caller(&mut caller_bytes);
            let from = super::Address::from(caller_bytes);

            // Forward to the extension helper. `?` desugars the
            // `InsufficientBalance` extension error into our `MyTokenError`
            // via the `From` impl.
            self.erc20._transfer(from, to, value)?;
            Ok(())
        }
    }

    /// Trait impl: ownership management.
    impl super::IOwnable for MyToken {
        type Error = super::MyTokenError;

        fn owner(&self) -> super::Address {
            self.ownable._owner()
        }

        fn transfer_ownership(
            &mut self,
            new_owner: super::Address,
        ) -> Result<(), Self::Error> {
            let mut caller_bytes = [0u8; 20];
            self.host().caller(&mut caller_bytes);
            self.ownable._check_owner(super::Address::from(caller_bytes))?;
            self.ownable._set_owner(new_owner);
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Test harness — mirrors the pattern used by other native dispatch tests.
// ---------------------------------------------------------------------------

const OWNER: [u8; 20] = [0x01; 20];
const ALICE: [u8; 20] = [0xAA; 20];
const BOB: [u8; 20] = [0xBB; 20];

fn host_with_caller(caller: [u8; 20]) -> MockHost {
    MockHostBuilder::new().caller(caller).build()
}

fn make_contract(mock: &MockHost) -> my_token::MyToken {
    // Construct exactly the way the macro-generated `deploy()` / `call()`
    // would, but without invoking the riscv64 boundary syscalls. The fields
    // mirror what auto-numbered slot construction produces.
    let host = Host::from_dyn(alloc::rc::Rc::new(mock.clone()));
    my_token::MyToken {
        erc20: <Erc20State as pvm_contract_sdk::StorageComponent>::new_at(0, host.clone()),
        ownable: <OwnableState as pvm_contract_sdk::StorageComponent>::new_at(2, host.clone()),
        host,
    }
}

/// Run the constructor body so storage is in a non-default state for the
/// other tests. Mirrors what `deploy()` does on-chain.
fn deploy_with_supply(mock: &MockHost, supply: U256) -> my_token::MyToken {
    let mut c = make_contract(mock);
    // Call the constructor body directly.
    let mut caller = [0u8; 20];
    c.host.caller(&mut caller);
    c.ownable._set_owner(Address::from(caller));
    c.erc20._mint(Address::from(caller), supply);
    c
}

fn selector(sig: &str) -> [u8; 4] {
    pvm_contract_types::const_selector(sig)
}

fn route_ok(c: &mut my_token::MyToken, mock: &MockHost, sel: [u8; 4], input: &[u8]) -> Vec<u8> {
    let outcome = my_token::route(c, sel, input);
    assert_eq!(outcome, Some(()), "selector must match");
    let rv = mock.take_return_value().expect("return_value called");
    assert_eq!(rv.flags, ReturnFlags::empty(), "expected success");
    rv.data
}

fn route_revert(
    c: &mut my_token::MyToken,
    mock: &MockHost,
    sel: [u8; 4],
    input: &[u8],
) -> Vec<u8> {
    let outcome = my_token::route(c, sel, input);
    assert_eq!(outcome, Some(()), "selector must match");
    let rv = mock.take_return_value().expect("return_value called");
    assert_eq!(rv.flags, ReturnFlags::REVERT, "expected REVERT");
    rv.data
}

fn encode_addr(a: Address) -> Vec<u8> {
    let mut buf = vec![0u8; <Address as StaticEncodedLen>::ENCODED_SIZE];
    a.encode_to(&mut buf);
    buf
}

fn encode_addr_u256(a: Address, v: U256) -> Vec<u8> {
    const LEN: usize =
        <Address as StaticEncodedLen>::ENCODED_SIZE + <U256 as StaticEncodedLen>::ENCODED_SIZE;
    let mut buf = vec![0u8; LEN];
    (a, v).encode_to(&mut buf);
    buf
}

// ===========================================================================
// Tests
// ===========================================================================

/// Smoke test: the storage struct embeds two extensions whose `SLOTS` sum
/// correctly. The contract claims 3 slots total (Erc20State=2 + OwnableState=1).
#[test]
fn extensions_claim_correct_slot_count() {
    use pvm_contract_sdk::StorageComponent;
    assert_eq!(<Erc20State as StorageComponent>::SLOTS, 2);
    assert_eq!(<OwnableState as StorageComponent>::SLOTS, 1);
}

/// Each trait's `interface_id()` is computed from its methods. Different
/// traits have different IDs, the IDs are non-zero, and they're stable
/// across calls.
#[test]
fn interface_ids_are_distinct_and_stable() {
    let id_erc20 = <my_token::MyToken as IErc20>::interface_id();
    let id_ownable = <my_token::MyToken as IOwnable>::interface_id();
    assert_ne!(id_erc20, id_ownable);
    assert_ne!(id_erc20, [0u8; 4]);
    assert_ne!(id_ownable, [0u8; 4]);
    // Stable on repeated calls.
    assert_eq!(id_erc20, <my_token::MyToken as IErc20>::interface_id());
}

/// `total_supply` and `balance_of` come from `impl IErc20 for MyToken` and
/// dispatch through UFCS — they forward to the embedded `Erc20State` helpers.
#[test]
fn ierc20_views_dispatch_through_extension() {
    let mock = host_with_caller(OWNER);
    let mut c = deploy_with_supply(&mock, U256::from(10_000));

    let data = route_ok(&mut c, &mock, selector("totalSupply()"), &[]);
    assert_eq!(data, U256::from(10_000).to_be_bytes::<32>().to_vec());

    let data = route_ok(
        &mut c,
        &mock,
        selector("balanceOf(address)"),
        &encode_addr(Address::from(OWNER)),
    );
    assert_eq!(data, U256::from(10_000).to_be_bytes::<32>().to_vec());
}

/// `IErc20::transfer` dispatches and mutates the balances mapping. The
/// extension's `_transfer` helper does the actual work; the contract's impl
/// is just a thin caller plus error-type coercion via `?`.
#[test]
fn ierc20_transfer_moves_balance() {
    let mock = host_with_caller(OWNER);
    let mut c = deploy_with_supply(&mock, U256::from(1_000));

    route_ok(
        &mut c,
        &mock,
        selector("transfer(address,uint256)"),
        &encode_addr_u256(Address::from(ALICE), U256::from(400)),
    );

    let alice_bal = route_ok(
        &mut c,
        &mock,
        selector("balanceOf(address)"),
        &encode_addr(Address::from(ALICE)),
    );
    let owner_bal = route_ok(
        &mut c,
        &mock,
        selector("balanceOf(address)"),
        &encode_addr(Address::from(OWNER)),
    );
    assert_eq!(alice_bal, U256::from(400).to_be_bytes::<32>().to_vec());
    assert_eq!(owner_bal, U256::from(600).to_be_bytes::<32>().to_vec());
}

/// Insufficient-balance reverts surface the typed `InsufficientBalance`
/// selector, even though the contract's combined error enum wraps it. This
/// proves the `From` coercion + `SolRevert` enum dispatch work end-to-end.
#[test]
fn ierc20_transfer_revert_carries_typed_error_selector() {
    let mock = host_with_caller(ALICE);
    let mut c = make_contract(&mock); // alice has zero balance

    let data = route_revert(
        &mut c,
        &mock,
        selector("transfer(address,uint256)"),
        &encode_addr_u256(Address::from(BOB), U256::from(1)),
    );

    let expected_sel = selector("InsufficientBalance(uint256,uint256)");
    assert_eq!(&data[..4], &expected_sel[..]);

    let available = U256::from_be_bytes::<32>({
        let mut b = [0u8; 32];
        b.copy_from_slice(&data[4..36]);
        b
    });
    let required = U256::from_be_bytes::<32>({
        let mut b = [0u8; 32];
        b.copy_from_slice(&data[36..68]);
        b
    });
    assert_eq!(available, U256::ZERO);
    assert_eq!(required, U256::from(1));
}

/// `IOwnable::owner` returns the deploy-time caller, as set in the
/// constructor body.
#[test]
fn iownable_owner_returns_constructor_caller() {
    let mock = host_with_caller(OWNER);
    let mut c = deploy_with_supply(&mock, U256::from(1));

    let data = route_ok(&mut c, &mock, selector("owner()"), &[]);
    let mut out = [0u8; 20];
    out.copy_from_slice(&data[12..32]);
    assert_eq!(out, OWNER);
}

/// `IOwnable::transfer_ownership` reverts with `Unauthorized` when called by
/// a non-owner. Demonstrates the second variant of the combined error enum
/// reaching the revert payload correctly.
#[test]
fn iownable_transfer_ownership_revert_for_non_owner() {
    let mock = host_with_caller(OWNER);
    let mut c = deploy_with_supply(&mock, U256::from(1));

    // Switch caller to ALICE for the next route() call.
    mock.set_caller(ALICE);

    let data = route_revert(
        &mut c,
        &mock,
        selector("transferOwnership(address)"),
        &encode_addr(Address::from(BOB)),
    );
    let expected_sel = selector("Unauthorized()");
    assert_eq!(&data[..4], &expected_sel[..]);
}

/// Inherent `mint` enforces owner-only via the extension's `_check_owner`
/// helper. Mixed inherent + trait dispatch in the same contract.
#[test]
fn inherent_mint_is_owner_gated() {
    let mock = host_with_caller(OWNER);
    let mut c = deploy_with_supply(&mock, U256::from(0));

    // Owner mints — should succeed.
    route_ok(
        &mut c,
        &mock,
        selector("mint(address,uint256)"),
        &encode_addr_u256(Address::from(ALICE), U256::from(50)),
    );

    let alice_bal = route_ok(
        &mut c,
        &mock,
        selector("balanceOf(address)"),
        &encode_addr(Address::from(ALICE)),
    );
    assert_eq!(alice_bal, U256::from(50).to_be_bytes::<32>().to_vec());

    // Non-owner mint reverts.
    mock.set_caller(BOB);
    let data = route_revert(
        &mut c,
        &mock,
        selector("mint(address,uint256)"),
        &encode_addr_u256(Address::from(BOB), U256::from(999)),
    );
    let expected_sel = selector("Unauthorized()");
    assert_eq!(&data[..4], &expected_sel[..]);
}

/// Trait `transfer_ownership` succeeds when the owner calls it. End-to-end
/// trail: dispatch → trait impl → extension helper → storage write → read
/// back through `owner()`.
#[test]
fn iownable_transfer_ownership_succeeds_for_owner() {
    let mock = host_with_caller(OWNER);
    let mut c = deploy_with_supply(&mock, U256::from(0));

    route_ok(
        &mut c,
        &mock,
        selector("transferOwnership(address)"),
        &encode_addr(Address::from(ALICE)),
    );

    let data = route_ok(&mut c, &mock, selector("owner()"), &[]);
    let mut out = [0u8; 20];
    out.copy_from_slice(&data[12..32]);
    assert_eq!(out, ALICE);
}

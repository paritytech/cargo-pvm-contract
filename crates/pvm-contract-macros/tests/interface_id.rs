//! `#[interface_id]` — the generated `const INTERFACE_ID` is the ERC-165
//! interface ID (XOR of the trait's method selectors).
//!
//! The constant is a *defaulted* associated const, so reading it requires a
//! concrete implementor; the impl bodies are never called (`unimplemented!()`).

use pvm_contract_sdk::{Address, const_selector};
use ruint::aliases::U256;

// --- ERC-165 itself: a single method, so the ID is just that selector. -------

#[pvm_contract_sdk::interface_id]
pub trait IErc165 {
    #[selector(name = "supportsInterface")]
    fn support_interface(&self, interface_id: [u8; 4]) -> bool;
}

struct Erc165;
impl IErc165 for Erc165 {
    fn support_interface(&self, _interface_id: [u8; 4]) -> bool {
        unimplemented!()
    }
}

#[test]
fn erc165_interface_id() {
    // supportsInterface(bytes4) == 0x01ffc9a7 (the canonical ERC-165 ID).
    assert_eq!(<Erc165 as IErc165>::INTERFACE_ID, [0x01, 0xff, 0xc9, 0xa7]);
}

// --- ERC-20: six methods, names camelCased from snake_case automatically. ----

#[pvm_contract_sdk::interface_id]
pub trait IErc20 {
    fn total_supply(&self) -> U256;
    fn balance_of(&self, account: Address) -> U256;
    fn transfer(&mut self, to: Address, amount: U256) -> bool;
    fn allowance(&self, owner: Address, spender: Address) -> U256;
    fn approve(&mut self, spender: Address, amount: U256) -> bool;
    fn transfer_from(&mut self, from: Address, to: Address, amount: U256) -> bool;
}

struct Erc20;
impl IErc20 for Erc20 {
    fn total_supply(&self) -> U256 {
        unimplemented!()
    }
    fn balance_of(&self, _account: Address) -> U256 {
        unimplemented!()
    }
    fn transfer(&mut self, _to: Address, _amount: U256) -> bool {
        unimplemented!()
    }
    fn allowance(&self, _owner: Address, _spender: Address) -> U256 {
        unimplemented!()
    }
    fn approve(&mut self, _spender: Address, _amount: U256) -> bool {
        unimplemented!()
    }
    fn transfer_from(&mut self, _from: Address, _to: Address, _amount: U256) -> bool {
        unimplemented!()
    }
}

#[test]
fn erc20_interface_id() {
    // 0x36372b07 is the canonical ERC-20 interface ID (EIP-165 / OpenZeppelin).
    assert_eq!(<Erc20 as IErc20>::INTERFACE_ID, [0x36, 0x37, 0x2b, 0x07]);
}

// --- Order independence: XOR is commutative. ---------------------------------

#[pvm_contract_sdk::interface_id]
pub trait IErc20Reordered {
    fn transfer_from(&mut self, from: Address, to: Address, amount: U256) -> bool;
    fn approve(&mut self, spender: Address, amount: U256) -> bool;
    fn total_supply(&self) -> U256;
    fn transfer(&mut self, to: Address, amount: U256) -> bool;
    fn allowance(&self, owner: Address, spender: Address) -> U256;
    fn balance_of(&self, account: Address) -> U256;
}

struct Erc20Reordered;
impl IErc20Reordered for Erc20Reordered {
    fn transfer_from(&mut self, _from: Address, _to: Address, _amount: U256) -> bool {
        unimplemented!()
    }
    fn approve(&mut self, _spender: Address, _amount: U256) -> bool {
        unimplemented!()
    }
    fn total_supply(&self) -> U256 {
        unimplemented!()
    }
    fn transfer(&mut self, _to: Address, _amount: U256) -> bool {
        unimplemented!()
    }
    fn allowance(&self, _owner: Address, _spender: Address) -> U256 {
        unimplemented!()
    }
    fn balance_of(&self, _account: Address) -> U256 {
        unimplemented!()
    }
}

#[test]
fn interface_id_is_order_independent() {
    assert_eq!(
        <Erc20Reordered as IErc20Reordered>::INTERFACE_ID,
        <Erc20 as IErc20>::INTERFACE_ID,
    );
}

// --- Custom + dynamic parameter types resolve their SOL_NAME at const-eval. ---

#[derive(pvm_contract_sdk::SolType)]
pub struct Point {
    pub x: u64,
    pub y: u64,
}

#[pvm_contract_sdk::interface_id]
pub trait IMixedParams {
    fn store(&mut self, p: Point) -> bool;
    fn note(&mut self, memo: String);
}

struct MixedParams;
impl IMixedParams for MixedParams {
    fn store(&mut self, _p: Point) -> bool {
        unimplemented!()
    }
    fn note(&mut self, _memo: String) {
        unimplemented!()
    }
}

#[test]
fn custom_and_dynamic_params_match_canonical_signatures() {
    let store = const_selector("store((uint64,uint64))");
    let note = const_selector("note(string)");
    let expected = [
        store[0] ^ note[0],
        store[1] ^ note[1],
        store[2] ^ note[2],
        store[3] ^ note[3],
    ];
    assert_eq!(<MixedParams as IMixedParams>::INTERFACE_ID, expected);
}

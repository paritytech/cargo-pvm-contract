#![cfg(not(feature = "abi-gen"))]
//! Runtime tests that exercise the `#[interface_id]` macro against known
//! ERC-165 interface IDs published in the Ethereum spec, plus our own
//! per-method selectors so the XOR reduction is verified independently of
//! any one trait.

use pvm_contract_sdk::{Address, U256, const_selector};

/// IERC-165 itself: `supportsInterface(bytes4)` → 0x01ffc9a7.
#[pvm_contract_sdk::interface_id]
pub trait IErc165 {
    fn supports_interface(&self, interface_id: [u8; 4]) -> bool;
}

#[test]
fn ierc165_interface_id_matches_spec() {
    // Implementing the trait on a unit type so we can call interface_id().
    struct Probe;
    impl IErc165 for Probe {
        fn supports_interface(&self, _id: [u8; 4]) -> bool {
            false
        }
    }
    let id = <Probe as IErc165>::interface_id();
    // ERC-165 interface ID for `IERC165` (one method: supportsInterface(bytes4))
    // keccak256("supportsInterface(bytes4)")[..4] = 0x01ffc9a7
    assert_eq!(id, [0x01, 0xff, 0xc9, 0xa7], "got 0x{}", hex(id));
}

/// IERC-20 standard: XOR of six method selectors.
/// Spec value: 0x36372b07 (per OpenZeppelin Contracts and EIP-165 references).
#[pvm_contract_sdk::interface_id]
pub trait IErc20 {
    fn total_supply(&self) -> U256;
    fn balance_of(&self, account: Address) -> U256;
    fn transfer(&mut self, to: Address, value: U256) -> bool;
    fn allowance(&self, owner: Address, spender: Address) -> U256;
    fn approve(&mut self, spender: Address, value: U256) -> bool;
    fn transfer_from(&mut self, from: Address, to: Address, value: U256) -> bool;
}

#[test]
fn ierc20_interface_id_matches_spec() {
    struct Probe;
    impl IErc20 for Probe {
        fn total_supply(&self) -> U256 {
            U256::ZERO
        }
        fn balance_of(&self, _: Address) -> U256 {
            U256::ZERO
        }
        fn transfer(&mut self, _: Address, _: U256) -> bool {
            false
        }
        fn allowance(&self, _: Address, _: Address) -> U256 {
            U256::ZERO
        }
        fn approve(&mut self, _: Address, _: U256) -> bool {
            false
        }
        fn transfer_from(&mut self, _: Address, _: Address, _: U256) -> bool {
            false
        }
    }
    let id = <Probe as IErc20>::interface_id();
    // ERC-20 interface ID: 0x36372b07
    // Cross-check by independently XOR'ing the six selectors:
    let totalsupply = const_selector("totalSupply()");
    let balanceof = const_selector("balanceOf(address)");
    let transfer = const_selector("transfer(address,uint256)");
    let allowance = const_selector("allowance(address,address)");
    let approve = const_selector("approve(address,uint256)");
    let transferfrom = const_selector("transferFrom(address,address,uint256)");
    let mut expected = [0u8; 4];
    for sel in [totalsupply, balanceof, transfer, allowance, approve, transferfrom] {
        for j in 0..4 {
            expected[j] ^= sel[j];
        }
    }
    assert_eq!(
        id,
        expected,
        "macro-generated 0x{} should match independently-computed 0x{}",
        hex(id),
        hex(expected),
    );
    assert_eq!(
        id,
        [0x36, 0x37, 0x2b, 0x07],
        "ERC-20 interface ID should be 0x36372b07 per spec; got 0x{}",
        hex(id),
    );
}

/// `#[selector(name = "...")]` overrides the camelCase default. Verify the
/// selector reflects the override, not the Rust ident.
#[pvm_contract_sdk::interface_id]
pub trait Renamed {
    #[selector(name = "myCustomFn")]
    fn snake_default(&self) -> bool;
}

#[test]
fn selector_rename_changes_signature_used_for_id() {
    struct Probe;
    impl Renamed for Probe {
        fn snake_default(&self) -> bool {
            false
        }
    }
    let id = <Probe as Renamed>::interface_id();
    let expected = const_selector("myCustomFn()");
    assert_eq!(id, expected, "rename should drive the selector");
    // Sanity: differs from what we'd get from the Rust ident.
    assert_ne!(id, const_selector("snakeDefault()"));
}

/// Hex helper for assertion messages.
fn hex(bytes: [u8; 4]) -> alloc::string::String {
    extern crate alloc;
    use alloc::string::String;
    let mut out = String::with_capacity(8);
    for b in bytes {
        out.push(NIBBLE[(b >> 4) as usize] as char);
        out.push(NIBBLE[(b & 0x0f) as usize] as char);
    }
    out
}

extern crate alloc;
const NIBBLE: &[u8; 16] = b"0123456789abcdef";

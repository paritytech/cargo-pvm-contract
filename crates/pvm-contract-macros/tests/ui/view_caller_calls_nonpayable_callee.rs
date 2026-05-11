// A `view` (`&self`) method must not be able to invoke a `nonpayable`
// callee through the typed cross-contract API. The borrow check rejects
// passing `&self` where `&mut impl ContractContext` is required.

extern crate alloc;

pvm_contract_sdk::abi_import! {
    #![abi_import(alloc = true)]
    // SPDX-License-Identifier: MIT
    pragma solidity ^0.8.0;

    interface CrossContract {
        function getValue() external view returns (uint256);
        function setValue(uint256 v) external;
    }
}

#[pvm_contract_sdk::contract]
mod caller {
    use pvm_contract_sdk::*;
    use super::*;
    use cross_contract::CrossContract;

    pub struct Caller;

    impl Caller {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), EmptyError> { Ok(()) }

        // The misuse: `&self` (view) caller tries to call the
        // `nonpayable` callee `setValue`. The `set_value` builder method
        // requires `&mut impl ContractContext`; we only have `&Self`.
        #[pvm_contract_sdk::method]
        pub fn cheat(&self, addr: Address) -> Result<(), CallError> {
            let cb = CrossContract::from_address(addr).set_value(U256::ZERO);
            cb.call(self)?;
            Ok(())
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), EmptyError> { Ok(()) }
    }
}

fn main() {}

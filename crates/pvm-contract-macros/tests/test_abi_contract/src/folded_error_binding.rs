#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

extern crate alloc;

use pvm_contract_sdk::U256;

// A folded interface method returning `Result<_, Self::Error>` whose concrete
// error type comes from the `implements(IVault<Error = VaultError>)` binding.
// Regression guard: the binding-vs-`type Error` const check must not reference
// the trait impl in the abi-gen build (where user impls aren't in scope), or ABI
// generation for any such contract fails to compile.
pub trait IVault {
    type Error;
    fn withdraw(&mut self, amount: U256) -> Result<U256, Self::Error>;
}

#[pvm_contract_sdk::contract(implements(IVault<Error = VaultError>))]
mod vault {
    use super::{IVault, U256};

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub struct VaultError {
        pub requested: U256,
    }

    pub struct Vault;

    impl Vault {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) {}
    }

    impl IVault for Vault {
        type Error = VaultError;
        fn withdraw(&mut self, amount: U256) -> Result<U256, Self::Error> {
            if amount == U256::ZERO {
                return Err(VaultError { requested: amount });
            }
            Ok(amount)
        }
    }
}

#![no_main]
#![no_std]

use pvm_contract_sdk::U256;

#[pvm_contract_sdk::contract("MyToken.sol", buffer = 256)]
mod my_token {
    use super::*;
    use pvm_contract_sdk::{Address, HostApi, Lazy, Mapping};

    #[derive(pvm_contract_sdk::SolStorage)]
    struct TokenStorage {
        #[slot(0)]
        total_supply: Lazy<U256>,
        #[slot(1)]
        balances: Mapping<Address, U256>,
    }

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub struct InsufficientBalance;

    pvm_contract_sdk::sol_revert_enum! {
        pub enum TokenError {
            InsufficientBalance(InsufficientBalance),
        }
    }

    pub struct MyToken;

    impl MyToken {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), TokenError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn total_supply(&self) -> U256 {
            storage.total_supply.get()
        }

        #[pvm_contract_sdk::method]
        pub fn balance_of(&self, account: Address) -> U256 {
            storage.balances.get(&account)
        }

        #[pvm_contract_sdk::method]
        pub fn transfer(&mut self, to: Address, amount: U256) -> Result<(), TokenError> {
            let caller = self.caller();

            let mut sender_cell = storage.balances.entry(&caller);
            let sender_balance = sender_cell.get();
            if sender_balance < amount {
                return Err(InsufficientBalance.into());
            }
            sender_cell.set(&(sender_balance - amount));

            let mut recipient_cell = storage.balances.entry(&to);
            let recipient_balance = recipient_cell.get();
            recipient_cell.set(&(recipient_balance + amount));

            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn mint(&mut self, to: Address, amount: U256) -> Result<(), TokenError> {
            let mut recipient_cell = storage.balances.entry(&to);
            let new_balance = recipient_cell.get().saturating_add(amount);
            recipient_cell.set(&new_balance);

            let new_supply = storage.total_supply.get().saturating_add(amount);
            storage.total_supply.set(&new_supply);
            Ok(())
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), TokenError> {
            Ok(())
        }

        fn caller(&self) -> Address {
            let mut caller = [0u8; 20];
            self.host().caller(&mut caller);
            Address(caller)
        }
    }
}

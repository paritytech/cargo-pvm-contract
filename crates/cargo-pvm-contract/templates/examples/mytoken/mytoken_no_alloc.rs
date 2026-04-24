#![no_main]
#![no_std]

use pvm_contract_sdk::U256;

#[pvm_contract_sdk::contract("MyToken.sol", buffer = 256)]
mod my_token {
    use super::*;
    use pvm_contract_sdk::{Address, StorageFlags};

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub struct InsufficientBalance;

    pub struct MyToken;

    impl MyToken {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn total_supply(&self) -> U256 {
            let key = total_supply_key();
            let mut supply_bytes = [0u8; 32];
            let mut supply_slice = &mut supply_bytes[..];

            match self.host().get_storage(StorageFlags::empty(), &key, &mut supply_slice) {
                Ok(_) => U256::from_be_bytes::<32>(supply_bytes),
                Err(_) => U256::ZERO,
            }
        }

        #[pvm_contract_sdk::method]
        pub fn balance_of(&self, account: Address) -> U256 {
            let account: [u8; 20] = account.into();
            let key = self.balance_key(&account);
            let mut balance_bytes = [0u8; 32];
            let mut balance_slice = &mut balance_bytes[..];

            match self.host().get_storage(StorageFlags::empty(), &key, &mut balance_slice) {
                Ok(_) => U256::from_be_bytes::<32>(balance_bytes),
                Err(_) => U256::ZERO,
            }
        }

        #[pvm_contract_sdk::method]
        pub fn transfer(&mut self, to: Address, amount: U256) -> Result<(), InsufficientBalance> {
            let caller = self.get_caller();
            let sender_balance = self.balance_of(caller.into());

            if sender_balance < amount {
                return Err(InsufficientBalance);
            }

            let new_sender_balance = sender_balance - amount;
            let recipient_balance = self.balance_of(to);
            let new_recipient_balance = recipient_balance + amount;

            let to: [u8; 20] = to.into();
            self.set_balance(&caller, new_sender_balance);
            self.set_balance(&to, new_recipient_balance);

            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn mint(
            &mut self,
            to: Address,
            amount: U256,
        ) -> Result<(), pvm_contract_sdk::EmptyError> {
            let new_recipient_balance = self.balance_of(to).saturating_add(amount);
            let to: [u8; 20] = to.into();
            self.set_balance(&to, new_recipient_balance);
            Ok(())
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        fn balance_key(&self, addr: &[u8; 20]) -> [u8; 32] {
            let mut input = [0u8; 64];
            input[12..32].copy_from_slice(addr);
            input[63] = 1;
            let mut key = [0u8; 32];
            self.host().hash_keccak_256(&input, &mut key);
            key
        }

        fn set_balance(&self, addr: &[u8; 20], amount: U256) {
            let key = self.balance_key(addr);
            self.host()
                .set_storage(StorageFlags::empty(), &key, &amount.to_be_bytes::<32>());
        }

        fn get_caller(&self) -> [u8; 20] {
            let mut caller = [0u8; 20];
            self.host().caller(&mut caller);
            caller
        }
    }

    fn total_supply_key() -> [u8; 32] {
        [0u8; 32]
    }
}

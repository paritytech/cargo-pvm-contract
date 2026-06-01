#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::U256;

#[pvm_contract_sdk::contract("MyToken.sol", allocator = "pico")]
mod my_token {
    use super::*;
    use alloc::vec;
    use pvm_contract_sdk::{Address, StorageFlags};

    #[derive(pvm_contract_sdk::SolEvent)]
    pub struct Transfer {
        #[indexed]
        pub from: Address,
        #[indexed]
        pub to: Address,
        pub value: U256,
    }

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub struct InsufficientBalance;

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub enum TokenError {
        InsufficientBalance(InsufficientBalance),
        SolDefaultError(pvm_contract_sdk::SolDefaultError),
    }

    pub struct MyToken;

    impl MyToken {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), TokenError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn total_supply(&self) -> U256 {
            let key = total_supply_key();
            let mut supply_bytes = vec![0u8; 32];
            let mut supply_output = supply_bytes.as_mut_slice();

            match self
                .host
                .get_storage(StorageFlags::empty(), &key, &mut supply_output)
            {
                Ok(_) => U256::from_be_bytes::<32>(supply_output[0..32].try_into().unwrap()),
                Err(_) => U256::ZERO,
            }
        }

        #[pvm_contract_sdk::method]
        pub fn balance_of(&self, account: Address) -> U256 {
            let account: [u8; 20] = account.into();
            let key = self.balance_key(&account);
            let mut balance_bytes = vec![0u8; 32];
            let mut balance_output = balance_bytes.as_mut_slice();

            match self
                .host
                .get_storage(StorageFlags::empty(), &key, &mut balance_output)
            {
                Ok(_) => U256::from_be_bytes::<32>(balance_output[0..32].try_into().unwrap()),
                Err(_) => U256::ZERO,
            }
        }

        #[pvm_contract_sdk::method]
        pub fn transfer(&mut self, to: Address, amount: U256) -> Result<(), TokenError> {
            let caller = self.get_caller();
            let sender_balance = self.balance_of(caller.into());

            if sender_balance < amount {
                return Err(InsufficientBalance.into());
            }

            let new_sender_balance = sender_balance - amount;
            let recipient_balance = self.balance_of(to);
            let new_recipient_balance = recipient_balance + amount;

            let to: [u8; 20] = to.into();
            self.set_balance(&caller, new_sender_balance);
            self.set_balance(&to, new_recipient_balance);
            self.emit_transfer(&caller, &to, amount);

            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn mint(&mut self, to: Address, amount: U256) -> Result<(), TokenError> {
            let new_recipient_balance = self.balance_of(to).saturating_add(amount);

            let to: [u8; 20] = to.into();
            self.set_balance(&to, new_recipient_balance);

            let new_supply = self.total_supply().saturating_add(amount);
            self.set_total_supply(new_supply);

            self.emit_transfer(&[0u8; 20], &to, amount);
            Ok(())
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), TokenError> {
            Ok(())
        }

        fn emit_transfer(&self, from: &[u8; 20], to: &[u8; 20], value: U256) {
            Transfer { from: Address(*from), to: Address(*to), value }.emit(self.host());
        }

        fn balance_key(&self, addr: &[u8; 20]) -> [u8; 32] {
            let mut input = [0u8; 64];
            input[12..32].copy_from_slice(addr);
            input[63] = 1;

            let mut key = [0u8; 32];
            self.host().hash_keccak_256(&input, &mut key);
            key
        }

        fn set_total_supply(&self, amount: U256) {
            let key = total_supply_key();
            self.host()
                .set_storage(StorageFlags::empty(), &key, &amount.to_be_bytes::<32>());
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

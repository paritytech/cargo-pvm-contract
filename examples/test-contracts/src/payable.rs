#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::{StorageFlags, U256};

#[pvm_contract_sdk::contract("Payable.sol", allocator = "pico")]
mod payable {
    use super::*;
    use pvm_contract_sdk::Address;

    pub struct Payable;

    impl Payable {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        #[pvm_contract_sdk::payable]
        pub fn deposit(&mut self) {
            let caller = self.get_caller();
            self.credit(&caller, self.msg_value());
        }

        #[pvm_contract_sdk::method]
        #[pvm_contract_sdk::payable]
        pub fn deposit_to(&mut self, to: Address) {
            let to: [u8; 20] = to.into();
            let amount = self.msg_value();
            self.credit(&to, amount);
        }

        #[pvm_contract_sdk::method]
        pub fn transfer(&mut self, to: Address, amount: U256) -> bool {
            let caller = self.get_caller();
            let from_balance = self.balance(&caller);
            if from_balance < amount {
                return false;
            }
            let to: [u8; 20] = to.into();
            self.set_balance(&caller, from_balance - amount);
            self.credit(&to, amount);
            true
        }

        #[pvm_contract_sdk::method]
        pub fn balance_of(&self, who: Address) -> U256 {
            let who: [u8; 20] = who.into();
            self.balance(&who)
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        fn msg_value(&self) -> U256 {
            let mut buf = [0u8; 32];
            self.host().value_transferred(&mut buf);
            U256::from_le_bytes(buf)
        }

        fn get_caller(&self) -> [u8; 20] {
            let mut caller = [0u8; 20];
            self.host().caller(&mut caller);
            caller
        }

        fn balance_key(&self, addr: &[u8; 20]) -> [u8; 32] {
            let mut input = [0u8; 64];
            input[12..32].copy_from_slice(addr);
            input[63] = 1;

            let mut key = [0u8; 32];
            self.host().hash_keccak_256(&input, &mut key);
            key
        }

        fn balance(&self, addr: &[u8; 20]) -> U256 {
            let key = self.balance_key(addr);
            let mut buf = [0u8; 32];
            let mut out = &mut buf[..];
            match self.host().get_storage(StorageFlags::empty(), &key, &mut out) {
                Ok(_) => U256::from_be_bytes::<32>(buf),
                Err(_) => U256::ZERO,
            }
        }

        fn set_balance(&self, addr: &[u8; 20], amount: U256) {
            let key = self.balance_key(addr);
            self.host()
                .set_storage(StorageFlags::empty(), &key, &amount.to_be_bytes::<32>());
        }

        fn credit(&self, addr: &[u8; 20], amount: U256) {
            let current = self.balance(addr);
            self.set_balance(addr, current.saturating_add(amount));
        }
    }
}

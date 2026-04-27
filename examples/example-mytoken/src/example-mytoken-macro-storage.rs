#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::U256;

#[cfg(not(feature = "abi-gen"))]
#[global_allocator]
static mut ALLOC: picoalloc::Mutex<picoalloc::Allocator<picoalloc::ArrayPointer<1024>>> = {
    static mut ARRAY: picoalloc::Array<1024> = picoalloc::Array([0u8; 1024]);

    picoalloc::Mutex::new(picoalloc::Allocator::new(unsafe {
        picoalloc::ArrayPointer::new(&raw mut ARRAY)
    }))
};

#[pvm_contract_sdk::contract("MyToken.sol", buffer = 256)]
mod my_token {
    use super::*;
    use pvm_contract_sdk::{Address, HostApi, Lazy, Mapping};

    #[derive(pvm_contract_sdk::SolStorage)]
    struct Storage {
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

            let caller_bytes: [u8; 20] = caller.into();
            let to_bytes: [u8; 20] = to.into();
            self.emit_transfer(&caller_bytes, &to_bytes, amount);

            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn mint(&mut self, to: Address, amount: U256) -> Result<(), TokenError> {
            let mut recipient_cell = storage.balances.entry(&to);
            let new_balance = recipient_cell.get().saturating_add(amount);
            recipient_cell.set(&new_balance);

            let new_supply = storage.total_supply.get().saturating_add(amount);
            storage.total_supply.set(&new_supply);

            let zero_address = [0u8; 20];
            let to_bytes: [u8; 20] = to.into();
            self.emit_transfer(&zero_address, &to_bytes, amount);
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

        fn emit_transfer(&self, from: &[u8; 20], to: &[u8; 20], value: U256) {
            const TRANSFER_EVENT_SIGNATURE: [u8; 32] = [
                0xdd, 0xf2, 0x52, 0xad, 0x1b, 0xe2, 0xc8, 0x9b, 0x69, 0xc2, 0xb0, 0x68, 0xfc, 0x37,
                0x8d, 0xaa, 0x95, 0x2b, 0xa7, 0xf1, 0x63, 0xc4, 0xa1, 0x16, 0x28, 0xf5, 0x5a, 0x4d,
                0xf5, 0x23, 0xb3, 0xef,
            ];

            let mut from_topic = [0u8; 32];
            from_topic[12..32].copy_from_slice(from);

            let mut to_topic = [0u8; 32];
            to_topic[12..32].copy_from_slice(to);

            let topics = [TRANSFER_EVENT_SIGNATURE, from_topic, to_topic];
            let data = value.to_be_bytes::<32>();
            self.host().deposit_event(&topics, &data);
        }
    }
}

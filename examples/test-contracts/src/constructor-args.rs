#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::U256;

#[pvm_contract_sdk::contract("ConstructorArgs.sol", allocator = "pico")]
mod constructor_args {
    use super::*;
    use pvm_contract_sdk::{Address, StorageFlags};

    const OWNER_KEY: [u8; 32] = key(0);
    const SUPPLY_KEY: [u8; 32] = key(1);

    const fn key(slot: u8) -> [u8; 32] {
        let mut k = [0u8; 32];
        k[31] = slot;
        k
    }

    pub struct ConstructorArgs;

    impl ConstructorArgs {
        #[pvm_contract_sdk::constructor]
        pub fn new(
            &mut self,
            owner: Address,
            initial_supply: U256,
        ) -> Result<(), pvm_contract_sdk::EmptyError> {
            let addr: [u8; 20] = owner.into();
            let mut buf = [0u8; 32];
            buf[12..32].copy_from_slice(&addr);
            self.host()
                .set_storage(StorageFlags::empty(), &OWNER_KEY, &buf);
            self.host().set_storage(
                StorageFlags::empty(),
                &SUPPLY_KEY,
                &initial_supply.to_be_bytes::<32>(),
            );
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn get_owner(&self) -> Address {
            let slot = self.read_slot(&OWNER_KEY);
            let mut addr = [0u8; 20];
            addr.copy_from_slice(&slot[12..32]);
            addr.into()
        }

        #[pvm_contract_sdk::method]
        pub fn get_initial_supply(&self) -> U256 {
            U256::from_be_bytes::<32>(self.read_slot(&SUPPLY_KEY))
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        fn read_slot(&self, key: &[u8; 32]) -> [u8; 32] {
            let mut buf = [0u8; 32];
            let mut out = &mut buf[..];
            match self
                .host()
                .get_storage(StorageFlags::empty(), key, &mut out)
            {
                Ok(_) => buf,
                Err(_) => [0u8; 32],
            }
        }
    }
}

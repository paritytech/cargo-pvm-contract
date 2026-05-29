#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::{StorageFlags, U256};

#[pvm_contract_sdk::contract("Receive.sol", allocator = "pico")]
mod receive_contract {
    use super::*;

    const TOTAL_KEY: [u8; 32] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0,
    ];
    const COUNT_KEY: [u8; 32] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 1,
    ];

    pub struct ReceiveContract;

    impl ReceiveContract {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        #[pvm_contract_sdk::receive]
        pub fn receive(&mut self) {
            let mut buf = [0u8; 32];
            self.host().value_transferred(&mut buf);
            let value = U256::from_le_bytes(buf);

            let total = self.read_u256(&TOTAL_KEY);
            self.write_u256(&TOTAL_KEY, total.saturating_add(value));

            let count = self.read_u256(&COUNT_KEY);
            self.write_u256(&COUNT_KEY, count.saturating_add(U256::from(1u8)));
        }

        #[pvm_contract_sdk::method]
        pub fn total_received(&self) -> U256 {
            self.read_u256(&TOTAL_KEY)
        }

        #[pvm_contract_sdk::method]
        pub fn receive_count(&self) -> U256 {
            self.read_u256(&COUNT_KEY)
        }

        fn read_u256(&self, key: &[u8; 32]) -> U256 {
            let mut buf = [0u8; 32];
            let mut out = &mut buf[..];
            match self.host().get_storage(StorageFlags::empty(), key, &mut out) {
                Ok(_) => U256::from_be_bytes::<32>(buf),
                Err(_) => U256::ZERO,
            }
        }

        fn write_u256(&self, key: &[u8; 32], value: U256) {
            self.host()
                .set_storage(StorageFlags::empty(), key, &value.to_be_bytes::<32>());
        }
    }
}

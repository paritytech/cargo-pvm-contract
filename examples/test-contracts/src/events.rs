#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::U256;

#[pvm_contract_sdk::contract("Events.sol", allocator = "pico")]
mod events {
    use super::*;
    use pvm_contract_sdk::{StorageFlags};

    const VALUE_KEY: [u8; 32] = [0u8; 32];

    // keccak256("ValueChanged(address,uint256,uint256)")
    const VALUE_CHANGED_SIG: [u8; 32] = [
        0x68, 0x27, 0x0d, 0x6a, 0x12, 0x84, 0x00, 0x2b, 0x2e, 0x5e, 0x73, 0x08, 0x39, 0x58, 0x41,
        0xf1, 0x54, 0xfe, 0x1d, 0xca, 0xa3, 0x2a, 0x17, 0x08, 0x0a, 0x7c, 0x67, 0x9d, 0x7c, 0xf8,
        0x95, 0x52,
    ];

    pub struct Events;

    impl Events {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn set_value(&mut self, val: U256) {
            let old = self.get_value();

            self.host()
                .set_storage(StorageFlags::empty(), &VALUE_KEY, &val.to_be_bytes::<32>());

            let mut caller = [0u8; 20];
            self.host().caller(&mut caller);

            let mut who_topic = [0u8; 32];
            who_topic[12..32].copy_from_slice(&caller);

            let topics = [VALUE_CHANGED_SIG, who_topic];

            let mut data = [0u8; 64];
            data[0..32].copy_from_slice(&old.to_be_bytes::<32>());
            data[32..64].copy_from_slice(&val.to_be_bytes::<32>());

            self.host().deposit_event(&topics, &data);
        }

        #[pvm_contract_sdk::method]
        pub fn get_value(&self) -> U256 {
            let mut buf = [0u8; 32];
            let mut out = &mut buf[..];
            match self.host().get_storage(StorageFlags::empty(), &VALUE_KEY, &mut out) {
                Ok(_) => U256::from_be_bytes::<32>(buf),
                Err(_) => U256::ZERO,
            }
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }
    }
}

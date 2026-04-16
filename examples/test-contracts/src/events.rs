#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::U256;

#[derive(pvm_contract_sdk::SolEvent)]
struct ValueChanged {
    #[indexed]
    who: pvm_contract_sdk::Address,
    old_value: U256,
    new_value: U256,
}

#[pvm_contract_sdk::contract("Events.sol", allocator = "pico")]
mod events {
    use super::*;
    use pvm_contract_sdk::StorageFlags;

    const VALUE_KEY: [u8; 32] = [0u8; 32];

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

            let event = ValueChanged {
                who: pvm_contract_sdk::Address(caller),
                old_value: old,
                new_value: val,
            };
            self.host().deposit_event(&event.topics(), &event.data());
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

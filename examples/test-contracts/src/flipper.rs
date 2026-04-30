#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

#[pvm_contract_sdk::contract("Flipper.sol", allocator = "pico")]
mod flipper {
    use pvm_contract_sdk::{StorageFlags};

    const STORAGE_KEY: [u8; 32] = [0u8; 32];

    pub struct Flipper;

    impl Flipper {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            self.host().set_storage(StorageFlags::empty(), &STORAGE_KEY, &[0u8; 32]);
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn flip(&mut self) {
            let current = self.read_value();
            let new_val = if current { 0u8 } else { 1u8 };
            let mut buf = [0u8; 32];
            buf[31] = new_val;
            self.host().set_storage(StorageFlags::empty(), &STORAGE_KEY, &buf);
        }

        #[pvm_contract_sdk::method]
        pub fn get(&self) -> bool {
            self.read_value()
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        fn read_value(&self) -> bool {
            let mut buf = [0u8; 32];
            let mut out = &mut buf[..];
            match self.host().get_storage(StorageFlags::empty(), &STORAGE_KEY, &mut out) {
                Ok(_) => buf[31] != 0,
                Err(_) => false,
            }
        }
    }
}

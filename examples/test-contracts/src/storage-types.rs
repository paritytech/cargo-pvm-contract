#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::U256;

#[pvm_contract_sdk::contract("StorageTypes.sol", allocator = "pico")]
mod storage_types {
    use super::*;
    use pvm_contract_sdk::{Address, StorageFlags};

    const KEY_U8: [u8; 32] = key(0);
    const KEY_U16: [u8; 32] = key(1);
    const KEY_U32: [u8; 32] = key(2);
    const KEY_U64: [u8; 32] = key(3);
    const KEY_U128: [u8; 32] = key(4);
    const KEY_U256: [u8; 32] = key(5);
    const KEY_BOOL: [u8; 32] = key(6);
    const KEY_ADDRESS: [u8; 32] = key(7);
    const KEY_BYTES32: [u8; 32] = key(8);

    const fn key(slot: u8) -> [u8; 32] {
        let mut k = [0u8; 32];
        k[31] = slot;
        k
    }

    pub struct StorageTypes;

    impl StorageTypes {
        #[pvm_contract_sdk::constructor]
        pub fn new(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        #[pvm_contract_sdk::method]
        pub fn set_u8(&mut self, val: u8) {
            let mut buf = [0u8; 32];
            buf[31] = val;
            self.host().set_storage(StorageFlags::empty(), &KEY_U8, &buf);
        }

        #[pvm_contract_sdk::method]
        pub fn get_u8(&self) -> u8 {
            self.read_slot(&KEY_U8)[31]
        }

        #[pvm_contract_sdk::method]
        pub fn set_u16(&mut self, val: u16) {
            let mut buf = [0u8; 32];
            buf[30..32].copy_from_slice(&val.to_be_bytes());
            self.host().set_storage(StorageFlags::empty(), &KEY_U16, &buf);
        }

        #[pvm_contract_sdk::method]
        pub fn get_u16(&self) -> u16 {
            let slot = self.read_slot(&KEY_U16);
            u16::from_be_bytes([slot[30], slot[31]])
        }

        #[pvm_contract_sdk::method]
        pub fn set_u32(&mut self, val: u32) {
            let mut buf = [0u8; 32];
            buf[28..32].copy_from_slice(&val.to_be_bytes());
            self.host().set_storage(StorageFlags::empty(), &KEY_U32, &buf);
        }

        #[pvm_contract_sdk::method]
        pub fn get_u32(&self) -> u32 {
            let slot = self.read_slot(&KEY_U32);
            u32::from_be_bytes([slot[28], slot[29], slot[30], slot[31]])
        }

        #[pvm_contract_sdk::method]
        pub fn set_u64(&mut self, val: u64) {
            let mut buf = [0u8; 32];
            buf[24..32].copy_from_slice(&val.to_be_bytes());
            self.host().set_storage(StorageFlags::empty(), &KEY_U64, &buf);
        }

        #[pvm_contract_sdk::method]
        pub fn get_u64(&self) -> u64 {
            let slot = self.read_slot(&KEY_U64);
            u64::from_be_bytes(slot[24..32].try_into().unwrap())
        }

        #[pvm_contract_sdk::method]
        pub fn set_u128(&mut self, val: u128) {
            let mut buf = [0u8; 32];
            buf[16..32].copy_from_slice(&val.to_be_bytes());
            self.host().set_storage(StorageFlags::empty(), &KEY_U128, &buf);
        }

        #[pvm_contract_sdk::method]
        pub fn get_u128(&self) -> u128 {
            let slot = self.read_slot(&KEY_U128);
            u128::from_be_bytes(slot[16..32].try_into().unwrap())
        }

        #[pvm_contract_sdk::method]
        pub fn set_u256(&mut self, val: U256) {
            self.host()
                .set_storage(StorageFlags::empty(), &KEY_U256, &val.to_be_bytes::<32>());
        }

        #[pvm_contract_sdk::method]
        pub fn get_u256(&self) -> U256 {
            U256::from_be_bytes::<32>(self.read_slot(&KEY_U256))
        }

        #[pvm_contract_sdk::method]
        pub fn set_bool(&mut self, val: bool) {
            let mut buf = [0u8; 32];
            buf[31] = if val { 1 } else { 0 };
            self.host().set_storage(StorageFlags::empty(), &KEY_BOOL, &buf);
        }

        #[pvm_contract_sdk::method]
        pub fn get_bool(&self) -> bool {
            self.read_slot(&KEY_BOOL)[31] != 0
        }

        #[pvm_contract_sdk::method]
        pub fn set_address(&mut self, val: Address) {
            let addr: [u8; 20] = val.into();
            let mut buf = [0u8; 32];
            buf[12..32].copy_from_slice(&addr);
            self.host().set_storage(StorageFlags::empty(), &KEY_ADDRESS, &buf);
        }

        #[pvm_contract_sdk::method]
        pub fn get_address(&self) -> Address {
            let slot = self.read_slot(&KEY_ADDRESS);
            let mut addr = [0u8; 20];
            addr.copy_from_slice(&slot[12..32]);
            addr.into()
        }

        #[pvm_contract_sdk::method]
        pub fn set_bytes32(&mut self, val: [u8; 32]) {
            self.host().set_storage(StorageFlags::empty(), &KEY_BYTES32, &val);
        }

        #[pvm_contract_sdk::method]
        pub fn get_bytes32(&self) -> [u8; 32] {
            self.read_slot(&KEY_BYTES32)
        }

        #[pvm_contract_sdk::fallback]
        pub fn fallback(&mut self) -> Result<(), pvm_contract_sdk::EmptyError> {
            Ok(())
        }

        fn read_slot(&self, key: &[u8; 32]) -> [u8; 32] {
            let mut buf = [0u8; 32];
            let mut out = &mut buf[..];
            match self.host().get_storage(StorageFlags::empty(), key, &mut out) {
                Ok(_) => buf,
                Err(_) => [0u8; 32],
            }
        }
    }
}

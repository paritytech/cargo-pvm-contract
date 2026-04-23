#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::{PolkaVmHost, StorageFlags};
use pvm_contract_sdk::U256;

#[pvm_contract_sdk::contract("StorageTypes.sol", allocator = "pico")]
mod storage_types {
    use super::*;
    use pvm_contract_sdk::Address;

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

    #[pvm_contract_sdk::constructor]
    pub fn new() -> Result<(), pvm_contract_sdk::EmptyError> {
        Ok(())
    }

    // --- u8 ---
    #[pvm_contract_sdk::method]
    pub fn set_u8(val: u8) {
        let mut buf = [0u8; 32];
        buf[31] = val;
        PolkaVmHost::set_storage(StorageFlags::empty(), &KEY_U8, &buf);
    }

    #[pvm_contract_sdk::method]
    pub fn get_u8() -> u8 {
        read_slot(&KEY_U8)[31]
    }

    // --- u16 ---
    #[pvm_contract_sdk::method]
    pub fn set_u16(val: u16) {
        let mut buf = [0u8; 32];
        buf[30..32].copy_from_slice(&val.to_be_bytes());
        PolkaVmHost::set_storage(StorageFlags::empty(), &KEY_U16, &buf);
    }

    #[pvm_contract_sdk::method]
    pub fn get_u16() -> u16 {
        let slot = read_slot(&KEY_U16);
        u16::from_be_bytes([slot[30], slot[31]])
    }

    // --- u32 ---
    #[pvm_contract_sdk::method]
    pub fn set_u32(val: u32) {
        let mut buf = [0u8; 32];
        buf[28..32].copy_from_slice(&val.to_be_bytes());
        PolkaVmHost::set_storage(StorageFlags::empty(), &KEY_U32, &buf);
    }

    #[pvm_contract_sdk::method]
    pub fn get_u32() -> u32 {
        let slot = read_slot(&KEY_U32);
        u32::from_be_bytes([slot[28], slot[29], slot[30], slot[31]])
    }

    // --- u64 ---
    #[pvm_contract_sdk::method]
    pub fn set_u64(val: u64) {
        let mut buf = [0u8; 32];
        buf[24..32].copy_from_slice(&val.to_be_bytes());
        PolkaVmHost::set_storage(StorageFlags::empty(), &KEY_U64, &buf);
    }

    #[pvm_contract_sdk::method]
    pub fn get_u64() -> u64 {
        let slot = read_slot(&KEY_U64);
        u64::from_be_bytes(slot[24..32].try_into().unwrap())
    }

    // --- u128 ---
    #[pvm_contract_sdk::method]
    pub fn set_u128(val: u128) {
        let mut buf = [0u8; 32];
        buf[16..32].copy_from_slice(&val.to_be_bytes());
        PolkaVmHost::set_storage(StorageFlags::empty(), &KEY_U128, &buf);
    }

    #[pvm_contract_sdk::method]
    pub fn get_u128() -> u128 {
        let slot = read_slot(&KEY_U128);
        u128::from_be_bytes(slot[16..32].try_into().unwrap())
    }

    // --- U256 ---
    #[pvm_contract_sdk::method]
    pub fn set_u256(val: U256) {
        PolkaVmHost::set_storage(StorageFlags::empty(), &KEY_U256, &val.to_be_bytes::<32>());
    }

    #[pvm_contract_sdk::method]
    pub fn get_u256() -> U256 {
        U256::from_be_bytes::<32>(read_slot(&KEY_U256))
    }

    // --- bool ---
    #[pvm_contract_sdk::method]
    pub fn set_bool(val: bool) {
        let mut buf = [0u8; 32];
        buf[31] = if val { 1 } else { 0 };
        PolkaVmHost::set_storage(StorageFlags::empty(), &KEY_BOOL, &buf);
    }

    #[pvm_contract_sdk::method]
    pub fn get_bool() -> bool {
        read_slot(&KEY_BOOL)[31] != 0
    }

    // --- address ---
    #[pvm_contract_sdk::method]
    pub fn set_address(val: Address) {
        let addr: [u8; 20] = val.into();
        let mut buf = [0u8; 32];
        buf[12..32].copy_from_slice(&addr);
        PolkaVmHost::set_storage(StorageFlags::empty(), &KEY_ADDRESS, &buf);
    }

    #[pvm_contract_sdk::method]
    pub fn get_address() -> Address {
        let slot = read_slot(&KEY_ADDRESS);
        let mut addr = [0u8; 20];
        addr.copy_from_slice(&slot[12..32]);
        addr.into()
    }

    // --- bytes32 ---
    #[pvm_contract_sdk::method]
    pub fn set_bytes32(val: [u8; 32]) {
        PolkaVmHost::set_storage(StorageFlags::empty(), &KEY_BYTES32, &val);
    }

    #[pvm_contract_sdk::method]
    pub fn get_bytes32() -> [u8; 32] {
        read_slot(&KEY_BYTES32)
    }

    #[pvm_contract_sdk::fallback]
    pub fn fallback() -> Result<(), pvm_contract_sdk::EmptyError> {
        Ok(())
    }

    fn read_slot(key: &[u8; 32]) -> [u8; 32] {
        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        match PolkaVmHost::get_storage(StorageFlags::empty(), key, &mut out) {
            Ok(_) => buf,
            Err(_) => [0u8; 32],
        }
    }
}

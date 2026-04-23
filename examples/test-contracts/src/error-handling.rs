#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pvm_contract_sdk::{PolkaVmHost, StorageFlags};
use pvm_contract_sdk::U256;

#[pvm_contract_sdk::contract("ErrorHandling.sol", allocator = "pico")]
mod error_handling {
    use super::*;

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub struct AlwaysReverts;

    #[derive(Debug, pvm_contract_sdk::SolError)]
    pub struct ZeroNotAllowed;

    pvm_contract_sdk::sol_revert_enum! {
        pub enum ContractError {
            AlwaysReverts(AlwaysReverts),
            ZeroNotAllowed(ZeroNotAllowed),
        }
    }

    const GUARDED_KEY: [u8; 32] = [0u8; 32];

    #[pvm_contract_sdk::constructor]
    pub fn new() -> Result<(), ContractError> {
        Ok(())
    }

    #[pvm_contract_sdk::method]
    pub fn will_revert() -> Result<(), ContractError> {
        Err(AlwaysReverts.into())
    }

    #[pvm_contract_sdk::method]
    pub fn will_succeed() -> bool {
        true
    }

    #[pvm_contract_sdk::method]
    pub fn set_guarded(val: U256) -> Result<(), ContractError> {
        if val == U256::ZERO {
            return Err(ZeroNotAllowed.into())
        }
        PolkaVmHost::set_storage(StorageFlags::empty(), &GUARDED_KEY, &val.to_be_bytes::<32>());
        Ok(())
    }

    #[pvm_contract_sdk::method]
    pub fn get_guarded() -> U256 {
        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        match PolkaVmHost::get_storage(StorageFlags::empty(), &GUARDED_KEY, &mut out) {
            Ok(_) => U256::from_be_bytes::<32>(buf),
            Err(_) => U256::ZERO,
        }
    }

    #[pvm_contract_sdk::fallback]
    pub fn fallback() -> Result<(), ContractError> {
        Ok(())
    }
}

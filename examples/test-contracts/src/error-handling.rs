#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use ruint::aliases::U256;

#[pvm_contract_macros::contract("ErrorHandling.sol", allocator = "pico")]
mod error_handling {
    use super::*;
    use pvm_contract_types::{HostApi, PolkaVmHost, StorageFlags};

    #[derive(Debug, pvm_contract_macros::SolError)]
    pub struct AlwaysReverts;

    #[derive(Debug, pvm_contract_macros::SolError)]
    pub struct ZeroNotAllowed;

    pvm_contract_types::sol_revert_enum! {
        pub enum ContractError {
            AlwaysReverts(AlwaysReverts),
            ZeroNotAllowed(ZeroNotAllowed),
        }
    }

    const GUARDED_KEY: [u8; 32] = [0u8; 32];

    pub struct ErrorHandling<H: HostApi = PolkaVmHost> {
        pub host: H,
    }

    impl<H: HostApi> ErrorHandling<H> {
        #[pvm_contract_macros::constructor]
        pub fn new(&mut self) -> Result<(), ContractError> {
            Ok(())
        }

        #[pvm_contract_macros::method]
        pub fn will_revert(&self) -> Result<(), ContractError> {
            Err(AlwaysReverts.into())
        }

        #[pvm_contract_macros::method]
        pub fn will_succeed(&self) -> bool {
            true
        }

        #[pvm_contract_macros::method]
        pub fn set_guarded(&mut self, val: U256) -> Result<(), ContractError> {
            if val == U256::ZERO {
                return Err(ZeroNotAllowed.into());
            }
            self.host
                .set_storage(StorageFlags::empty(), &GUARDED_KEY, &val.to_be_bytes::<32>());
            Ok(())
        }

        #[pvm_contract_macros::method]
        pub fn get_guarded(&self) -> U256 {
            let mut buf = [0u8; 32];
            let mut out = &mut buf[..];
            match self.host.get_storage(StorageFlags::empty(), &GUARDED_KEY, &mut out) {
                Ok(_) => U256::from_be_bytes::<32>(buf),
                Err(_) => U256::ZERO,
            }
        }

        #[pvm_contract_macros::fallback]
        pub fn fallback(&mut self) -> Result<(), ContractError> {
            Ok(())
        }
    }
}

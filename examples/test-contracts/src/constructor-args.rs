#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pallet_revive_uapi::{HostFnImpl as api, StorageFlags};
use ruint::aliases::U256;

#[pvm_contract_macros::contract("ConstructorArgs.sol", allocator = "pico")]
mod constructor_args {
    use super::*;
    use pvm_contract_types::Address;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum Error {
        Unexpected,
    }

    impl AsRef<[u8]> for Error {
        fn as_ref(&self) -> &[u8] {
            match *self {
                Error::Unexpected => b"Unexpected",
            }
        }
    }

    const OWNER_KEY: [u8; 32] = key(0);
    const SUPPLY_KEY: [u8; 32] = key(1);

    const fn key(slot: u8) -> [u8; 32] {
        let mut k = [0u8; 32];
        k[31] = slot;
        k
    }

    #[pvm_contract_macros::constructor]
    pub fn new(owner: Address, initial_supply: U256) -> Result<(), Error> {
        let addr: [u8; 20] = owner.into();
        let mut buf = [0u8; 32];
        buf[12..32].copy_from_slice(&addr);
        api::set_storage(StorageFlags::empty(), &OWNER_KEY, &buf);
        api::set_storage(
            StorageFlags::empty(),
            &SUPPLY_KEY,
            &initial_supply.to_be_bytes::<32>(),
        );
        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn get_owner() -> Address {
        let slot = read_slot(&OWNER_KEY);
        let mut addr = [0u8; 20];
        addr.copy_from_slice(&slot[12..32]);
        addr.into()
    }

    #[pvm_contract_macros::method]
    pub fn get_initial_supply() -> U256 {
        U256::from_be_bytes::<32>(read_slot(&SUPPLY_KEY))
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), Error> {
        Ok(())
    }

    fn read_slot(key: &[u8; 32]) -> [u8; 32] {
        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        match api::get_storage(StorageFlags::empty(), key, &mut out) {
            Ok(_) => buf,
            Err(_) => [0u8; 32],
        }
    }
}

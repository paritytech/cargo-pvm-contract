#![cfg_attr(not(feature = "abi-gen"), no_main, no_std)]

use pallet_revive_uapi::{HostFnImpl as api, StorageFlags};

pvm_contract_macros::abi_import!(
    alloc = true,
    flipper,
    concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/target/flipper.release.abi.json"
    )
);

#[pvm_contract_macros::contract("DelegateFlipper.sol", allocator = "pico")]
mod flipper_delegate {
    use super::*;
    use pvm_contract_core::call::CallError;
    type Error = pvm_contract_types::EmptyError;
    const STORAGE_KEY: [u8; 32] = [0u8; 32];

    #[pvm_contract_macros::constructor]
    pub fn new() -> Result<(), Error> {
        // Initialize to false (0)
        api::set_storage(StorageFlags::empty(), &STORAGE_KEY, &[0u8; 32]);
        Ok(())
    }

    #[pvm_contract_macros::method]
    pub fn delegate_flipper(addr: Address) -> Result<(), CallError> {
        let flip = Flipper::from_address(addr).flip();
        let mut input = [0u8; 512];
        let mut output = [0u8; 512];
        flip.delegate_call_raw(&mut input, &mut output)
    }

    #[pvm_contract_macros::method]
    pub fn get() -> bool {
        read_value()
    }

    #[pvm_contract_macros::fallback]
    pub fn fallback() -> Result<(), Error> {
        Ok(())
    }

    fn read_value() -> bool {
        let mut buf = [0u8; 32];
        let mut out = &mut buf[..];
        match api::get_storage(StorageFlags::empty(), &STORAGE_KEY, &mut out) {
            Ok(_) => buf[31] != 0,
            Err(_) => false,
        }
    }
}
